//! Public records about addresses — a signed snapshot from bitcoin.mineracks.com/records/, matched ON THE DEVICE.
//! (docs/PROVENANCE.md "Public records"; the builder lives in the infrastructure repo, applications/public-records.)
//!
//! The only request this module ever makes is for the snapshot itself: the manifest, its signature, the whole records
//! file. It carries no address, no hash, no count — the server, and anyone in front of it, learns that a copy of the
//! app fetched the day's snapshot and nothing about any coin. The manifest must verify against a key pinned here and
//! the records file must hash to what the manifest says, or nothing is kept. A record is a record: a source listed an
//! address on a date. The screens show it as that — source, date, what it says — never as a verdict or a score.
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use sha2::Digest;

pub const BASE: &str = "https://bitcoin.mineracks.com/records/";
/// ed25519 public keys (32 bytes, hex) a manifest may be signed with: the fleet's public-records key. Two during a
/// rotation (applications/public-records/README.md "Rotating the key"). Empty = this build cannot accept a snapshot.
const PUBKEYS: &[&str] = &[];
const DAY: u64 = 86_400;
/// what a record says, for the screens — the kind words the builder emits
pub const KINDS: &[&str] = &["sanctioned", "ransomware", "scam", "hack", "exchange", "service", "gambling", "market", "mixer", "coinjoin", "miner"];

/// One record against an address: the kind of thing a source says it is, the source's own label, the source, and the
/// day it was listed (empty when the source gives none).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Rec { pub kind: String, pub label: String, pub source: String, pub url: String, pub listed: String, #[serde(skip_serializing_if = "Option::is_none")] pub confidence: Option<String> }

/// A record against an address one of a coin's transactions touched, and where that address stood: `from` (an input
/// of the transaction that created the coin — where it came from), `with` (another output of that transaction — the
/// sender's change or a fellow recipient), `to` (an output of the transaction that spent the coin — where it went).
/// `n` = how many such addresses carried the same record (an exchange's batch withdrawal has dozens); `addr` is one.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Hit { pub at: &'static str, pub addr: String, pub n: u32, #[serde(flatten)] pub rec: Rec }

#[derive(Debug, Clone, Serialize, Default)]
pub struct Status {
    /// the setting (App settings → Public records)
    pub on: bool,
    pub loaded: bool,
    pub built: String,
    pub rows: u64,
    pub sources: u32,
    pub by_kind: BTreeMap<String, u64>,
    /// when the server was last asked (unix seconds); 0 = not this session
    pub checked: u64,
    /// a day or more since the last check, or never checked
    pub stale: bool,
    pub error: String,
    pub base: String,
    pub keys: usize,
}

#[derive(Debug, Deserialize, Clone)]
struct Source { id: String, name: String, url: String, #[serde(default)] licence: String, #[serde(default)] confidence: Option<String> }
#[derive(Deserialize)]
struct Manifest { v: u32, built: String, rows: u64, #[serde(default)] by_kind: BTreeMap<String, u64>, sources: Vec<Source>, files: BTreeMap<String, serde_json::Value> }
#[derive(Deserialize)]
struct Raw { built: String, sources: Vec<Source>, kinds: Vec<String>, rows: Vec<(String, String, String, u16, String)> }

/// one row, 12 bytes: the kind, the label and the listed-day as indexes into the string table, the source index
struct Row { kind: u8, label: u32, src: u16, listed: u32 }
pub struct Snap { built: String, sources: Vec<Source>, kinds: Vec<String>, strings: Vec<String>, rows: Vec<(u64, Row)>, by_kind: BTreeMap<String, u64> }

#[derive(Default)]
struct State { snap: Option<Arc<Snap>>, checked: u64, error: String, tried_cache: bool }
static STATE: Mutex<State> = Mutex::new(State { snap: None, checked: 0, error: String::new(), tried_cache: false });

fn dir() -> PathBuf { let d = crate::profiles::root().join("public-records"); std::fs::create_dir_all(&d).ok(); d }
fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }
fn sha_hex(b: &[u8]) -> String { hex::encode(sha2::Sha256::digest(b)) }
fn http() -> &'static reqwest::blocking::Client {
    static H: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    H.get_or_init(|| reqwest::blocking::Client::builder().user_agent("anytwokeys/0.1").timeout(std::time::Duration::from_secs(120)).build().expect("an HTTP client"))
}
fn get(path: &str, bust: &str) -> Result<Vec<u8>, String> {
    let url = format!("{BASE}{path}?v={bust}");
    let r = http().get(&url).send().map_err(|e| format!("{path}: {e}"))?;
    if !r.status().is_success() { return Err(format!("{path}: HTTP {}", r.status().as_u16())); }
    r.bytes().map(|b| b.to_vec()).map_err(|e| format!("{path}: {e}"))
}

