//! A vault on its way from one profile to another.
//!
//! Profiles are sealed under different keys and only one is ever open, so a vault cannot be handed across directly.
//! The move is two halves with a sealed parcel between them, under `<root>/moves/`:
//!
//!   <id>.a2kv   the BUNDLE — the vault, its snapshot, labels/notes/attachments, its keys' places, pictures and
//!               firmware facts, its owner, the transfer answers that name it, its node credential or scan key —
//!               sealed under a one-time key K (role "move")
//!   <id>.k      K sealed under the SOURCE profile's store key (role "move-key"): the way back
//!   <id>.json   unsealed: from, to, when. No vault name: the names of the two profiles are already public.
//!
//! K stays in memory across the switch (the window reloads; the process does not), so the destination opens the
//! parcel as soon as it is unlocked. If the app quits in between, K is gone — but `<id>.k` opens under the source
//! key, so the next time the SOURCE profile opens, the vault comes back there. Nothing is ever only in memory.
//!
//! The source vault is removed only after the parcel has been written and read back. What `vault_forget` clears
//! (the scan key, the node credential) travels in the parcel and is cleared from the source like a forget.

use crate::wallet::{self, Store, Vault};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Bundle {
    pub v: u8,
    pub vault: Vault,
    pub seen: Option<wallet::Seen>,
    pub answers: BTreeMap<String, crate::transfers::Answer>,
    pub entity: Option<wallet::Entity>,
    pub places: Vec<wallet::Place>,
    pub key_place: HashMap<String, String>,
    pub icons: HashMap<String, String>,
    pub forms: HashMap<String, u8>,
    pub key_first: HashMap<String, u64>,
    pub key_fw: HashMap<String, String>,
    /// attachments by content hash: the vault's files, their thumbnails, its keys' pictures
    pub files: Vec<(String, Vec<u8>)>,
    pub node_cred: Option<String>,
    pub sp_key: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Manifest { pub id: String, pub from: String, pub to: String, pub made: u64 }

/// K for parcels made in this process, by move id.
static KEYS: Mutex<Vec<(String, [u8; 32])>> = Mutex::new(Vec::new());

fn moves_dir() -> PathBuf { let d = crate::profiles::root().join("moves"); std::fs::create_dir_all(&d).ok(); d }

fn hashes_of(v: &Vault, s: &Store) -> Vec<String> {
    let mut out: HashSet<String> = HashSet::new();
    for list in v.files.values() { for f in list { for k in ["sha256", "thumb"] { if let Some(h) = f.get(k).and_then(|x| x.as_str()) { out.insert(h.to_string()); } } } }
    for k in &v.keys { if let Some(ic) = s.icons.get(&format!("key:{}", k.pubkey)) { if let Some(h) = ic.strip_prefix("img:") { out.insert(h.to_string()); } } }
    let mut v: Vec<String> = out.into_iter().collect(); v.sort(); v
}

/// Everything about one vault, out of the open store. Attachments that are not on this computer are left out, named.
pub fn build(s: &Store, name: &str) -> Result<(Bundle, Vec<String>), String> {
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
    let keys: HashSet<&str> = v.keys.iter().map(|k| k.pubkey.as_str()).collect();
    let mut b = Bundle { v: 1, vault: v.clone(), seen: s.seen.get(name).cloned(), answers: BTreeMap::new(), entity: None, places: vec![], key_place: HashMap::new(), icons: HashMap::new(), forms: HashMap::new(), key_first: HashMap::new(), key_fw: HashMap::new(), files: vec![], node_cred: None, sp_key: None };
    for (k, a) in &s.transfer_answers { if a.pair.from == name || a.pair.to == name { b.answers.insert(k.clone(), a.clone()); } }
    b.entity = v.entity.as_ref().and_then(|id| s.entities.iter().find(|e| &e.id == id)).cloned();
    for k in keys.iter() {
        if let Some(p) = s.key_place.get(*k) { b.key_place.insert(k.to_string(), p.clone()); if let Some(pl) = s.places.iter().find(|x| &x.id == p) { if !b.places.iter().any(|x| x.id == pl.id) { b.places.push(pl.clone()); } } }
        if let Some(ic) = s.icons.get(&format!("key:{k}")) { b.icons.insert(format!("key:{k}"), ic.clone()); }
        if let Some(f) = s.forms.get(*k) { b.forms.insert(k.to_string(), *f); }
        if let Some(t) = s.key_first.get(*k) { b.key_first.insert(k.to_string(), *t); }
        if let Some(fw) = s.key_fw.get(*k) { b.key_fw.insert(k.to_string(), fw.clone()); }
    }
    let mut missing = vec![];
    for h in hashes_of(&v, s) { match crate::attach::read(&h) { Ok(bytes) => b.files.push((h, bytes)), Err(_) => missing.push(h) } }
    if let Some(n) = v.node.as_ref() { b.node_cred = crate::node::cred(&crate::node::id_of(&n.url, &n.pubkey)).ok().map(|c| c.to_string()); }
    if let Some(sp) = v.watch.as_ref().and_then(|w| w.sp.as_ref()) { b.sp_key = crate::sp::scan_key(&crate::sp::id_of(&sp.spend)).ok().map(|k| k.bytes().to_vec()); }
    Ok((b, missing))
}

/// Put the bundle into the open store. A name already taken gets " · 2"; an owner or place of the same name is
/// reused rather than doubled. Answers naming the vault come along; derived matches rebuild themselves.
pub fn import(b: Bundle) -> Result<String, String> {
    let mut v = b.vault.clone();
    for (h, bytes) in &b.files { crate::attach::keep(h, bytes)?; }
    if let (Some(n), Some(c)) = (v.node.as_ref(), b.node_cred.as_ref()) { crate::node::keep_cred(&crate::node::id_of(&n.url, &n.pubkey), c)?; }
    if let (Some(sp), Some(k)) = (v.watch.as_ref().and_then(|w| w.sp.as_ref()), b.sp_key.as_ref()) { crate::sp::keep_scan_key(&crate::sp::id_of(&sp.spend), &crate::sp::ScanKey::from_bytes(k)?)?; }
    let name_out = Mutex::new(String::new());
    wallet::update(|s| {
        let mut name = v.name.clone(); let mut n = 2;
        while s.vaults.iter().any(|x| x.name == name) { name = format!("{} · {n}", v.name); n += 1; }
        let renamed = name != v.name;
        if let Some(e) = &b.entity {
            let id = match s.entities.iter().find(|x| x.name.eq_ignore_ascii_case(&e.name)) { Some(x) => x.id.clone(), None => {
                let mut e2 = e.clone(); if s.entities.iter().any(|x| x.id == e2.id) { e2.id = hex::encode(crate::lockbox::random::<6>()); } s.entities.push(e2.clone()); e2.id } };
            v.entity = Some(id);
        } else if v.entity.as_ref().map(|id| !s.entities.iter().any(|x| &x.id == id)).unwrap_or(false) { v.entity = None; }
        let mut place_ids: HashMap<String, String> = HashMap::new();
        for p in &b.places {
            let id = match s.places.iter().find(|x| x.name.eq_ignore_ascii_case(&p.name)) { Some(x) => x.id.clone(), None => {
                let mut p2 = p.clone(); if s.places.iter().any(|x| x.id == p2.id) { p2.id = hex::encode(crate::lockbox::random::<6>()); } s.places.push(p2.clone()); p2.id } };
            place_ids.insert(p.id.clone(), id);
        }
        for (k, p) in &b.key_place { if let Some(id) = place_ids.get(p) { s.key_place.entry(k.clone()).or_insert(id.clone()); } }
        for (k, ic) in &b.icons { s.icons.entry(k.clone()).or_insert(ic.clone()); }
        for (k, f) in &b.forms { s.forms.entry(k.clone()).or_insert(*f); }
        for (k, t) in &b.key_first { let e = s.key_first.entry(k.clone()).or_insert(*t); if *t < *e { *e = *t; } }
        for (k, fw) in &b.key_fw { s.key_fw.entry(k.clone()).or_insert(fw.clone()); }
        if let Some(seen) = &b.seen { s.seen.insert(name.clone(), seen.clone()); }
        for (k, a) in &b.answers { let mut a = a.clone(); if renamed { if a.pair.from == v.name { a.pair.from = name.clone(); } if a.pair.to == v.name { a.pair.to = name.clone(); } } s.transfer_answers.entry(k.clone()).or_insert(a); }
        v.name = name.clone();
        s.vaults.push(v.clone());
        *name_out.lock().unwrap() = name;
    })?;
    crate::transfers::clear();
    Ok(name_out.into_inner().unwrap())
}

#[derive(Serialize, Clone, Debug)]
pub struct Started { pub id: String, pub missing: Vec<String> }

/// First half, in the source profile: parcel written and proven, then the vault leaves the source store.
pub fn start(name: &str, to: &str, now: u64) -> Result<Started, String> {
    let from = crate::profiles::current().ok_or("no profile is open")?;
    if from.id == to { return Err("that is the profile the vault is already in".into()); }
    if !crate::profiles::index()?.profiles.iter().any(|p| p.id == to) { return Err("no such profile".into()); }
    let s = wallet::load()?;
    let (b, missing) = build(&s, name)?;
    let id = hex::encode(crate::lockbox::random::<8>());
    let k: [u8; 32] = crate::lockbox::random();
    let d = moves_dir();
    let plain = serde_json::to_vec(&b).map_err(|e| e.to_string())?;
    std::fs::write(d.join(format!("{id}.a2kv")), crate::lockbox::seal_with(&k, "move", &plain, crate::lockbox::random())).map_err(|e| e.to_string())?;
    let back = crate::lockbox::open_with(&k, "move", &std::fs::read(d.join(format!("{id}.a2kv"))).map_err(|e| e.to_string())?)?;
    if back != plain { return Err("the parcel did not read back the same; nothing was moved".into()); }
    crate::lockbox::write_sealed(&d.join(format!("{id}.k")), "move-key", &k)?;
    std::fs::write(d.join(format!("{id}.json")), serde_json::to_vec(&Manifest { id: id.clone(), from: from.id.clone(), to: to.into(), made: now }).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if let Ok(mut g) = KEYS.lock() { g.push((id.clone(), k)); }
    // now, and only now, the source lets go — the same clearing a forget does
    if let Some(sp) = b.vault.watch.as_ref().and_then(|w| w.sp.as_ref()) { crate::sp::forget_scan_key(&crate::sp::id_of(&sp.spend)); }
    if let Some(n) = b.vault.node.as_ref() { crate::node::forget_cred(&crate::node::id_of(&n.url, &n.pubkey)); }
    wallet::update(|s| { s.vaults.retain(|v| v.name != name); s.seen.remove(name); })?;
    crate::transfers::clear();
    Ok(Started { id, missing })
}

#[derive(Serialize, Clone, Debug)]
pub struct Landed { pub vault: String, pub back: bool, pub from: String }

fn manifests() -> Vec<Manifest> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(moves_dir()) { for e in rd.flatten() { let p = e.path(); if p.extension().and_then(|x| x.to_str()) == Some("json") { if let Ok(m) = std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<Manifest>(&b).map_err(|e| e.to_string())) { out.push(m); } } } }
    out.sort_by_key(|m| m.made); out
}
fn remove(id: &str) { let d = moves_dir(); for ext in ["a2kv", "k", "json"] { let _ = std::fs::remove_file(d.join(format!("{id}.{ext}"))); } if let Ok(mut g) = KEYS.lock() { g.retain(|(i, _)| i != id); } }

