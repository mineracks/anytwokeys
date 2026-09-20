//! Labels, backed up encrypted on a VSS host (LDK Versioned Storage Service) so that any k of the vault's
//! keys bring them back with the vault.
//!
//! The key is not a new secret. A sealed vault's shares (seal.rs) fix a random polynomial that only k cards
//! with their PINs can rebuild, and every share of it is known to whoever holds k of them. HKDF over the
//! full set of shares therefore gives a 32-byte root that is (a) the same on every computer that has
//! restored the vault from its keys, (b) unknown to anyone with fewer than k keys, and (c) stored nowhere
//! but the cards and this app's own store. From the root: an XChaCha20-Poly1305 content key, a secp256k1
//! key that signs the VSS requests (the host sees only its public key, unrelated to the vault's), and the
//! store id. The host holds ciphertext it cannot open and cannot link to any address or transaction.
//!
//! One object per vault, key "labels": magic ‖ 24-byte nonce ‖ ciphertext of the JSON envelope, with the
//! store id as associated data. VSS versions give optimistic concurrency; a clash merges newest-wins per
//! label and retries. Deleting a label leaves a dated tombstone so the deletion also wins on another machine.
//! A vault that is not sealed has no shares and is not backed up; the UI says so.
use bitcoin::secp256k1::SecretKey;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use vss_client_ng::client::VssClient;
use vss_client_ng::error::VssError;
use vss_client_ng::headers::sigs_auth::SigsAuthProvider;
use vss_client_ng::types::{GetObjectRequest, KeyValue, PutObjectRequest};
use vss_client_ng::util::retry::{ExponentialBackoffRetryPolicy, RetryPolicy};

pub const DEFAULT_VSS: &str = "https://vss.mineracks.com/vss";
const MAGIC: &[u8; 5] = b"A2KL\x01";
const OBJECT: &str = "labels";

pub struct Keys { pub content: [u8; 32], pub auth: SecretKey, pub store: String, /// names attachments on the host, so the host cannot test for a file it knows
    pub att: [u8; 32] }

/// The root from a vault's shares (hex, one per key, in key order). None when the vault is not sealed.
pub fn root_from_shares(shares: &[String]) -> Option<[u8; 32]> {
    if shares.is_empty() { return None; }
    let mut ikm = Vec::new();
    for s in shares { ikm.extend(hex::decode(s).ok()?); }
    let hk = Hkdf::<Sha256>::new(Some(b"a2k-labels-v1"), &ikm);
    let mut root = [0u8; 32];
    hk.expand(b"root", &mut root).ok()?;
    Some(root)
}

pub fn keys(root: &[u8; 32]) -> Keys {
    let hk = Hkdf::<Sha256>::from_prk(root).expect("32-byte prk");
    let mut content = [0u8; 32]; hk.expand(b"content", &mut content).unwrap();
    let mut store = [0u8; 16]; hk.expand(b"store", &mut store).unwrap();
    // a 32-byte HKDF output is a valid secp256k1 secret with overwhelming probability; walk a counter for the rest
    let mut i = 0u8;
    let auth = loop {
        let mut a = [0u8; 32]; hk.expand(&[b"auth".as_slice(), &[i]].concat(), &mut a).unwrap();
        if let Ok(k) = SecretKey::from_slice(&a) { break k; }
        i += 1;
    };
    let mut att = [0u8; 32]; hk.expand(b"att-name", &mut att).unwrap();
    Keys { content, auth, store: hex::encode(store), att }
}

/// One label's state. `label` empty = deleted (a tombstone), `t` = when it last changed (unix seconds).
/// `note` and `files` are reserved for longer text and attachments (kept as-is by older builds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Rec {
    pub label: String,
    pub t: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")] pub note: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub files: Vec<serde_json::Value>,
    /// Everything else in the record, known to this build or not (`ctx`: where, on what and with which keys). A build
    /// that drops what it does not understand erases it for every computer on its next write, so nothing is dropped.
    #[serde(flatten)] pub more: serde_json::Map<String, serde_json::Value>,
}

