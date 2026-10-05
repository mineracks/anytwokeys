//! A profile kept the same on every device, through a VSS host and nothing else.
//!
//! WHAT TRAVELS. The profile's own name and picture, its vaults (definition, labels, notes, the lot), its address book, places, entities and
//! what it has learnt about its keys. Not the settings (a node on the home network is not reachable from a phone),
//! not a half-built spend, not the chain cache, not what this device has already been told about. Nothing in a
//! profile can spend: the keys that sign live on the YubiKeys. What this protects is privacy, and it is protected
//! from the host too.
//!
//! THE KEY. Each synced profile has one random 32-byte root, made on the device that switches sync on. From it:
//! a content key, a key that signs the requests to the host, and the name of the store on the host. The host holds
//! ciphertext it cannot open, under names it cannot read.
//!
//! A NEW DEVICE. Every YubiKey this app uses keeps a PIN-protected secret (OpenPGP private object 0103, the one
//! that already locks its seal share and can open the app). From that secret comes a second, tiny store on the
//! host: the key's POINTER, a sealed list of the profile roots that key may fetch. A blank device with one enrolled
//! key and its PIN reads the pointer, takes the root, and pulls the profile. No cloud account, no typing.
//!
//! MERGING. Every device keeps the profile as it stood at its last sync (the base). Sync is a three-way merge of
//! base, this device and the host, field by field: what only one side changed is taken from that side, including a
//! deletion; what both changed to the same thing is that thing; a true clash goes to this device, which makes the
//! last device to sync the winner and never ping-pongs. Lists of things with an identity (vaults, contacts, places,
//! entities, a vault's keys) are merged per item. Address counters only ever move forward, so two devices can
//! never hand out the same receive address twice.
use bitcoin::secp256k1::SecretKey;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use vss_client_ng::client::VssClient;
use vss_client_ng::error::VssError;
use vss_client_ng::headers::sigs_auth::SigsAuthProvider;
use vss_client_ng::types::{DeleteObjectRequest, GetObjectRequest, KeyValue, ListKeyVersionsRequest, PutObjectRequest};
use vss_client_ng::util::retry::{ExponentialBackoffRetryPolicy, RetryPolicy};

const MAGIC: &[u8; 5] = b"A2KP\x01";
/// The host refuses a request over 4 MiB; an object is kept well under it.
const MAX_OBJECT: usize = 3 * 1024 * 1024;
/// The parts of the store that travel. Everything else stays on the device it was made on.
const SYNCED: &[&str] = &["profile", "vaults", "contacts", "places", "key_place", "entities", "icons", "records", "forms", "key_first", "key_fw", "key_ready", "transfer_answers"];
/// Inside a vault: when THIS device last backed its labels up is nobody else's business.
const VAULT_LOCAL: &[&str] = &["labels_synced"];
/// Counters that only move forward: the next unused address, and how far the chain has been searched.
const FORWARD_ONLY: &[&str] = &["next", "next_change", "scan_max", "scan_max_change"];

// ------------------------------------------------------------------ keys --

pub struct Keys { pub content: [u8; 32], pub auth: SecretKey, pub store: String, names: [u8; 32] }

fn keys_from(ikm: &[u8; 32], salt: &[u8]) -> Keys {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut content = [0u8; 32]; hk.expand(b"content", &mut content).unwrap();
    let mut store = [0u8; 16]; hk.expand(b"store", &mut store).unwrap();
    let mut names = [0u8; 32]; hk.expand(b"names", &mut names).unwrap();
    // a 32-byte HKDF output is a valid secp256k1 secret with overwhelming probability; walk a counter for the rest
    let mut i = 0u8;
    let auth = loop {
        let mut a = [0u8; 32]; hk.expand(&[b"auth".as_slice(), &[i]].concat(), &mut a).unwrap();
        if let Ok(k) = SecretKey::from_slice(&a) { break k; }
        i += 1;
    };
    Keys { content, auth, store: hex::encode(store), names }
}
/// A profile's own keys, from its root.
pub fn profile_keys(root: &[u8; 32]) -> Keys { keys_from(root, b"a2k-profile-sync-v1") }
/// A YubiKey's pointer keys, from the secret it gives up after its PIN.
pub fn pointer_keys(card_secret: &[u8; 32]) -> Keys { keys_from(card_secret, b"a2k-sync-pointer-v1") }

impl Keys {
    /// What an object is called on the host: nothing the host can read a vault's identity out of.
    fn name_of(&self, kind: &str, id: &str) -> String {
        use hmac::{Hmac, Mac};
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&self.names).expect("any key length");
        m.update(kind.as_bytes()); m.update(b"|"); m.update(id.as_bytes());
        format!("{kind}/{}", hex::encode(&m.finalize().into_bytes()[..16]))
    }
    fn seal(&self, object: &str, plain: &[u8]) -> Result<Vec<u8>, String> {
        use std::io::Write;
        let mut z = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(plain).map_err(|e| e.to_string())?;
        let packed = z.finish().map_err(|e| e.to_string())?;
        let nonce: [u8; 24] = crate::lockbox::random();
        let aad = [self.store.as_bytes(), b"|", object.as_bytes()].concat();
        let ct = XChaCha20Poly1305::new((&self.content).into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: &packed, aad: &aad }).map_err(|_| "could not encrypt".to_string())?;
        Ok([MAGIC.as_slice(), &nonce, &ct].concat())
    }
    fn open(&self, object: &str, wire: &[u8]) -> Result<Vec<u8>, String> {
        use std::io::Read;
        if wire.len() < MAGIC.len() + 24 + 16 || &wire[..MAGIC.len()] != MAGIC { return Err("what the sync host holds is not in a form this build knows".into()); }
        let (nonce, ct) = wire[MAGIC.len()..].split_at(24);
        let aad = [self.store.as_bytes(), b"|", object.as_bytes()].concat();
        let packed = XChaCha20Poly1305::new((&self.content).into()).decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: &aad }).map_err(|_| "what the sync host holds did not open with this profile's key".to_string())?;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(packed.as_slice()).take(64 * 1024 * 1024).read_to_end(&mut out).map_err(|e| format!("could not unpack: {e}"))?;
        Ok(out)
    }
}

// ------------------------------------------------------------------ host --