/// Second half, in whichever profile just unlocked: parcels addressed here land; parcels FROM here whose key is
/// gone (the app quit mid-move) come back. One at a time; the window asks again.
pub fn land(now: u64) -> Result<Option<Landed>, String> {
    let cur = crate::profiles::current().ok_or("no profile is open")?;
    crate::lockbox::key()?;
    for m in manifests() {
        let d = moves_dir();
        let parcel = match std::fs::read(d.join(format!("{}.a2kv", m.id))) { Ok(p) => p, Err(_) => { remove(&m.id); continue; } };
        let k_mem = KEYS.lock().ok().and_then(|g| g.iter().find(|(i, _)| i == &m.id).map(|(_, k)| *k));
        if m.to == cur.id {
            if let Some(k) = k_mem {
                let b: Bundle = serde_json::from_slice(&crate::lockbox::open_with(&k, "move", &parcel)?).map_err(|e| e.to_string())?;
                let name = import(b)?; remove(&m.id);
                return Ok(Some(Landed { vault: name, back: false, from: m.from }));
            }
            continue;   // its key is gone: it will go back to the profile it came from when that one opens
        }
        if m.from == cur.id && (k_mem.is_none() || now > m.made + 3600) {
            let k = crate::lockbox::read_sealed(&d.join(format!("{}.k", m.id)), "move-key")?.ok_or("the way back is missing")?;
            let k: [u8; 32] = k.try_into().map_err(|_| "the way back is damaged".to_string())?;
            let b: Bundle = serde_json::from_slice(&crate::lockbox::open_with(&k, "move", &parcel)?).map_err(|e| e.to_string())?;
            let name = import(b)?; remove(&m.id);
            return Ok(Some(Landed { vault: name, back: true, from: m.to }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(name: &str) -> Vault {
        serde_json::from_value(serde_json::json!({ "name": name, "network": "signet", "threshold": 2, "descriptor": "wsh(multi(2,…))", "address": "tb1q…", "created": 5,
            "keys": [{ "name": "laptop", "serial": 1, "pubkey": "02aa" }, { "name": "keyring", "serial": 2, "pubkey": "02bb" }] })).unwrap()
    }

    #[test]
    fn a_vault_moves_whole_and_comes_back_if_the_app_quit_in_between() {
        let d = std::env::temp_dir().join(format!("a2k-bundle-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap();
        crate::profiles::test_root(Some(d.clone()));
        std::env::set_var("A2K_STORE_KEY", "44".repeat(32));
        crate::profiles::migrate_legacy(1).unwrap();
        let a = crate::profiles::create("A", crate::profiles::Class::Work, None, 1).unwrap();
        let b = crate::profiles::create("B", crate::profiles::Class::Work, None, 1).unwrap();
        crate::profiles::open(&a.id, 2).unwrap();
        let mut v = vault("demo"); v.labels.insert("tx:ab".into(), "lunch".into()); v.entity = Some("e1".into());
        wallet::update(|s| { s.vaults.push(v.clone()); s.vaults.push(vault("other")); s.entities.push(wallet::Entity { id: "e1".into(), name: "SMSF".into(), kind: "smsf".into(), note: String::new(), added: 1 });
            s.places.push(wallet::Place { id: "p1".into(), name: "Home safe".into(), ..Default::default() }); s.key_place.insert("02aa".into(), "p1".into());
            s.key_fw.insert("02aa".into(), "5.4.3".into()); s.seen.insert("demo".into(), wallet::Seen { txs: [("ab".to_string(), 1)].into_iter().collect(), coins: Default::default(), primed: true }); }).unwrap();
        let st = start("demo", &b.id, 10).unwrap();
        assert!(st.missing.is_empty());
        let s = wallet::load().unwrap();
        assert_eq!(s.vaults.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), vec!["other"], "the source let go");
        assert!(!s.seen.contains_key("demo"));
        assert!(land(11).unwrap().is_none(), "nothing is addressed to the source while its key is in memory");
        // the destination
        crate::profiles::open(&b.id, 12).unwrap();
        wallet::update(|s| { s.entities.push(wallet::Entity { id: "e9".into(), name: "smsf".into(), kind: "smsf".into(), note: String::new(), added: 1 }); }).unwrap();
        let l = land(13).unwrap().unwrap();
        assert_eq!((l.vault.as_str(), l.back), ("demo", false));
        let s = wallet::load().unwrap();
        let v = s.vaults.iter().find(|v| v.name == "demo").unwrap();
        assert_eq!(v.labels.get("tx:ab").map(String::as_str), Some("lunch"));
        assert_eq!(v.entity.as_deref(), Some("e9"), "an owner of the same name is reused");
        assert_eq!(s.places.len(), 1); assert_eq!(s.key_place.get("02aa").map(String::as_str), Some("p1"));
        assert_eq!(s.key_fw.get("02aa").map(String::as_str), Some("5.4.3"));
        assert!(s.seen.get("demo").map(|x| x.primed).unwrap_or(false));
        assert!(std::fs::read_dir(moves_dir()).unwrap().count() == 0, "the parcel is gone");
        assert!(land(14).unwrap().is_none());
        // a second move of a vault with the same name lands as "demo · 2"; then the app "quits": its key is gone
        crate::profiles::open(&a.id, 15).unwrap();
        wallet::update(|s| s.vaults.push(vault("demo"))).unwrap();
        let st2 = start("demo", &b.id, 16).unwrap();
        if let Ok(mut g) = KEYS.lock() { g.clear(); }
        crate::profiles::open(&b.id, 17).unwrap();
        assert!(land(18).unwrap().is_none(), "the destination cannot open a parcel whose key died with the process");
        crate::profiles::open(&a.id, 19).unwrap();
        let back = land(20).unwrap().unwrap();
        assert!(back.back && back.vault == "demo", "{back:?}");
        assert!(wallet::load().unwrap().vaults.iter().any(|v| v.name == "demo"));
        let _ = st2;
        crate::profiles::close(); crate::profiles::test_root(None); std::env::remove_var("A2K_STORE_KEY");
    }
}
