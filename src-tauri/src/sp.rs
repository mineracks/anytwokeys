//! Silent payments (BIP 352) for a WATCHED wallet: finding the taproot outputs a sender made for it, which no xpub
//! can see. A silent payment address is two public keys; the output key is derived from the SENDER's inputs, so a
//! wallet that only knows its own extended key walks straight past its own money. Finding it needs the wallet's
//! scan private key — a key that can read every payment ever made to it but cannot spend one — and the spend
//! public key.
//!
//! What is kept: the scan key, sealed in its own file under the store's key (never in the store, which is handed
//! to the window whole); the spend public key and what was found, in the vault. What is never kept: the words, the
//! spend private key, the per-output tweaks a spender would need.
//!
//! Where the tweaks come from: a BlindBit oracle (`sp.mineracks.com` is ours), which serves per block the
//! input_hash·A point of every transaction that pays to taproot, pre-multiplied, and 8-byte prefixes of the output
//! keys. One point multiplication per transaction, one prefix compare per output; a prefix hit fetches the block's
//! full outputs and is checked against the whole key. The oracle never sees a key: every client fetches every block.
//!
//! Math checked against the BIP's own test vectors (bip-0352/send_and_receive_test_vectors.json), fetched, not
//! recalled: see the tests at the foot.
use bitcoin::bip32::{ChildNumber, Xpriv, Xpub};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{ecdh, PublicKey, Scalar, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use zeroize::Zeroizing;

type R<T> = Result<T, String>;

pub const PURPOSE: u32 = 352;
/// The BIP's ceiling on outputs one transaction can make for one address; a receiver looks no further.
const K_MAX: u32 = 2323;
/// The point the BIP names as "no key here": a taproot input whose internal key is this is skipped.
const NUMS_H: &str = "50929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac0";

// ---------- hashes ----------
/// BIP 340 tagged hash: sha256(sha256(tag) ‖ sha256(tag) ‖ data).
fn tagged(tag: &str, data: &[u8]) -> [u8; 32] {
    let t = sha256::Hash::hash(tag.as_bytes());
    let mut e = sha256::Hash::engine();
    use bitcoin::hashes::HashEngine;
    e.input(t.as_byte_array()); e.input(t.as_byte_array()); e.input(data);
    sha256::Hash::from_engine(e).to_byte_array()
}
fn scalar(bytes: [u8; 32], what: &str) -> R<Scalar> { Scalar::from_be_bytes(bytes).map_err(|_| format!("{what} is not a valid scalar")) }

// ---------- keys ----------
/// The scan private key, erased when dropped.
pub struct ScanKey(SecretKey);
impl Drop for ScanKey { fn drop(&mut self) { self.0.non_secure_erase(); } }
impl ScanKey {
    pub fn from_bytes(b: &[u8]) -> R<Self> { SecretKey::from_slice(b).map(ScanKey).map_err(|_| "the scan key is damaged".to_string()) }
    pub fn public(&self) -> PublicKey { self.0.public_key(&Secp256k1::new()) }
    pub fn bytes(&self) -> Zeroizing<[u8; 32]> { Zeroizing::new(self.0.secret_bytes()) }
}
struct Root(Xpriv);
impl Drop for Root { fn drop(&mut self) { self.0.private_key.non_secure_erase(); self.0.chain_code = [0u8; 32].into(); } }

/// Everything the words give that is kept or shown: the scan key, both public keys, and the wallet's ordinary
/// BIP 84 account key — which is how the words are proved to be THIS wallet's, by matching the key it exports.
pub struct Derived { pub scan: ScanKey, pub scan_pub: PublicKey, pub spend_pub: PublicKey, pub account84: Xpub }

pub fn derive(words: &str, mainnet: bool) -> R<Derived> {
    let mut norm = Zeroizing::new(String::new());
    for w in words.split_whitespace() { if !norm.is_empty() { norm.push(' '); } for c in w.chars() { norm.push(c.to_ascii_lowercase()); } }
    let mnemonic = bip39::Mnemonic::parse_normalized(&norm).map_err(|_| "check the seed words and their checksum")?;
    let seed = Zeroizing::new(mnemonic.to_seed(""));
    drop(mnemonic);
    let net = if mainnet { bitcoin::Network::Bitcoin } else { bitcoin::Network::Testnet };
    let master = Root(Xpriv::new_master(net, &seed[..]).map_err(|_| "could not derive the wallet")?);
    drop(seed);
    let secp = Secp256k1::new();
    let h = |i: u32| ChildNumber::from_hardened_idx(i).map_err(|_| "invalid path".to_string());
    let coin = if mainnet { 0 } else { 1 };
    let walk = |path: Vec<ChildNumber>| -> R<Root> { Ok(Root(master.0.derive_priv(&secp, &path).map_err(|_| "could not derive the wallet")?)) };
    let scan = walk(vec![h(PURPOSE)?, h(coin)?, h(0)?, h(1)?, ChildNumber::from_normal_idx(0).unwrap()])?;
    let spend = walk(vec![h(PURPOSE)?, h(coin)?, h(0)?, h(0)?, ChildNumber::from_normal_idx(0).unwrap()])?;
    let acct = walk(vec![h(84)?, h(coin)?, h(0)?])?;
    let scan_key = ScanKey(scan.0.private_key);
    let out = Derived { scan_pub: scan_key.public(), spend_pub: spend.0.private_key.public_key(&secp), account84: Xpub::from_priv(&secp, &acct.0), scan: scan_key };
    drop(scan); drop(spend); drop(acct);   // every private key and chain code erased; the master goes with them
    Ok(out)
}

/// The `sp1…` address: bech32m, version 0, B_scan ‖ B_spend.
pub fn address(scan_pub: &PublicKey, spend_pub: &PublicKey, mainnet: bool) -> R<String> {
    use bech32::primitives::encode::Encoder;
    use bech32::{Bech32m, Fe32, Hrp};
    let mut data = scan_pub.serialize().to_vec(); data.extend_from_slice(&spend_pub.serialize());
    let hrp = Hrp::parse(if mainnet { "sp" } else { "tsp" }).map_err(|e| e.to_string())?;
    let fes = bech32::primitives::iter::ByteIterExt::bytes_to_fes(data.iter().copied());
    Ok(Encoder::<_, Bech32m>::new(fes, &hrp).with_witness_version(Fe32::Q).chars().collect())
}

// ---------- the shared secret and what it points at ----------
fn point_of(scan: &SecretKey, tweak: &PublicKey) -> R<PublicKey> {
    let xy = ecdh::shared_secret_point(tweak, scan);
    let mut ser = [4u8; 65]; ser[1..].copy_from_slice(&xy);
    PublicKey::from_slice(&ser).map_err(|_| "shared secret is not a point".to_string())
}
fn t_k(shared: &PublicKey, k: u32) -> R<Scalar> {
    let mut d = shared.serialize().to_vec(); d.extend_from_slice(&k.to_be_bytes());
    scalar(tagged("BIP0352/SharedSecret", &d), "t_k")
}
/// A label the wallet uses; m = 0 is the change label every BIP 352 wallet has.
pub fn label(scan: &ScanKey, m: u32) -> R<Scalar> {
    let mut d = Zeroizing::new(scan.0.secret_bytes().to_vec()); d.extend_from_slice(&m.to_be_bytes());
    scalar(tagged("BIP0352/Label", &d), "label")
}
/// input_hash = hash(outpoint_L ‖ A): what the oracle multiplies in for us, computed here for the vectors and for
/// any block the app ever reads for itself.
pub fn input_tweak(secp: &Secp256k1<bitcoin::secp256k1::All>, outpoints: &[(bitcoin::Txid, u32)], keys: &[PublicKey]) -> R<Option<PublicKey>> {
    if keys.is_empty() { return Ok(None); }
    let a = match PublicKey::combine_keys(&keys.iter().collect::<Vec<_>>()) { Ok(a) => a, Err(_) => return Ok(None) };   // infinity: skip the transaction
    let mut sers: Vec<[u8; 36]> = outpoints.iter().map(|(t, v)| { let mut o = [0u8; 36]; o[..32].copy_from_slice(t.as_byte_array()); o[32..].copy_from_slice(&v.to_le_bytes()); o }).collect();
    sers.sort();
    let mut d = sers[0].to_vec(); d.extend_from_slice(&a.serialize());
    let h = scalar(tagged("BIP0352/Inputs", &d), "input_hash")?;
    Ok(Some(a.mul_tweak(secp, &h).map_err(|_| "input tweak failed")?))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit { pub key: [u8; 32], pub k: u32, pub label: Option<u32>, pub tweak: [u8; 32] }

/// The BIP's receiving loop for one transaction: k = 0, 1, … while something is found at k. An output matches P_k
/// outright, or P_k plus one of the wallet's labels — and since outputs are x-only, comparing x(P_k + L) covers the
/// negated case the BIP asks for as well. `outputs` may be whole keys or prefixes: a candidate matches when the
/// output is a prefix of it, so the oracle's 8-byte index can be scanned first and a hit confirmed on the full key.
pub fn matches(secp: &Secp256k1<bitcoin::secp256k1::All>, scan: &ScanKey, spend_pub: &PublicKey, tweak: &PublicKey, outputs: &[Vec<u8>], labels: &[(u32, Scalar)]) -> R<Vec<Hit>> {
    let shared = point_of(&scan.0, tweak)?;
    let mut left: Vec<&Vec<u8>> = outputs.iter().filter(|o| !o.is_empty() && o.len() <= 32).collect();
    let mut hits = Vec::new();
    let mut k = 0u32;
    while !left.is_empty() && k <= K_MAX {
        let t = t_k(&shared, k)?;
        let pk = spend_pub.add_exp_tweak(secp, &t).map_err(|_| "output key derivation failed")?;
        let mut candidates: Vec<([u8; 32], Option<u32>, [u8; 32])> = vec![(pk.x_only_public_key().0.serialize(), None, t.to_be_bytes())];
        for (m, l) in labels {
            let q = pk.add_exp_tweak(secp, l).map_err(|_| "label derivation failed")?;
            let mut tl = SecretKey::from_slice(&t.to_be_bytes()).map_err(|_| "t_k is not a key")?;
            tl = tl.add_tweak(l).map_err(|_| "label tweak failed")?;
            candidates.push((q.x_only_public_key().0.serialize(), Some(*m), tl.secret_bytes()));
        }
        let before = hits.len();
        left.retain(|o| {
            if let Some((key, m, tw)) = candidates.iter().find(|(key, _, _)| key.starts_with(o)) { hits.push(Hit { key: *key, k, label: *m, tweak: *tw }); false } else { true }
        });
        if hits.len() == before { break; }
        k += 1;
    }
    Ok(hits)
}

// ---------- the oracle ----------
#[derive(Deserialize)] struct Info { network: String, height: u32 }
#[derive(Deserialize)] struct Indexed { txid: String, tweak: String, #[serde(default)] outputs: Vec<String> }
#[derive(Deserialize)] struct Utxo { txid: String, vout: u32, amount: u64, pubkey: String }
#[derive(Deserialize)] struct Page<T> { #[serde(default = "Vec::new")] index: Vec<T> }

pub struct Oracle { pub base: String }
impl Oracle {
    fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> R<T> {
        crate::net::out("the silent payments oracle")?;
        static H: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
        let http = H.get_or_init(|| reqwest::blocking::Client::builder().user_agent("anytwokeys/0.1").timeout(std::time::Duration::from_secs(30)).build().expect("an HTTP client"));
        let url = format!("{}/{path}", self.base.trim_end_matches('/'));
        let mut last = String::new();
        for attempt in 0..3 {
            if attempt > 0 { std::thread::sleep(std::time::Duration::from_millis(400 * attempt as u64)); }
            match http.get(&url).send() {
                Ok(r) if r.status().is_success() => return r.json::<T>().map_err(|e| format!("the oracle's answer for {path} could not be read: {e}")),
                Ok(r) => last = format!("the oracle answered HTTP {} for {path}", r.status().as_u16()),
                Err(e) => last = format!("the oracle could not be reached ({})", if e.is_timeout() { "timed out".to_string() } else { "connection failed".to_string() }),
            }
        }
        Err(last)
    }
    /// (network, indexed height) — the oracle says how far IT has got, which may be behind the chain.
    pub fn info(&self) -> R<(String, u32)> { let i: Info = self.get("info")?; Ok((i.network, i.height)) }
}

// ---------- the scan ----------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Found { pub key: String, pub height: u32, pub txid: String, pub vout: u32, pub value: u64, #[serde(default)] pub label: Option<u32> }

#[derive(Debug, Serialize)]
pub struct Scan { pub found: Vec<Found>, pub reached: u32, pub error: Option<String> }

static CANCEL: AtomicU64 = AtomicU64::new(0);
pub fn cancel() { CANCEL.fetch_add(1, Ordering::SeqCst); }

/// Blocks `from..=to`, in chunks so that `reached` is always a height every block up to which was read: a chunk
/// that fails leaves the ones before it counted and is picked up next time. Six fetches at once, since the oracle
/// round trip is what the scan waits on; the arithmetic is nothing beside it.
pub fn discover(oracle: &Oracle, scan: &ScanKey, spend_pub: &PublicKey, from: u32, to: u32, labels: &[(u32, Scalar)], vault: &str) -> Scan {
    let secp = Secp256k1::new();
    let gen = CANCEL.load(Ordering::SeqCst);
    let mut out = Scan { found: Vec::new(), reached: from.saturating_sub(1), error: None };
    if from > to { return out; }
    const CHUNK: u32 = 96;
    let mut start = from;
    while start <= to {
        let end = (start + CHUNK - 1).min(to);
        let next = AtomicU32::new(start);
        let found: Mutex<Vec<Found>> = Mutex::new(Vec::new());
        let failed: Mutex<Option<String>> = Mutex::new(None);
        std::thread::scope(|s| {
            for _ in 0..6 {
                s.spawn(|| loop {
                    if CANCEL.load(Ordering::SeqCst) != gen || failed.lock().map(|f| f.is_some()).unwrap_or(true) { break; }
                    let h = next.fetch_add(1, Ordering::SeqCst);
                    if h > end { break; }
                    match block(oracle, &secp, scan, spend_pub, labels, h) {
                        Ok(f) => { if !f.is_empty() { if let Ok(mut g) = found.lock() { g.extend(f); } } }
                        Err(e) => { if let Ok(mut g) = failed.lock() { if g.is_none() { *g = Some(format!("block {h}: {e}")); } } }
                    }
                });
            }
        });
        let f = failed.into_inner().unwrap_or(None);
        if CANCEL.load(Ordering::SeqCst) != gen { out.error = Some("scan cancelled".into()); return out; }
        if let Some(e) = f { out.error = Some(e); return out; }
        out.found.extend(found.into_inner().unwrap_or_default());
        out.reached = end;
        crate::net::emit("sp-scan", serde_json::json!({ "vault": vault, "at": end, "from": from, "to": to, "found": out.found.len() }));
        start = end + 1;
    }
    out.found.sort_by_key(|f| (f.height, f.txid.clone(), f.vout));
    out
}

fn block(oracle: &Oracle, secp: &Secp256k1<bitcoin::secp256k1::All>, scan: &ScanKey, spend_pub: &PublicKey, labels: &[(u32, Scalar)], h: u32) -> R<Vec<Found>> {
    let page: Page<Indexed> = oracle.get(&format!("compute-index/{h}"))?;
    let mut hits: Vec<(String, Vec<Hit>)> = Vec::new();
    for tx in &page.index {
        let tweak = match hex::decode(&tx.tweak).ok().and_then(|b| PublicKey::from_slice(&b).ok()) { Some(t) => t, None => continue };
        let outputs: Vec<Vec<u8>> = tx.outputs.iter().filter_map(|o| hex::decode(o).ok()).collect();
        let found = matches(secp, scan, spend_pub, &tweak, &outputs, labels)?;
        if !found.is_empty() { hits.push((tx.txid.clone(), found)); }
    }
    if hits.is_empty() { return Ok(Vec::new()); }
    // a prefix hit is confirmed on the whole key, and given its outpoint and value
    let utxos: Page<Utxo> = oracle.get(&format!("utxos/{h}"))?;
    let mut out = Vec::new();
    for (txid, list) in hits {
        for hit in list {
            let key = hex::encode(hit.key);
            if let Some(u) = utxos.index.iter().find(|u| u.txid == txid && u.pubkey == key) {
                out.push(Found { key, height: h, txid: u.txid.clone(), vout: u.vout, value: u.amount, label: hit.label });
            }
        }
    }
    Ok(out)
}

// ---------- the sealed scan key ----------
fn key_path(id: &str) -> std::path::PathBuf { crate::lockbox::dir().join(format!("sp-{id}.key")) }
/// An id for the wallet that says nothing secret: the first bytes of a hash of its spend public key.
pub fn id_of(spend_pub_hex: &str) -> String { hex::encode(&sha256::Hash::hash(spend_pub_hex.as_bytes()).to_byte_array()[..8]) }
pub fn keep_scan_key(id: &str, scan: &ScanKey) -> R<()> {
    if !crate::lockbox::sealed_mode() { return Err("silent payments need this computer's sealed store: seal it in App settings first, so the scan key is never written in the clear".into()); }
    let b = scan.bytes();
    crate::lockbox::write_sealed(&key_path(id), "sp-scan", &b[..])
}
pub fn scan_key(id: &str) -> R<ScanKey> {
    let b = crate::lockbox::read_sealed(&key_path(id), "sp-scan")?.ok_or("this wallet's scan key is not on this computer; enter the words again to watch its silent payments")?;
    let b = Zeroizing::new(b);
    ScanKey::from_bytes(&b)
}
pub fn forget_scan_key(id: &str) { let _ = std::fs::remove_file(key_path(id)); }

// ---------- reading a transaction's inputs for ourselves (the vectors, and any block read without the oracle) ----------
/// The public key an input contributes to A, by the BIP's rules, or None when it contributes nothing.
pub fn input_key(prev_spk: &[u8], script_sig: &[u8], witness: &[Vec<u8>]) -> Option<PublicKey> {
    let compressed = |b: &[u8]| if b.len() == 33 && (b[0] == 2 || b[0] == 3) { PublicKey::from_slice(b).ok() } else { None };
    // P2PKH: the key is wherever it is in the scriptSig — the last 33 bytes that hash to the address, scanning back
    if prev_spk.len() == 25 && prev_spk[0] == 0x76 && prev_spk[1] == 0xa9 && prev_spk[2] == 0x14 && prev_spk[23] == 0x88 && prev_spk[24] == 0xac {
        let pkh = &prev_spk[3..23];
        let mut i = script_sig.len();
        while i >= 33 {
            let c = &script_sig[i - 33..i];
            if bitcoin::hashes::hash160::Hash::hash(c).as_byte_array() == pkh { return compressed(c); }
            i -= 1;
        }
        return None;
    }
    // P2SH-P2WPKH: redeem script 0014{20} in the scriptSig, key last in the witness
    if prev_spk.len() == 23 && prev_spk[0] == 0xa9 && prev_spk[1] == 0x14 && prev_spk[22] == 0x87 {
        if script_sig.len() == 23 && script_sig[0] == 0x16 && script_sig[1] == 0x00 && script_sig[2] == 0x14 { return witness.last().and_then(|w| compressed(w)); }
        return None;
    }
    // P2WPKH: key last in the witness
    if prev_spk.len() == 22 && prev_spk[0] == 0x00 && prev_spk[1] == 0x14 { return witness.last().and_then(|w| compressed(w)); }
    // P2TR: the output key itself, unless a script path shows the NUMS internal key
    if prev_spk.len() == 34 && prev_spk[0] == 0x51 && prev_spk[1] == 0x20 {
        let mut w: Vec<&Vec<u8>> = witness.iter().collect();
        if w.len() > 1 && w.last().map(|a| a.first() == Some(&0x50)).unwrap_or(false) { w.pop(); }
        if w.len() > 1 {
            let control = w.last().unwrap();
            if control.len() >= 33 && hex::encode(&control[1..33]) == NUMS_H { return None; }
        }
        let mut ser = vec![2u8]; ser.extend_from_slice(&prev_spk[2..34]);
        return PublicKey::from_slice(&ser).ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // bip-0352/send_and_receive_test_vectors.json, fetched 2026-09-25 from bitcoin/bips master
    const VECTORS: &str = include_str!("../../test/fixtures/bip352-vectors.json");

    fn witness_items(hex_str: &str) -> Vec<Vec<u8>> {
        let b = hex::decode(hex_str).unwrap_or_default();
        if b.is_empty() { return Vec::new(); }
        let (mut i, mut items) = (0usize, Vec::new());
        let n = b[i] as usize; i += 1;
        for _ in 0..n {
            let len = b[i] as usize; i += 1;   // the vectors' witnesses are all short
            items.push(b[i..i + len].to_vec()); i += len;
        }
        items
    }

    #[test]
    fn every_receiving_vector_is_found_with_its_tweak_and_its_label() {
        let v: serde_json::Value = serde_json::from_str(VECTORS).unwrap();
        let secp = Secp256k1::new();
        let (mut cases, mut found_total) = (0, 0);
        for case in v.as_array().unwrap() {
            for r in case["receiving"].as_array().unwrap() {
                // one vector exceeds K_max: sending fails there, and a receiver does not look past the limit
                if r["expected"]["outputs"].as_array().is_none() { continue; }
                cases += 1;
                let g = &r["given"];
                let scan = ScanKey(SecretKey::from_str(g["key_material"]["scan_priv_key"].as_str().unwrap()).unwrap());
                let spend = SecretKey::from_str(g["key_material"]["spend_priv_key"].as_str().unwrap()).unwrap();
                let spend_pub = spend.public_key(&secp);
                let labels: Vec<(u32, Scalar)> = g["labels"].as_array().unwrap().iter().map(|m| { let m = m.as_u64().unwrap() as u32; (m, label(&scan, m).unwrap()) }).collect();
                // the address, with the wallet's labels applied to B_spend as the BIP does for each
                let mut addresses = vec![address(&scan.public(), &spend_pub, true).unwrap()];
                for (_, l) in &labels { addresses.push(address(&scan.public(), &spend_pub.add_exp_tweak(&secp, l).unwrap(), true).unwrap()); }
                let expected_addr: Vec<String> = r["expected"]["addresses"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect();
                assert_eq!(addresses, expected_addr, "{}", case["comment"]);
                // the inputs
                let (mut keys, mut outpoints) = (Vec::new(), Vec::new());
                for vin in g["vin"].as_array().unwrap() {
                    let spk = hex::decode(vin["prevout"]["scriptPubKey"]["hex"].as_str().unwrap()).unwrap();
                    let sig = hex::decode(vin["scriptSig"].as_str().unwrap_or("")).unwrap();
                    let wit = witness_items(vin["txinwitness"].as_str().unwrap_or(""));
                    outpoints.push((bitcoin::Txid::from_str(vin["txid"].as_str().unwrap()).unwrap(), vin["vout"].as_u64().unwrap() as u32));
                    if let Some(k) = input_key(&spk, &sig, &wit) { keys.push(k); }
                }
                let outputs: Vec<Vec<u8>> = g["outputs"].as_array().unwrap().iter().map(|o| hex::decode(o.as_str().unwrap()).unwrap()).collect();
                let expected: Vec<(String, String)> = r["expected"]["outputs"].as_array().unwrap().iter().map(|o| (o["pub_key"].as_str().unwrap().to_string(), o["priv_key_tweak"].as_str().unwrap().to_string())).collect();
                let hits = match input_tweak(&secp, &outpoints, &keys).unwrap() { Some(t) => matches(&secp, &scan, &spend_pub, &t, &outputs, &labels).unwrap(), None => Vec::new() };
                let mut got: Vec<(String, String)> = hits.iter().map(|h| (hex::encode(h.key), hex::encode(h.tweak))).collect();
                let mut want = expected.clone();
                got.sort(); want.sort();
                assert_eq!(got, want, "{}", case["comment"]);
                found_total += hits.len();
            }
        }
        assert!(cases >= 20 && found_total >= 10, "the vectors were read: {cases} cases, {found_total} outputs");
    }

    #[test]
    fn a_prefix_finds_what_the_whole_key_finds_and_nothing_else() {
        let secp = Secp256k1::new();
        let scan = ScanKey(SecretKey::from_slice(&[7u8; 32]).unwrap());
        let spend_pub = SecretKey::from_slice(&[9u8; 32]).unwrap().public_key(&secp);
        let tweak = SecretKey::from_slice(&[11u8; 32]).unwrap().public_key(&secp);
        let shared = point_of(&scan.0, &tweak).unwrap();
        let p0 = spend_pub.add_exp_tweak(&secp, &t_k(&shared, 0).unwrap()).unwrap().x_only_public_key().0.serialize();
        let full = matches(&secp, &scan, &spend_pub, &tweak, &[p0.to_vec(), vec![1u8; 32]], &[]).unwrap();
        let short = matches(&secp, &scan, &spend_pub, &tweak, &[p0[..8].to_vec(), vec![1u8; 8]], &[]).unwrap();
        assert_eq!(full.len(), 1); assert_eq!(full, short);
        assert_eq!(matches(&secp, &scan, &spend_pub, &tweak, &[vec![1u8; 32]], &[]).unwrap(), Vec::<Hit>::new());
    }

    #[test]
    fn the_words_give_the_bip_paths_and_the_account_key() {
        // BIP 39 "abandon…about": the BIP 84 account key is the published one; the scan and spend keys are hardened
        // children of purpose 352, so the address is stable for this phrase
        let d = derive("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about", true).unwrap();
        assert_eq!(d.account84.to_string(), "xpub6CatWdiZiodmUeTDp8LT5or8nmbKNcuyvz7WyksVFkKB4RHwCD3XyuvPEbvqAQY3rAPshWcMLoP2fMFMKHPJ4ZeZXYVUhLv1VMrjPC7PW6V");
        let a = address(&d.scan_pub, &d.spend_pub, true).unwrap();
        assert!(a.starts_with("sp1q") && a.len() == 116, "{a}");
        assert!(derive("abandon abandon zoo", true).is_err());
    }

    #[test]
    fn a_taproot_input_with_the_nums_key_contributes_nothing() {
        let spk = hex::decode("512079be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798").unwrap();   // x of G: a real point
        let mut control = vec![0xc0u8]; control.extend(hex::decode(NUMS_H).unwrap());
        assert!(input_key(&spk, &[], &[vec![1, 2, 3], control.clone()]).is_none());
        let mut other = vec![0xc0u8]; other.extend([0x22u8; 32]);
        assert!(input_key(&spk, &[], &[vec![1, 2, 3], other]).is_some());
        assert!(input_key(&spk, &[], &[vec![1, 2, 3]]).is_some());   // key path: the output key counts
    }
}