fn client(url: &str, keys: &Keys) -> VssClient<impl RetryPolicy<E = VssError>> {
    let headers = Arc::new(SigsAuthProvider::new(keys.auth, HashMap::new()));
    // the default policy has NO attempt cap: a stale put would be retried forever
    let retry = ExponentialBackoffRetryPolicy::new(Duration::from_millis(200)).with_max_attempts(3);
    VssClient::new_with_headers(url.to_string(), retry, headers)
}
async fn list(url: &str, keys: &Keys) -> Result<BTreeMap<String, i64>, String> {
    let c = client(url, keys);
    let (mut out, mut token) = (BTreeMap::new(), None);
    for _ in 0..50 {
        let r = c.list_key_versions(&ListKeyVersionsRequest { store_id: keys.store.clone(), key_prefix: None, page_size: Some(1000), page_token: token.clone() }).await.map_err(|e| format!("sync host: {e}"))?;
        out.extend(r.key_versions.into_iter().map(|k| (k.key, k.version)));
        match r.next_page_token { Some(t) if !t.is_empty() => token = Some(t), _ => break }
    }
    Ok(out)
}
async fn get(url: &str, keys: &Keys, object: &str) -> Result<Option<(Vec<u8>, i64)>, String> {
    match client(url, keys).get_object(&GetObjectRequest { store_id: keys.store.clone(), key: object.into() }).await {
        Ok(r) => { let kv = r.value.ok_or("empty answer from the sync host")?; Ok(Some((keys.open(object, &kv.value)?, kv.version))) }
        Err(VssError::NoSuchKeyError(_)) => Ok(None),
        Err(e) => Err(format!("sync host: {e}")),
    }
}
/// `version` 0 creates. A clash (someone wrote since we read) is `Err("conflict")`.
async fn put(url: &str, keys: &Keys, object: &str, version: i64, plain: &[u8]) -> Result<(), String> {
    let wire = keys.seal(object, plain)?;
    if wire.len() > MAX_OBJECT { return Err(format!("one record is too large to sync yet ({} KB packed)", wire.len() / 1024)); }
    client(url, keys).put_object(&PutObjectRequest { store_id: keys.store.clone(), global_version: None, transaction_items: vec![KeyValue { key: object.into(), version, value: wire }], delete_items: vec![] })
        .await.map(|_| ()).map_err(|e| match e { VssError::ConflictError(_) => "conflict".to_string(), e => format!("sync host: {e}") })
}
async fn del(url: &str, keys: &Keys, object: &str) -> Result<(), String> {
    client(url, keys).delete_object(&DeleteObjectRequest { store_id: keys.store.clone(), key_value: Some(KeyValue { key: object.into(), version: -1, value: vec![] }) }).await.map(|_| ()).map_err(|e| format!("sync host: {e}"))
}

// ----------------------------------------------------------- what travels --

/// A vault is the same vault under any name: what it is made of, and when.
fn vault_id(v: &Value) -> String {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    hex::encode(&Sha256::digest(format!("{}|{}|{}", s("descriptor"), s("ext"), v.get("created").and_then(Value::as_u64).unwrap_or(0)).as_bytes())[..16])
}
/// What makes two items of a list the same item, by the name of the list. `None`: the list is one value.
fn identity(list: &str, item: &Value) -> Option<String> {
    let s = |k: &str| item.get(k).and_then(Value::as_str).map(str::to_string);
    match list {
        "vaults" => Some(vault_id(item)),
        "contacts" => Some(format!("{}|{}", s("network")?, s("address")?)),
        "places" | "entities" => s("id"),
        "keys" => s("pubkey"),
        _ => None,
    }
}

/// The part of a store that travels, as JSON.
pub fn view(store: &Value) -> Value {
    let mut out = Map::new();
    for k in SYNCED { if let Some(v) = store.get(*k) { out.insert((*k).into(), v.clone()); } }
    if let Some(Value::Array(vs)) = out.get_mut("vaults") { for v in vs { if let Some(o) = v.as_object_mut() { for f in VAULT_LOCAL { o.remove(*f); } } } }
    Value::Object(out)
}
/// Put a merged view back into a store, keeping everything that does not travel exactly as this device had it.
pub fn apply(store: &mut Value, merged: &Value) {
    let local_vaults: HashMap<String, Value> = store.get("vaults").and_then(Value::as_array).map(|a| a.iter().map(|v| (vault_id(v), v.clone())).collect()).unwrap_or_default();
    let Some(s) = store.as_object_mut() else { return };
    for k in SYNCED {
        match merged.get(*k) { Some(v) => { s.insert((*k).into(), v.clone()); } None => { s.remove(*k); } }
    }
    if let Some(Value::Array(vs)) = s.get_mut("vaults") {
        for v in vs { let id = vault_id(v); if let (Some(o), Some(mine)) = (v.as_object_mut(), local_vaults.get(&id)) { for f in VAULT_LOCAL { if let Some(x) = mine.get(*f) { o.insert((*f).into(), x.clone()); } } } }
    }
    if !s.contains_key("vaults") { s.insert("vaults".into(), Value::Array(vec![])); }
}

// ----------------------------------------------------------------- merge --

fn keyed<'a>(list: &str, v: Option<&'a Value>) -> Option<Vec<(String, &'a Value)>> {
    match v { None | Some(Value::Null) => Some(vec![]), Some(Value::Array(a)) => a.iter().map(|i| identity(list, i).map(|id| (id, i))).collect(), _ => None }
}

/// Three-way merge. `None` is "not there". `name` is the field this value sits under (it decides how a list is merged
/// and which counters only move forward).
pub fn merge3(name: &str, base: Option<&Value>, local: Option<&Value>, remote: Option<&Value>) -> Option<Value> { merge(name, base, local, remote, false) }
/// `joining`: this device is meeting a profile that already exists elsewhere. It has no last-synced copy to judge
/// by, so where the two differ the profile as it exists wins: a new device's defaults (the picture it was made
/// with, say) must never overwrite what the person set on the device they came from.
pub fn merge(name: &str, base: Option<&Value>, local: Option<&Value>, remote: Option<&Value>, joining: bool) -> Option<Value> {
    if FORWARD_ONLY.contains(&name) {
        let n = |v: Option<&Value>| v.and_then(Value::as_u64);
        return match (n(local), n(remote)) { (Some(a), Some(b)) => Some(Value::from(a.max(b))), _ => local.or(remote).cloned() };
    }
    if local == remote { return local.cloned(); }
    if local == base { return remote.cloned(); }
    if remote == base { return local.cloned(); }
    // both sides changed, differently
    match (local, remote) {
        (Some(Value::Object(l)), Some(Value::Object(r))) => {
            let b = base.and_then(Value::as_object);
            let mut out = Map::new();
            let mut keys: Vec<&String> = l.keys().collect();
            for k in r.keys() { if !l.contains_key(k) { keys.push(k); } }
            for k in keys { if let Some(v) = merge(k, b.and_then(|b| b.get(k)), l.get(k), r.get(k), joining) { out.insert(k.clone(), v); } }
            Some(Value::Object(out))
        }
        (Some(Value::Array(_)), Some(Value::Array(_))) => {
            // a list of things with an identity is merged item by item, in this device's order with new arrivals after
            match (keyed(name, base), keyed(name, local), keyed(name, remote)) {
                (Some(b), Some(l), Some(r)) => {
                    let (bm, rm): (HashMap<&str, &Value>, HashMap<&str, &Value>) = (b.iter().map(|(k, v)| (k.as_str(), *v)).collect(), r.iter().map(|(k, v)| (k.as_str(), *v)).collect());
                    let lm: HashMap<&str, &Value> = l.iter().map(|(k, v)| (k.as_str(), *v)).collect();
                    let mut out = Vec::new();
                    for (id, item) in &l { if let Some(v) = merge("", bm.get(id.as_str()).copied(), Some(item), rm.get(id.as_str()).copied(), joining) { out.push(v); } }
                    for (id, item) in &r { if !lm.contains_key(id.as_str()) { if let Some(v) = merge("", bm.get(id.as_str()).copied(), None, Some(item), joining) { out.push(v); } } }
                    Some(Value::Array(out))
                }
                _ => local.cloned(),
            }
        }
        // a true clash, or one side deleted what the other changed: this device's word stands (a change beats a deletion)
        (Some(_), Some(_)) if joining => remote.cloned(),
        (Some(_), _) => local.cloned(),
        (None, r) => r.cloned(),
    }
}