impl Rec {
    /// A record with a label, a note or a file is alive; one with none of them is a deletion that travels.
    pub fn live(&self) -> bool { !self.label.is_empty() || !self.note.is_empty() || !self.files.is_empty() || !self.more.is_empty() }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Envelope { pub v: u32, pub records: BTreeMap<String, Rec> }

impl Envelope {
    /// Everything this computer holds for each key. A key that only appears in `meta` is a tombstone.
    pub fn from_vault(labels: &HashMap<String, String>, meta: &HashMap<String, u64>, notes: &HashMap<String, String>, files: &HashMap<String, Vec<serde_json::Value>>, more: &HashMap<String, serde_json::Map<String, serde_json::Value>>) -> Envelope {
        let mut records = BTreeMap::new();
        for k in labels.keys().chain(notes.keys()).chain(files.keys()).chain(more.keys()).chain(meta.keys()) {
            records.entry(k.clone()).or_insert_with(|| Rec { label: labels.get(k).cloned().unwrap_or_default(), t: *meta.get(k).unwrap_or(&0),
                note: notes.get(k).cloned().unwrap_or_default(), files: files.get(k).cloned().unwrap_or_default(), more: more.get(k).cloned().unwrap_or_default() });
        }
        Envelope { v: 1, records }
    }
    /// Newest wins per key; equal times prefer the non-empty (a label over its own tombstone). The extra fields are
    /// written once and never edited (a removal is itself a value), so the winner also keeps any the loser had and it
    /// lacks: where a payment was made must not vanish because a label was typed on another computer first.
    pub fn merge(&self, other: &Envelope) -> Envelope {
        let mut out = self.clone();
        for (k, r) in &other.records {
            let (mut win, lose) = match out.records.get(k) {
                Some(mine) if mine.t > r.t || (mine.t == r.t && (mine.live() || !r.live())) => (mine.clone(), r),
                Some(mine) => (r.clone(), &mine.clone()),
                None => { out.records.insert(k.clone(), r.clone()); continue }
            };
            for (f, val) in &lose.more { if !win.more.contains_key(f) { win.more.insert(f.clone(), val.clone()); } }
            out.records.insert(k.clone(), win);
        }
        out.v = 1;
        out
    }
    pub fn seal(&self, keys: &Keys, nonce: [u8; 24]) -> Result<Vec<u8>, String> {
        let plain = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        let ct = XChaCha20Poly1305::new((&keys.content).into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: &plain, aad: keys.store.as_bytes() }).map_err(|_| "could not encrypt the labels".to_string())?;
        Ok([MAGIC.as_slice(), &nonce, &ct].concat())
    }
    pub fn open(keys: &Keys, wire: &[u8]) -> Result<Envelope, String> {
        if wire.len() < MAGIC.len() + 24 + 16 || &wire[..MAGIC.len()] != MAGIC { return Err("the stored labels are not in a form this build knows".into()); }
        let (nonce, ct) = wire[MAGIC.len()..].split_at(24);
        let plain = XChaCha20Poly1305::new((&keys.content).into()).decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: keys.store.as_bytes() }).map_err(|_| "the stored labels did not open with this vault's keys".to_string())?;
        serde_json::from_slice(&plain).map_err(|e| e.to_string())
    }
}

macro_rules! client { ($url:expr, $keys:expr) => {{
    let headers = Arc::new(SigsAuthProvider::new($keys.auth, HashMap::new()));
    // the default policy has NO attempt cap: a stale put retried forever until .with_max_attempts (found on the VSS probe)
    let retry = ExponentialBackoffRetryPolicy::new(Duration::from_millis(200)).with_max_attempts(3);
    VssClient::new_with_headers($url.to_string(), retry, headers)
}} }

