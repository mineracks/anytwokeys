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
    #[serde(default)] pub next_change: u32,              // first unused change position (change chain = CHANGE | pos)
    #[serde(default)] pub scan_max_change: u32,
    /// BIP 329 style labels: "addr:<address>", "tx:<txid>", "out:<txid:vout>"
    #[serde(default)] pub labels: std::collections::HashMap<String, String>,
    /// when each label last changed (unix seconds), including deletions, so two computers merge newest-wins (labels.rs)
    #[serde(default)] pub label_meta: std::collections::HashMap<String, u64>,
    /// Long text per record key, and the attachment list of each record kept as the opaque JSON it arrived as:
    /// both ride in the same synced record as the label (labels.rs), so a build that does not draw them must not drop them.
    #[serde(default)] pub notes: std::collections::HashMap<String, String>,
    #[serde(default)] pub files: std::collections::HashMap<String, Vec<serde_json::Value>>,
    /// the rest of each synced record, kept whole: `ctx` (where / on what / with which keys) and anything a newer build adds
    #[serde(default)] pub more: std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>>,
    /// last time the labels were confirmed in step with the label host; 0 = never
    #[serde(default)] pub labels_synced: u64,
    #[serde(default)] pub seal: Seal,
}

/// The recovery record split across the cards (seal.rs). Shares are made once so cards sealed on different days agree.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Seal {
    #[serde(default)] pub shares: Vec<String>,   // hex, share i belongs to keys[i] (x = i + 1)
    #[serde(default)] pub sealed: Vec<String>,   // public keys of the cards seen carrying their share
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub network: String,
    pub custom: bool,
    pub electrum: String,
    pub explorer: String,
    /// how many unused addresses past the last used one a scan looks at, per chain
    #[serde(default = "default_gap")] pub gap: u32,
    /// currency the Home screen values mainnet coins in: "auto", an ISO code, or empty for none
    #[serde(default = "default_fiat")] pub fiat: String,
    /// price service when the person runs their own servers; empty there = no fiat
    #[serde(default)] pub prices: String,
    /// the VSS host the vaults' labels are backed up to, encrypted; empty = off
    #[serde(default = "default_vss")] pub vss: String,
}
pub fn default_vss() -> String { crate::labels::DEFAULT_VSS.into() }
pub fn default_fiat() -> String { "auto".into() }   // "auto" = from the device's region; "" = none; else an ISO code
pub fn default_gap() -> u32 { GAP }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedMessage { pub vault: String, pub address: String, pub message: String, pub index: u32 }

/// A saved destination. Addresses are per network; `used` counts payments made to it from this app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact { pub name: String, pub address: String, pub network: String, #[serde(default)] pub added: u64, #[serde(default)] pub used: u32, #[serde(default)] pub last_used: u64 }

/// The address as the network spells it (bech32 lower-cased), or why it is not one for this network.
pub fn checked_address(address: &str, network: &str) -> Result<String, String> {
    let a = Address::from_str(address.trim()).map_err(|_| "that is not a Bitcoin address".to_string())?
        .require_network(network_of(network)).map_err(|_| format!("that address is not a {network} address"))?;
    Ok(a.to_string())
}

/// What the watcher last saw for a vault: txid → height, and the coins it held.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Seen { pub txs: std::collections::HashMap<String, i32>, pub coins: std::collections::HashMap<String, u64>, pub primed: bool }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Store {
    pub vaults: Vec<Vault>,
    pub settings: Settings,
    pub last_psbt: Option<String>,
    /// when `last_psbt` is a BIP 322 proof rather than a spend
    #[serde(default)] pub last_message: Option<StagedMessage>,
    #[serde(default)] pub seen: std::collections::HashMap<String, Seen>,
    /// the address book: places this person pays more than once
    #[serde(default)] pub contacts: Vec<Contact>,
    /// public key → YubiKey form factor, learnt when the card is plugged in, so a Nano is drawn as one
    #[serde(default)] pub forms: std::collections::HashMap<String, u8>,
}

pub fn network_of(s: &str) -> Network {
    if s == "mainnet" { Network::Bitcoin } else { Network::Signet }
}

pub fn store_path() -> std::path::PathBuf { crate::lockbox::dir().join("store.json") }