/// The manifest's bytes must carry a signature by one of the pinned keys. ed25519 (pure Rust, `ed25519-dalek`).
fn verify(manifest: &[u8], sig_hex: &str) -> Result<(), String> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    if PUBKEYS.is_empty() { return Err("this build pins no signing key yet, so no snapshot can be accepted".into()); }
    let sig = hex::decode(sig_hex.trim()).map_err(|_| "the signature is not hex")?;
    let sig = Signature::from_slice(&sig).map_err(|_| "the signature is not 64 bytes")?;
    for k in PUBKEYS {
        let Ok(bytes) = hex::decode(k) else { continue };
        let Ok(bytes) = <[u8; 32]>::try_from(bytes.as_slice()) else { continue };
        let Ok(key) = VerifyingKey::from_bytes(&bytes) else { continue };
        if key.verify(manifest, &sig).is_ok() { return Ok(()); }
    }
    Err("the snapshot's manifest is not signed by the pinned key — nothing was kept".into())
}

/// gzip JSON → the compact table. Labels and dates are interned (a few hundred distinct strings over half a million rows).
fn parse(gz: &[u8]) -> Result<Snap, String> {
    let raw: Raw = serde_json::from_reader(flate2::read::GzDecoder::new(gz)).map_err(|e| format!("records file: {e}"))?;
    let mut strings: Vec<String> = Vec::new();
    let mut index: HashMap<String, u32> = HashMap::new();
    let mut intern = |s: &str| -> u32 { if let Some(i) = index.get(s) { return *i; } let i = strings.len() as u32; strings.push(s.to_string()); index.insert(s.to_string(), i); i };
    let mut rows: Vec<(u64, Row)> = Vec::with_capacity(raw.rows.len());
    let mut by_kind: BTreeMap<String, u64> = BTreeMap::new();
    for (h, kind, label, src, listed) in &raw.rows {
        let Ok(key) = u64::from_str_radix(h, 16) else { continue };
        let Some(k) = raw.kinds.iter().position(|x| x == kind) else { continue };
        if *src as usize >= raw.sources.len() { continue; }
        *by_kind.entry(kind.clone()).or_default() += 1;
        rows.push((key, Row { kind: k as u8, label: intern(label), src: *src, listed: intern(listed) }));
    }
    rows.sort_by_key(|(k, _)| *k);
    Ok(Snap { built: raw.built, sources: raw.sources, kinds: raw.kinds, strings, rows, by_kind })
}

fn install(snap: Snap, checked: Option<u64>, error: String) {
    let mut st = STATE.lock().unwrap_or_else(|p| p.into_inner());
    st.snap = Some(Arc::new(snap));
    if let Some(c) = checked { st.checked = c; }
    st.error = error;
}