/// Fetch the vault's object: (envelope, version) or None when nothing is stored yet.
pub async fn fetch(url: &str, keys: &Keys) -> Result<Option<(Envelope, i64)>, String> {
    let c = client!(url, keys);
    match c.get_object(&GetObjectRequest { store_id: keys.store.clone(), key: OBJECT.into() }).await {
        Ok(r) => { let kv = r.value.ok_or("empty answer from the label host")?; Ok(Some((Envelope::open(keys, &kv.value)?, kv.version))) }
        Err(VssError::NoSuchKeyError(_)) => Ok(None),
        Err(e) => Err(format!("label host: {e}")),
    }
}

/// Put `env` as version `version` (0 = create). Err on a version clash, so the caller merges and retries.
pub async fn store(url: &str, keys: &Keys, env: &Envelope, version: i64) -> Result<(), String> {
    use bitcoin::secp256k1::rand::RngCore;
    let mut nonce = [0u8; 24];
    bitcoin::secp256k1::rand::rngs::OsRng.fill_bytes(&mut nonce);
    let wire = env.seal(keys, nonce)?;
    let c = client!(url, keys);
    c.put_object(&PutObjectRequest { store_id: keys.store.clone(), global_version: None, transaction_items: vec![KeyValue { key: OBJECT.into(), version, value: wire }], delete_items: vec![] })
        .await.map(|_| ()).map_err(|e| match e { VssError::ConflictError(_) => "conflict".to_string(), e => format!("label host: {e}") })
}