// ------------------------------------------------------- objects on the host --

const BOOK: &str = "book";
/// Left alone in a store whose profile has moved to a new key (see `retire`).
const GONE: &str = "gone";
/// The whole error when the profile's key was replaced on another device: this one must be shown a key again.
pub const ROTATED: &str = "this profile\u{2019}s key was replaced on another device. Plug in one of its keys and enter its PIN to carry on.";

/// The view cut into the objects the host keeps: one per vault, and one for everything else. The book also lists
/// which vault objects exist, so a vault the host has quietly dropped is told apart from one a device forgot.
fn objects(keys: &Keys, view: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let mut book = view.as_object().cloned().unwrap_or_default();
    if let Some(Value::Array(vs)) = book.remove("vaults") { for v in vs { out.insert(keys.name_of("v", &vault_id(&v)), v); } }
    book.insert("index".into(), Value::Array(out.keys().map(|k| Value::String(k.clone())).collect()));
    out.insert(BOOK.into(), Value::Object(book));
    out
}
fn assemble(objs: &BTreeMap<String, Value>) -> Value {
    let mut out = objs.get(BOOK).and_then(Value::as_object).cloned().unwrap_or_default();
    out.remove("index");
    let vaults: Vec<Value> = objs.iter().filter(|(k, _)| k.starts_with("v/")).map(|(_, v)| v.clone()).collect();
    out.insert("vaults".into(), Value::Array(vaults));
    Value::Object(out)
}

/// A record as the host holds it: a counter that only ever goes up, and the record. A device remembers the highest
/// counter it has seen for each record, so a host that serves an older copy is caught rather than believed.
fn wrap(n: u64, body: &Value) -> Result<Vec<u8>, String> { serde_json::to_vec(&serde_json::json!({ "n": n, "body": body })).map_err(|e| e.to_string()) }
fn unwrap(plain: &[u8]) -> Result<(u64, Value), String> {
    let v: Value = serde_json::from_slice(plain).map_err(|e| format!("a synced record is damaged: {e}"))?;
    match v.as_object() {
        Some(o) if o.len() == 2 && o.contains_key("body") && o.get("n").map(Value::is_u64).unwrap_or(false) => Ok((o["n"].as_u64().unwrap_or(0), o["body"].clone())),
        _ => Ok((0, v)),   // the first builds that synced wrote the record bare
    }
}

/// A record too large for one request is sent as parts, and the record's own object says where they are. The parts
/// are named after their content, so the same record always lands in the same parts and a half-sent set is harmless.
const PART: usize = 1024 * 1024;
fn parts_base(keys: &Keys, name: &str, plain: &[u8]) -> Option<String> {
    (plain.len() > PART).then(|| keys.name_of("p", &format!("{name}|{}", hex::encode(&Sha256::digest(plain)[..8]))))
}
async fn put_record(url: &str, keys: &Keys, name: &str, version: i64, plain: &[u8]) -> Result<(), String> {
    let Some(base) = parts_base(keys, name, plain) else { return put(url, keys, name, version, plain).await };
    let n = plain.chunks(PART).count();
    for (i, part) in plain.chunks(PART).enumerate() {
        match put(url, keys, &format!("{base}/{i}"), 0, part).await { Ok(()) => {} Err(e) if e == "conflict" => {} Err(e) => return Err(e) }   // already there: same content, same name
    }
    put(url, keys, name, version, &serde_json::to_vec(&serde_json::json!({ "a2k_parts": { "base": base, "n": n, "len": plain.len() } })).map_err(|e| e.to_string())?).await
}
/// (the record, its version on the host, where its parts are if it has any)
async fn get_record(url: &str, keys: &Keys, name: &str) -> Result<Option<(Vec<u8>, i64, Option<String>)>, String> {
    let Some((plain, version)) = get(url, keys, name).await? else { return Ok(None) };
    let head: Option<Value> = serde_json::from_slice::<Value>(&plain).ok().filter(|v| v.as_object().map(|o| o.len() == 1 && o.contains_key("a2k_parts")).unwrap_or(false));
    let Some(head) = head else { return Ok(Some((plain, version, None))) };
    let (base, n, len) = (head["a2k_parts"]["base"].as_str().unwrap_or("").to_string(), head["a2k_parts"]["n"].as_u64().unwrap_or(0) as usize, head["a2k_parts"]["len"].as_u64().unwrap_or(0) as usize);
    if !base.starts_with("p/") || n == 0 || n > 512 { return Err("a synced record is damaged".into()); }
    let mut out = Vec::with_capacity(len);
    for i in 0..n { out.extend(get(url, keys, &format!("{base}/{i}")).await?.ok_or("a record on the sync host is not all there yet; it will be tried again")?.0); }
    if out.len() != len { return Err("a record on the sync host is not all there yet; it will be tried again".into()); }
    Ok(Some((out, version, Some(base))))
}

// ------------------------------------------------------------ local state --

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CardRef {
    pub pubkey: String, pub name: String, pub added: u64,
    /// the profile's key was replaced since this key was last plugged in: its pointer still names the old one
    #[serde(default)] pub stale: bool,
}

/// What this device remembers about a synced profile. Sealed on disk like everything else (`sync.enc`).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct State {
    pub v: u8,
    /// what names this profile on every device and in every key's pointer
    pub id: String,
    pub root: String,
    pub url: String,
    /// how many times the profile's key has been replaced; the higher one is the current one
    #[serde(default)] pub gen: u32,
    /// the view as it stood on the host after this device's last sync
    #[serde(default)] pub base: Value,
    #[serde(default)] pub last: u64,
    /// the keys that can bring this profile to a new device
    #[serde(default)] pub cards: Vec<CardRef>,
    /// the host's records and their versions as this device last left them: one cheap listing against this says
    /// whether another device has written since, without fetching anything
    #[serde(default)] pub seen: BTreeMap<String, i64>,
    /// the highest counter this device has seen inside each record: the host is never believed below it
    #[serde(default)] pub counters: BTreeMap<String, u64>,
    /// parts on the host that no record names, and when that was first noticed: removed an hour later, never at once,
    /// because another device may be half-way through sending the record that will name them
    #[serde(default)] pub orphans: BTreeMap<String, u64>,
    /// the key this profile had before it was replaced here, until its store on the host has been emptied
    #[serde(default)] pub retiring: Option<String>,
    /// files this device holds that the host does not have all of yet
    #[serde(default)] pub files_waiting: usize,
}
fn state_path() -> std::path::PathBuf { crate::lockbox::dir().join("sync.enc") }
pub fn state() -> Result<Option<State>, String> {
    match crate::lockbox::read_sealed(&state_path(), "sync")? { None => Ok(None), Some(b) => serde_json::from_slice(&b).map(Some).map_err(|e| format!("this device's sync record is damaged: {e}")) }
}
pub fn save_state(s: &State) -> Result<(), String> { crate::lockbox::write_sealed(&state_path(), "sync", &serde_json::to_vec(s).map_err(|e| e.to_string())?) }
pub fn forget_state() -> Result<(), String> { match std::fs::remove_file(state_path()) { Ok(()) => Ok(()), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()), Err(e) => Err(e.to_string()) } }
impl State {
    pub fn root(&self) -> Result<[u8; 32], String> { hex::decode(&self.root).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| "this device's sync key is damaged".to_string()) }
    /// Take up a different key for the same profile: everything remembered about the old store is dropped.
    pub fn rekey(&mut self, root: String, gen: u32) {
        self.root = root; self.gen = gen; self.base = Value::Null; self.seen.clear(); self.counters.clear(); self.orphans.clear(); self.files_waiting = 0;
    }
}