/// A store that is not there yet is an empty one. A store that is there and cannot be read is an ERROR, never an
/// empty one: the next save would write that emptiness over every vault, label and contact on this computer.
/// Once this computer seals what it keeps (lockbox.rs) the store is `store.enc`, and reading it needs the app unlocked.
pub fn load() -> Result<Store, String> {
    let p = store_path();
    let bytes = if crate::lockbox::sealed_mode() {
        let key = crate::lockbox::key()?;   // locked is locked, whatever is or is not on disk
        match std::fs::read(p.with_extension("enc")) {
            Ok(blob) => Some(crate::lockbox::open_with(&key, "store", &blob).map_err(|e| format!("the app's own file was left alone ({}): {e}", p.with_extension("enc").display()))?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("the app's own file could not be read ({}): {e}", p.with_extension("enc").display())),
        }
    } else { None };
    // no sealed store: the readable one, if there is one — before the move to sealed files, or part-way through it.
    // Once that move is finished a readable store.json is never read again: anyone who can write a file could
    // otherwise hand this app a store of their own, with their own addresses in it.
    let finished = crate::lockbox::slots()?.map(|s| s.migrated).unwrap_or(false);
    let bytes = match bytes { Some(b) => Some(b), None if finished => None, None => match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("the app's own file could not be read ({}): {e}", p.display())),
    } };
    let mut s: Store = match bytes { None => Store::default(), Some(b) => parse_store(&b).map_err(|e| format!("the app's own file is damaged and was left alone: {e}"))? };
    if s.settings.network.is_empty() { s.settings.network = "signet".into(); }
    Ok(s)
}
pub fn parse_store(bytes: &[u8]) -> Result<Store, String> { serde_json::from_slice(bytes).map_err(|e| e.to_string()) }

static STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Load, change, save, under one lock: for the short write after a long scan, so two threads that both
/// scanned cannot erase each other's marks. Never hold it across network calls.
pub fn update<F: FnOnce(&mut Store)>(f: F) -> Result<Store, String> {
    let _g = STORE_LOCK.lock().map_err(|_| "store lock poisoned".to_string())?;
    let mut s = load()?;
    f(&mut s);
    save(&s)?;
    Ok(s)
}