/// Pull, merge, push if needed. Returns the merged envelope and whether the host was written.
pub async fn sync(url: &str, keys: &Keys, local: &Envelope) -> Result<(Envelope, bool), String> {
    for _ in 0..3 {
        let (remote, version) = match fetch(url, keys).await? { Some((e, v)) => (Some(e), v), None => (None, 0) };
        let merged = match &remote { Some(r) => r.merge(local), None => local.clone() };
        if remote.as_ref() == Some(&merged) { return Ok((merged, false)); }
        match store(url, keys, &merged, version).await {
            Ok(()) => return Ok((merged, true)),
            Err(e) if e == "conflict" => continue,
            Err(e) => return Err(e),
        }
    }
    Err("the label host kept changing under us; try again".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::Secp256k1;
    fn shares() -> Vec<String> { vec!["aa".repeat(40), "bb".repeat(40), "cc".repeat(40)] }
    #[test]
    fn the_root_is_a_function_of_the_shares_only() {
        let a = root_from_shares(&shares()).unwrap();
        assert_eq!(a, root_from_shares(&shares()).unwrap());
        assert_ne!(a, root_from_shares(&vec!["aa".repeat(40), "bb".repeat(40), "cd".repeat(40)]).unwrap());
        assert!(root_from_shares(&[]).is_none());
        let k = keys(&a);
        assert_eq!(k.store.len(), 32);
        assert_ne!(k.content.to_vec(), k.auth.secret_bytes().to_vec());
        let _ = k.auth.public_key(&Secp256k1::new());
    }
    #[test]
    fn an_envelope_survives_seal_and_open_and_refuses_the_wrong_key() {
        let k = keys(&root_from_shares(&shares()).unwrap());
        let mut labels = HashMap::new(); labels.insert("tx:ab".to_string(), "rent".to_string());
        let mut meta = HashMap::new(); meta.insert("tx:ab".to_string(), 5u64); meta.insert("addr:gone".to_string(), 7u64);
        let e = Envelope::from_vault(&labels, &meta, &HashMap::new(), &HashMap::new(), &HashMap::new());
        assert_eq!(e.records.len(), 2);
        assert_eq!(e.records["addr:gone"].label, "");
        let wire = e.seal(&k, [7u8; 24]).unwrap();
        assert_eq!(Envelope::open(&k, &wire).unwrap(), e);
        let other = keys(&root_from_shares(&vec!["aa".repeat(40), "bb".repeat(40), "cd".repeat(40)]).unwrap());
        assert!(Envelope::open(&other, &wire).is_err());
        assert!(Envelope::open(&k, &wire[..30]).is_err());
    }
    #[test]
    fn a_note_or_a_file_keeps_a_record_alive_and_is_never_dropped_on_the_way_through() {
        // a transaction with a note and a receipt but NO label is not a deletion
        let mut notes = HashMap::new(); notes.insert("tx:cd".to_string(), "paid the plumber; invoice attached".to_string());
        let mut files = HashMap::new(); files.insert("tx:cd".to_string(), vec![serde_json::json!({"name": "invoice.pdf", "sha256": "ab", "size": 9})]);
        let mut meta = HashMap::new(); meta.insert("tx:cd".to_string(), 5u64);
        let mine = Envelope::from_vault(&HashMap::new(), &meta, &notes, &files, &HashMap::new());
        assert!(mine.records["tx:cd"].live() && mine.records["tx:cd"].label.is_empty());
        // same second, the other computer deleted its (empty) label: the live record wins the tie, both ways round
        let mut theirs = Envelope { v: 1, records: BTreeMap::new() }; theirs.records.insert("tx:cd".into(), Rec { t: 5, ..Default::default() });
        assert_eq!(mine.merge(&theirs).records["tx:cd"].note, "paid the plumber; invoice attached");
        assert_eq!(theirs.merge(&mine).records["tx:cd"].files.len(), 1);
        // an older build's label edit here must not wipe what another computer attached: from_vault carries all of it
        let mut labels = HashMap::new(); labels.insert("tx:cd".to_string(), "plumber".to_string()); meta.insert("tx:cd".to_string(), 9);
        let newer = Envelope::from_vault(&labels, &meta, &notes, &files, &HashMap::new());
        let out = theirs.merge(&mine).merge(&newer);
        assert_eq!((out.records["tx:cd"].label.as_str(), out.records["tx:cd"].note.len() > 0, out.records["tx:cd"].files.len()), ("plumber", true, 1));
    }
    #[test]
    fn what_this_build_does_not_know_rides_through_and_where_survives_a_later_label() {
        // a record written by a NEWER build, with a field this one has never heard of
        let wire = r#"{"v":1,"records":{"tx:ab":{"label":"rent","t":5,"ctx":{"place":"near Brisbane"},"from_the_future":[1,2]}}}"#;
        let theirs: Envelope = serde_json::from_str(wire).unwrap();
        assert_eq!(theirs.records["tx:ab"].more["from_the_future"], serde_json::json!([1, 2]));
        // it comes into the vault's maps and goes out again whole
        let more: HashMap<String, serde_json::Map<String, serde_json::Value>> = theirs.records.iter().map(|(k, r)| (k.clone(), r.more.clone())).collect();
        let (mut labels, mut meta) = (HashMap::new(), HashMap::new()); labels.insert("tx:ab".to_string(), "rent".to_string()); meta.insert("tx:ab".to_string(), 5u64);
        let again = serde_json::to_value(Envelope::from_vault(&labels, &meta, &HashMap::new(), &HashMap::new(), &more)).unwrap();
        assert_eq!(again, serde_json::from_str::<serde_json::Value>(wire).unwrap());
        // a label typed LATER on a computer that never saw the place wins the record and keeps the place
        let mut later = Envelope { v: 1, records: BTreeMap::new() }; later.records.insert("tx:ab".into(), Rec { label: "rent, March".into(), t: 9, ..Default::default() });
        for m in [later.merge(&theirs), theirs.merge(&later)] { assert_eq!(m.records["tx:ab"].label, "rent, March"); assert_eq!(m.records["tx:ab"].more["ctx"]["place"], "near Brisbane"); }
        // a removal is a value, so it is not undone by the copy that still has the place
        let mut removed = later.clone(); removed.records.get_mut("tx:ab").unwrap().more.insert("ctx".into(), serde_json::json!({ "removed": true }));
        assert_eq!(removed.merge(&theirs).records["tx:ab"].more["ctx"], serde_json::json!({ "removed": true }));
        // a record that is only a place is alive
        assert!(Rec { more: theirs.records["tx:ab"].more.clone(), ..Default::default() }.live());
    }

    #[test]
    fn merge_is_newest_wins_and_a_tombstone_beats_an_older_label() {
        let r = |l: &str, t: u64| Rec { label: l.into(), t, ..Default::default() };
        let mut a = Envelope { v: 1, records: BTreeMap::new() }; a.records.insert("x".into(), r("old", 1)); a.records.insert("y".into(), r("", 9)); a.records.insert("z".into(), r("keep", 3));
        let mut b = Envelope { v: 1, records: BTreeMap::new() }; b.records.insert("x".into(), r("new", 2)); b.records.insert("y".into(), r("revived?", 4)); b.records.insert("w".into(), r("only b", 1));
        let m = a.merge(&b);
        assert_eq!(m.records["x"].label, "new"); assert_eq!(m.records["y"].label, ""); assert_eq!(m.records["z"].label, "keep"); assert_eq!(m.records["w"].label, "only b");
        assert_eq!(a.merge(&b), b.merge(&a));
    }
}

// ------------------------------------------------------------ attachments --
// A file's bytes are their own objects on the host: `att/<keyed name>/<n>`, one CHUNK of plaintext each (a VSS host caps
// request bodies; ours at 4 MiB), each sealed under the content key with its own nonce and its object name as
// associated data — so chunks cannot be swapped between files or positions. The name is HMAC(att key, sha256), not the
// hash itself: a host that knew the hash could test whether you hold a file it already has.

// 256 KiB, not the 1 MiB first planned: the VSS client gives every request a fixed 10 seconds, and a 1 MiB piece did not
// make it through a busy home link in that time (2026-09-20). A piece this size needs about 0.2 Mbit/s.
pub const CHUNK: usize = 256 * 1024;
const ATT_MAGIC: &[u8; 5] = b"A2KA\x01";

pub fn att_name(keys: &Keys, sha256: &str) -> String {
    use bitcoin::hashes::{sha256, Hash, HashEngine, Hmac, HmacEngine};
    let mut e = HmacEngine::<sha256::Hash>::new(&keys.att); e.input(sha256.as_bytes());
    format!("att/{}", hex::encode(&Hmac::<sha256::Hash>::from_engine(e).to_byte_array()[..16]))
}
pub fn chunks_of(size: usize) -> usize { size.div_ceil(CHUNK).max(1) }

pub fn seal_chunk(keys: &Keys, object: &str, plain: &[u8], nonce: [u8; 24]) -> Vec<u8> {
    let ct = XChaCha20Poly1305::new((&keys.content).into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: object.as_bytes() }).expect("encryption has no failing input");
    [ATT_MAGIC.as_slice(), &nonce, &ct].concat()
}
pub fn open_chunk(keys: &Keys, object: &str, wire: &[u8]) -> Result<Vec<u8>, String> {
    if wire.len() < ATT_MAGIC.len() + 24 + 16 || &wire[..ATT_MAGIC.len()] != ATT_MAGIC { return Err("the host returned something that is not one of our attachments".into()); }
    let (nonce, ct) = wire[ATT_MAGIC.len()..].split_at(24);
    XChaCha20Poly1305::new((&keys.content).into()).decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: object.as_bytes() }).map_err(|_| "a piece of the attachment did not open with this vault's keys, or is not the piece it claims to be".to_string())
}

