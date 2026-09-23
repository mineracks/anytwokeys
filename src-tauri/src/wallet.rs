//! Vaults: fixed-key descriptors over card keys, the store, building and finalising spends.
use std::collections::HashMap;
use bitcoin::hashes::Hash;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{ecdsa, PublicKey, Secp256k1};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::{absolute, transaction, Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
use miniscript::descriptor::Descriptor;
use miniscript::psbt::PsbtExt;
use miniscript::{DescriptorPublicKey, ForEachKey};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::chain::{self, Servers};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRef { pub name: String, pub serial: u32, pub pubkey: String,
    /// which signing device holds it, from the app's catalogue ("yubikey", "seedsigner", "coldcard-mk4", …)
    #[serde(default)] pub device: Option<String> }

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
    /// A vault whose keys are EXTENDED keys held by other signers (a SeedSigner, a Coldcard, Sparrow): the standard
    /// wallet descriptor exactly as it came, multipath `<0;1>/*` for receive and change. Present = every address comes
    /// from here by ordinary BIP32, and none of the card machinery (chain code, seal, card signing) applies.
    #[serde(default)] pub ext: Option<String>,
    /// this vault's own method, when it belongs to a different taxpayer from the app's (a company's, a client's)
    #[serde(default)] pub cost_method: Option<String>,
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
    /// keep where each transaction was made (system Location Services); off until the person turns it on
    #[serde(default)] pub location: bool,
    /// keep the place exactly rather than to about 110 m
    #[serde(default)] pub location_exact: bool,
    /// the one .pmtiles file the little maps are drawn from; empty = no maps
    #[serde(default = "default_tiles")] pub tiles: String,
    /// the place-search host (Photon); empty = the place is plain text
    #[serde(default = "default_geocode")] pub geocode: String,
    /// work offline: no request leaves the app until this is turned off again
    #[serde(default)] pub offline: bool,
    /// how often a vault should be proved recoverable from its cards, in days; 0 = never ask
    #[serde(default = "default_drill")] pub drill_days: u32,
    /// how a disposal's cost is found: "utxo" (the coins actually spent, as the chain records them), "fifo", "average"
    #[serde(default = "default_method")] pub cost_method: String,
}
pub fn default_geocode() -> String { "https://geocode.mineracks.com".into() }
pub fn default_tiles() -> String { "https://tiles.mineracks.com/planet.pmtiles".into() }
pub fn default_vss() -> String { crate::labels::DEFAULT_VSS.into() }
pub fn default_fiat() -> String { "auto".into() }   // "auto" = from the device's region; "" = none; else an ISO code
pub fn default_gap() -> u32 { GAP }
/// 90 days. A key that has died, been wiped or had its PIN locked out fails SILENTLY: nothing tells you until the
/// day you need it. The interval is a guess at how long is tolerable to have been wrong for, not a wear estimate.
pub fn default_drill() -> u32 { 90 }
/// The coins a transaction actually spent: specific identification, with the chain as the record. Accepted in
/// Australia ("your selection of the identity", with records) and the US (specific identification); see
/// roadmap/anytwokeys/ACCOUNTING.md for the sources and for where it is NOT the rule (the UK pools).
pub fn default_method() -> String { "utxo".into() }

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
    /// when this computer FIRST saw each key, by public key. A YubiKey carries no manufacture date, so this is the
    /// only age this app can honestly claim: how long it has known the key, which is a floor and never its real age.
    #[serde(default)] pub key_first: std::collections::HashMap<String, u64>,
    /// the firmware each key reported, by public key. Yubico does not allow firmware to be changed on a key that
    /// already exists, so a version once seen stays true — which is what makes it safe to answer for a key that is
    /// not plugged in, and it is the only way an off-site key can be checked against an advisory at all.
    #[serde(default)] pub key_fw: std::collections::HashMap<String, String>,
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
    // multi() is NOT sortedmulti(): its key order is part of the script. Rebuilding one as sortedmulti would give
    // different addresses and watch the wrong ones, silently. Any Two Keys' own vaults are sortedmulti; a multi() over
    // extended keys goes the faithful way (ext_parse), and one over fixed keys is refused rather than changed.
    if inner.starts_with("wsh(multi(") {
        return Err("this descriptor is multi(), where the order of the keys is part of every address; this app only rebuilds sortedmulti() from fixed keys, and changing one into the other would watch the wrong addresses".into());
    }
    if let Some(rest) = inner.strip_prefix("wsh(sortedmulti(") {
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

/// Does this vault give a FRESH address for every deposit? A card vault does when it has a chain code; an
/// extended-key vault always does, by BIP32. Ask this for "rotating" — and ask `cc_of` only when the chain code's
/// VALUE is needed for the card arithmetic. Using the one for the other made an extended-key vault scan a single
/// address, skip its change chain, hand out the same receive address every time, and not recognise its own change.
pub fn rotates(v: &Vault) -> bool { v.ext.is_some() || v.chain_code.is_some() }

pub fn cc_of(v: &Vault) -> Option<[u8; 32]> {
    let h = v.chain_code.as_ref()?;
    let b = hex::decode(h).ok()?;
    b.try_into().ok()
}

/// The cosigner keys for address `index`: the card keys, or their multiplicative children.
pub fn keys_at(v: &Vault, index: u32) -> Result<Vec<String>, String> {
    if let Some(ext) = &v.ext { return ext_keys_at(ext, index); }
    match cc_of(v) {
        None => Ok(v.keys.iter().map(|k| k.pubkey.clone()).collect()),
        Some(cc) => v.keys.iter().map(|k| {
            let q = PublicKey::from_str(&k.pubkey).map_err(|e| e.to_string())?;
            Ok(hex::encode(crate::hd::child_pubkey(&cc, &q, index).serialize()))
        }).collect(),
    }
}

// ---------- addresses, derived once ----------
// Deriving an address from descriptor TEXT costs ~10 ms (the text is parsed, every xpub base58-decoded and
// checksummed, then printed and parsed again). Every scan, every "is this output ours?" and every address list did
// that afresh, in loops. A vault's scripts never change, so each is derived once per run and kept both ways.
#[derive(Default)]
struct Derived { fwd: HashMap<u32, ScriptBuf>, rev: HashMap<ScriptBuf, u32> }
static DERIVED: std::sync::Mutex<Option<HashMap<String, Derived>>> = std::sync::Mutex::new(None);
static CHAINS: std::sync::Mutex<Option<HashMap<String, Vec<Descriptor<DescriptorPublicKey>>>>> = std::sync::Mutex::new(None);
/// What a vault's scripts depend on, and nothing else: the same text always derives the same scripts.
fn identity(v: &Vault) -> String {
    match &v.ext { Some(e) => format!("x|{e}"), None => format!("c|{}|{}|{}", v.threshold, v.chain_code.as_deref().unwrap_or(""), v.keys.iter().map(|k| k.pubkey.as_str()).collect::<Vec<_>>().join(",")) }
}
/// The receive and change chains of an extended-key descriptor, parsed once.
fn ext_chains(ext: &str) -> Result<Vec<Descriptor<DescriptorPublicKey>>, String> {
    if let Some(c) = CHAINS.lock().map_err(|_| "derivation cache poisoned")?.as_ref().and_then(|m| m.get(ext)) { return Ok(c.clone()); }
    let singles = ext_desc(ext)?.into_single_descriptors().map_err(|e| e.to_string())?;
    CHAINS.lock().map_err(|_| "derivation cache poisoned")?.get_or_insert_with(HashMap::new).insert(ext.to_string(), singles.clone());
    Ok(singles)
}
/// The script at an address index, derived at most once per run.
pub fn script_at(v: &Vault, index: u32) -> Result<ScriptBuf, String> {
    let id = identity(v);
    if let Some(s) = DERIVED.lock().map_err(|_| "derivation cache poisoned")?.as_ref().and_then(|m| m.get(&id)).and_then(|d| d.fwd.get(&index)) { return Ok(s.clone()); }
    let spk = match &v.ext {
        Some(_) => ext_definite_of(v, index)?.script_pubkey(),
        None => script_of(&descriptor_for(v.threshold, &keys_at(v, index)?)?)?,
    };
    let mut g = DERIVED.lock().map_err(|_| "derivation cache poisoned")?;
    let d = g.get_or_insert_with(HashMap::new).entry(id).or_default();
    d.fwd.insert(index, spk.clone()); d.rev.insert(spk.clone(), index);
    Ok(spk)
}
fn ext_definite_of(v: &Vault, index: u32) -> Result<Descriptor<miniscript::DefiniteDescriptorKey>, String> {
    ext_definite(v.ext.as_deref().ok_or("not a vault of extended keys")?, index)
}

pub fn descriptor_at(v: &Vault, index: u32) -> Result<String, String> {
    // an extended-key vault is read AS WRITTEN — never rebuilt through descriptor_for, which only makes sortedmulti
    if let Some(ext) = &v.ext { return ext_definite(ext, index).map(|d| d.to_string()); }
    descriptor_for(v.threshold, &keys_at(v, index)?)
}

// ---------- vaults of extended keys held by other signers ----------

/// One key of an extended-key descriptor, for showing and for building a PSBT a hardware signer will recognise:
/// the signer matches BOTH the master fingerprint and the full path from master, or refuses it as "not my key".
#[derive(Debug, Clone, Serialize)]
pub struct ExtKey { pub fingerprint: String, pub path: String, pub xpub: String,
    /// a FIXED key — a card in a mixed vault: the same key at every address, with no origin
    #[serde(default)] pub fixed: bool }

#[derive(Debug, Clone, Serialize)]
pub struct ExtInfo { pub descriptor: String, pub canonical: String, pub threshold: usize, pub keys: Vec<ExtKey>, pub network: String, pub kind: String }

fn ext_desc(text: &str) -> Result<Descriptor<DescriptorPublicKey>, String> {
    // parsed whole, checksum and all, and case left alone: base58 is case-sensitive
    Descriptor::<DescriptorPublicKey>::from_str(text.trim()).map_err(|e| format!("that is not a descriptor this app can read: {e}"))
}

/// Is this the wallet descriptor of a vault of extended keys — and if so, what is in it. Refuses, with the reason,
/// anything it could watch but not stand behind: a mixture of extended and fixed keys (their signers differ), a
/// receive-only descriptor (a wallet needs its change too), and a key without the origin a signer needs to know it.
pub fn ext_parse(text: &str) -> Result<Option<ExtInfo>, String> {
    let d = ext_desc(text)?;
    let mut keys = Vec::new(); let mut fixed = 0usize; let mut net: Option<bitcoin::NetworkKind> = None;
    let mut all: Vec<DescriptorPublicKey> = Vec::new(); d.for_each_key(|k| { all.push(k.clone()); true });
    for k in all {
        match &k {
            DescriptorPublicKey::Single(sk) => { fixed += 1; let hex = match &sk.key { miniscript::descriptor::SinglePubKey::FullKey(pk) => pk.to_string(), miniscript::descriptor::SinglePubKey::XOnly(x) => x.to_string() };
                keys.push(ExtKey { fingerprint: String::new(), path: String::new(), xpub: hex, fixed: true }); }
            DescriptorPublicKey::XPub(x) => {
                if x.origin.is_none() { return Err("each key needs its origin, like [dc567276/48h/0h/0h/2h]xpub…: without the fingerprint and path a hardware signer cannot tell it is one of its own keys".into()); }
                net = Some(x.xkey.network);
                keys.push(ExtKey { fingerprint: k.master_fingerprint().to_string(), path: format!("m/{}", x.origin.as_ref().unwrap().1), xpub: x.xkey.to_string(), fixed: false });
            }
            DescriptorPublicKey::MultiXPub(x) => {
                if x.origin.is_none() { return Err("each key needs its origin, like [dc567276/48h/0h/0h/2h]xpub…: without the fingerprint and path a hardware signer cannot tell it is one of its own keys".into()); }
                net = Some(x.xkey.network);
                keys.push(ExtKey { fingerprint: k.master_fingerprint().to_string(), path: format!("m/{}", x.origin.as_ref().unwrap().1), xpub: x.xkey.to_string(), fixed: false });
            }
        }
    }
    if keys.iter().all(|k| k.fixed) { return Ok(None); }   // fixed keys only: the card path reads it
    // Extended keys beside fixed ones: a card in a mixed vault. Standard, so other wallets and signers can derive it;
    // the cost — the card's key is the same in every spend — is said where the vault is made.
    let _ = fixed;
    if !d.is_multipath() { return Err("this descriptor names one chain only. Paste the whole wallet descriptor, with <0;1>/* on each key, so both receive and change addresses are watched".into()); }
    let singles = d.clone().into_single_descriptors().map_err(|e| e.to_string())?;
    if singles.len() != 2 { return Err("a wallet descriptor has exactly two chains, <0;1> — receive and change".into()); }
    let (kind, threshold) = match d.desc_type() {
        miniscript::descriptor::DescriptorType::WshSortedMulti => ("sortedmulti", sorted_threshold(&d)?),
        // wsh(multi(...)) is reported as a general wsh; it is accepted only when the text says exactly that
        miniscript::descriptor::DescriptorType::Wsh if d.to_string().starts_with("wsh(multi(") => ("multi", sorted_threshold(&d)?),
        miniscript::descriptor::DescriptorType::Wpkh => ("single", 1),
        other => return Err(format!("{other:?} descriptors are not supported yet: this app watches wsh(sortedmulti), wsh(multi) and wpkh wallets")),
    };
    let network = match net { Some(bitcoin::NetworkKind::Main) => "mainnet", _ => "signet" }.to_string();
    // Kept AS WRITTEN when it came with a checksum that verified: miniscript re-writes 48h as 48' and the checksum
    // covers the text, so its canonical form would carry a different checksum from the one Sparrow or the SeedSigner
    // shows for the very same wallet (#ewpd0dfe became #h3zcql09). Without a checksum, the canonical form, which has one.
    let trimmed = text.trim();
    let descriptor = if trimmed.contains('#') { trimmed.to_string() } else { d.to_string() };
    // origins on FIXED keys are not part of which wallet this is (they say only what the key already says), so the
    // same wallet written either way compares equal — as `48h` and `48'` already do
    Ok(Some(ExtInfo { descriptor, canonical: without_key_origins(&d.to_string()), threshold, keys, network, kind: kind.into() }))
}

fn sorted_threshold(d: &Descriptor<DescriptorPublicKey>) -> Result<usize, String> {
    // the k of k-of-n, read from the canonical text rather than from miniscript's internals
    let s = d.to_string(); let inner = s.split_once("multi(").map(|x| x.1).ok_or("no multi")?;
    inner.split(',').next().and_then(|k| k.trim().parse().ok()).ok_or_else(|| "threshold".into())
}

/// The concrete descriptor at an address index: receive `pos`, or change `CHANGE | pos`.
fn ext_definite(ext: &str, index: u32) -> Result<Descriptor<miniscript::DefiniteDescriptorKey>, String> {
    let singles = ext_chains(ext)?;
    let chain = singles.get(if is_change(index) { 1 } else { 0 }).ok_or("the descriptor has no change chain")?;
    chain.at_derivation_index(position(index)).map_err(|e| e.to_string())
}

/// The cosigner keys at an index, in the descriptor's own order, as compressed hex.
fn ext_keys_at(ext: &str, index: u32) -> Result<Vec<String>, String> {
    let d = ext_definite(ext, index)?;
    let secp = Secp256k1::verification_only();
    let mut out = Vec::new(); let mut bad = None;
    d.for_each_key(|k| { match k.derive_public_key(&secp) { Ok(p) => out.push(hex::encode(p.inner.serialize())), Err(e) => bad = Some(e.to_string()) } true });
    match bad { Some(e) => Err(e), None => Ok(out) }
}

pub fn address_at(v: &Vault, index: u32) -> Result<String, String> {
    Ok(Address::from_script(&script_at(v, index)?, network_of(&v.network)).map_err(|e| e.to_string())?.to_string())
}

/// The recovery line: the base descriptor, plus the chain code when addresses rotate.
/// A fixed key in a mixed vault is written plainly, with no origin — nothing derived it. That is legal (BIP 380) and
/// every wallet derives the same addresses from it, but **SeedSigner cannot draw such a wallet**: before it will
/// verify change it shows the descriptor's cosigner fingerprints, and `hexlify(key.fingerprint)` on a key with no
/// origin raised `TypeError: a bytes-like object is required, not 'NoneType'` on the bench (2026-09-23,
/// `seed_views.py:2067`) — so the verification Piers wanted could not even be reached.
///
/// So each fixed key is shown with the origin it implies: `[fingerprint]02ab…`, the fingerprint a master holding
/// exactly that key would have (hash160(key)[..4]), and an empty path, because nothing derived it. Checked against
/// embit — the library SeedSigner uses: it parses, the addresses are identical, and `descriptor.owns(change)` is
/// still true. Only the way the line is WRITTEN changes; the wallet does not.
pub fn with_key_origins(text: &str) -> String {
    match key_origins(text, true) { Some(s) => s, None => text.to_string() }
}

/// The same machinery, the other way: origins on fixed keys are dropped before a descriptor is compared, so a line
/// written either way is recognised as the SAME wallet (as `48h` and `48'` already are).
pub fn without_key_origins(text: &str) -> String {
    match key_origins(text, false) { Some(s) => s, None => text.to_string() }
}

fn key_origins(text: &str, add: bool) -> Option<String> {
    use miniscript::descriptor::{SinglePub, SinglePubKey};
    use miniscript::TranslatePk;
    struct T(bool);
    impl miniscript::Translator<DescriptorPublicKey, DescriptorPublicKey, ()> for T {
        fn pk(&mut self, k: &DescriptorPublicKey) -> Result<DescriptorPublicKey, ()> {
            Ok(match k {
                DescriptorPublicKey::Single(sk) => {
                    let origin = if !self.0 { None } else if sk.origin.is_some() { sk.origin.clone() } else {
                        match &sk.key {
                            SinglePubKey::FullKey(pk) => {
                                let h = pk.pubkey_hash();   // hash160 of the key, as a master's fingerprint is
                                let mut fp = [0u8; 4]; fp.copy_from_slice(&h[..4]);
                                Some((bitcoin::bip32::Fingerprint::from(fp), bitcoin::bip32::DerivationPath::master()))
                            }
                            SinglePubKey::XOnly(_) => sk.origin.clone(),
                        }
                    };
                    DescriptorPublicKey::Single(SinglePub { origin, key: sk.key.clone() })
                }
                other => other.clone(),
            })
        }
        miniscript::translate_hash_clone!(DescriptorPublicKey, DescriptorPublicKey, ());
    }
    let (body, tail) = match text.trim().split_once('#') { Some((b, _)) => (b, true), None => (text.trim(), false) };
    let d: Descriptor<DescriptorPublicKey> = body.parse().ok()?;
    let out = d.translate_pk(&mut T(add)).ok()?.to_string();
    // miniscript's own text always carries a checksum; a line that came without one keeps none
    Some(if tail { out } else { out.split('#').next()?.to_string() })
}

pub fn recovery_line(v: &Vault) -> String {
    if let Some(ext) = &v.ext { return with_key_origins(ext); }   // the wallet descriptor IS its recovery line, with the seeds
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
    let rotating = rotates(v);
    let mut coins = Vec::new();
    let mut history = Vec::new();
    // Each chain is asked in batches: the next window of unasked addresses in one request, until `gap` unused ones
    // follow the last used. The same addresses, in the same order, as one request per address used to ask.
    let mut chain = |base: u32| -> Result<(u32, u32), String> {
        let mut last_used: Option<u32> = None;
        let mut next: Option<u32> = None;
        let mut used: Vec<(u32, ScriptBuf)> = Vec::new();
        let mut i = 0u32;
        loop {
            let end = if !rotating { 1 } else { last_used.map(|u| u + gap + 1).unwrap_or(gap) };
            if i >= end { break; }
            let idx: Vec<u32> = (i..end).collect();
            let spks = idx.iter().map(|&p| script_at(v, base | p)).collect::<Result<Vec<_>, _>>()?;
            // a failed query is a failed scan, never "no history": an empty answer resets the next address and the watcher's memory
            let answers = chain::histories(servers, &spks)?;
            for ((p, spk), h) in idx.iter().zip(spks).zip(answers) {
                if !h.is_empty() {
                    last_used = Some(*p);
                    for item in h { history.push((item.txid, item.height, base | p)); }
                    used.push((base | p, spk));
                } else if next.is_none() { next = Some(*p); }
            }
            i = end;
        }
        let spks: Vec<ScriptBuf> = used.iter().map(|(_, s)| s.clone()).collect();
        for ((index, _), list) in used.iter().zip(chain::unspents(servers, &spks)?) {
            for c in list { coins.push(CoinAt { txid: c.txid, vout: c.vout, value: c.value, height: c.height, index: *index }); }
        }
        let last = i.saturating_sub(1);
        Ok((next.unwrap_or(last + 1), last))
    };
    let (next, scan_max) = chain(0)?;
    let (next_change, scan_max_change) = if rotating { chain(CHANGE)? } else { (0, 0) };
    Ok(Scanned { coins, history, next, scan_max, next_change, scan_max_change })
}

/// "Is this script ours, and at which index?" for a loop over many outputs: the range is derived once and the answer
/// is one lookup in a table taken once, instead of a lock and a range check per output.
pub fn ours_table(v: &Vault) -> Result<impl Fn(&ScriptBuf) -> Option<u32>, String> {
    let (recv, chg, rotating) = (v.scan_max.max(v.next) + GAP, v.scan_max_change.max(v.next_change) + GAP, rotates(v));
    let mut table: HashMap<ScriptBuf, u32> = HashMap::new();
    if !rotating { table.insert(script_at(v, 0)?, 0); } else {
        for i in 0..=recv { table.insert(script_at(v, i)?, i); }
        for i in 0..=chg { table.insert(script_at(v, CHANGE | i)?, CHANGE | i); }
    }
    Ok(move |spk: &ScriptBuf| table.get(spk).copied())
}

/// Which index a PSBT input pays to, by matching its script against the vault's.
pub fn index_of_input(v: &Vault, spk: &ScriptBuf) -> Result<Option<u32>, String> {
    if !rotates(v) { return Ok(if &script_at(v, 0)? == spk { Some(0) } else { None }); }
    let (recv, chg) = (v.scan_max.max(v.next) + GAP, v.scan_max_change.max(v.next_change) + GAP);
    // derive the range once (a no-op after the first call), then one lookup — but only inside that range, as before
    for i in 0..=recv { script_at(v, i)?; }
    for i in 0..=chg { script_at(v, CHANGE | i)?; }
    let hit = DERIVED.lock().map_err(|_| "derivation cache poisoned")?.as_ref().and_then(|m| m.get(&identity(v))).and_then(|d| d.rev.get(spk).copied());
    Ok(hit.filter(|&i| if is_change(i) { position(i) <= chg } else { i <= recv }))
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
    /// the coins it chose, as `txid:vout` — what Send shows before anything is signed
    #[serde(default)] pub chosen: Vec<String>,
    /// the transaction's id, which signatures cannot change (segwit), so what Send records against it stands
    #[serde(default)] pub txid: String,
}

/// What a spend WOULD take, worked out without building anything: the same choice `build` makes, so what Send shows
/// and what gets signed cannot drift apart.
#[derive(Serialize, Clone, Debug)]
pub struct Choice {
    pub chosen: Vec<String>,
    pub spending: u64,   // the coins' total
    pub send: u64,
    pub change: u64,
    pub fee: u64,
    pub vsize: u64,
    /// what actually leaves the vault: the payment and the fee, never the change
    pub leaving: u64,
}

/// Which coins a spend takes, and what it costs — the whole arithmetic of a spend with no transaction made. Send
/// shows this before anything is signed; `build` then calls it rather than choosing again, so what is shown and
/// what is signed are the same choice.
pub fn select(vault: &Vault, coins: &[CoinAt], change_index: u32, to: &str, amount: u64, rate: f64, send_all: bool, pick: Option<&[String]>) -> Result<Choice, String> {
    let network = network_of(&vault.network);
    let to_addr = Address::from_str(to).map_err(|e| format!("address: {e}"))?.require_network(network).map_err(|_| "that address is for another network".to_string())?;
    let in_vb = in_vbytes(vault.threshold, vault.keys.len());
    let out_vb = 9 + to_addr.script_pubkey().len() as u64;
    let change_spk = parse(&descriptor_at(vault, change_index)?)?.at_derivation_index(0).map_err(|e| e.to_string())?.script_pubkey();
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
    Ok(Choice { chosen: chosen.iter().map(|c| format!("{}:{}", c.txid, c.vout)).collect(), spending: have, send, change, fee, vsize, leaving: have - change })
}

/// A card's key in a mixed vault is a plain key: it is derived from nothing, so miniscript gives it an origin with an
/// EMPTY path. Such an entry tells a signer nothing — and SeedSigner reads the path of the FIRST key on a change
/// output two levels from its end (`"m".split("/")[-2]`), which threw `IndexError: list index out of range` and
/// stopped it before it could show the spend (2026-09-23, on the bench). Keys with no derivation are left out.
/// Derivations are not covered by any sighash, so removing them never disturbs a signature already on the PSBT.
pub fn strip_pathless(p: &mut bitcoin::psbt::Psbt) -> bool {
    let n = |p: &bitcoin::psbt::Psbt| p.inputs.iter().map(|i| i.bip32_derivation.len()).sum::<usize>() + p.outputs.iter().map(|o| o.bip32_derivation.len()).sum::<usize>();
    let before = n(p);
    for i in p.inputs.iter_mut() { i.bip32_derivation.retain(|_, (_, path)| !path.is_empty()); }
    for o in p.outputs.iter_mut() { o.bip32_derivation.retain(|_, (_, path)| !path.is_empty()); }
    n(p) != before
}

/// The same for a stored spend — one built before the fix above is repaired where it lies, rather than made again.
/// `Some` only when something had to go.
pub fn without_pathless(b64: &str) -> Option<String> {
    use std::str::FromStr;
    let mut p = bitcoin::psbt::Psbt::from_str(b64.trim()).ok()?;
    strip_pathless(&mut p).then(|| p.to_string())
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
    let ch = select(vault, coins, change_index, to, amount, rate, send_all, pick)?;
    let mut chosen: Vec<&CoinAt> = Vec::new();
    for op in &ch.chosen { chosen.push(coins.iter().find(|c| format!("{}:{}", c.txid, c.vout) == *op).ok_or("a chosen coin went missing")?); }
    let (vsize, fee, send, change) = (ch.vsize, ch.fee, ch.send, ch.change);
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
    if let Some(ext) = &vault.ext {
        use miniscript::psbt::PsbtExt;
        for (i, c) in chosen.iter().enumerate() { psbt.update_input_with_descriptor(i, &ext_definite(ext, c.index)?).map_err(|e| format!("input {}: {e}", i + 1))?; }
        if change > 0 { psbt.update_output_with_descriptor(1, &ext_definite(ext, change_index)?).map_err(|e| format!("change: {e}"))?; }
    }
    strip_pathless(&mut psbt);
    Ok(Built { txid: psbt.unsigned_tx.compute_txid().to_string(), psbt: psbt.to_string(), fee, vsize, change, inputs: chosen.len(), chosen: ch.chosen })
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
    if let Some(ext) = &v.ext {
        // the descriptor's keys at that index, in its own order — which is the order v.keys was made in
        let pos = v.keys.iter().position(|k| k.pubkey == base).ok_or("not one of this vault's keys")?;
        let at = ext_keys_at(ext, index)?;
        return PublicKey::from_str(at.get(pos).ok_or("the descriptor has fewer keys than the vault")?).map(bitcoin::PublicKey::new).map_err(|e| e.to_string());
    }
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
    if let Some(cur) = current {
        let mine = Psbt::from_str(cur.trim()).map_err(|e| format!("psbt: {e}"))?;
        if mine.unsigned_tx == psbt.unsigned_tx {
            for (theirs, ours) in psbt.inputs.iter_mut().zip(mine.inputs.iter()) {
                if theirs.witness_utxo.is_none() { theirs.witness_utxo = ours.witness_utxo.clone(); }
                if theirs.witness_script.is_none() { theirs.witness_script = ours.witness_script.clone(); }
                for (k, v) in &ours.bip32_derivation { theirs.bip32_derivation.entry(*k).or_insert_with(|| v.clone()); }
            }
            for (theirs, ours) in psbt.outputs.iter_mut().zip(mine.outputs.iter()) {
                if theirs.witness_script.is_none() { theirs.witness_script = ours.witness_script.clone(); }
                for (k, v) in &ours.bip32_derivation { theirs.bip32_derivation.entry(*k).or_insert_with(|| v.clone()); }
            }
        }
    }
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
        Vault { name: "t".into(), network: "signet".into(), threshold: 2, keys: keys.iter().enumerate().map(|(i, k)| KeyRef { name: format!("k{i}"), serial: 0, pubkey: k.to_string() , device: None }).collect(),
            descriptor, address: String::new(), created: 0, chain_code: cc.map(|c| c.to_string()), next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default(), ext: None, cost_method: None }
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

#[cfg(test)]
mod ext_tests {
    use super::*;
    fn vault_of(desc: &str) -> Vault {
        let info = ext_parse(desc).unwrap().unwrap();
        Vault { name: "ext".into(), network: info.network.clone(), threshold: info.threshold, keys: vec![], descriptor: info.descriptor.clone(), address: String::new(), created: 0,
            chain_code: None, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(),
            files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default(), ext: Some(info.descriptor), cost_method: None }
    }
    fn fixtures() -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/ext-descriptors.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap() }
    fn check_against_embit(case: &str) {
        let f = &fixtures()[case];
        let v = vault_of(f["descriptor"].as_str().unwrap());
        for (chain, base) in [("recv", 0u32), ("chg", CHANGE)] {
            for (i, want) in f[chain].as_array().unwrap().iter().enumerate() {
                assert_eq!(address_at(&v, base | i as u32).unwrap(), want.as_str().unwrap(), "{case} {chain} {i}");
            }
        }
    }
    // expected addresses computed by embit, the library SeedSigner uses — not by this app
    #[test] fn a_sortedmulti_xpub_wallet_derives_what_embit_derives() { check_against_embit("sortedmulti"); }
    #[test] fn a_multi_wallet_keeps_its_key_order_and_derives_what_embit_derives() { check_against_embit("multi"); }
    #[test] fn a_single_key_xpub_wallet_derives_what_embit_derives() { check_against_embit("wpkh"); }
    #[test]
    fn multi_and_sortedmulti_are_never_confused() {
        let f = fixtures();
        let (m, sm) = (vault_of(f["multi"]["descriptor"].as_str().unwrap()), vault_of(f["sortedmulti"]["descriptor"].as_str().unwrap()));
        assert_ne!(address_at(&m, 0).unwrap(), address_at(&sm, 0).unwrap(), "the same keys, two different wallets");
    }
    #[test]
    fn what_it_will_not_stand_behind_is_refused_with_the_reason() {
        let sm = fixtures()["sortedmulti"]["descriptor"].as_str().unwrap().split('#').next().unwrap().to_string();
        let no_origin = sm.replacen("[c9ad277c/48h/0h/0h/2h]", "", 1);
        assert!(ext_parse(&no_origin).unwrap_err().contains("origin"));
        let one_chain = sm.replace("/<0;1>/*", "/0/*");
        assert!(ext_parse(&one_chain).unwrap_err().contains("<0;1>"));
        let fixed = "02e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13";
        let first_key_end = sm.find(",[").map(|i| &sm[i + 1..]).unwrap();
        let mixed = format!("wsh(sortedmulti(2,{fixed},{first_key_end}");
        // a mixed vault is now one the app builds itself (a card as a FIXED key beside device keys) — accepted, the card marked
        let m = ext_parse(&mixed).unwrap().unwrap();
        assert!(m.keys.iter().any(|k| k.fixed && k.xpub == fixed) && m.keys.iter().any(|k| !k.fixed), "{m:?}");
        assert!(keys_of("wsh(multi(2,02e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13,03e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13))").unwrap_err().contains("order"),
            "a fixed-key multi() is refused, never silently rebuilt as sortedmulti()");
    }
    #[test]
    fn case_survives_and_the_checksum_is_checked() {
        let good = fixtures()["sortedmulti"]["descriptor"].as_str().unwrap().to_string();
        assert!(ext_parse(&good).unwrap().is_some());
        assert!(ext_parse(&good.to_lowercase()).is_err(), "base58 is case-sensitive: a lowercased xpub is a different, invalid key");
        let bad_sum = format!("{}#aaaaaaaa", good.split('#').next().unwrap());
        assert!(ext_parse(&bad_sum).is_err(), "a wrong checksum is refused, not ignored");
    }
    #[test]
    fn an_extended_key_vault_ROTATES_even_though_it_has_no_chain_code() {
        // the trap: "rotating" had been read off the chain code, which only card vaults have
        let v = vault_of(fixtures()["sortedmulti"]["descriptor"].as_str().unwrap());
        assert!(v.chain_code.is_none() && rotates(&v));
        assert_ne!(address_at(&v, 0).unwrap(), address_at(&v, 1).unwrap(), "a fresh address per deposit, never the same one");
    }
    #[test]
    fn its_own_change_is_recognised_as_its_own() {
        let mut v = vault_of(fixtures()["sortedmulti"]["descriptor"].as_str().unwrap());
        v.scan_max = 3; v.scan_max_change = 3;
        let chg2 = script_of(&descriptor_at(&v, CHANGE | 2).unwrap()).unwrap();
        let recv3 = script_of(&descriptor_at(&v, 3).unwrap()).unwrap();
        assert_eq!(index_of_input(&v, &chg2).unwrap(), Some(CHANGE | 2), "change output 2 is this vault's");
        assert_eq!(index_of_input(&v, &recv3).unwrap(), Some(3));
        let stranger = script_of(&descriptor_at(&vault_of(fixtures()["multi"]["descriptor"].as_str().unwrap()), 0).unwrap()).unwrap();
        assert_eq!(index_of_input(&v, &stranger).unwrap(), None, "and a different wallet's address is not");
    }
    #[test]
    fn a_descriptor_is_kept_as_written_so_its_checksum_matches_the_other_wallets() {
        // embit prints a descriptor WITHOUT a checksum, so give the 48h text its own, the way Sparrow shows it
        let bare_h = fixtures()["sortedmulti"]["descriptor"].as_str().unwrap().split('#').next().unwrap().to_string();
        let written = format!("{bare_h}#{}", miniscript::descriptor::checksum::desc_checksum(&bare_h).unwrap());
        let info = ext_parse(&written).unwrap().unwrap();
        assert_eq!(info.descriptor, written, "stored exactly as it came, checksum and all");
        assert!(info.descriptor.contains("/48h/") && !info.canonical.contains("/48h/"), "the canonical form re-writes h as ' — which is why it is not what is kept");
        let bare = written.split('#').next().unwrap();
        let from_bare = ext_parse(bare).unwrap().unwrap();
        assert!(from_bare.descriptor.contains('#'), "with no checksum given, one is added");
        assert_eq!(from_bare.canonical, info.canonical, "and the two notations are recognised as the same wallet");
    }
    #[test]
    fn a_mixed_vault_of_a_device_key_and_two_cards_derives_what_embit_derives() {
        let f: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/mixed-descriptor.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
        let v = vault_of(f["descriptor"].as_str().unwrap());
        for (chain, base) in [("recv", 0u32), ("chg", CHANGE)] {
            for (i, want) in f[chain].as_array().unwrap().iter().enumerate() { assert_eq!(address_at(&v, base | i as u32).unwrap(), want.as_str().unwrap(), "{chain} {i}"); }
        }
        let info = ext_parse(f["descriptor"].as_str().unwrap()).unwrap().unwrap();
        let cards: Vec<&str> = f["cards"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
        assert_eq!(info.keys.iter().filter(|k| k.fixed).map(|k| k.xpub.as_str()).collect::<Vec<_>>().len(), 2);
        assert!(info.keys.iter().filter(|k| k.fixed).all(|k| cards.contains(&k.xpub.as_str())), "each card is the bare key it holds, so a card plugged in is recognised");
        assert!(rotates(&v) && address_at(&v, 0).unwrap() != address_at(&v, 1).unwrap(), "still a fresh address every deposit: the device key moves");
    }
    /// A real wallet scanned against the real server — the thing no mock can show: that the scan goes past the first
    /// address and down the change chain. `EXT_DESC='…' cargo test ext_live_scan -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ext_live_scan() {
        let v = vault_of(&std::env::var("EXT_DESC").expect("EXT_DESC"));
        let servers = Servers { electrum: "ssl://electrum.mineracks.com:50002".into(), explorer: "https://mempool.mineracks.com".into() };
        let sc = scan(&v, &servers, 20).expect("the scan answered");
        let bal: u64 = sc.coins.iter().map(|c| c.value).sum();
        let recv_used = sc.history.iter().filter(|h| !is_change(h.2)).map(|h| h.2).collect::<std::collections::BTreeSet<_>>();
        let chg_used = sc.history.iter().filter(|h| is_change(h.2)).map(|h| position(h.2)).collect::<std::collections::BTreeSet<_>>();
        println!("SCAN history={} coins={} balance={} sats | receive indexes used {:?} (next {}, looked to {}) | change indexes used {:?} (next {}, looked to {})",
            sc.history.len(), sc.coins.len(), bal, recv_used, sc.next, sc.scan_max, chg_used, sc.next_change, sc.scan_max_change);
        assert!(sc.scan_max >= 19, "the receive chain was scanned past the gap, not stopped at index 0");
        assert!(sc.scan_max_change >= 19, "and the change chain was scanned at all");
    }
    /// Prints this app's addresses for a descriptor given in EXT_DESC, to compare with another implementation.
    /// A real wallet's descriptor is never committed: `EXT_DESC='…' cargo test ext_print -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ext_print() {
        let v = vault_of(&std::env::var("EXT_DESC").expect("EXT_DESC"));
        for (name, base) in [("recv", 0u32), ("chg", CHANGE)] { for i in 0..5 { println!("{name} {i} {}", address_at(&v, base | i).unwrap()); } }
    }
}