/// The copy kept under the app's data dir, verified again before it is trusted: a tampered cache is as bad as a
/// tampered download. Ok(false) = nothing cached.
fn load_cached() -> Result<bool, String> {
    let d = dir();
    let (Ok(m), Ok(sig), Ok(gz)) = (std::fs::read(d.join("manifest.json")), std::fs::read_to_string(d.join("manifest.sig")), std::fs::read(d.join("records.json.gz"))) else { return Ok(false) };
    verify(&m, &sig)?;
    let man: Manifest = serde_json::from_slice(&m).map_err(|e| format!("cached manifest: {e}"))?;
    let want = man.files.get("records.json.gz").and_then(|f| f.get("sha256")).and_then(|s| s.as_str()).unwrap_or("");
    if sha_hex(&gz) != want { return Err("the cached records file does not hash to its manifest".into()); }
    let snap = parse(&gz)?;
    install(snap, None, String::new());
    Ok(true)
}

/// The snapshot in memory, loading the cached one on first use. None = none yet (or the setting is off).
pub fn snapshot() -> Option<Arc<Snap>> {
    let tried = { let mut st = STATE.lock().unwrap_or_else(|p| p.into_inner()); if st.snap.is_some() { return st.snap.clone(); } std::mem::replace(&mut st.tried_cache, true) };
    if !tried {
        match load_cached() { Ok(_) => {}, Err(e) => { let mut st = STATE.lock().unwrap_or_else(|p| p.into_inner()); st.error = e; } }
    }
    STATE.lock().unwrap_or_else(|p| p.into_inner()).snap.clone()
}

pub fn status() -> Status {
    let on = crate::wallet::load().map(|s| s.settings.public_records).unwrap_or(true);
    let snap = if on { snapshot() } else { None };
    let st = STATE.lock().unwrap_or_else(|p| p.into_inner());
    let mut out = Status { on, checked: st.checked, stale: st.checked == 0 || now().saturating_sub(st.checked) >= DAY, error: st.error.clone(), base: BASE.into(), keys: PUBKEYS.len(), ..Default::default() };
    if let Some(s) = snap { out.loaded = true; out.built = s.built.clone(); out.rows = s.rows.len() as u64; out.sources = s.sources.len() as u32; out.by_kind = s.by_kind.clone(); }
    out
}

/// Ask the server for today's snapshot: manifest + signature always (16 KB), the records file only when its hash
/// changed. `force` ignores the once-a-day pacing. Respects App settings → Offline and → Public records.
pub fn refresh(force: bool) -> Result<Status, String> {
    let s = crate::wallet::load()?;
    if !s.settings.public_records { return Ok(status()); }
    if s.settings.offline { return Err("the app is offline (App settings); the snapshot was not checked".into()); }
    let checked = STATE.lock().unwrap_or_else(|p| p.into_inner()).checked;
    if !force && checked != 0 && now().saturating_sub(checked) < DAY && snapshot().is_some() { return Ok(status()); }
    let r = (|| -> Result<(), String> {
        let bust = now().to_string();
        let m = get("manifest.json", &bust)?;
        let sig = String::from_utf8(get("manifest.sig", &bust)?).map_err(|_| "the signature file is not text")?;
        verify(&m, &sig)?;
        let man: Manifest = serde_json::from_slice(&m).map_err(|e| format!("manifest: {e}"))?;
        if man.v != 1 { return Err(format!("manifest version {} is newer than this app understands", man.v)); }
        let want = man.files.get("records.json.gz").and_then(|f| f.get("sha256")).and_then(|s| s.as_str()).ok_or("the manifest names no records file")?.to_string();
        let d = dir();
        let cached = std::fs::read(d.join("records.json.gz")).ok().filter(|gz| sha_hex(gz) == want);
        let same_loaded = snapshot().map(|s| s.built == man.built).unwrap_or(false);
        let gz = match cached { Some(gz) => gz, None => { let gz = get("records.json.gz", &want[..16])?; if sha_hex(&gz) != want { return Err("the records file does not hash to what its manifest says — nothing was kept".into()); } gz } };
        if !same_loaded { let snap = parse(&gz)?; if snap.rows.len() as u64 + 1000 < man.rows { return Err("the records file holds far fewer rows than its manifest says".into()); } install(snap, None, String::new()); }
        // keep it: the three files, written beside then renamed, so a half-written file is never read back
        for (name, bytes) in [("manifest.json", m.as_slice()), ("manifest.sig", sig.as_bytes()), ("records.json.gz", gz.as_slice())] {
            let tmp = d.join(format!("{name}.part")); std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, d.join(name))).map_err(|e| format!("keeping {name}: {e}"))?;
        }
        let _ = man.sources; Ok(())
    })();
    let mut st = STATE.lock().unwrap_or_else(|p| p.into_inner());
    st.checked = now();
    st.error = r.err().unwrap_or_default();
    drop(st);
    Ok(status())
}