/// One VSS call over our own HTTP client. The library's client gives every request a fixed 10 seconds and, with bodies
/// of a few hundred kilobytes, stalled before the request reached the host at all (2026-09-20: the host's log showed
/// nothing arriving; `curl` put a megabyte through the same path in three seconds). Same protocol, same signed headers.
async fn call<Req: prost::Message, Resp: prost::Message + Default>(url: &str, keys: &Keys, method: &str, req: &Req) -> Result<Resp, VssError> {
    use vss_client_ng::headers::VssHeaderProvider;
    let body = req.encode_to_vec();
    let headers = SigsAuthProvider::new(keys.auth, HashMap::new()).get_headers(&body).await.map_err(|e| VssError::AuthError(e.to_string()))?;
    let client = reqwest::Client::builder().timeout(Duration::from_secs(90)).build().map_err(|e| VssError::InternalError(e.to_string()))?;
    let mut r = client.post(format!("{}/{method}", url.trim_end_matches('/'))).header("content-type", "application/octet-stream").body(body);
    for (k, v) in headers { r = r.header(k, v); }
    let resp = r.send().await.map_err(|e| VssError::InternalError(if e.is_timeout() { "the label host did not answer in time".into() } else { "the label host could not be reached".into() }))?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| VssError::InternalError(e.to_string()))?;
    if !status.is_success() { return Err(VssError::new(status.as_u16() as i32, bytes.to_vec())); }
    Resp::decode(&bytes[..]).map_err(|e| VssError::InternalError(format!("the label host's answer could not be read: {e}")))
}

