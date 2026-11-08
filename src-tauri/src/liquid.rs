//! A **watched** Liquid wallet — Aqua's spending accounts, Blockstream Green's single-signature Liquid wallet, and
//! anything else built on their engine (GDK). Liquid is a sidechain: every amount and every asset in a transaction
//! is hidden by a confidential commitment, so an account key alone shows WHICH transactions touch a wallet and
//! nothing else. Reading the figures takes the wallet's SLIP-77 master blinding key as well — the view key — which
//! Aqua derives from the same twelve words and never exports. So the words are read once, here, and only the two
//! keys that reading needs are kept: the account key (BIP 84 or 49, coin 1776) and the blinding key. Neither can
//! spend. The outputs are unblinded on this computer; our own Liquid node (Blockstream's when ours is down) is asked
//! only which scripts have history and for the raw transactions, the same questions any Liquid wallet asks it.
//!
//! What the chain gives: dates, amounts, assets, fees, and whether a transaction was a receive, a send, or a swap
//! (L-BTC out and another asset in, in the one transaction — a SideSwap trade). What it cannot give: the labels the
//! app itself wrote ("Funded …", "USDt fee"), which live only in the app. L-BTC is counted in sats like every other
//! vault; the other assets ride on each row in their own units, shown but not priced.
use bitcoin::bip32::{ChildNumber, DerivationPath, Xpriv, Xpub};
use elements::encode::deserialize;
use hmac::{Hmac, Mac};
use sha2::{Sha256, Sha512};
use elements::secp256k1_zkp as zkp;
use elements::{Address, AddressParams, AssetId, Script, Transaction};
use electrum_client::ElectrumApi;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