fn norm(addr: &str) -> String { let a = addr.trim(); if a.len() > 3 && a[..3].eq_ignore_ascii_case("bc1") { a.to_ascii_lowercase() } else { a.to_string() } }
fn key_of(addr: &str) -> u64 { let h = sha2::Sha256::digest(norm(addr).as_bytes()); u64::from_be_bytes(h[..8].try_into().expect("8 bytes")) }

impl Snap {
    fn rec(&self, r: &Row) -> Rec {
        let s = &self.sources[r.src as usize];
        Rec { kind: self.kinds[r.kind as usize].clone(), label: self.strings[r.label as usize].clone(), source: s.name.clone(), url: s.url.clone(), listed: self.strings[r.listed as usize].clone(), confidence: s.confidence.clone() }
    }
    pub fn lookup(&self, addr: &str) -> Vec<Rec> {
        let k = key_of(addr);
        let lo = self.rows.partition_point(|(x, _)| *x < k);
        self.rows[lo..].iter().take_while(|(x, _)| *x == k).map(|(_, r)| self.rec(r)).collect()
    }
}

/// The records against one address: empty when none, or when no snapshot is loaded.
pub fn lookup(addr: &str) -> Vec<Rec> { snapshot().map(|s| s.lookup(addr)).unwrap_or_default() }