/// Every attachment object the host holds for this vault (names only; one cheap call per thousand).
pub async fn att_present(url: &str, keys: &Keys) -> Result<std::collections::HashSet<String>, String> {
    use vss_client_ng::types::ListKeyVersionsRequest;
    let c = client!(url, keys);
    let (mut out, mut token) = (std::collections::HashSet::new(), None);
    for _ in 0..200 {
        let r = c.list_key_versions(&ListKeyVersionsRequest { store_id: keys.store.clone(), key_prefix: Some("att/".into()), page_size: Some(1000), page_token: token.clone() }).await.map_err(|e| format!("label host: {e}"))?;
        out.extend(r.key_versions.into_iter().map(|k| k.key));
        match r.next_page_token { Some(t) if !t.is_empty() => token = Some(t), _ => break }
    }
    Ok(out)
}

/// Put whatever pieces of this file the host lacks. Returns how many pieces were sent.
pub async fn att_upload(url: &str, keys: &Keys, sha256: &str, bytes: &[u8], present: &std::collections::HashSet<String>) -> Result<usize, String> {
    use bitcoin::secp256k1::rand::RngCore;
    let (base, mut sent) = (att_name(keys, sha256), 0);
    for (n, plain) in bytes.chunks(CHUNK).enumerate() {
        let object = format!("{base}/{n}");
        if present.contains(&object) { continue; }
        let mut nonce = [0u8; 24]; bitcoin::secp256k1::rand::rngs::OsRng.fill_bytes(&mut nonce);
        let kv = KeyValue { key: object.clone(), version: 0, value: seal_chunk(keys, &object, plain, nonce) };
        match call::<_, vss_client_ng::types::PutObjectResponse>(url, keys, "putObjects", &PutObjectRequest { store_id: keys.store.clone(), global_version: None, transaction_items: vec![kv], delete_items: vec![] }).await {
            Ok(_) => sent += 1,
            Err(VssError::ConflictError(_)) => {}     // another computer put the same piece first: same content, nothing to do
            Err(e) => return Err(format!("label host: {e}")),
        }
    }
    Ok(sent)
}