/// Set by every save of the store; cleared when a sync has carried this device's changes to the host.
pub static DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// ------------------------------------------------------------------- sync --

#[derive(Serialize, Clone, Debug, Default)]
pub struct Outcome { pub pulled: bool, pub pushed: usize, pub last: u64, #[serde(default)] pub files: Files }

const OLDER: &str = "the sync host handed back an older copy of this profile than this device has already seen. Nothing was taken from it.";
const DROPPED: &str = "the sync host no longer has a record this profile had there. Nothing was taken from it or sent to it.";

/// One round: read the host, merge three ways, write what differs, take what arrived. `load`/`store` are the
/// device's store as JSON; `store` is handed the merge of what came back with whatever changed here meanwhile.
pub async fn run(st: &mut State, now: u64, load: impl Fn() -> Result<Value, String>, store: impl Fn(&dyn Fn(&mut Value) -> bool) -> Result<bool, String>) -> Result<Outcome, String> {
    let keys = profile_keys(&st.root()?);
    for _ in 0..3 {
        let local0 = view(&load()?);
        // the host, as it is now
        let all = list(&st.url, &keys).await?;
        if all.contains_key(GONE) { return Err(ROTATED.into()); }
        let versions: BTreeMap<String, i64> = all.iter().filter(|(name, _)| is_record(name)).map(|(k, v)| (k.clone(), *v)).collect();
        let (mut remote_objs, mut remote_n) = (BTreeMap::new(), BTreeMap::new());
        let mut parts_of: BTreeMap<String, String> = BTreeMap::new();   // record → where its parts are, as the host will have it after this round
        for name in versions.keys() {
            let Some((plain, _, parts)) = get_record(&st.url, &keys, name).await? else { continue };
            let (n, body) = unwrap(&plain)?;
            if n < *st.counters.get(name).unwrap_or(&0) { return Err(OLDER.into()); }
            if let Some(p) = parts { parts_of.insert(name.clone(), p); }
            remote_n.insert(name.clone(), n); remote_objs.insert(name.clone(), body);
        }
        // a record this device has seen there before and is gone: forgotten by a device (the book says so), or dropped
        let index: Option<Vec<String>> = remote_objs.get(BOOK).and_then(|b| b.get("index")).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
        for name in st.counters.keys() {
            if remote_objs.contains_key(name) { continue; }
            if name == BOOK || index.as_ref().map(|ix| ix.contains(name)).unwrap_or(false) { return Err(DROPPED.into()); }
        }
        // a vault object the book does not list is the tail of a deletion another device has not finished: not there
        if let Some(ix) = &index { remote_objs.retain(|name, _| name == BOOK || ix.contains(name)); }
        let never = versions.is_empty();   // and nothing remembered, or the loop above would have refused
        let remote = assemble(&remote_objs);
        let base = if st.base.is_null() { None } else { Some(&st.base) };
        // a device joining a profile that already exists takes it as it is; its own first contents are added to it
        let merged = if never { local0.clone() } else {
            let mut out = Map::new();
            for k in SYNCED { if let Some(v) = merge(k, base.and_then(|b| b.get(*k)), local0.get(*k), remote.get(*k), base.is_none()) { out.insert((*k).into(), v); } }
            Value::Object(out)
        };
        // write what differs from the host, the book first (it names the vaults that are about to arrive); a clash
        // means someone else wrote first: read again
        let want = objects(&keys, &merged);
        let (mut pushed, mut clash) = (0usize, false);
        for (name, body) in &want {
            if remote_objs.get(name) == Some(body) { st.counters.insert(name.clone(), *remote_n.get(name).unwrap_or(&0)); continue; }
            let n = (*st.counters.get(name).unwrap_or(&0)).max(*remote_n.get(name).unwrap_or(&0)) + 1;
            let plain = wrap(n, body)?;
            match put_record(&st.url, &keys, name, *versions.get(name).unwrap_or(&0), &plain).await {
                Ok(()) => { pushed += 1; st.counters.insert(name.clone(), n); match parts_base(&keys, name, &plain) { Some(p) => { parts_of.insert(name.clone(), p); } None => { parts_of.remove(name); } } }
                Err(e) if e == "conflict" => { clash = true; break }
                Err(e) => return Err(e),
            }
        }
        if clash { continue; }
        for name in versions.keys() { if !want.contains_key(name) { del(&st.url, &keys, name).await?; st.counters.remove(name); parts_of.remove(name); pushed += 1; } }
        let live_parts: std::collections::BTreeSet<&str> = parts_of.values().map(String::as_str).collect();
        // parts no record names any more: noted now, removed once they have been unnamed for an hour
        let mut loose: BTreeMap<String, Vec<&String>> = BTreeMap::new();
        for name in all.keys().filter(|n| n.starts_with("p/")) { if let Some((b, _)) = name.rsplit_once('/') { if !live_parts.contains(b) { loose.entry(b.to_string()).or_default().push(name); } } }
        st.orphans.retain(|b, _| loose.contains_key(b));
        for (b, names) in &loose {
            let first = *st.orphans.entry(b.clone()).or_insert(now);
            if now.saturating_sub(first) >= 3600 { for n in names { del(&st.url, &keys, n).await?; } st.orphans.remove(b); }
        }
        // take it here too, without losing anything typed on this device while the host was being talked to
        let merged_for_store = merged.clone();
        let local_at_read = local0.clone();
        let pulled = store(&move |s: &mut Value| {
            let now_local = view(s);
            let mut out = Map::new();
            for k in SYNCED { if let Some(v) = merge3(k, local_at_read.get(*k), now_local.get(*k), merged_for_store.get(*k)) { out.insert((*k).into(), v); } }
            let fin = Value::Object(out);
            if fin == now_local { return false; }
            apply(s, &fin); true
        })?;
        st.base = merged; st.last = now;
        st.seen = list(&st.url, &keys).await.map(|mut l| { l.retain(|name, _| is_record(name)); l }).unwrap_or_default();
        save_state(st)?;
        return Ok(Outcome { pulled, pushed, last: now, files: Files::default() });
    }
    Err("the sync host kept changing while this device was writing; it will be tried again".into())
}

/// Has another device written since this one last synced? One listing, nothing fetched.
pub async fn changed(st: &State) -> Result<bool, String> {
    let mut now = list(&st.url, &profile_keys(&st.root()?)).await?;
    now.retain(|name, _| is_record(name));
    Ok(now != st.seen)
}

/// Empty a store whose profile has moved to a new key, and leave the one word that tells every other device so.
/// A device that still holds the old key learns nothing from here on: there is nothing left to read, and what the
/// others write goes where it cannot follow.
pub async fn retire(url: &str, old_root: &[u8; 32]) -> Result<(), String> {
    let keys = profile_keys(old_root);
    let all = list(url, &keys).await?;
    if !all.contains_key(GONE) { put(url, &keys, GONE, 0, b"{\"a2k_rotated\":true}").await.or_else(|e| if e == "conflict" { Ok(()) } else { Err(e) })?; }
    for name in all.keys().filter(|n| *n != GONE) { del(url, &keys, name).await?; }
    Ok(())
}

// ------------------------------------------------------------------ files --
//
// A record names its files by the SHA-256 of their bytes (and a picture's thumbnail by its own). The record travels
// with the profile; the bytes travel here, cut into pieces the host will accept, each sealed under the profile's
// key and filed under a name the host cannot connect to the file. A device sends what it holds and the host lacks,
// fetches every thumbnail (they are small, and a record without its picture looks broken), and fetches whole files
// either as they are opened or, where the person keeps everything, in the background. A file leaves the host only
// when the merged profile no longer names it anywhere.

const PIECE: usize = 256 * 1024;
fn is_record(name: &str) -> bool { !name.starts_with("a/") && !name.starts_with("p/") }
fn is_hash(s: &str) -> bool { s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) }