/// Give every derived coin the records against the addresses its transactions touched (see `Hit`). Same record on
/// many addresses of one transaction = one hit with `n`; at most 12 hits a coin, the sanctions ones first.
pub fn attach(txs: &[crate::provenance::Tx], coins: &mut HashMap<String, crate::provenance::Derived>) {
    let Some(snap) = snapshot() else { return };
    let rank = |k: &str| KINDS.iter().position(|x| *x == k).unwrap_or(KINDS.len());
    let mut hits: HashMap<String, Vec<Hit>> = HashMap::new();
    let mut push = |op: &str, at: &'static str, addr: &str| {
        for rec in snap.lookup(addr) {
            let v = hits.entry(op.to_string()).or_default();
            if let Some(h) = v.iter_mut().find(|h| h.at == at && h.rec.kind == rec.kind && h.rec.label == rec.label && h.rec.source == rec.source) { h.n += 1; continue; }
            v.push(Hit { at, addr: addr.to_string(), n: 1, rec });
        }
    };
    for tx in txs {
        let foreign_out: Vec<&str> = tx.outs.iter().filter(|o| o.vault.is_none()).filter_map(|o| o.addr.as_deref()).collect();
        let foreign_in: Vec<&str> = tx.ins.iter().filter(|i| i.vault.is_none()).filter_map(|i| i.addr.as_deref()).collect();
        for o in tx.outs.iter().filter(|o| o.vault.is_some()) {
            if !coins.contains_key(&o.op) { continue; }
            for a in &foreign_in { push(&o.op, "from", a); }
            for a in &foreign_out { push(&o.op, "with", a); }
        }
        for i in tx.ins.iter().filter(|i| i.vault.is_some()) {
            if !coins.contains_key(&i.op) { continue; }
            for a in &foreign_out { push(&i.op, "to", a); }
        }
    }
    for (op, mut v) in hits {
        v.sort_by_key(|h| (rank(&h.rec.kind), h.at != "from", std::cmp::Reverse(h.n)));
        v.truncate(12);
        if let Some(d) = coins.get_mut(&op) { d.records = v; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snap() -> Snap {
        // two rows: an exchange address and a sanctioned one, as the builder would emit them
        let a1 = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2"; let a2 = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
        let rows = vec![(hex::encode(&sha2::Sha256::digest(a1.as_bytes())[..8]), "exchange".into(), "Bitmex".into(), 0u16, "2022-11-16".into()),
                        (hex::encode(&sha2::Sha256::digest(a2.as_bytes())[..8]), "sanctioned".into(), "OFAC SDN list".into(), 1u16, "2026-11-14".into())];
        let raw = Raw { built: "2026-11-14T00:00:00Z".into(), kinds: KINDS.iter().map(|k| k.to_string()).collect(), rows,
            sources: vec![Source { id: "g".into(), name: "GraphSense".into(), url: "u".into(), licence: "MIT".into(), confidence: Some("service_data".into()) }, Source { id: "ofac".into(), name: "OFAC".into(), url: "u2".into(), licence: String::new(), confidence: None }] };
        let gz = { use std::io::Write; let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()); e.write_all(&serde_json::to_vec(&serde_json::json!({"built": raw.built, "kinds": raw.kinds, "sources": [{"id":"g","name":"GraphSense","url":"u","licence":"MIT","confidence":"service_data"},{"id":"ofac","name":"OFAC","url":"u2"}], "rows": raw.rows})).unwrap()).unwrap(); e.finish().unwrap() };
        parse(&gz).unwrap()
    }
    #[test]
    fn lookup_matches_the_hashed_address_and_lower_cases_bech32() {
        let s = snap();
        assert_eq!(s.lookup("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2").iter().map(|r| (r.kind.as_str(), r.label.as_str(), r.source.as_str(), r.listed.as_str())).collect::<Vec<_>>(), vec![("exchange", "Bitmex", "GraphSense", "2022-11-16")]);
        assert_eq!(s.lookup("BC1QAR0SRRR7XFKVY5L643LYDNW9RE59GTZZWF5MDQ")[0].kind, "sanctioned");
        assert!(s.lookup("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN3").is_empty());
        assert_eq!(s.by_kind.get("exchange"), Some(&1));
    }
    #[test]
    fn a_manifest_needs_a_pinned_key_and_a_real_signature() {
        // whatever the key list holds, a wrong signature never verifies
        assert!(verify(b"{}", &"00".repeat(64)).is_err());
        assert!(verify(b"{}", "zz").is_err());
    }
    #[test]
    fn attach_says_where_the_record_stood() {
        use crate::provenance::{Derived, In, Out, Tx};
        let s = snap(); install(s, Some(1), String::new());
        let ex = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2"; let bad = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
        // t1: funded from two exchange addresses, our coin t1:0, sender's change t1:1 (unknown); t2: we spend t1:0 to the sanctioned address
        let txs = vec![
            Tx { id: "t1".into(), time: 1, ins: vec![In { op: "p:0".into(), sats: 5, vault: None, addr: Some(ex.into()) }, In { op: "p:1".into(), sats: 5, vault: None, addr: Some(ex.into()) }], outs: vec![Out { op: "t1:0".into(), sats: 8, vault: Some("v".into()), addr: None }, Out { op: "t1:1".into(), sats: 1, vault: None, addr: Some("1zzz".into()) }] },
            Tx { id: "t2".into(), time: 2, ins: vec![In { op: "t1:0".into(), sats: 8, vault: Some("v".into()), addr: None }], outs: vec![Out { op: "t2:0".into(), sats: 7, vault: None, addr: Some(bad.into()) }] },
        ];
        let mut coins: HashMap<String, Derived> = HashMap::new(); coins.insert("t1:0".into(), Derived::default());
        attach(&txs, &mut coins);
        let h = &coins["t1:0"].records;
        assert_eq!(h.iter().map(|h| (h.at, h.rec.kind.as_str(), h.n)).collect::<Vec<_>>(), vec![("to", "sanctioned", 1), ("from", "exchange", 2)]);
    }
}