pub fn save(s: &Store) -> Result<(), String> {
    let p = store_path();
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    if crate::lockbox::sealed_mode() {
        let finished = crate::lockbox::slots()?.map(|s| s.migrated).unwrap_or(false);
        if !finished && p.exists() { return Err("this computer is part-way through sealing its files; open the app again to finish before anything is saved".into()); }
        return crate::lockbox::write_sealed(&p.with_extension("enc"), "store", text.as_bytes());
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
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
/// Change addresses live on their own chain: index = CHANGE | position. Receive addresses are plain positions.
pub const CHANGE: u32 = 0x8000_0000;
pub fn is_change(index: u32) -> bool { index & CHANGE != 0 }
pub fn position(index: u32) -> u32 { index & !CHANGE }

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
pub struct Scanned { pub coins: Vec<CoinAt>, pub history: Vec<(String, i32, u32)>, pub next: u32, pub scan_max: u32, pub next_change: u32, pub scan_max_change: u32 }

/// Every index up to `gap` past the last one with history, on the receive chain and, for a rotating
/// vault, the change chain too. A fixed vault has index 0 only.
pub fn scan(v: &Vault, servers: &Servers, gap: u32) -> Result<Scanned, String> {
    let gap = gap.max(1);
    let rotating = cc_of(v).is_some();
    let mut coins = Vec::new();
    let mut history = Vec::new();
    let mut chain = |base: u32| -> Result<(u32, u32), String> {
        let mut last_used: Option<u32> = None;
        let mut next: Option<u32> = None;
        let mut i = 0u32;
        loop {
            let spk = script_of(&descriptor_at(v, base | i)?)?;
            // a failed query is a failed scan, never "no history": an empty answer resets the next address and the watcher's memory
            let h = chain::history(servers, &spk)?;
            if !h.is_empty() {
                last_used = Some(i);
                for item in h { history.push((item.txid, item.height, base | i)); }
                for c in chain::coins(servers, &spk)? { coins.push(CoinAt { txid: c.txid, vout: c.vout, value: c.value, height: c.height, index: base | i }); }
            } else if next.is_none() {
                next = Some(i);
            }
            if !rotating { break; }
            if i >= last_used.map(|u| u + gap).unwrap_or(gap - 1) { break; }
            i += 1;
        }
        Ok((next.unwrap_or(i + 1), i))
    };
    let (next, scan_max) = chain(0)?;
    let (next_change, scan_max_change) = if rotating { chain(CHANGE)? } else { (0, 0) };
    Ok(Scanned { coins, history, next, scan_max, next_change, scan_max_change })
}

/// Which index a PSBT input pays to, by matching its script against the vault's.
pub fn index_of_input(v: &Vault, spk: &ScriptBuf) -> Result<Option<u32>, String> {
    if cc_of(v).is_none() { return Ok(if &script_of(&descriptor_at(v, 0)?)? == spk { Some(0) } else { None }); }
    for i in 0..=v.scan_max.max(v.next) + GAP {
        if &script_of(&descriptor_at(v, i)?)? == spk { return Ok(Some(i)); }
    }
    for i in 0..=v.scan_max_change.max(v.next_change) + GAP {
        if &script_of(&descriptor_at(v, CHANGE | i)?)? == spk { return Ok(Some(CHANGE | i)); }
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
/// `pick`: `None` lets the builder choose (largest first); `Some(outpoints)` spends exactly those coins.
pub fn build(vault: &Vault, coins: &[CoinAt], change_index: u32, to: &str, amount: u64, rate: f64, send_all: bool, pick: Option<&[String]>) -> Result<Built, String> {
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
    if let Some(p) = pick {
        for op in p { if !coins.iter().any(|c| format!("{}:{}", c.txid, c.vout) == *op) { return Err(format!("coin {op} is not in the vault any more; sync and choose again")); } }
        coins.retain(|c| p.iter().any(|op| format!("{}:{}", c.txid, c.vout) == *op));
        if coins.is_empty() { return Err("choose at least one coin".into()); }
    }
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
        // chosen coins are all spent, as asked; otherwise stop as soon as the amount and fee are covered
        if pick.is_none() && (send_all || have >= want + fee) { break; }
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
        let Some(index) = index_of_input(v, &utxo.script_pubkey)? else { continue };
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

/// One of the vault's keys, as the ceremony sees it: how many of the spend's inputs carry its signature.
#[derive(Debug, Clone, Serialize)]
pub struct KeySig { pub name: String, pub pubkey: String, pub signed: usize }
#[derive(Debug, Clone, Serialize)]
pub struct Pays { pub address: String, pub sats: u64, pub ours: bool }
/// Everything the Sign screen, the QR and the PDF say about a spend in progress.
#[derive(Debug, Clone, Serialize)]
pub struct Ceremony { pub ours: bool, pub inputs: usize, pub need: usize, pub have: usize, pub ready: bool, pub keys: Vec<KeySig>, pub pays: Vec<Pays>, pub fee: u64, pub txid: String, pub bytes: usize }

/// The key that signs input `inp` for vault key `base`: the key itself, or its child at the input's index.
fn signer_at(v: &Vault, base: &str, index: u32) -> Result<bitcoin::PublicKey, String> {
    let q = PublicKey::from_str(base).map_err(|e| e.to_string())?;
    Ok(bitcoin::PublicKey::new(match cc_of(v) { Some(cc) => crate::hd::child_pubkey(&cc, &q, index), None => q }))
}

/// The secp256k1 key that vault key `base` uses at address `index` (the key itself in a fixed vault).
pub fn child_key(v: &Vault, base: &str, index: u32) -> Result<PublicKey, String> { signer_at(v, base, index).map(|k| k.inner) }

pub fn ceremony(v: &Vault, psbt_b64: &str) -> Result<Ceremony, String> {
    let psbt = Psbt::from_str(psbt_b64.trim()).map_err(|e| format!("psbt: {e}"))?;
    let mut keys: Vec<KeySig> = v.keys.iter().map(|k| KeySig { name: k.name.clone(), pubkey: k.pubkey.clone(), signed: 0 }).collect();
    let mut total_in = 0u64;
    for inp in &psbt.inputs {
        let utxo = inp.witness_utxo.as_ref().ok_or("an input does not say what it spends")?;
        total_in += utxo.value.to_sat();
        let Some(index) = index_of_input(v, &utxo.script_pubkey)? else { continue };
        for k in keys.iter_mut() { if inp.partial_sigs.contains_key(&signer_at(v, &k.pubkey, index)?) { k.signed += 1; } }
    }
    let have = psbt.inputs.iter().map(|i| i.partial_sigs.len()).min().unwrap_or(0).min(v.threshold);
    let net = network_of(&v.network);
    let pays = psbt.unsigned_tx.output.iter().map(|o| Ok(Pays {
        address: Address::from_script(&o.script_pubkey, net).map(|a| a.to_string()).unwrap_or_else(|_| format!("script {}", o.script_pubkey.to_hex_string())),
        sats: o.value.to_sat(), ours: index_of_input(v, &o.script_pubkey)?.is_some() })).collect::<Result<Vec<_>, String>>()?;
    let out: u64 = pays.iter().map(|p| p.sats).sum();
    let ours = psbt.inputs.iter().all(|i| i.witness_utxo.as_ref().map(|u| index_of_input(v, &u.script_pubkey).ok().flatten().is_some()).unwrap_or(false));
    Ok(Ceremony { ours, inputs: psbt.inputs.len(), need: v.threshold, have, ready: psbt.inputs.iter().all(|i| i.partial_sigs.len() >= v.threshold), keys, pays,
        fee: total_in.saturating_sub(out), txid: psbt.unsigned_tx.compute_txid().to_string(), bytes: psbt.serialize().len() })
}

/// Take in a PSBT made or part-signed elsewhere. Every input must spend this vault's coins, and every signature
/// already on it must be by one of this vault's keys and must verify: a bad one is refused here, not at broadcast.
/// If the same spend is already in progress the signatures are merged.
pub fn adopt(v: &Vault, mut psbt: Psbt, current: Option<&str>) -> Result<String, String> {
    if psbt.inputs.is_empty() { return Err("that PSBT spends nothing".into()); }
    let secp = Secp256k1::verification_only();
    let tx = psbt.unsigned_tx.clone();
    let mut cache = SighashCache::new(&tx);
    for (i, inp) in psbt.inputs.iter_mut().enumerate() {
        if inp.final_script_witness.is_some() { return Err("that PSBT is already finalised; broadcast it from the wallet that finished it".into()); }
        let utxo = inp.witness_utxo.clone().ok_or("an input does not say what it spends (no witness UTXO); this app only signs segwit coins")?;
        let index = index_of_input(v, &utxo.script_pubkey)?.ok_or(format!("input {} does not spend this vault's coins", i + 1))?;
        let single = v.keys.len() == 1;
        if !single {
            let ours = parse(&descriptor_at(v, index)?)?.at_derivation_index(0).map_err(|e| e.to_string())?.explicit_script().map_err(|e| e.to_string())?;
            match &inp.witness_script { Some(ws) if ws != &ours => return Err(format!("input {} carries a script that is not this vault's", i + 1)), Some(_) => {}, None => inp.witness_script = Some(ours) }
        }
        let digest = if single { cache.p2wpkh_signature_hash(i, &utxo.script_pubkey, utxo.value, EcdsaSighashType::All).map_err(|e| e.to_string())?.to_byte_array() }
            else { cache.p2wsh_signature_hash(i, inp.witness_script.as_ref().unwrap(), utxo.value, EcdsaSighashType::All).map_err(|e| e.to_string())?.to_byte_array() };
        let allowed: Vec<bitcoin::PublicKey> = v.keys.iter().map(|k| signer_at(v, &k.pubkey, index)).collect::<Result<_, _>>()?;
        for (pk, sig) in &inp.partial_sigs {
            if !allowed.contains(pk) { return Err(format!("input {} is signed by a key that is not this vault's", i + 1)); }
            if sig.sighash_type != EcdsaSighashType::All { return Err("a signature commits to only part of the transaction (not SIGHASH_ALL); refused".into()); }
            secp.verify_ecdsa(&bitcoin::secp256k1::Message::from_digest(digest), &sig.signature, &pk.inner).map_err(|_| format!("a signature on input {} does not verify", i + 1))?;
        }
    }
    if let Some(cur) = current {
        let mut mine = Psbt::from_str(cur.trim()).map_err(|e| format!("psbt: {e}"))?;
        if mine.unsigned_tx == psbt.unsigned_tx { mine.combine(psbt).map_err(|e| format!("could not merge: {e}"))?; return Ok(mine.to_string()); }
    }
    Ok(psbt.to_string())
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
            descriptor, address: String::new(), created: 0, chain_code: cc.map(|c| c.to_string()), next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default() }
    }
    #[test]
    fn a_part_signed_psbt_from_elsewhere_is_checked_before_it_is_taken_in() {
        let raw = include_bytes!("../../test/fixtures/sample.psbt");   // the signet vault's spend with one card's signature
        let v = vault(None);
        let taken = adopt(&v, Psbt::deserialize(raw).unwrap(), None).unwrap();
        let c = ceremony(&v, &taken).unwrap();
        assert_eq!((c.have, c.need, c.ready, c.inputs), (1, 2, false, 1));
        assert_eq!(c.keys.iter().filter(|k| k.signed == 1).count(), 1, "exactly one named key has signed");
        assert!(c.fee > 0 && c.fee < 10_000 && !c.pays.is_empty());
        // a flipped bit in the signature is refused at the door
        let mut forged = Psbt::deserialize(raw).unwrap();
        let (pk, mut sig) = forged.inputs[0].partial_sigs.iter().next().map(|(k, s)| (*k, *s)).unwrap();
        let mut c64 = sig.signature.serialize_compact(); c64[40] ^= 1;
        sig.signature = ecdsa::Signature::from_compact(&c64).unwrap();
        forged.inputs[0].partial_sigs.insert(pk, sig);
        assert!(adopt(&v, forged, None).unwrap_err().contains("does not verify"));
        // another vault's coins are not ours to sign
        assert!(adopt(&vault(Some(&"ab".repeat(32))), Psbt::deserialize(raw).unwrap(), None).unwrap_err().contains("does not spend this vault"));
        // the same spend arriving twice merges rather than replaces
        let mut bare = Psbt::deserialize(raw).unwrap(); bare.inputs[0].partial_sigs.clear();
        let bare = adopt(&v, bare, None).unwrap();
        assert_eq!(ceremony(&v, &bare).unwrap().have, 0);
        assert_eq!(ceremony(&v, &adopt(&v, Psbt::deserialize(raw).unwrap(), Some(&bare)).unwrap()).unwrap().have, 1);
    }
    #[test]
    fn a_scan_that_cannot_reach_the_server_fails_instead_of_reporting_an_empty_vault() {
        // an empty Ok here once reset the next address and made the watcher announce every old transaction again
        let dead = Servers { electrum: "tcp://127.0.0.1:1".into(), explorer: "http://127.0.0.1:1".into() };
        assert!(scan(&vault(Some(&"ab".repeat(32))), &dead, 20).is_err());
        assert!(scan(&vault(None), &dead, 20).is_err());
    }
    #[test]
    fn an_address_is_checked_against_its_network_before_it_is_saved() {
        let main = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";   // BIP 173's own example
        assert_eq!(checked_address(&format!("  {main} "), "mainnet").unwrap(), main);
        assert_eq!(checked_address(&main.to_uppercase(), "mainnet").unwrap(), main);
        assert!(checked_address(main, "signet").unwrap_err().contains("not a signet address"));
        assert!(checked_address("tb1q2n7mza35dzyea6du9dh4amzqxjg6ysgtc45kfklfd7revryppuxqfegkkp", "mainnet").is_err());
        assert!(checked_address("tb1q2n7mza35dzyea6du9dh4amzqxjg6ysgtc45kfklfd7revryppuxqfegkkp", "signet").is_ok());
        assert!(checked_address("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdx", "mainnet").unwrap_err().contains("not a Bitcoin address"));   // one character wrong: the checksum catches it
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
        assert_eq!(index_of_input(&v, &spk).unwrap(), Some(5));
        let (d, cc) = split_recovery(&recovery_line(&v));
        assert_eq!(d, v.descriptor);
        assert_eq!(cc.as_deref(), Some("ab".repeat(32).as_str()));
    }
}