/// Every file the view names: hash → is it only ever a thumbnail.
pub fn file_refs(view: &Value) -> BTreeMap<String, bool> {
    fn walk(v: &Value, out: &mut BTreeMap<String, bool>) {
        match v {
            Value::Object(o) => {
                if let Some(h) = o.get("sha256").and_then(Value::as_str).filter(|h| is_hash(h)) {
                    out.insert(h.to_lowercase(), false);
                    if let Some(t) = o.get("thumb").and_then(Value::as_str).filter(|t| is_hash(t)) { out.entry(t.to_lowercase()).or_insert(true); }
                }
                for x in o.values() { walk(x, out); }
            }
            Value::Array(a) => for x in a { walk(x, out); },
            _ => {}
        }
    }
    let mut out = BTreeMap::new(); walk(view, &mut out); out
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Files { pub sent: usize, pub waiting: usize, pub fetched: usize, pub removed: usize }

/// One whole file from the host, or `None` when the host does not have all of it (yet). The caller checks the hash.
pub async fn download(st: &State, sha256: &str) -> Result<Option<Vec<u8>>, String> {
    let keys = profile_keys(&st.root()?);
    let base = keys.name_of("a", &sha256.to_lowercase());
    let mut out = Vec::new();
    for i in 0..(crate::attach::MAX_FILE as usize / PIECE + 2) {
        match get(&st.url, &keys, &format!("{base}/{i}")).await? { Some((piece, _)) => { let last = piece.len() < PIECE; out.extend(piece); if last { break; } } None => break }
    }
    Ok(if out.is_empty() { None } else { Some(out) })
}

/// Bring the host's files in line with the profile. `budget`: how many pieces may be sent this time, so one long
/// upload never holds everything else up; what is left is counted in `waiting` and sent on the next round.
pub async fn files(st: &State, keep_all: bool, mut budget: usize, have: impl Fn(&str) -> bool, read: impl Fn(&str) -> Result<Vec<u8>, String>, keep: impl Fn(&str, &[u8]) -> Result<(), String>) -> Result<Files, String> {
    let keys = profile_keys(&st.root()?);
    let refs = file_refs(&st.base);
    let mut out = Files::default();
    // what the host holds, by file
    let mut held: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in list(&st.url, &keys).await?.into_keys().filter(|n| n.starts_with("a/")) { if let Some((base, _)) = name.rsplit_once('/') { held.entry(base.to_string()).or_default().push(name); } }
    let named: BTreeMap<String, &String> = refs.keys().map(|h| (keys.name_of("a", h), h)).collect();
    // send what this device holds and the host lacks
    for (base, hash) in &named {
        if !have(hash) { continue; }
        let on_host = held.get(base).map(Vec::len).unwrap_or(0);
        let Ok(bytes) = read(hash) else { continue };
        // a file that is an exact number of pieces ends with an empty one, so "a short piece" always marks the end
        let mut pieces: Vec<&[u8]> = bytes.chunks(PIECE).collect();
        if bytes.len() % PIECE == 0 { pieces.push(&[]); }
        if on_host >= pieces.len() { continue; }
        let mut done = true;
        for (i, piece) in pieces.iter().enumerate() {
            let name = format!("{base}/{i}");
            if held.get(base).map(|h| h.contains(&name)).unwrap_or(false) { continue; }
            if budget == 0 { done = false; break; }
            match put(&st.url, &keys, &name, 0, piece).await { Ok(()) => {} Err(e) if e == "conflict" => {} Err(e) => return Err(e) }   // a clash here is another device sending the same piece
            budget -= 1;
        }
        if done { out.sent += 1; } else { out.waiting += 1; }
    }
    // fetch what the host holds and this device lacks: every thumbnail, and whole files where everything is kept
    for (base, hash) in &named {
        if have(hash) || !held.contains_key(base) || !(refs[*hash] || keep_all) { continue; }
        if let Some(bytes) = download(st, hash).await? { if keep(hash, &bytes).is_ok() { out.fetched += 1; } }   // a file still being sent by its device fails its hash: next time
    }
    // a file the profile no longer names anywhere leaves the host
    for (base, names) in &held { if !named.contains_key(base) { for n in names { del(&st.url, &keys, n).await?; } out.removed += 1; } }
    Ok(out)
}

// ---------------------------------------------------------------- pointers --

/// One profile a key may fetch.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pointed { pub id: String, pub root: String, pub url: String, pub name: String, pub class: crate::profiles::Class, pub added: u64,
    /// how many times the profile's key had been replaced when this was written (see `State::gen`)
    #[serde(default)] pub gen: u32 }
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Pointer { v: u8, profiles: Vec<Pointed> }
const POINTER: &str = "profiles";

/// The profiles this key's secret can fetch from `url`.
pub async fn pointed(url: &str, card_secret: &[u8; 32]) -> Result<Vec<Pointed>, String> {
    let keys = pointer_keys(card_secret);
    Ok(match get(url, &keys, POINTER).await? { Some((plain, _)) => serde_json::from_slice::<Pointer>(&plain).map_err(|e| format!("this key's sync record is damaged: {e}"))?.profiles, None => vec![] })
}
/// Add (or refresh) one profile in a key's pointer; `None` takes it out.
pub async fn point(url: &str, card_secret: &[u8; 32], id: &str, entry: Option<Pointed>) -> Result<(), String> {
    let keys = pointer_keys(card_secret);
    for _ in 0..3 {
        let (mut p, version) = match get(url, &keys, POINTER).await? { Some((plain, v)) => (serde_json::from_slice::<Pointer>(&plain).map_err(|e| format!("this key's sync record is damaged: {e}"))?, v), None => (Pointer::default(), 0) };
        p.v = 1; p.profiles.retain(|x| x.id != id);
        if let Some(e) = &entry { p.profiles.push(e.clone()); }
        match put(url, &keys, POINTER, version, &serde_json::to_vec(&p).map_err(|e| e.to_string())?).await { Ok(()) => return Ok(()), Err(e) if e == "conflict" => continue, Err(e) => return Err(e) }
    }
    Err("this key's sync record kept changing; try again".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn m(base: Value, local: Value, remote: Value) -> Value {
        let mut out = Map::new();
        for k in SYNCED { if let Some(v) = merge3(k, base.get(*k), local.get(*k), remote.get(*k)) { out.insert((*k).into(), v); } }
        Value::Object(out)
    }
    fn vault(name: &str, desc: &str, extra: Value) -> Value { let mut v = json!({ "name": name, "descriptor": desc, "created": 100, "labels": {}, "next": 0 }); for (k, x) in extra.as_object().unwrap() { v[k] = x.clone(); } v }

    #[test]
    fn what_only_one_side_changed_is_taken_from_that_side_deletions_included() {
        let base = json!({ "vaults": [vault("a", "wsh(A)", json!({})), vault("b", "wsh(B)", json!({}))], "contacts": [{ "name": "Ann", "address": "bc1a", "network": "mainnet" }, { "name": "Bob", "address": "bc1b", "network": "mainnet" }] });
        // here: vault a renamed, Bob deleted. there: vault b got a label, a contact was added.
        let local = json!({ "vaults": [vault("a renamed", "wsh(A)", json!({})), vault("b", "wsh(B)", json!({}))], "contacts": [{ "name": "Ann", "address": "bc1a", "network": "mainnet" }] });
        let remote = json!({ "vaults": [vault("a", "wsh(A)", json!({})), vault("b", "wsh(B)", json!({ "labels": { "tx1": "rent" } }))], "contacts": [{ "name": "Ann", "address": "bc1a", "network": "mainnet" }, { "name": "Bob", "address": "bc1b", "network": "mainnet" }, { "name": "Cy", "address": "bc1c", "network": "mainnet" }] });
        let out = m(base, local, remote);
        assert_eq!(out["vaults"][0]["name"], "a renamed");
        assert_eq!(out["vaults"][1]["labels"]["tx1"], "rent");
        let names: Vec<&str> = out["contacts"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Ann", "Cy"], "Bob stays deleted, Cy arrives");
    }

    #[test]
    fn two_devices_editing_different_fields_of_one_vault_both_keep_their_edit() {
        let base = json!({ "vaults": [vault("a", "wsh(A)", json!({ "labels": { "t1": "x" }, "drill_days": 90 }))] });
        let local = json!({ "vaults": [vault("a", "wsh(A)", json!({ "labels": { "t1": "x", "t2": "here" }, "drill_days": 90 }))] });
        let remote = json!({ "vaults": [vault("a", "wsh(A)", json!({ "labels": { "t1": "x", "t3": "there" }, "drill_days": 30 }))] });
        let out = m(base, local, remote);
        assert_eq!(out["vaults"][0]["labels"], json!({ "t1": "x", "t2": "here", "t3": "there" }));
        assert_eq!(out["vaults"][0]["drill_days"], 30);
    }

    #[test]
    fn a_true_clash_goes_to_this_device_and_a_change_beats_a_deletion() {
        let base = json!({ "vaults": [vault("a", "wsh(A)", json!({}))] });
        let local = json!({ "vaults": [vault("mine", "wsh(A)", json!({}))] });
        let remote = json!({ "vaults": [vault("theirs", "wsh(A)", json!({}))] });
        assert_eq!(m(base.clone(), local.clone(), remote)["vaults"][0]["name"], "mine");
        // the other device forgot the vault while this one renamed it: it stays
        assert_eq!(m(base.clone(), local, json!({ "vaults": [] }))["vaults"][0]["name"], "mine");
        // forgotten there, untouched here: it goes
        assert_eq!(m(base.clone(), base.clone(), json!({ "vaults": [] }))["vaults"], json!([]));
    }

    #[test]
    fn a_device_joining_a_profile_takes_it_as_it_is_and_adds_what_only_it_has() {
        let here = json!({ "profile": { "name": "Household", "picture": "glyph:keys" }, "contacts": [{ "name": "Mine", "address": "bc1m", "network": "mainnet" }] });
        let there = json!({ "profile": { "name": "Household", "picture": "img:AAAA" }, "contacts": [{ "name": "Ann", "address": "bc1a", "network": "mainnet" }] });
        let mut out = Map::new();
        for k in SYNCED { if let Some(v) = merge(k, None, here.get(*k), there.get(*k), true) { out.insert((*k).into(), v); } }
        assert_eq!(out["profile"]["picture"], "img:AAAA", "the default it was made with does not overwrite the real one");
        assert_eq!(out["contacts"].as_array().unwrap().len(), 2);
        // once it has synced, its own change is a change like any other
        assert_eq!(merge3("profile", Some(&there["profile"]), Some(&json!({ "name": "Household", "picture": "img:BBBB" })), Some(&there["profile"])).unwrap()["picture"], "img:BBBB");
    }

    #[test]
    fn address_counters_only_move_forward() {
        let base = json!({ "vaults": [vault("a", "wsh(A)", json!({ "next": 5 }))] });
        let local = json!({ "vaults": [vault("a", "wsh(A)", json!({ "next": 9, "labels": { "k": "v" } }))] });
        let remote = json!({ "vaults": [vault("a", "wsh(A)", json!({ "next": 7, "name": "a2" }))] });
        let out = m(base, local.clone(), remote.clone());
        assert_eq!(out["vaults"][0]["next"], 9);
        assert_eq!(m(json!({}), remote, local)["vaults"][0]["next"], 9, "whichever side is asked");
    }

    #[test]
    fn a_vault_new_on_each_side_ends_up_on_both_in_this_devices_order() {
        let base = json!({ "vaults": [vault("a", "wsh(A)", json!({}))] });
        let local = json!({ "vaults": [vault("a", "wsh(A)", json!({})), vault("here", "wsh(H)", json!({}))] });
        let remote = json!({ "vaults": [vault("there", "wsh(T)", json!({})), vault("a", "wsh(A)", json!({}))] });
        let out = m(base, local, remote);
        let names: Vec<&str> = out["vaults"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["a", "here", "there"]);
    }

    #[test]
    fn only_what_travels_leaves_the_device_and_what_stays_is_kept_when_a_merge_comes_back() {
        let mut store = json!({ "vaults": [{ "name": "a", "descriptor": "wsh(A)", "created": 1, "labels_synced": 77, "labels": {} }], "settings": { "electrum": "tcp://10.0.0.5:50001" }, "last_psbt": "cHNidP8=", "seen": { "a": {} }, "contacts": [], "key_skip": [1], "key_begun": [2] });
        let v = view(&store);
        for gone in ["settings", "last_psbt", "seen", "key_skip", "key_begun"] { assert!(v.get(gone).is_none(), "{gone} must not travel"); }
        assert!(v["vaults"][0].get("labels_synced").is_none());
        let mut merged = v.clone(); merged["vaults"][0]["name"] = json!("renamed elsewhere"); merged["contacts"] = json!([{ "name": "Ann", "address": "bc1a", "network": "mainnet" }]);
        apply(&mut store, &merged);
        assert_eq!(store["vaults"][0]["name"], "renamed elsewhere");
        assert_eq!(store["vaults"][0]["labels_synced"], 77);
        assert_eq!(store["settings"]["electrum"], "tcp://10.0.0.5:50001");
        assert_eq!(store["last_psbt"], "cHNidP8=");
        assert_eq!(store["contacts"][0]["name"], "Ann");
    }

    #[test]
    fn an_object_sealed_for_one_name_does_not_open_under_another_or_another_profile() {
        let (a, b) = (profile_keys(&[1u8; 32]), profile_keys(&[2u8; 32]));
        let wire = a.seal("book", b"{\"contacts\":[]}").unwrap();
        assert_eq!(a.open("book", &wire).unwrap(), b"{\"contacts\":[]}");
        assert!(a.open("v/abc", &wire).is_err());
        assert!(b.open("book", &wire).is_err());
        assert_ne!(a.store, b.store);
        assert_ne!(a.name_of("v", "x"), b.name_of("v", "x"));
        assert_ne!(pointer_keys(&[1u8; 32]).store, a.store, "a key's pointer and a profile never share a store");
    }

    #[test]
    fn the_view_is_cut_into_one_object_per_vault_and_put_back_whole() {
        let k = profile_keys(&[3u8; 32]);
        let v = json!({ "vaults": [vault("a", "wsh(A)", json!({})), vault("b", "wsh(B)", json!({}))], "contacts": [{ "name": "Ann", "address": "bc1a", "network": "mainnet" }] });
        let objs = objects(&k, &v);
        assert_eq!(objs.len(), 3);
        assert!(objs.keys().all(|n| !n.contains("wsh")), "an object's name says nothing about the vault");
        let back = assemble(&objs);
        assert_eq!(back["contacts"], v["contacts"]);
        assert_eq!(back["vaults"].as_array().unwrap().len(), 2);
    }

    /// Two devices and a key, against the real host: `cargo test host_round_trip -- --ignored --nocapture`.
    /// Writes a few small objects under a throwaway store and removes every one of them afterwards.
    #[test]
    #[ignore]
    fn host_round_trip() {
        use std::sync::Mutex;
        let url = crate::labels::DEFAULT_VSS;
        unsafe { std::env::set_var("A2K_STORE_KEY", "44".repeat(32)); }
        let tmp = std::env::temp_dir().join(format!("a2k-sync-{}", hex::encode(crate::lockbox::random::<6>())));
        let (da, db) = (tmp.join("a"), tmp.join("b"));
        let root: [u8; 32] = crate::lockbox::random();
        // whatever happens below, the host is left as it was found
        struct Sweep(&'static str, Vec<Keys>);
        impl Drop for Sweep { fn drop(&mut self) { let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap(); for k in &self.1 { if let Ok(names) = rt.block_on(list(self.0, k)) { for n in names.keys() { let _ = rt.block_on(del(self.0, k, n)); } } } } }
        let root2: [u8; 32] = crate::lockbox::random();
        let secret: [u8; 32] = crate::lockbox::random();
        let _sweep = Sweep(url, vec![profile_keys(&root), profile_keys(&root2), pointer_keys(&secret)]);
        let fresh = || State { v: 1, id: "t".into(), root: hex::encode(root), url: url.into(), ..Default::default() };
        let (mut sa, mut sb) = (fresh(), fresh());
        let clock = std::cell::Cell::new(1u64);
        let a = Mutex::new(json!({ "vaults": [vault("a", "wsh(A)", json!({}))], "contacts": [{ "name": "Ann", "address": "bc1a", "network": "mainnet" }], "settings": { "electrum": "home" } }));
        let b = Mutex::new(json!({ "vaults": [], "settings": { "electrum": "phone" } }));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let go = |dir: &std::path::Path, st: &mut State, dev: &Mutex<Value>| {
            crate::profiles::test_root(Some(dir.to_path_buf()));
            rt.block_on(run(st, clock.get(), || Ok(dev.lock().unwrap().clone()), |f| { let mut g = dev.lock().unwrap(); Ok(f(&mut g)) }))
        };
        let try_go = &go;
        let go = |dir: &std::path::Path, st: &mut State, dev: &Mutex<Value>| try_go(dir, st, dev).unwrap();
        let o = go(&da, &mut sa, &a); assert!(o.pushed >= 2 && !o.pulled, "{o:?}");
        let o = go(&db, &mut sb, &b); assert!(o.pulled && o.pushed == 0, "{o:?}");
        assert_eq!(b.lock().unwrap()["vaults"][0]["name"], "a");
        assert_eq!(b.lock().unwrap()["settings"]["electrum"], "phone", "settings stay where they were made");
        // each device changes something different
        { let mut g = b.lock().unwrap(); g["vaults"][0]["name"] = json!("renamed on b"); g["contacts"].as_array_mut().unwrap().push(json!({ "name": "Cy", "address": "bc1c", "network": "mainnet" })); }
        a.lock().unwrap()["vaults"][0]["labels"]["tx1"] = json!("rent");
        go(&db, &mut sb, &b);
        let o = go(&da, &mut sa, &a); assert!(o.pulled && o.pushed >= 1, "{o:?}");
        go(&db, &mut sb, &b);
        for d in [&a, &b] { let g = d.lock().unwrap(); assert_eq!(g["vaults"][0]["name"], "renamed on b"); assert_eq!(g["vaults"][0]["labels"]["tx1"], "rent"); assert_eq!(g["contacts"].as_array().unwrap().len(), 2); }
        // a deletion travels, and so does forgetting a vault
        a.lock().unwrap()["contacts"].as_array_mut().unwrap().retain(|c| c["name"] != "Ann");
        go(&da, &mut sa, &a); go(&db, &mut sb, &b);
        assert_eq!(b.lock().unwrap()["contacts"].as_array().unwrap().len(), 1);
        b.lock().unwrap()["vaults"] = json!([]);
        go(&db, &mut sb, &b); go(&da, &mut sa, &a);
        assert_eq!(a.lock().unwrap()["vaults"], json!([]));
        // nothing to say: nothing is written
        let o = go(&da, &mut sa, &a); assert!(o.pushed == 0 && !o.pulled, "{o:?}");
        // a file: named by a record on a, sent by a, fetched by b (whole, because b keeps everything), gone when unnamed
        let big: Vec<u8> = (0..PIECE * 2).map(|i| (i % 251) as u8).collect();   // exactly two pieces: the awkward size
        let thumb = b"a small picture".to_vec();
        let (hb, ht) = (hex::encode(Sha256::digest(&big)), hex::encode(Sha256::digest(&thumb)));
        let disk_a: Mutex<HashMap<String, Vec<u8>>> = Mutex::new([(hb.clone(), big.clone()), (ht.clone(), thumb.clone())].into_iter().collect());
        let disk_b: Mutex<HashMap<String, Vec<u8>>> = Mutex::new(HashMap::new());
        a.lock().unwrap()["records"] = json!({ "files": { "place:1": [{ "name": "receipt.pdf", "sha256": hb, "thumb": ht, "size": big.len() }] } });
        go(&da, &mut sa, &a);
        let fgo = |st: &State, disk: &Mutex<HashMap<String, Vec<u8>>>, keep_all: bool, budget: usize| rt.block_on(files(st, keep_all, budget,
            |h| disk.lock().unwrap().contains_key(h), |h| disk.lock().unwrap().get(h).cloned().ok_or("gone".to_string()),
            |h, b| { if hex::encode(Sha256::digest(b)) != h { return Err("wrong bytes".into()); } disk.lock().unwrap().insert(h.to_string(), b.to_vec()); Ok(()) })).unwrap();
        let f = fgo(&sa, &disk_a, true, 2); assert!(f.waiting >= 1 && f.fetched == 0, "a small budget leaves some waiting: {f:?}");
        go(&db, &mut sb, &b);
        let f = fgo(&sb, &disk_b, false, 40); assert!(disk_b.lock().unwrap().get(&hb).is_none(), "a half-sent file is not taken: {f:?}");
        let f = fgo(&sa, &disk_a, true, 40); assert!(f.waiting == 0 && f.sent >= 1, "{f:?}");
        let f = fgo(&sb, &disk_b, false, 40); assert!(disk_b.lock().unwrap().get(&ht) == Some(&thumb) && !disk_b.lock().unwrap().contains_key(&hb), "a phone takes the thumbnail and leaves the file: {f:?}");
        assert_eq!(rt.block_on(download(&sb, &hb)).unwrap(), Some(big.clone()), "and fetches the whole file when it is opened");
        let f = fgo(&sb, &disk_b, true, 40); assert!(f.fetched == 1 && disk_b.lock().unwrap().get(&hb) == Some(&big), "a device that keeps everything takes it unasked: {f:?}");
        assert_eq!(fgo(&sa, &disk_a, true, 40), Files::default(), "nothing to do twice");
        assert!(rt.block_on(changed(&sa)).unwrap() == false, "files being sent are not mistaken for another device's changes");
        a.lock().unwrap()["records"] = json!({});
        go(&da, &mut sa, &a);
        let f = fgo(&sa, &disk_a, true, 40); assert!(f.removed == 2, "{f:?}");
        assert!(rt.block_on(list(url, &profile_keys(&root))).unwrap().keys().all(|n| is_record(n)));
        // one cheap listing tells b that a wrote
        assert!(rt.block_on(changed(&sb)).unwrap());
        go(&db, &mut sb, &b); assert!(!rt.block_on(changed(&sb)).unwrap());
        // a record too large for one request goes as parts and comes back whole; its parts leave an hour after it shrinks
        let pad: String = (0..(2 * PART + 5000)).map(|i| char::from(b'a' + ((i * 7 + i / 13) % 26) as u8)).collect();
        a.lock().unwrap()["vaults"] = json!([vault("big", "wsh(BIG)", json!({ "notes": { "tx1": pad.clone() } }))]);
        go(&da, &mut sa, &a); go(&db, &mut sb, &b);
        assert_eq!(b.lock().unwrap()["vaults"][0]["notes"]["tx1"].as_str().map(str::len), Some(pad.len()));
        let parts = |rt: &tokio::runtime::Runtime| rt.block_on(list(url, &profile_keys(&root))).unwrap().keys().filter(|n| n.starts_with("p/")).count();
        assert!(parts(&rt) >= 3, "sent as parts");
        a.lock().unwrap()["vaults"][0]["notes"] = json!({});
        go(&da, &mut sa, &a); assert!(parts(&rt) >= 3, "unnamed parts are not removed at once: another device may be half-way through naming them");
        clock.set(1 + 3601); go(&da, &mut sa, &a); assert_eq!(parts(&rt), 0, "and are an hour later");
        go(&db, &mut sb, &b);
        // a host that hands back an older copy, or loses a record, is refused and nothing is taken or sent
        let keys = profile_keys(&root);
        let book_now = rt.block_on(get(url, &keys, BOOK)).unwrap().unwrap();
        let (n_now, body_now) = unwrap(&book_now.0).unwrap();
        rt.block_on(put(url, &keys, BOOK, book_now.1, &wrap(n_now - 1, &body_now).unwrap())).unwrap();
        let before = b.lock().unwrap().clone();
        assert_eq!(try_go(&db, &mut sb, &b).unwrap_err(), OLDER); assert_eq!(*b.lock().unwrap(), before);
        let v = rt.block_on(get(url, &keys, BOOK)).unwrap().unwrap().1;
        rt.block_on(put(url, &keys, BOOK, v, &wrap(n_now, &body_now).unwrap())).unwrap();
        go(&db, &mut sb, &b);
        let vname = keys.name_of("v", &vault_id(&b.lock().unwrap()["vaults"][0]));
        let held = rt.block_on(get(url, &keys, &vname)).unwrap().unwrap().0;
        rt.block_on(del(url, &keys, &vname)).unwrap();
        assert_eq!(try_go(&db, &mut sb, &b).unwrap_err(), DROPPED);
        rt.block_on(put(url, &keys, &vname, 0, &held)).unwrap();
        go(&db, &mut sb, &b); go(&da, &mut sa, &a);
        // the profile's key is replaced on a: the old store is emptied and says so; b is refused until shown the new key
        sa.rekey(hex::encode(root2), 1); sa.retiring = None;
        go(&da, &mut sa, &a);
        rt.block_on(retire(url, &root)).unwrap();
        assert_eq!(rt.block_on(list(url, &profile_keys(&root))).unwrap().keys().cloned().collect::<Vec<_>>(), vec![GONE.to_string()]);
        b.lock().unwrap()["contacts"].as_array_mut().unwrap().push(json!({ "name": "Late", "address": "bc1late", "network": "mainnet" }));
        assert_eq!(try_go(&db, &mut sb, &b).unwrap_err(), ROTATED, "and it does not write a thing into the old store");
        assert_eq!(rt.block_on(list(url, &profile_keys(&root))).unwrap().len(), 1);
        sb.rekey(hex::encode(root2), 1);
        go(&db, &mut sb, &b); go(&da, &mut sa, &a);
        assert!(a.lock().unwrap()["contacts"].as_array().unwrap().iter().any(|c| c["name"] == "Late"), "what b did meanwhile is not lost");
        assert_eq!(a.lock().unwrap()["vaults"], b.lock().unwrap()["vaults"]);
        for name in rt.block_on(list(url, &profile_keys(&root2))).unwrap().keys() { rt.block_on(del(url, &profile_keys(&root2), name)).unwrap(); }
        // a key's pointer
        let entry = Pointed { id: "t".into(), root: hex::encode(root), url: url.into(), name: "Test".into(), class: crate::profiles::Class::Work, added: 1, gen: 0 };
        rt.block_on(point(url, &secret, "t", Some(entry.clone()))).unwrap();
        assert_eq!(rt.block_on(pointed(url, &secret)).unwrap(), vec![entry]);
        assert!(rt.block_on(pointed(url, &crate::lockbox::random())).unwrap().is_empty(), "another key sees nothing");
        rt.block_on(point(url, &secret, "t", None)).unwrap();
        assert!(rt.block_on(pointed(url, &secret)).unwrap().is_empty());
        // leave the host as it was found
        for keys in [profile_keys(&root), pointer_keys(&secret)] { for name in rt.block_on(list(url, &keys)).unwrap().keys() { rt.block_on(del(url, &keys, name)).unwrap(); } assert!(rt.block_on(list(url, &keys)).unwrap().is_empty()); }
        crate::profiles::test_root(None); let _ = std::fs::remove_dir_all(&tmp);
    }
}