/// Fetch a file and PROVE it: the pieces must open under their own names and the whole must hash to what the record says.
pub async fn att_download(url: &str, keys: &Keys, sha256: &str, size: usize) -> Result<Vec<u8>, String> {
    use bitcoin::hashes::Hash;
    let (base, mut out) = (att_name(keys, sha256), Vec::with_capacity(size));
    for n in 0..chunks_of(size) {
        let object = format!("{base}/{n}");
        let kv = match call::<_, vss_client_ng::types::GetObjectResponse>(url, keys, "getObject", &GetObjectRequest { store_id: keys.store.clone(), key: object.clone() }).await {
            Ok(r) => r.value.ok_or("empty answer from the label host")?,
            Err(VssError::NoSuchKeyError(_)) => return Err("the computer that added this file has not finished backing it up yet".into()),
            Err(e) => return Err(format!("label host: {e}")),
        };
        out.extend(open_chunk(keys, &object, &kv.value)?);
    }
    if hex::encode(bitcoin::hashes::sha256::Hash::hash(&out)) != sha256 { return Err("what came back is not the file the record names; it was thrown away".into()); }
    Ok(out)
}

/// Remove a file's pieces from the host: whichever of them the host actually lists, and no others.
pub async fn att_delete(url: &str, keys: &Keys, sha256: &str) -> Result<usize, String> {
    use vss_client_ng::types::DeleteObjectRequest;
    let (c, base) = (client!(url, keys), format!("{}/", att_name(keys, sha256)));
    let mine: Vec<String> = att_present(url, keys).await?.into_iter().filter(|k| k.starts_with(&base)).collect();
    for key in &mine {
        c.delete_object(&DeleteObjectRequest { store_id: keys.store.clone(), key_value: Some(KeyValue { key: key.clone(), version: -1, value: vec![] }) }).await.map_err(|e| format!("label host: {e}"))?;
    }
    Ok(mine.len())
}

#[cfg(test)]
mod att_tests {
    use super::*;
    #[test]
    fn a_piece_opens_only_as_itself_and_the_host_never_sees_the_file_hash() {
        let k = keys(&root_from_shares(&vec!["aa".repeat(40), "bb".repeat(40)]).unwrap());
        let sha = "ab".repeat(32);
        let base = att_name(&k, &sha);
        assert!(base.starts_with("att/") && base.len() == 4 + 32 && !base.contains(&sha[..16]), "{base}");
        assert_ne!(base, att_name(&keys(&root_from_shares(&vec!["aa".repeat(40), "cc".repeat(40)]).unwrap()), &sha), "another vault names the same file differently");
        let wire = seal_chunk(&k, &format!("{base}/0"), b"receipt", [7u8; 24]);
        assert_eq!(open_chunk(&k, &format!("{base}/0"), &wire).unwrap(), b"receipt");
        assert!(open_chunk(&k, &format!("{base}/1"), &wire).is_err(), "a piece must not pass as another position");
        assert!(open_chunk(&k, "att/ffffffffffffffffffffffffffffffff/0", &wire).is_err(), "or as a piece of another file");
        assert_eq!((chunks_of(0), chunks_of(1), chunks_of(CHUNK), chunks_of(CHUNK + 1)), (1, 1, 1, 2));
    }
    /// `A2K_VSS=https://vss.mineracks.com/vss cargo test live_attachment -- --ignored --nocapture`: a throwaway vault, two and a half pieces up, down, proved, deleted.
    #[test]
    #[ignore]
    fn live_attachment() {
        use bitcoin::hashes::Hash; use bitcoin::secp256k1::rand::RngCore;
        let url = std::env::var("A2K_VSS").expect("set A2K_VSS");
        let mut r = [0u8; 40]; bitcoin::secp256k1::rand::rngs::OsRng.fill_bytes(&mut r);
        let k = keys(&root_from_shares(&vec![hex::encode(&r[..20]), hex::encode(&r[20..])]).unwrap());
        let mut file = vec![0u8; CHUNK * 2 + CHUNK / 2]; bitcoin::secp256k1::rand::rngs::OsRng.fill_bytes(&mut file);
        let sha = hex::encode(bitcoin::hashes::sha256::Hash::hash(&file));
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert!(rt.block_on(att_present(&url, &k)).unwrap().is_empty());
        assert_eq!(rt.block_on(att_upload(&url, &k, &sha, &file, &Default::default())).unwrap(), 3);
        let there = rt.block_on(att_present(&url, &k)).unwrap(); assert_eq!(there.len(), 3);
        assert_eq!(rt.block_on(att_upload(&url, &k, &sha, &file, &there)).unwrap(), 0, "nothing is sent twice");
        assert_eq!(rt.block_on(att_upload(&url, &k, &sha, &file, &Default::default())).unwrap(), 0, "and a clash with a piece already there is not an error");
        assert_eq!(rt.block_on(att_download(&url, &k, &sha, file.len())).unwrap(), file);
        assert!(rt.block_on(att_download(&url, &k, &"00".repeat(32), 10)).unwrap_err().contains("not finished backing it up"));
        assert_eq!(rt.block_on(att_delete(&url, &k, &sha)).unwrap(), 3);
        assert_eq!(rt.block_on(att_delete(&url, &k, &sha)).unwrap(), 0, "deleting what is not there asks the host for nothing more");
        assert!(rt.block_on(att_present(&url, &k)).unwrap().is_empty(), "deleted pieces are gone");
        println!("store {}: 3 pieces up, proved down, deleted", k.store);
    }
}