/// Liquid Bitcoin's asset id, and Tether's: the two every Aqua wallet holds
pub const LBTC: &str = "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d";
pub const USDT: &str = "ce091c998b83c78bb71a632313ba3760f1763d9cfcffae02258ffa9865a37bd2";
/// Liquid's coin type in a derivation path
const COIN: u32 = 1776;
/// Our own Liquid node (elementsd + electrs on the liquid-node VM, liquid.mineracks.com in front of it) is asked
/// first; Blockstream's server only when ours does not answer, and then the figures say so. Which one answered is
/// kept on the vault for that line.
pub fn servers() -> crate::chain::Servers { crate::chain::Servers { electrum: "ssl://liquid-electrum.mineracks.com:50002".into(), explorer: "https://liquid.mineracks.com".into() } }
pub fn fallback() -> crate::chain::Servers { crate::chain::Servers { electrum: "ssl://blockstream.info:995".into(), explorer: "https://blockstream.info/liquid".into() } }
/// A connection to our node, or to Blockstream's when ours is down: the servers used come back with it.
fn connect() -> Result<(std::sync::Arc<electrum_client::Client>, crate::chain::Servers), String> {
    match crate::chain::client(&servers()) {
        Ok(c) => Ok((c, servers())),
        Err(ours) => { crate::chain::forget(&servers()); let c = crate::chain::client(&fallback()).map_err(|e| format!("our Liquid node did not answer ({ours}) and neither did Blockstream's ({e})"))?; Ok((c, fallback())) }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Liquid {
    /// the account key, an ordinary xpub: which transactions are the wallet's
    pub xpub: String,
    pub fingerprint: String,
    /// "84'/1776'/0'" — the path the account key sits at, as discovered from the words
    pub path: String,
    /// BIP 49 (an address wrapped in pay-to-script-hash, Green's older default) rather than BIP 84 native segwit
    #[serde(default)] pub wrapped: bool,
    /// the SLIP-77 master blinding key, hex: what reads the amounts and assets. It sees; it cannot spend.
    pub blinding: String,
    #[serde(default)] pub next: u32,
    #[serde(default)] pub next_change: u32,
    /// every transaction read and unblinded, by id. A confirmed one never changes; an unconfirmed one is read again.
    #[serde(default)] pub txs: BTreeMap<String, Tx>,
    /// what each asset id is called, from Blockstream's asset registry, read once
    #[serde(default)] pub assets: BTreeMap<String, AssetInfo>,
    /// asset id → units held, as of the last sync (L-BTC in sats, USDt in hundred-millionths)
    #[serde(default)] pub balances: BTreeMap<String, u64>,
    /// when the chain was last read, unix seconds
    #[serde(default)] pub read_at: u64,
    /// which Electrum server answered the last read: ours, or Blockstream's as the fallback
    #[serde(default)] pub server: String,
    /// the ONE asset this vault shows. The account record (keys, transactions) is the same on every sibling vault
    /// of the account; each projects its own asset — L-BTC in sats, Tether in USDt — because they are different
    /// classes of asset with different accounting, and a swap between them is a disposal of one for the other.
    /// Absent in stores written before 8 Nov 2026, evening: L-BTC.
    #[serde(default = "lbtc_id")] pub asset: String,
}
pub fn lbtc_id() -> String { LBTC.into() }
impl Liquid {
    /// this vault's asset in its own units: the sats row for L-BTC, Tether's hundred-millionths for USDt
    pub fn held(&self) -> u64 { self.balances.get(&self.asset).copied().unwrap_or(0) }
    pub fn ticker(&self) -> String { ticker(self, &self.asset) }
    pub fn is_lbtc(&self) -> bool { self.asset == LBTC }
    /// the same account record, projected onto another of its assets
    pub fn for_asset(&self, asset: &str) -> Liquid { Liquid { asset: asset.into(), ..self.clone() } }
    /// every asset the account has ever moved, L-BTC first
    pub fn assets_moved(&self) -> Vec<String> {
        let mut v: Vec<String> = self.txs.values().flat_map(|t| t.moves.keys().cloned()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        v.sort_by_key(|a| a != LBTC);
        v
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AssetInfo { pub ticker: String, pub name: String, pub precision: u8 }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Tx {
    pub id: String,
    /// 0 = unconfirmed
    pub height: u32,
    pub time: u32,
    /// the transaction's fee, in L-BTC sats, and whether this wallet paid it (it had an input)
    pub fee: u64,
    #[serde(default)] pub paid_fee: bool,
    /// asset id → this wallet's net movement in that asset's units (in minus out), fee included for L-BTC
    pub moves: BTreeMap<String, i64>,
    /// this wallet's own outputs: vout, asset id, units — what a later transaction may spend
    pub outs: Vec<(u32, String, u64)>,
    /// every input's outpoint (txid, vout): which of this wallet's outputs it spent is decided once all are known
    #[serde(default)] pub ins: Vec<(String, u32)>,
    /// how many inputs and outputs the transaction has in all: more than ours alone says it was shared (a swap)
    #[serde(default)] pub ins_all: u32,
    #[serde(default)] pub outs_all: u32,
}

/// The SLIP-77 master blinding key from a BIP 39 seed, exactly as libwally and Green derive it: the SLIP-21
/// symmetric node labelled "SLIP-0077", of which the key half is kept.
pub fn master_blinding_key(seed: &[u8]) -> [u8; 32] {
    let mut root = Hmac::<Sha512>::new_from_slice(b"Symmetric key seed").expect("any key length");
    root.update(seed);
    let root = root.finalize().into_bytes();
    let mut node = Hmac::<Sha512>::new_from_slice(&root[0..32]).expect("any key length");
    node.update(&[0]);
    node.update(b"SLIP-0077");
    let node = node.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&node[32..64]);
    out
}
/// The blinding private key of one script: HMAC-SHA256 of the script under the master key.
fn blinding_private_key(master: &[u8; 32], spk: &Script) -> Result<zkp::SecretKey, String> {
    let mut eng = Hmac::<Sha256>::new_from_slice(master).expect("any key length");
    eng.update(spk.as_bytes());
    zkp::SecretKey::from_slice(&eng.finalize().into_bytes()).map_err(|e| format!("blinding key: {e}"))
}

fn master_of(l: &Liquid) -> Result<[u8; 32], String> {
    let v = hex::decode(&l.blinding).map_err(|_| "this vault's blinding key is damaged")?;
    v.try_into().map_err(|_| "this vault's blinding key is the wrong length".to_string())
}

/// The public key at chain/index under the account key.
fn pubkey_at(xpub: &Xpub, chain: u32, index: u32) -> Result<bitcoin::PublicKey, String> {
    let secp = bitcoin::secp256k1::Secp256k1::verification_only();
    let k = xpub.derive_pub(&secp, &[ChildNumber::from_normal_idx(chain).map_err(|e| e.to_string())?, ChildNumber::from_normal_idx(index).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
    Ok(bitcoin::PublicKey::new(k.public_key))
}
/// The unconfidential script at chain/index — what the chain is asked about, and what the blinding key is keyed on.
fn script_at(l: &Liquid, xpub: &Xpub, chain: u32, index: u32) -> Result<Script, String> {
    let pk = pubkey_at(xpub, chain, index)?;
    let a = if l.wrapped { Address::p2shwpkh(&pk, None, &AddressParams::LIQUID) } else { Address::p2wpkh(&pk, None, &AddressParams::LIQUID) };
    Ok(a.script_pubkey())
}
/// The confidential address at chain/index: the one Aqua itself would show for that slot.
pub fn address_at(l: &Liquid, chain: u32, index: u32) -> Result<String, String> {
    let xpub = Xpub::from_str(&l.xpub).map_err(|e| format!("account key: {e}"))?;
    let pk = pubkey_at(&xpub, chain, index)?;
    let secp = zkp::Secp256k1::signing_only();
    let spk = script_at(l, &xpub, chain, index)?;
    let blinder = zkp::PublicKey::from_secret_key(&secp, &blinding_private_key(&master_of(l)?, &spk)?);
    let a = if l.wrapped { Address::p2shwpkh(&pk, Some(blinder), &AddressParams::LIQUID) } else { Address::p2wpkh(&pk, Some(blinder), &AddressParams::LIQUID) };
    Ok(a.to_string())
}

/// From the words: the account key and the blinding key, and nothing else kept. Aqua keeps its Liquid account at
/// BIP 84 account 0; Green's older single-signature default was BIP 49; a person who made more accounts has them at
/// 1 and up. The first path with any history on its first twenty receiving addresses is the wallet. The words live
/// for this call only.
pub fn from_words(words: zeroize::Zeroizing<String>, gap: u32) -> Result<Liquid, String> {
    use zeroize::Zeroize;
    let norm = words.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let m = bip39::Mnemonic::parse_in_normalized(bip39::Language::English, &norm).map_err(|e| format!("those words do not check out: {e}"))?;
    let mut seed = m.to_seed("");
    let blinding = master_blinding_key(&seed);
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let master = Xpriv::new_master(bitcoin::Network::Bitcoin, &seed).map_err(|e| e.to_string())?;
    seed.zeroize();
    drop(m); drop(words);
    let fingerprint = master.fingerprint(&secp).to_string();
    let mut tried = Vec::new();
    for (purpose, account) in [(84u32, 0u32), (49, 0), (84, 1), (49, 1)] {
        let path = DerivationPath::from_str(&format!("m/{purpose}'/{COIN}'/{account}'")).map_err(|e| e.to_string())?;
        let xpub = Xpub::from_priv(&secp, &master.derive_priv(&secp, &path).map_err(|e| e.to_string())?);
        let mut l = Liquid { xpub: xpub.to_string(), fingerprint: fingerprint.clone(), path: format!("{purpose}'/{COIN}'/{account}'"), wrapped: purpose == 49,
            blinding: hex::encode(blinding), next: 0, next_change: 0, txs: Default::default(), assets: Default::default(), balances: Default::default(), read_at: 0, server: String::new(), asset: LBTC.into() };
        let (c, sv) = connect()?;
        let scripts: Vec<Script> = (0..gap.max(20)).map(|i| script_at(&l, &xpub, 0, i)).collect::<Result<_, _>>()?;
        let bscripts: Vec<bitcoin::ScriptBuf> = scripts.iter().map(|s| bitcoin::ScriptBuf::from_bytes(s.as_bytes().to_vec())).collect();
        let hist = c.batch_script_get_history(bscripts.iter().map(|s| s.as_script())).map_err(|e| { crate::chain::forget(&sv); format!("Liquid server {}: {e}", sv.electrum) })?;
        let used = hist.iter().filter(|h| !h.is_empty()).count();
        tried.push(format!("{purpose}'/{COIN}'/{account}' ({used} of {} addresses used)", scripts.len()));
        if used > 0 {
            sync(&mut l, gap)?;
            return Ok(l);
        }
    }
    Err(format!("those words derive Liquid keys, but no history was found under them on the chain ({}). The wallet may be empty, or keep its account at a path not tried here.", tried.join("; ")))
}

/// Read the chain: which transactions touch the wallet's scripts (twenty past the last used, on both chains), the raw
/// transactions not read before, each of this wallet's outputs unblinded, then every movement per asset and the
/// balances. Everything read is kept on the vault; a sync after this one asks only about what is new.
pub fn sync(l: &mut Liquid, gap: u32) -> Result<(), String> {
    let gap = gap.max(20);
    let xpub = Xpub::from_str(&l.xpub).map_err(|e| format!("account key: {e}"))?;
    let master = master_of(l)?;
    let (c, sv) = connect()?;
    let fail = |e: electrum_client::Error| { crate::chain::forget(&sv); format!("Liquid server {}: {e}", sv.electrum) };
    // 1. which scripts have history — chain 0 (receiving) and 1 (change), each scanned to a gap past its last use
    let mut ours: HashMap<Vec<u8>, (u32, u32)> = HashMap::new();   // script bytes → (chain, index)
    let mut listed: HashMap<String, i32> = HashMap::new();         // txid → height as the server says it (0/-1 unconfirmed)
    for chain in [0u32, 1] {
        let (mut from, mut last_used) = (0u32, None::<u32>);
        loop {
            let to = last_used.map(|u| u + 1 + gap).unwrap_or(gap).max(from + 1);
            let scripts: Vec<Script> = (from..to).map(|i| script_at(l, &xpub, chain, i)).collect::<Result<_, _>>()?;
            let bscripts: Vec<bitcoin::ScriptBuf> = scripts.iter().map(|s| bitcoin::ScriptBuf::from_bytes(s.as_bytes().to_vec())).collect();
            let hist = c.batch_script_get_history(bscripts.iter().map(|s| s.as_script())).map_err(fail)?;
            for (k, h) in hist.iter().enumerate() {
                let index = from + k as u32;
                ours.insert(scripts[k].as_bytes().to_vec(), (chain, index));
                if !h.is_empty() { last_used = Some(index); }
                for e in h { let id = e.tx_hash.to_string(); let v = listed.entry(id).or_insert(e.height); if e.height > *v { *v = e.height; } }
            }
            from = to;
            if last_used.map(|u| from >= u + 1 + gap).unwrap_or(true) { break; }
            if from > 10_000 { return Err("more than ten thousand addresses in use; refusing to scan further".into()); }
        }
        if chain == 0 { l.next = last_used.map(|u| u + 1).unwrap_or(0); } else { l.next_change = last_used.map(|u| u + 1).unwrap_or(0); }
    }
    // 2. the raw transactions: those never read, those read while unconfirmed, and those read by a build that did
    // not keep their inputs (before 8 Nov 2026, evening)
    let want: Vec<(String, i32)> = listed.iter().filter(|(id, h)| l.txs.get(*id).map(|t| (t.height == 0 && **h > 0) || (t.ins.is_empty() && t.ins_all > 0)).unwrap_or(true)).map(|(id, h)| (id.clone(), *h)).collect();
    let secp = zkp::Secp256k1::new();
    let mut times: HashMap<u32, u32> = HashMap::new();
    for (id, height) in want {
        let txid = bitcoin::Txid::from_str(&id).map_err(|e| e.to_string())?;
        let raw = c.transaction_get_raw(&txid).map_err(fail)?;
        let tx: Transaction = deserialize(&raw).map_err(|e| format!("transaction {id}: {e}"))?;
        let height = height.max(0) as u32;
        let time = if height > 0 {
            match times.get(&height) { Some(t) => *t, None => {
                let hdr = c.block_header_raw(height as usize).map_err(fail)?;
                let h: elements::BlockHeader = deserialize(&hdr).map_err(|e| format!("block {height}: {e}"))?;
                times.insert(height, h.time); h.time } }
        } else { 0 };
        let mut t = Tx { id: id.clone(), height, time, fee: 0, paid_fee: false, moves: Default::default(), outs: Vec::new(), ins: tx.input.iter().map(|i| (i.previous_output.txid.to_string(), i.previous_output.vout)).collect(), ins_all: tx.input.len() as u32, outs_all: tx.output.len() as u32 };
        for (vout, o) in tx.output.iter().enumerate() {
            if o.is_fee() { if let elements::confidential::Value::Explicit(v) = o.value { t.fee += v; } continue; }
            if !ours.contains_key(o.script_pubkey.as_bytes()) { continue; }
            let (asset, value) = match (o.asset, o.value) {
                (elements::confidential::Asset::Explicit(a), elements::confidential::Value::Explicit(v)) => (a, v),
                _ => { let s = o.unblind(&secp, blinding_private_key(&master, &o.script_pubkey)?).map_err(|e| format!("output {id}:{vout} could not be unblinded with this wallet's blinding key ({e})"))?; (s.asset, s.value) }
            };
            t.outs.push((vout as u32, asset.to_string(), value));
        }
        l.txs.insert(id.clone(), t);
    }
    // a transaction the server no longer lists (replaced, or dropped from the mempool) goes, unless it was confirmed
    l.txs.retain(|id, t| t.height > 0 || listed.contains_key(id));
    // 3. the movements, for every transaction, now that every output of ours is known: each input that spends one
    // of this wallet's outputs takes that asset away, each output of ours brings it in. Done over the whole set and
    // not just the new ones — the order the server handed them over must not decide what a send looks like (the
    // first build booked sends as receives of their change, because a spend was looked up before its coin was read).
    book(&mut l.txs);
    // 4. balances: the sum of every movement is what is held
    let mut bal: BTreeMap<String, i64> = BTreeMap::new();
    for t in l.txs.values() { for (a, v) in &t.moves { *bal.entry(a.clone()).or_insert(0) += v; } }
    l.balances = bal.into_iter().filter(|(_, v)| *v > 0).map(|(a, v)| (a, v as u64)).collect();
    // 5. what the assets are called, asked once each
    let unknown: Vec<String> = l.txs.values().flat_map(|t| t.moves.keys().cloned()).filter(|a| a != LBTC && !l.assets.contains_key(a)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    for a in unknown { if let Some(info) = registry(&a) { l.assets.insert(a, info); } }
    l.server = sv.electrum.clone();
    l.read_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    Ok(())
}

/// Every transaction's movements from the whole set's inputs and outputs — the same answer whatever order the
/// transactions were read in. `paid_fee` is true only when one of our own L-BTC coins went in: that is the only
/// coin a fee can come from.
pub fn book(txs: &mut BTreeMap<String, Tx>) {
    let outs: HashMap<(String, u32), (String, u64)> = txs.values().flat_map(|t| t.outs.iter().map(move |(v, a, n)| ((t.id.clone(), *v), (a.clone(), *n)))).collect();
    for t in txs.values_mut() {
        let mut moves: BTreeMap<String, i64> = BTreeMap::new();
        let mut paid = false;
        for (ptx, vout) in &t.ins {
            if let Some((asset, value)) = outs.get(&(ptx.clone(), *vout)) {
                *moves.entry(asset.clone()).or_insert(0) -= *value as i64;
                // Only L-BTC pays a Liquid fee: a wallet whose inputs are all another asset had the fee paid for it
                // (Aqua's "pay fees in USDt" sends a little more USDt to a service that adds the L-BTC input).
                if asset == LBTC { paid = true; }
            }
        }
        for (_, asset, value) in &t.outs { *moves.entry(asset.clone()).or_insert(0) += *value as i64; }
        moves.retain(|_, v| *v != 0);
        t.paid_fee = paid;
        t.moves = moves;
    }
}

/// Blockstream's asset registry: ticker, name and precision for an asset id. Tether is known without asking.
fn registry(asset: &str) -> Option<AssetInfo> {
    if asset == USDT { return Some(AssetInfo { ticker: "USDt".into(), name: "Tether USD".into(), precision: 8 }); }
    let url = format!("https://assets.blockstream.info/{asset}.json");
    let v: serde_json::Value = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(10)).build().ok()?.get(&url).send().ok()?.json().ok()?;
    Some(AssetInfo { ticker: v["ticker"].as_str().unwrap_or("").to_string(), name: v["name"].as_str().unwrap_or("").to_string(), precision: v["precision"].as_u64().unwrap_or(0) as u8 })
}

/// What one unit of an asset is priced as: L-BTC at a bitcoin, USDt at a US dollar; any other asset has no price here.
pub fn unit_of(asset: &str) -> Option<crate::activity::Unit> {
    match asset { LBTC => Some(crate::activity::Unit::Btc), USDT => Some(crate::activity::Unit::Usd), _ => None }
}
/// What an asset is called on a row: its ticker, or the first eight characters of its id.
pub fn ticker(l: &Liquid, asset: &str) -> String {
    if asset == LBTC { return "L-BTC".into(); }
    l.assets.get(asset).filter(|a| !a.ticker.is_empty()).map(|a| a.ticker.clone()).unwrap_or_else(|| asset.chars().take(8).collect())
}
/// Units in words: L-BTC in its own precision ("0.02064676"); a money-like asset to the cent ("190.74", "1000.09"),
/// however many decimals it carries — Tether has eight, and nobody reads 1000.09317005. The units themselves are
/// never rounded, only the words.
pub fn amount(l: &Liquid, asset: &str, units: i64) -> String {
    let p = if asset == LBTC { 8 } else { l.assets.get(asset).map(|a| a.precision).unwrap_or(0) } as u32;
    let (sign, mut n) = (if units < 0 { "−" } else { "" }, units.unsigned_abs());
    if p == 0 { return format!("{sign}{n}"); }
    if asset != LBTC && p > 2 { let f = 10u64.pow(p - 2); n = (n + f / 2) / f; return format!("{sign}{}.{:02}", n / 100, n % 100); }
    let d = 10u64.pow(p);
    let frac = format!("{:0width$}", n % d, width = p as usize);
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() { format!("{sign}{}", n / d) } else { format!("{sign}{}.{frac}", n / d) }
}

/// The asset id of Liquid Bitcoin as a typed value, for callers that compare
pub fn lbtc() -> AssetId { AssetId::from_str(LBTC).expect("constant") }

/// One transaction as the activity list shows it: what kind of thing it was, and each asset's movement in words.
#[derive(Serialize, Clone, Debug)]
pub struct RowDetail { pub id: String, pub kind: String, pub height: u32, pub time: u32, pub fee: u64, pub paid_fee: bool, pub moves: Vec<Move> }
#[derive(Serialize, Clone, Debug)]
pub struct Move { pub asset: String, pub ticker: String, pub units: i64, pub amount: String }

/// What kind of transaction each was, as THIS vault's asset saw it: the asset in and another out (or the reverse)
/// is a swap — a SideSwap trade is one transaction, a disposal of one asset for the other; in alone a receive; out
/// alone a send; nothing of this asset moved but the fee paid while another asset moved is, on the L-BTC vault, the
/// fee of that other asset's send; only the fee gone with nothing else is the wallet paying itself. The fee is set
/// aside first so a send of USDt with its L-BTC fee reads as a send, not a swap. Transactions that touched neither
/// this asset nor (for L-BTC) the fee are not this vault's rows.
pub fn rows(l: &Liquid) -> Vec<RowDetail> {
    let mut out: Vec<RowDetail> = l.txs.values().filter(|t| t.moves.contains_key(&l.asset) || (l.is_lbtc() && t.paid_fee)).map(|t| {
        let mut moves: Vec<(String, i64)> = t.moves.iter().map(|(a, v)| (a.clone(), *v)).collect();
        if t.paid_fee { if let Some(m) = moves.iter_mut().find(|(a, _)| a == LBTC) { m.1 += t.fee as i64; } else { moves.push((LBTC.into(), t.fee as i64)); } }
        let mine = moves.iter().find(|(a, _)| a == &l.asset).map(|m| m.1).unwrap_or(0);
        let (other_up, other_down) = (moves.iter().any(|(a, v)| a != &l.asset && *v > 0), moves.iter().any(|(a, v)| a != &l.asset && *v < 0));
        let kind = if (mine > 0 && other_down) || (mine < 0 && other_up) { "swap" } else if mine > 0 { "receive" } else if mine < 0 { "send" }
            else if t.paid_fee && (other_up || other_down) { "fee" } else { "self" };
        let shown: Vec<Move> = t.moves.iter().map(|(a, v)| Move { asset: a.clone(), ticker: ticker(l, a), units: *v, amount: amount(l, a, *v) }).collect();
        RowDetail { id: t.id.clone(), kind: kind.into(), height: t.height, time: t.time, fee: t.fee, paid_fee: t.paid_fee, moves: shown }
    }).collect();
    out.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| a.id.cmp(&b.id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unhex(s: &str) -> Vec<u8> { hex::decode(s).unwrap() }

    /// libwally's own vectors (src/test/test_confidential_addr.py), as carried by elements-miniscript 0.4
    #[test]
    fn the_master_blinding_key_matches_libwally() {
        let mbk = master_blinding_key(&unhex("c76c4ac4f4e4a00d6b274d5c39c700bb4a7ddc04fbc6f78e85ca75007b5b495f74a9043eeb77bdd53aa6fc3a0e31462270316fa04b8c19114c8798706cd02ac8"));
        assert_eq!(hex::encode(mbk), "6c2de18eabeff3f7822bc724ad482bef0557f3e1c1e1c75b7a393a5ced4de616");
        let mbk = master_blinding_key(&unhex("731e9b42eb9774f8a6b51af35a06f6ef1cdb6cf04402163ceacf0c8bace2831a"));
        assert_eq!(hex::encode(mbk), "c2f338e32ad1a2bd9cac569e67728163bf4c326a1770ec2293ba65548a581e97");
        let spk = Script::from_hex("a914afa92d77cd3541b443771649572db096cf49bf8c87").unwrap();
        assert_eq!(hex::encode(blinding_private_key(&mbk, &spk).unwrap().secret_bytes()), "02b067c374bb56c54c016fae29218c000ada60f81ef45b4aeebbeb24931bb8bc");
    }

    #[test]
    fn a_confidential_address_carries_the_scripts_blinding_key() {
        let mbk = master_blinding_key(&unhex("c76c4ac4f4e4a00d6b274d5c39c700bb4a7ddc04fbc6f78e85ca75007b5b495f74a9043eeb77bdd53aa6fc3a0e31462270316fa04b8c19114c8798706cd02ac8"));
        let secp = zkp::Secp256k1::new();
        let spk = Script::from_hex("76a914a579388225827d9f2fe9014add644487808c695d88ac").unwrap();
        let mut addr = Address::from_str("2dpWh6jbhAowNsQ5agtFzi7j6nKscj6UnEr").unwrap();
        addr.blinding_pubkey = Some(zkp::PublicKey::from_secret_key(&secp, &blinding_private_key(&mbk, &spk).unwrap()));
        assert_eq!(addr.to_string(), "CTEkf75DFff5ReB7juTg2oehrj41aMj21kvvJaQdWsEAQohz1EDhu7Ayh6goxpz3GZRVKidTtaXaXYEJ");
    }

    #[test]
    fn amounts_are_shown_in_the_assets_own_precision() {
        let mut l = Liquid { xpub: String::new(), fingerprint: String::new(), path: String::new(), wrapped: false, blinding: String::new(), next: 0, next_change: 0, txs: Default::default(), assets: Default::default(), balances: Default::default(), read_at: 0, server: String::new(), asset: LBTC.into() };
        l.assets.insert(USDT.into(), AssetInfo { ticker: "USDt".into(), name: "Tether USD".into(), precision: 8 });
        assert_eq!(amount(&l, USDT, 19_074_000_000), "190.74");
        assert_eq!(amount(&l, USDT, -5_000_000), "−0.05");
        assert_eq!(amount(&l, USDT, -100_009_317_005), "−1000.09", "to the cent, not eight places");
        assert_eq!(amount(&l, USDT, 1_000_000_000), "10.00");
        assert_eq!(amount(&l, USDT, 9_999_500_000), "100.00", "half a cent rounds up");
        assert_eq!(amount(&l, LBTC, 2_064_676), "0.02064676");
        assert_eq!(amount(&l, LBTC, 100_000_000), "1");
        assert_eq!(ticker(&l, USDT), "USDt");
        assert_eq!(ticker(&l, "abcdef0123456789"), "abcdef01");
    }

    /// The bug of the first build: a send read before the coin it spent was booked as a receive of its change.
    #[test]
    fn a_send_is_a_send_whatever_order_the_transactions_were_read_in() {
        let mut txs = BTreeMap::new();
        // "b…" sorts before "a…"? No — BTreeMap orders by id, so put the spend FIRST by id to be sure order cannot help
        txs.insert("0spend".into(), Tx { id: "0spend".into(), height: 2, time: 2, fee: 10, outs: vec![(1, LBTC.into(), 400)], ins: vec![("1coin".into(), 0)], ins_all: 1, outs_all: 3, ..Default::default() });
        txs.insert("1coin".into(), Tx { id: "1coin".into(), height: 1, time: 1, fee: 0, outs: vec![(0, LBTC.into(), 1000)], ins: vec![("other".into(), 3)], ins_all: 1, outs_all: 2, ..Default::default() });
        book(&mut txs);
        assert_eq!(txs["1coin"].moves[LBTC], 1000); assert!(!txs["1coin"].paid_fee);
        assert_eq!(txs["0spend"].moves[LBTC], -600, "sent 590 and paid 10: the change of 400 came back"); assert!(txs["0spend"].paid_fee);
        let l = Liquid { xpub: String::new(), fingerprint: String::new(), path: String::new(), wrapped: false, blinding: String::new(), next: 0, next_change: 0, txs, assets: Default::default(), balances: Default::default(), read_at: 0, server: String::new(), asset: LBTC.into() };
        let rs = rows(&l);
        assert_eq!(rs.iter().find(|r| r.id == "0spend").unwrap().kind, "send");
        assert_eq!(rs.iter().find(|r| r.id == "1coin").unwrap().kind, "receive");
        // a swap: L-BTC out, USDt in, in the one transaction
        let mut txs = BTreeMap::new();
        txs.insert("coin".into(), Tx { id: "coin".into(), height: 1, time: 1, outs: vec![(0, LBTC.into(), 2_500_000)], ..Default::default() });
        txs.insert("swap".into(), Tx { id: "swap".into(), height: 2, time: 2, fee: 635, outs: vec![(0, USDT.into(), 258_766_109_69)], ins: vec![("coin".into(), 0), ("theirs".into(), 0)], ins_all: 2, outs_all: 4, ..Default::default() });
        book(&mut txs);
        let l = Liquid { txs, ..l };
        assert_eq!(rows(&l).iter().find(|r| r.id == "swap").unwrap().kind, "swap");
    }

    /// Aqua's "pay fees in USDt": the wallet spends only USDt, a service adds the L-BTC input that pays the fee and
    /// keeps a little USDt for it. The three such sends of 19 Jan 2025 read as swaps, and as 0-sat rows of the
    /// L-BTC vault, when "paid the fee" meant "had any input".
    #[test]
    fn a_usdt_send_whose_fee_a_service_paid_is_a_send_and_not_an_lbtc_row() {
        let mut txs = BTreeMap::new();
        txs.insert("coin".into(), Tx { id: "coin".into(), height: 1, time: 1, outs: vec![(0, USDT.into(), 2000_00000000)], ..Default::default() });
        txs.insert("send".into(), Tx { id: "send".into(), height: 2, time: 2, fee: 40, outs: vec![(2, USDT.into(), 999_90682995)], ins: vec![("coin".into(), 0), ("service".into(), 1)], ins_all: 2, outs_all: 5, ..Default::default() });
        book(&mut txs);
        assert!(!txs["send"].paid_fee, "no L-BTC of ours went in, so the fee was not ours");
        assert_eq!(txs["send"].moves[USDT], -1000_09317005);
        assert!(txs["send"].moves.get(LBTC).is_none());
        let usdt = Liquid { xpub: String::new(), fingerprint: String::new(), path: String::new(), wrapped: false, blinding: String::new(), next: 0, next_change: 0, txs, assets: Default::default(), balances: Default::default(), read_at: 0, server: String::new(), asset: USDT.into() };
        assert_eq!(rows(&usdt).iter().find(|r| r.id == "send").unwrap().kind, "send");
        let lbtc = Liquid { asset: LBTC.into(), ..usdt };
        assert!(rows(&lbtc).iter().all(|r| r.id != "send"), "the L-BTC vault has no row for it");
    }

    /// Throwaway words, derived and scanned against the live server: proves the derivation, the server and the
    /// scan loop, and prints a confidential address a few sats can be sent to for an unblinding check.
    #[test] #[ignore]
    fn live_a_throwaway_wallet_scans_clean_and_prints_an_address() {
        let words = std::env::var("LIQUID_WORDS").expect("LIQUID_WORDS");
        match from_words(zeroize::Zeroizing::new(words.clone()), 20) {
            Ok(l) => { println!("FOUND {} wrapped={} next={} txs={} balances={:?}", l.path, l.wrapped, l.next, l.txs.len(), l.balances); for t in l.txs.values() { println!("  {} h={} t={} fee={} paid={} moves={:?}", t.id, t.height, t.time, t.fee, t.paid_fee, t.moves); } }
            Err(e) => {
                println!("NO HISTORY: {e}");
                let m = bip39::Mnemonic::parse_in_normalized(bip39::Language::English, &words).unwrap();
                let seed = m.to_seed("");
                let secp = bitcoin::secp256k1::Secp256k1::new();
                let master = Xpriv::new_master(bitcoin::Network::Bitcoin, &seed).unwrap();
                let xpub = Xpub::from_priv(&secp, &master.derive_priv(&secp, &DerivationPath::from_str("m/84'/1776'/0'").unwrap()).unwrap());
                let l = Liquid { xpub: xpub.to_string(), fingerprint: String::new(), path: "84'/1776'/0'".into(), wrapped: false, blinding: hex::encode(master_blinding_key(&seed)), next: 0, next_change: 0, txs: Default::default(), assets: Default::default(), balances: Default::default(), read_at: 0, server: String::new(), asset: LBTC.into() };
                println!("ADDRESS 0: {}", address_at(&l, 0, 0).unwrap());
            }
        }
    }
}
