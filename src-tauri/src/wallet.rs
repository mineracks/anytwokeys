//! Vaults: fixed-key descriptors over card keys, the store, building and finalising spends.
use bitcoin::hashes::Hash;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{ecdsa, PublicKey, Secp256k1};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::{absolute, transaction, Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
use miniscript::descriptor::Descriptor;
use miniscript::psbt::PsbtExt;
use miniscript::DescriptorPublicKey;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::chain::{self, Servers};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRef { pub name: String, pub serial: u32, pub pubkey: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vault {
    pub name: String,
    pub network: String,       // "mainnet" | "signet"
    pub threshold: usize,
    pub keys: Vec<KeyRef>,
    pub descriptor: String,
    pub address: String,
    pub created: u64,
    #[serde(default)] pub chain_code: Option<String>,   // hex; present = rotating addresses (hd.rs)
    #[serde(default)] pub next: u32,                     // first unused index, from the last scan
    #[serde(default)] pub scan_max: u32,                 // highest index the last scan looked at
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub network: String,
    pub custom: bool,
    pub electrum: String,
    pub explorer: String,
}

/// What the watcher last saw for a vault: txid → height, and the coins it held.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Seen { pub txs: std::collections::HashMap<String, i32>, pub coins: std::collections::HashMap<String, u64>, pub primed: bool }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Store {
    pub vaults: Vec<Vault>,
    pub settings: Settings,
    pub last_psbt: Option<String>,
    #[serde(default)] pub seen: std::collections::HashMap<String, Seen>,
}

pub fn network_of(s: &str) -> Network {
    if s == "mainnet" { Network::Bitcoin } else { Network::Signet }
}

pub fn store_path() -> std::path::PathBuf {
    let dir = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("Any Two Keys");
    std::fs::create_dir_all(&dir).ok();
    dir.join("store.json")
}

pub fn load() -> Store {
    let mut s: Store = std::fs::read_to_string(store_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if s.settings.network.is_empty() { s.settings.network = "signet".into(); }
    s
}

pub fn save(s: &Store) -> Result<(), String> {
    let p = store_path();
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

/// The servers for a network: the user's own when they set some for the network Settings is
/// on, otherwise ours for that chain. An Electrum server serves one chain, so the custom pair
/// only applies to the network it was entered under.
pub fn servers_for(s: &Store, network: &str) -> Servers {
    if s.settings.custom && !s.settings.electrum.is_empty() && s.settings.network == network {
        Servers { electrum: s.settings.electrum.clone(), explorer: if s.settings.explorer.is_empty() { chain::defaults(network_of(network)).explorer } else { s.settings.explorer.clone() } }
    } else {
        chain::defaults(network_of(network))
    }
}

pub fn servers(s: &Store) -> Servers { servers_for(s, &s.settings.network) }

/// The descriptor for k of the given compressed pubkeys (hex). One key → wpkh; more → wsh(sortedmulti).
pub fn descriptor_for(threshold: usize, pubkeys: &[String]) -> Result<String, String> {
    if pubkeys.is_empty() { return Err("no keys".into()); }
    if threshold < 1 || threshold > pubkeys.len() { return Err("threshold out of range".into()); }
    for p in pubkeys { PublicKey::from_str(p).map_err(|_| format!("not a public key: {p}"))?; }
    let body = if pubkeys.len() == 1 { format!("wpkh({})", pubkeys[0]) } else { format!("wsh(sortedmulti({},{}))", threshold, pubkeys.join(",")) };
    let d = Descriptor::<DescriptorPublicKey>::from_str(&body).map_err(|e| format!("descriptor: {e}"))?;
    Ok(d.to_string())   // carries the checksum
}

pub fn parse(desc: &str) -> Result<Descriptor<DescriptorPublicKey>, String> {
    Descriptor::<DescriptorPublicKey>::from_str(desc.trim()).map_err(|e| format!("descriptor: {e}"))
}

pub fn address_of(desc: &str, network: Network) -> Result<Address, String> {
    let d = parse(desc)?;
    let d = d.at_derivation_index(0).map_err(|e| e.to_string())?;
    d.address(network).map_err(|e| e.to_string())
}

pub fn script_of(desc: &str) -> Result<ScriptBuf, String> {
    let d = parse(desc)?.at_derivation_index(0).map_err(|e| e.to_string())?;
    Ok(d.script_pubkey())
}

/// Keys named in a descriptor (compressed hex), and its threshold.
pub fn keys_of(desc: &str) -> Result<(usize, Vec<String>), String> {
    let s = desc.trim();
    let inner = s.split('#').next().unwrap_or(s);
    if let Some(rest) = inner.strip_prefix("wsh(sortedmulti(").or_else(|| inner.strip_prefix("wsh(multi(")) {
        let rest = rest.trim_end_matches(')');
        let mut parts = rest.split(',');
        let k: usize = parts.next().unwrap_or("0").trim().parse().map_err(|_| "threshold".to_string())?;
        let keys: Vec<String> = parts.map(|p| p.trim().to_lowercase()).collect();
        return Ok((k, keys));
    }
    if let Some(rest) = inner.strip_prefix("wpkh(") {
        return Ok((1, vec![rest.trim_end_matches(')').trim().to_lowercase()]));
    }
    Err("only wpkh(key) and wsh(sortedmulti(k,keys…)) over fixed keys are supported".into())
}

pub const GAP: u32 = 20;

pub fn cc_of(v: &Vault) -> Option<[u8; 32]> {
    let h = v.chain_code.as_ref()?;
    let b = hex::decode(h).ok()?;
    b.try_into().ok()
}

/// The cosigner keys for address `index`: the card keys, or their multiplicative children.
pub fn keys_at(v: &Vault, index: u32) -> Result<Vec<String>, String> {
    match cc_of(v) {
        None => Ok(v.keys.iter().map(|k| k.pubkey.clone()).collect()),
        Some(cc) => v.keys.iter().map(|k| {
            let q = PublicKey::from_str(&k.pubkey).map_err(|e| e.to_string())?;
            Ok(hex::encode(crate::hd::child_pubkey(&cc, &q, index).serialize()))
        }).collect(),
    }
}

pub fn descriptor_at(v: &Vault, index: u32) -> Result<String, String> {
    descriptor_for(v.threshold, &keys_at(v, index)?)
}

pub fn address_at(v: &Vault, index: u32) -> Result<String, String> {
    Ok(address_of(&descriptor_at(v, index)?, network_of(&v.network))?.to_string())
}

/// The recovery line: the base descriptor, plus the chain code when addresses rotate.
pub fn recovery_line(v: &Vault) -> String {
    match &v.chain_code {
        Some(cc) => format!("{} cc={cc}", v.descriptor),
        None => v.descriptor.clone(),
    }
}

/// "wsh(sortedmulti(…))#chk cc=<hex>" → (descriptor text, chain code)
pub fn split_recovery(text: &str) -> (String, Option<String>) {
    let t = text.trim();
    if let Some((d, cc)) = t.split_once(" cc=") {
        let cc = cc.trim().to_lowercase();
        if cc.len() == 64 && cc.chars().all(|c| c.is_ascii_hexdigit()) {
            return (d.trim().to_string(), Some(cc));
        }
    }
    (t.to_string(), None)
}

#[derive(Debug, Clone, Serialize)]
pub struct CoinAt { pub txid: String, pub vout: u32, pub value: u64, pub height: u32, pub index: u32 }

#[derive(Debug, Clone, Serialize)]
pub struct Scanned { pub coins: Vec<CoinAt>, pub history: Vec<(String, i32, u32)>, pub next: u32, pub scan_max: u32 }

/// Every index up to GAP past the last one with history. A fixed vault has index 0 only.
pub fn scan(v: &Vault, servers: &Servers) -> Result<Scanned, String> {
    let mut coins = Vec::new();
    let mut history = Vec::new();
    let rotating = cc_of(v).is_some();
    let mut last_used: Option<u32> = None;
    let mut next: Option<u32> = None;
    let mut i = 0u32;
    loop {
        let spk = script_of(&descriptor_at(v, i)?)?;
        let h = chain::history(servers, &spk).unwrap_or_default();
        if !h.is_empty() {
            last_used = Some(i);
            for item in h { history.push((item.txid, item.height, i)); }
            for c in chain::coins(servers, &spk)? { coins.push(CoinAt { txid: c.txid, vout: c.vout, value: c.value, height: c.height, index: i }); }
        } else if next.is_none() {
            next = Some(i);
        }
        if !rotating { break; }
        if i >= last_used.map(|u| u + GAP).unwrap_or(GAP - 1) { break; }
        i += 1;
    }
    Ok(Scanned { coins, history, next: next.unwrap_or(i + 1), scan_max: i })
}

/// Which index a PSBT input pays to, by matching its script against the vault's.
pub fn index_of_input(v: &Vault, spk: &ScriptBuf, upto: u32) -> Result<Option<u32>, String> {
    let max = if cc_of(v).is_some() { upto + GAP } else { 0 };
    for i in 0..=max {
        if &script_of(&descriptor_at(v, i)?)? == spk { return Ok(Some(i)); }
    }
    Ok(None)
}

fn in_vbytes(threshold: usize, n: usize) -> u64 {
    if n == 1 { return 68; }
    let script_len = 1 + 34 * n + 2;
    let witness = 1 + 1 + threshold * 73 + (if script_len < 76 { 1 } else { 2 }) + script_len;
    41 + (witness as u64 + 3) / 4
}

#[derive(Debug, Clone, Serialize)]
pub struct Built {
    pub psbt: String,
    pub fee: u64,
    pub vsize: u64,
    pub change: u64,
    pub inputs: usize,
}

/// Spend `amount` sats to `to` from the vault's coins at `rate` sat/vB; change to a fresh address.
pub fn build(vault: &Vault, coins: &[CoinAt], change_index: u32, to: &str, amount: u64, rate: f64, send_all: bool) -> Result<Built, String> {
    let network = network_of(&vault.network);
    let to_addr = Address::from_str(to).map_err(|e| format!("address: {e}"))?.require_network(network).map_err(|_| "that address is for another network".to_string())?;
    let n = vault.keys.len();
    let in_vb = in_vbytes(vault.threshold, n);
    let out_vb = 9 + to_addr.script_pubkey().len() as u64;
    let change_d = parse(&descriptor_at(vault, change_index)?)?.at_derivation_index(0).map_err(|e| e.to_string())?;
    let change_spk = change_d.script_pubkey();
    let change_vb = 9 + change_spk.len() as u64;
    let mut coins: Vec<&CoinAt> = coins.iter().collect();
    coins.sort_by(|a, b| b.value.cmp(&a.value));
    let total: u64 = coins.iter().map(|c| c.value).sum();
    if coins.is_empty() { return Err("the vault holds no coins".into()); }
    let mut chosen: Vec<&CoinAt> = Vec::new();
    let mut have = 0u64;
    let want = if send_all { total } else { amount };
    for c in &coins {
        chosen.push(c);
        have += c.value;
        let vsize = 11 + in_vb * chosen.len() as u64 + out_vb + change_vb;
        let fee = (rate * vsize as f64).ceil() as u64;
        if send_all || have >= want + fee { break; }
    }
    let (vsize, fee, send, change) = if send_all {
        let vsize = 11 + in_vb * chosen.len() as u64 + out_vb;
        let fee = (rate * vsize as f64).ceil() as u64;
        if have <= fee + 330 { return Err("not enough to cover the fee".into()); }
        (vsize, fee, have - fee, 0)
    } else {
        let vsize2 = 11 + in_vb * chosen.len() as u64 + out_vb + change_vb;
        let fee2 = (rate * vsize2 as f64).ceil() as u64;
        if have < amount + fee2 { return Err(format!("the vault holds {have} sats; {amount} plus a {fee2} sat fee needs more")); }
        let change = have - amount - fee2;
        if change >= 546 { (vsize2, fee2, amount, change) } else {
            let vsize1 = 11 + in_vb * chosen.len() as u64 + out_vb;
            (vsize1, have - amount, amount, 0)
        }
    };
    if send < 330 { return Err("amount is below dust".into()); }
    let mut tx = Transaction { version: transaction::Version::TWO, lock_time: absolute::LockTime::ZERO, input: vec![], output: vec![] };
    for c in &chosen {
        let txid = bitcoin::Txid::from_str(&c.txid).map_err(|e| e.to_string())?;
        tx.input.push(TxIn { previous_output: OutPoint { txid, vout: c.vout }, script_sig: ScriptBuf::new(), sequence: Sequence::from_consensus(0xFFFFFFFD), witness: Witness::new() });
    }
    tx.output.push(TxOut { value: Amount::from_sat(send), script_pubkey: to_addr.script_pubkey() });
    if change > 0 { tx.output.push(TxOut { value: Amount::from_sat(change), script_pubkey: change_spk }); }
    let mut psbt = Psbt::from_unsigned_tx(tx).map_err(|e| e.to_string())?;
    for (i, c) in chosen.iter().enumerate() {
        let d = parse(&descriptor_at(vault, c.index)?)?.at_derivation_index(0).map_err(|e| e.to_string())?;
        psbt.inputs[i].witness_utxo = Some(TxOut { value: Amount::from_sat(c.value), script_pubkey: d.script_pubkey() });
        if n > 1 { psbt.inputs[i].witness_script = Some(d.explicit_script().map_err(|e| e.to_string())?); }
    }
    Ok(Built { psbt: psbt.to_string(), fee, vsize, change, inputs: chosen.len() })
}

pub struct ToSign { pub input: usize, pub digest: [u8; 32], pub tweak: Option<[u8; 32]>, pub child: String }

/// Which inputs a card with this base pubkey can still sign, with the digest and, for a
/// rotating vault, the tweak that turns the card's signature into the child key's.
pub fn sighashes_for(v: &Vault, psbt_b64: &str, pubkey_hex: &str) -> Result<Vec<ToSign>, String> {
    let psbt = Psbt::from_str(psbt_b64.trim()).map_err(|e| format!("psbt: {e}"))?;
    let q = PublicKey::from_str(pubkey_hex).map_err(|e| e.to_string())?;
    let cc = cc_of(v);
    let mut cache = SighashCache::new(&psbt.unsigned_tx);
    let mut out = Vec::new();
    for (i, inp) in psbt.inputs.iter().enumerate() {
        let Some(utxo) = &inp.witness_utxo else { continue };
        let Some(index) = index_of_input(v, &utxo.script_pubkey, v.scan_max)? else { continue };
        let (child, tweak) = match cc {
            Some(cc) => (crate::hd::child_pubkey(&cc, &q, index), Some(crate::hd::child_scalar(&cc, &q, index))),
            None => (q, None),
        };
        let bchild = bitcoin::PublicKey::new(child);
        if inp.partial_sigs.contains_key(&bchild) { continue; }
        let h = if let Some(ws) = &inp.witness_script {
            if !ws.as_bytes().windows(33).any(|w| w == child.serialize()) { continue; }
            cache.p2wsh_signature_hash(i, ws, utxo.value, EcdsaSighashType::All).map_err(|e| e.to_string())?.to_byte_array()
        } else if utxo.script_pubkey == ScriptBuf::new_p2wpkh(&bchild.wpubkey_hash().map_err(|e| e.to_string())?) {
            cache.p2wpkh_signature_hash(i, &utxo.script_pubkey, utxo.value, EcdsaSighashType::All).map_err(|e| e.to_string())?.to_byte_array()
        } else { continue };
        out.push(ToSign { input: i, digest: h, tweak, child: hex::encode(child.serialize()) });
    }
    Ok(out)
}

pub fn add_signature(psbt_b64: &str, pubkey_hex: &str, input: usize, r: &[u8; 32], s: &[u8; 32], digest: &[u8; 32]) -> Result<String, String> {
    let mut psbt = Psbt::from_str(psbt_b64.trim()).map_err(|e| format!("psbt: {e}"))?;
    let pk = PublicKey::from_str(pubkey_hex).map_err(|e| e.to_string())?;
    let mut compact = [0u8; 64];
    compact[..32].copy_from_slice(r);
    compact[32..].copy_from_slice(s);
    let sig = ecdsa::Signature::from_compact(&compact).map_err(|e| e.to_string())?;
    let secp = Secp256k1::verification_only();
    let msg = bitcoin::secp256k1::Message::from_digest(*digest);
    secp.verify_ecdsa(&msg, &sig, &pk).map_err(|_| "the card's signature did not verify".to_string())?;
    psbt.inputs.get_mut(input).ok_or("no such input")?.partial_sigs.insert(bitcoin::PublicKey::new(pk), bitcoin::ecdsa::Signature { signature: sig, sighash_type: EcdsaSighashType::All });
    Ok(psbt.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct SigStatus { pub inputs: usize, pub have: Vec<usize>, pub need: usize, pub ready: bool, pub signers: Vec<String> }

pub fn status(psbt_b64: &str, threshold: usize) -> Result<SigStatus, String> {
    let psbt = Psbt::from_str(psbt_b64.trim()).map_err(|e| format!("psbt: {e}"))?;
    let have: Vec<usize> = psbt.inputs.iter().map(|i| i.partial_sigs.len()).collect();
    let signers: Vec<String> = psbt.inputs.first().map(|i| i.partial_sigs.keys().map(|k| k.to_string()).collect()).unwrap_or_default();
    let ready = have.iter().all(|&n| n >= threshold);
    Ok(SigStatus { inputs: psbt.inputs.len(), have, need: threshold, ready, signers })
}

pub fn finalize(psbt_b64: &str) -> Result<Transaction, String> {
    let mut psbt = Psbt::from_str(psbt_b64.trim()).map_err(|e| format!("psbt: {e}"))?;
    let secp = Secp256k1::verification_only();
    psbt.finalize_mut(&secp).map_err(|e| format!("cannot finalise: {:?}", e))?;
    psbt.extract_tx().map_err(|e| format!("extract: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vault(cc: Option<&str>) -> Vault {
        let keys = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe", "038410f240613b33fe73d968f331ca82eb1fbd018c6189f89f8c5f9b9fb26f6a47"];
        let descriptor = descriptor_for(2, &keys.iter().map(|k| k.to_string()).collect::<Vec<_>>()).unwrap();
        Vault { name: "t".into(), network: "signet".into(), threshold: 2, keys: keys.iter().enumerate().map(|(i, k)| KeyRef { name: format!("k{i}"), serial: 0, pubkey: k.to_string() }).collect(),
            descriptor, address: String::new(), created: 0, chain_code: cc.map(|c| c.to_string()), next: 0, scan_max: 0 }
    }
    #[test]
    fn fixed_vault_keeps_the_known_address() {
        let v = vault(None);
        assert_eq!(address_at(&v, 0).unwrap(), "tb1q2n7mza35dzyea6du9dh4amzqxjg6ysgtc45kfklfd7revryppuxqfegkkp");
        assert_eq!(address_at(&v, 7).unwrap(), address_at(&v, 0).unwrap());
    }
    #[test]
    fn rotating_vault_gives_fresh_addresses_and_finds_them_again() {
        let v = vault(Some(&"ab".repeat(32)));
        let a0 = address_at(&v, 0).unwrap();
        let a1 = address_at(&v, 1).unwrap();
        assert_ne!(a0, a1);
        assert_ne!(a0, "tb1q2n7mza35dzyea6du9dh4amzqxjg6ysgtc45kfklfd7revryppuxqfegkkp");
        let spk = script_of(&descriptor_at(&v, 5).unwrap()).unwrap();
        assert_eq!(index_of_input(&v, &spk, 0).unwrap(), Some(5));
        let (d, cc) = split_recovery(&recovery_line(&v));
        assert_eq!(d, v.descriptor);
        assert_eq!(cc.as_deref(), Some("ab".repeat(32).as_str()));
    }
}