#[cfg(test)]
mod live {
    //! `A2K_VSS=https://vss.mineracks.com/vss cargo test live_round_trip -- --ignored`: two "computers" that share
    //! nothing but the shares converge through the host, a deletion travels, and a stale write is merged, not lost.
    use super::*;
    #[test]
    #[ignore]
    fn live_round_trip() {
        let url = std::env::var("A2K_VSS").expect("set A2K_VSS");
        use bitcoin::secp256k1::rand::RngCore;
        let mut r = [0u8; 40]; bitcoin::secp256k1::rand::rngs::OsRng.fill_bytes(&mut r);   // a fresh vault each run: no leftovers on the host
        let shares = vec![hex::encode(&r[..20]), hex::encode(&r[20..])];
        let k = keys(&root_from_shares(&shares).unwrap());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let rec = |l: &str, t: u64| Rec { label: l.into(), t, ..Default::default() };
        // computer A labels two things
        let mut a = Envelope { v: 1, records: BTreeMap::new() }; a.records.insert("tx:1".into(), rec("rent", 10)); a.records.insert("addr:2".into(), rec("savings", 11));
        let (m, wrote) = rt.block_on(sync(&url, &k, &a)).unwrap(); assert!(wrote); assert_eq!(m, a);
        // computer B, empty, pulls everything
        let (m, wrote) = rt.block_on(sync(&url, &k, &Envelope::default())).unwrap(); assert!(!wrote); assert_eq!(m, a);
        // B deletes one and adds one; A meanwhile renamed the other: both survive
        let mut b = m.clone(); b.records.insert("tx:1".into(), rec("", 20)); b.records.insert("out:3:0".into(), rec("coffee", 21));
        let (_, wrote) = rt.block_on(sync(&url, &k, &b)).unwrap(); assert!(wrote);
        a.records.insert("addr:2".into(), rec("savings (main)", 22));
        let (m, wrote) = rt.block_on(sync(&url, &k, &a)).unwrap(); assert!(wrote);
        assert_eq!(m.records["tx:1"].label, ""); assert_eq!(m.records["addr:2"].label, "savings (main)"); assert_eq!(m.records["out:3:0"].label, "coffee");
        // the wrong key sees ciphertext it cannot open, and a fresh key sees nothing at all
        let other = keys(&root_from_shares(&vec![hex::encode(&r[..20]), hex::encode(&r[..20])]).unwrap());
        assert!(rt.block_on(fetch(&url, &other)).unwrap().is_none());
        println!("store {} on {url}: 3 records, host wrote 3 times, converged", k.store);
    }
}
