mod spark;
mod muun;
mod activity;
mod net;
mod signers;
mod attach;
mod cards;
mod chain;
mod db;
mod urdesc;
mod sskrdesc;
mod context;
mod export;
mod hd;
mod labels;
mod location;
mod lockbox;
mod map;
mod message;
mod places;
mod seal;
mod wallet;

use serde::{Deserialize, Serialize};
use tauri::async_runtime::spawn_blocking;
use wallet::{Store, Vault};

type R<T> = Result<T, String>;

async fn block<T: Send + 'static>(f: impl FnOnce() -> R<T> + Send + 'static) -> R<T> {
    spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn cards_list() -> R<Vec<cards::CardInfo>> {
    block(|| cards::list().map_err(|e| e.to_string())).await
}

/// A run of daily prices for the value-over-time charts: one request to the price service's `series` question, cached
/// for good in `activity::price_series`. Answers the step it used, so the window can say exactly what it drew.
#[derive(serde::Serialize)]
struct PriceRun { every: u32, points: Vec<(String, Option<activity::Quote>)> }
#[tauri::command]
async fn prices_series(ccy: String, from: String, to: String) -> R<PriceRun> {
    block(move || {
        if !(ccy.len() == 3 && ccy.chars().all(|c| c.is_ascii_uppercase())) { return Err("the currency is a three-letter code".into()); }
        let well_formed = |d: &str| d.len() == 10 && d.as_bytes()[4] == b'-' && d.as_bytes()[7] == b'-' && d.chars().filter(|c| c.is_ascii_digit()).count() == 8;
        if !(well_formed(&from) && well_formed(&to)) || from > to { return Err("a range is two days, YYYY-MM-DD, in order".into()); }
        let s = wallet::load()?;
        let prices = if s.settings.custom { s.settings.prices.clone() } else { activity::PRICES.to_string() };
        let every = activity::series_every(&from, &to);
        if prices.is_empty() { return Ok(PriceRun { every, points: vec![] }); }
        Ok(PriceRun { every, points: activity::price_series(&prices, &ccy, &from, &to, every)? })
    }).await
}

/// Yubico devices that are plugged in but cannot hold a key for this app — shown so a person who plugs one in is
/// told what it is and why it will not do, rather than seeing nothing happen.
#[tauri::command]
async fn cards_other() -> R<Vec<cards::OtherDevice>> {
    block(|| Ok(cards::others())).await
}

#[tauri::command]
async fn card_generate(reader: String, admin_pin: String) -> R<String> {
    block(move || cards::generate(&reader, &admin_pin).map_err(|e| e.to_string())).await
}

#[tauri::command]
fn open_url(url: String) -> R<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) { return Err("only web links can be opened".into()); }
    open::that(&url).map_err(|e| format!("could not open the browser: {e}"))?;
    Ok(())
}

#[tauri::command]
fn vault_key_seen(name: String, pubkey: String, serial: u32) -> R<Store> {
    // a restored vault knows its keys only by public key; the first time that card is plugged in, remember its serial
    let mut s = wallet::load()?;
    let v = s.vaults.iter_mut().find(|v| v.name == name).ok_or("no such vault")?;
    let mut changed = false;
    for k in v.keys.iter_mut() { if k.pubkey == pubkey && k.serial == 0 && serial != 0 { k.serial = serial; changed = true; } }
    if changed { wallet::save(&s)?; }
    Ok(s)
}

#[tauri::command]
fn key_form_seen(pubkey: String, form: u8, nfc: bool, firmware: Option<String>) -> R<Store> {
    // one byte per key: the form factor, 0x40 = NFC was asked, 0x80 = the key has NFC
    let t = now();
    wallet::update(move |s| {
        s.forms.insert(pubkey.clone(), (form & 0x0F) | 0x40 | if nfc { 0x80 } else { 0 });
        s.key_first.entry(pubkey.clone()).or_insert(t);   // written ONCE: the first time is the only one that means anything
        match firmware.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
            Some(f) => { s.key_fw.insert(pubkey, f.to_string()); }
            None => {}   // a key that would not say keeps whatever it said last time; it cannot have changed
        }
    })
}

/// When each transaction happened, what it moved, and what that was worth. Separate from the sync so the balance never waits on it.
#[tauri::command]
async fn vault_activity(name: String, history: Vec<(String, i32)>, ccy: String) -> R<activity::Activity> {
    block(move || {
        let s = wallet::load()?;
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        let servers = wallet::servers_for(&s, &v.network);
        // our price service goes with our servers; someone running their own names their own, or has none
        let prices = if s.settings.custom { s.settings.prices.clone() } else { activity::PRICES.to_string() };
        // `ccy` is what the screen resolved the setting to ("auto" is worked out from the device there); empty = no fiat
        if !(ccy.is_empty() || (ccy.len() == 3 && ccy.chars().all(|c| c.is_ascii_uppercase()))) { return Err("the currency is a three-letter code".into()); }
        // A Spark wallet's history comes from an operator, in whole transfers: no inputs, no outputs, no fee of its
        // own, and an id that is Spark's rather than a txid. Priced the same way every other row is.
        if let Some(sp) = &v.spark {
            let id = hex::decode(&sp.identity).map_err(|_| "this vault's Spark key is damaged")?;
            let (list, _) = spark::transfers(&id, &v.network, 200, 0)?;
            let rows: Vec<activity::Row> = list.iter().map(|t| activity::Row {
                txid: t.id.clone(),
                time: if t.at > 0 { Some(t.at as u32) } else { None },
                net: Some(if t.direction == "out" { -(t.sats as i64) } else { t.sats as i64 }),
                fee: None, quote: None, ins: Vec::new(), outs: Vec::new(),
            }).collect();
            let mut a = activity::priced(&v, rows, &ccy, &prices)?;
            // A wallet that holds sats but shows no transfers is not an empty wallet: it is one whose history Spark
            // is withholding (its owner turned on private mode). Said plainly rather than drawn as "nothing here".
            if a.rows.is_empty() && spark::balance(&id, &v.network).unwrap_or(0) > 0 {
                a.note = Some("Spark says this wallet holds a balance but will not list its transfers — its privacy setting is on, so the history cannot be read here.".into());
            }
            return Ok(a);
        }
        let mut a = activity::build(&v, &servers, &history, &ccy, &prices)?;
        // a transaction this person's other vault also holds is a transfer between the two (the watcher records every
        // vault's transactions each minute, so this needs no request of its own)
        for (txid, _) in &history {
            let peers: Vec<String> = s.seen.iter().filter(|(n, seen)| *n != &v.name && seen.txs.contains_key(txid) && s.vaults.iter().any(|o| &o.name == *n && o.network == v.network)).map(|(n, _)| n.clone()).collect();
            if !peers.is_empty() { a.transfers.insert(txid.clone(), peers); }
        }
        Ok(a)
    }).await
}

// ---------- the address book ----------
/// Save a destination, or rename the one already saved for that address. The address is checked against the network first.
#[tauri::command]
fn contact_save(name: String, address: String, network: String) -> R<Store> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 { return Err("give it a name of up to 60 characters".into()); }
    let address = wallet::checked_address(&address, &network)?;
    wallet::update(move |s| match s.contacts.iter_mut().find(|c| c.address == address && c.network == network) {
        Some(c) => c.name = name,
        None => s.contacts.push(wallet::Contact { name, address, network, added: now(), used: 0, last_used: 0 }),
    })
}

#[tauri::command]
fn contact_delete(address: String, network: String) -> R<Store> {
    wallet::update(move |s| s.contacts.retain(|c| !(c.address == address && c.network == network)))
}

/// A payment to a saved address went out: count it, so the book can say how often that address has been used.
#[tauri::command]
fn contact_used(address: String, network: String) -> R<Store> {
    wallet::update(move |s| if let Some(c) = s.contacts.iter_mut().find(|c| c.address == address && c.network == network) { c.used += 1; c.last_used = now(); })
}

/// A fresh address of one of this person's OWN vaults: the way to pay yourself without ever reusing an address.
#[tauri::command]
fn vault_receive_address(name: String) -> R<String> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    wallet::address_at(v, if wallet::rotates(v) { v.next } else { 0 })
}

#[derive(Serialize)]
struct NodeStatus { electrum: String, height: u32, ms: u128, ours: bool }

/// Does the Electrum server for this network answer right now, and how fast? Opens a fresh connection on purpose:
/// a light that stays green on a cached answer is worse than no light.
#[tauri::command]
async fn node_status(network: String) -> R<NodeStatus> {
    block(move || {
        let s = wallet::load()?;
        let servers = wallet::servers_for(&s, &network);
        let ours = servers.electrum == chain::defaults(wallet::network_of(&network)).electrum;
        let t = std::time::Instant::now();
        let height = chain::height_fresh(&servers)?;
        Ok(NodeStatus { electrum: servers.electrum, height, ms: t.elapsed().as_millis(), ours })
    }).await
}

#[tauri::command]
fn device_locale() -> activity::Device { activity::device() }

/// One transaction in full: decoded inputs and outputs, which are this vault's, which keys signed, the fee, and whether the
/// node PROVED it is in its block. `height` is what the history said (0 = unconfirmed).
#[tauri::command]
async fn tx_detail(name: String, txid: String, height: i32) -> R<activity::Detail> {
    block(move || {
        let s = wallet::load()?;
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        let mut d = activity::detail(&v, &wallet::servers_for(&s, &v.network), &txid, height)?;
        mark_peers(&s, &v, &mut d);
        Ok(d)
    }).await
}

/// This person's OTHER vaults on the same network, by name: an address of theirs → (vault, position there).
/// Used to say that a transaction is a transfer between two of their vaults, and to link each end to the other.
fn peer_tables(s: &wallet::Store, v: &Vault) -> Vec<(String, Box<dyn Fn(&bitcoin::ScriptBuf) -> Option<u32>>)> {
    s.vaults.iter().filter(|o| o.name != v.name && o.network == v.network)
        .filter_map(|o| wallet::ours_table(o).ok().map(|f| (o.name.clone(), Box::new(f) as Box<dyn Fn(&bitcoin::ScriptBuf) -> Option<u32>>))).collect()
}
fn mark_peers(s: &wallet::Store, v: &Vault, d: &mut activity::Detail) {
    use std::str::FromStr;
    let tables = peer_tables(s, v);
    let owner = |a: &Option<String>| -> Option<(String, u32)> {
        let spk = bitcoin::Address::from_str(a.as_deref()?).ok()?.assume_checked().script_pubkey();
        tables.iter().find_map(|(n, f)| f(&spk).map(|i| (n.clone(), i)))
    };
    for i in d.inputs.iter_mut().filter(|i| i.ours.is_none()) { i.peer = owner(&i.address); }
    for o in d.outputs.iter_mut().filter(|o| o.ours.is_none()) { o.peer = owner(&o.address); }
    let mut peers: Vec<String> = d.inputs.iter().filter_map(|i| i.peer.as_ref().map(|p| p.0.clone())).chain(d.outputs.iter().filter_map(|o| o.peer.as_ref().map(|p| p.0.clone()))).collect();
    // and any other vault whose own record already holds this transaction
    for (n, seen) in &s.seen { if n != &v.name && seen.txs.contains_key(&d.txid) && s.vaults.iter().any(|o| &o.name == n && o.network == v.network) { peers.push(n.clone()); } }
    peers.sort(); peers.dedup(); d.peers = peers;
}

#[tauri::command]
fn store_get() -> Result<Store, String> {
    let s = wallet::load()?;
    net::seed(s.settings.offline);   // the window asks for the store on every start and after every unlock
    Ok(s)
}

const COST_METHODS: [&str; 3] = ["utxo", "fifo", "average"];

/// How the cost of what leaves a vault is found. It belongs to the TAXPAYER, so it is set once for the app.
#[tauri::command]
fn cost_method_set(method: String) -> R<Store> {
    if !COST_METHODS.contains(&method.as_str()) { return Err(format!("{method:?} is not a method this app computes")); }
    wallet::update(|s| s.settings.cost_method = method)
}

/// A vault that belongs to a different taxpayer — a company's, a client's — may follow that taxpayer's method.
/// None puts it back on the app's.
#[tauri::command]
fn vault_cost_method(name: String, method: Option<String>) -> R<Store> {
    if let Some(m) = &method { if !COST_METHODS.contains(&m.as_str()) { return Err(format!("{m:?} is not a method this app computes")); } }
    let mut s = wallet::load()?;
    let v = s.vaults.iter_mut().find(|v| v.name == name).ok_or("no such vault")?;
    v.cost_method = method;
    wallet::save(&s)?;
    Ok(s)
}

/// How often each vault should be proved recoverable from its cards. 0 means never ask.
#[tauri::command]
fn drill_every(days: u32) -> R<Store> {
    if days > 3650 { return Err("that is longer than ten years".into()); }
    wallet::update(|s| s.settings.drill_days = days)
}

/// Work offline, or stop. Everything the app would send is refused while this is on (net.rs is the one gate), and the
/// setting is written down so quitting and opening again does not quietly put the app back on the network.
#[tauri::command]
fn net_offline(on: bool) -> R<Store> {
    let s = wallet::update(|s| s.settings.offline = on)?;
    net::seed(on);
    Ok(s)
}

#[derive(Deserialize)]
struct NewKey { name: String, serial: u32, pubkey: String }

fn fresh_chain_code() -> String {
    use bitcoin::secp256k1::rand::RngCore;
    let mut cc = [0u8; 32];
    bitcoin::secp256k1::rand::rngs::OsRng.fill_bytes(&mut cc);
    hex::encode(cc)
}

#[tauri::command]
fn vault_create(name: String, threshold: usize, keys: Vec<NewKey>, rotating: Option<bool>, network: Option<String>) -> R<Vault> {
    let mut s = wallet::load()?;
    if let Some(n) = network { if n != "mainnet" && n != "signet" { return Err("network must be mainnet or signet".into()); } s.settings.network = n; }
    if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
    if threshold < 1 || threshold > keys.len() || keys.len() > 15 { return Err("threshold must be between 1 and the number of keys, and at most 15 keys".into()); }
    let pubs: Vec<String> = keys.iter().map(|k| k.pubkey.to_lowercase()).collect();
    let descriptor = wallet::descriptor_for(threshold, &pubs)?;
    let network = wallet::network_of(&s.settings.network);
    let mut v = Vault { watch: None, spark: None, muun: None, name, network: s.settings.network.clone(), threshold,
        keys: keys.into_iter().map(|k| wallet::KeyRef { name: k.name, serial: k.serial, pubkey: k.pubkey.to_lowercase(), device: None }).collect(),
        descriptor, address: String::new(), created: now(),
        chain_code: if rotating.unwrap_or(true) { Some(fresh_chain_code()) } else { None }, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default(), ext: None, cost_method: None };
    v.address = wallet::address_at(&v, 0)?;
    let _ = network;
    s.vaults.push(v.clone());
    wallet::save(&s)?;
    Ok(v)
}

#[derive(Serialize)]
struct Parsed { threshold: usize, keys: Vec<String>, descriptor: String, address: String, rotating: bool,
    /// extended keys held by other signers: each with the fingerprint and path a hardware signer matches on
    #[serde(default)] ext: Option<Vec<wallet::ExtKey>>, #[serde(default)] kind: String , #[serde(default)] name: Option<String>}

/// Text that names extended keys goes the faithful way (miniscript, as written); anything else is a card vault's own
/// line. Decided by what is IN the text, because a card vault's recovery line ends in ` cc=…`, which is not
/// descriptor syntax and must not be sent where it would be refused.
fn names_extended_keys(text: &str) -> bool {
    let t = text.split(" cc=").next().unwrap_or(text);
    ["xpub", "tpub", "ypub", "zpub", "Ypub", "Zpub", "vpub", "upub"].iter().any(|p| t.contains(p)) || t.contains('[')
}
fn ext_for_this_network(text: &str, network: &str) -> R<wallet::ExtInfo> {
    let info = wallet::ext_parse(text)?.ok_or("that descriptor names no extended keys")?;
    if info.network != network {
        return Err(format!("this is a {} descriptor and the app is on {} just now: each network keeps its own vaults. Switch networks in App settings, then paste it again", info.network, network));
    }
    Ok(info)
}

#[tauri::command]
fn descriptor_parse(text: String) -> R<Parsed> {
    let s = wallet::load()?;
    // a descriptor backed up as UR (Sparrow's QR, a metal plate): decoded to its text, then checked like any other
    // shares of a split recovery line (SSKR): rebuilt first, when enough are in; what they rebuild is then read as any line is
    let text = if sskrdesc::is_share(&text) { sskrdesc::join(&text)?.text } else { text };
    let (text, ur_name) = if urdesc::is_ur(&text) { let d = urdesc::decode(&text)?; (urdesc::as_wallet(&d.descriptor), d.name) } else { (text, None) };
    if names_extended_keys(&text) {
        let info = ext_for_this_network(&text, &s.settings.network)?;
        let mut v = ext_vault(String::new(), &info, &[]);
        v.address = wallet::address_at(&v, 0)?;
        return Ok(Parsed { threshold: info.threshold, keys: info.keys.iter().map(|k| format!("[{}/{}]{}", k.fingerprint, k.path.trim_start_matches("m/"), k.xpub)).collect(),
            descriptor: info.descriptor.clone(), address: v.address, rotating: true, ext: Some(info.keys.clone()), kind: info.kind.clone(), name: ur_name });
    }
    let (desc_text, cc) = wallet::split_recovery(&text);
    let (threshold, keys) = wallet::keys_of(&desc_text)?;
    let descriptor = wallet::descriptor_for(threshold, &keys)?;
    let tmp = Vault { watch: None, spark: None, muun: None, name: String::new(), network: s.settings.network.clone(), threshold, keys: keys.iter().map(|k| wallet::KeyRef { name: String::new(), serial: 0, pubkey: k.clone(), device: None }).collect(),
        descriptor: descriptor.clone(), address: String::new(), created: 0, chain_code: cc.clone(), next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default(), ext: None, cost_method: None };
    let address = wallet::address_at(&tmp, 0)?;
    Ok(Parsed { threshold, keys, descriptor: wallet::recovery_line(&tmp), address, rotating: cc.is_some(), ext: None, kind: "cards".into(), name: ur_name })
}

/// A Muun wallet, WATCHED. The kit's two keys are decrypted here, the account xpubs at `m/1h/1h` are taken from
/// them, and the private halves go out of scope with this function: what the vault keeps cannot spend a satoshi.
///
/// The salt always comes from the SECOND key — the same salt was used for both, but Muun's older format left it out
/// of the first one (their `keys_generator.go` says exactly this).
#[tauri::command]
fn muun_restore(name: String, key1: String, key2: String, code: String, expect: Option<String>, names: Vec<String>) -> R<Vault> {
    use bitcoin::bip32::{ChildNumber, Xpub};
    let _ = names;
    let mut s = wallet::load()?;
    if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
    let net = wallet::network_of(&s.settings.network);
    let (k1, k2) = (muun::decode(&key1)?, muun::decode(&key2)?);
    let unlock = muun::code_key(&code, k2.salt.or(k1.salt))?;
    let secp = bitcoin::secp256k1::Secp256k1::new();
    // Muun's two keys do NOT arrive at the same depth. Their own tool says so in one line:
    //     decryptedKeys[0].Key.Path = "m/1'/1'"   // a little adjustment for legacy users
    // The owner's key is ALREADY the account key; Muun's is a master that still needs m/1h/1h. Deriving both the
    // same way sends one of them a level too deep, and the wallet then watches addresses that do not exist —
    // which is exactly what happened on Piers's kit (2026-09-24): no transactions, and a fingerprint that missed.
    //
    // So the kit's own descriptors decide it. Each key is taken both ways, and the reading whose fingerprint the kit
    // names is the right one. That check IS the proof the recovery code was right, so it is not guesswork either way.
    let both = |k: &muun::KitKey| -> Result<[(String, String); 2], String> {
        use bitcoin::bip32::{ChildNumber, Xpub};
        let (xpriv, _) = muun::decrypt(k, unlock, net)?;
        let path = [ChildNumber::from_hardened_idx(1).map_err(|e| e.to_string())?, ChildNumber::from_hardened_idx(1).map_err(|e| e.to_string())?];
        let lower = xpriv.derive_priv(&secp, &path).map_err(|e| e.to_string())?;
        Ok([(Xpub::from_priv(&secp, &xpriv).to_string(), xpriv.fingerprint(&secp).to_string()),
            (Xpub::from_priv(&secp, &lower).to_string(), lower.fingerprint(&secp).to_string())])
    };
    let (a, b) = (both(&k1)?, both(&k2)?);
    let named: Vec<String> = expect.as_deref().map(str::trim).filter(|l| !l.is_empty())
        .map(|line| line.to_lowercase().as_bytes().windows(8)
            .filter(|w| w.iter().all(|c| c.is_ascii_hexdigit()))
            .map(|w| String::from_utf8_lossy(w).to_string()).collect())
        .unwrap_or_default();
    let pick = |v: &[(String, String); 2], fallback: usize| -> Result<(String, String), String> {
        if named.is_empty() { return Ok(v[fallback].clone()); }
        v.iter().find(|(_, fp)| named.contains(fp)).cloned()
            .ok_or_else(|| format!("that recovery code does not match this kit: it unlocked a key whose fingerprint is {} (or {} one level down), and the line you pasted names neither. Check the code character by character. Nothing was saved.", v[0].1, v[1].1))
    };
    // without a line to check against, fall back to what Muun's own tool assumes: the owner's key is the account
    // key already, Muun's is a master
    let (user, fp_user) = pick(&a, 0)?;
    let (cosign, fp_cosign) = pick(&b, 1)?;
    let m = wallet::Muun { user, cosign, birthday: k2.birthday.max(k1.birthday), fp_user, fp_cosign };
    if m.user == m.cosign { return Err("both of those are the same key — a kit has two different ones".into()); }
    if let Some(v) = s.vaults.iter().find(|v| v.muun.as_ref().map(|o| o.user == m.user).unwrap_or(false)) {
        return Err(format!("this Muun wallet is already here, as '{}'", v.name));
    }
    let mut v = Vault { watch: None, spark: None, muun: Some(m), name, network: s.settings.network.clone(), threshold: 2,
        keys: Vec::new(), descriptor: String::new(), address: String::new(), created: now(), chain_code: None,
        next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(),
        notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0,
        seal: Default::default(), ext: None, cost_method: None };
    v.address = wallet::address_at(&v, 0)?;
    s.vaults.push(v.clone());
    wallet::save(&s)?;
    Ok(v)
}

/// A wallet watched from ONE extended key — what most wallets offer when they say "export xpub".
///
/// An account key does not say which kind of address it is for. `ypub`/`zpub` do (SLIP-132) and are believed; a
/// plain `xpub` does not, and wallets export a plain one for legacy, wrapped segwit, native segwit and taproot
/// alike. Guessing one would show a confidently wrong balance, so a plain key is watched for ALL FOUR at once —
/// the same answer a Muun wallet needed. Nothing here can spend it.
#[tauri::command]
fn xpub_restore(name: String, text: String, names: Vec<String>) -> R<Vault> {
    let _ = names;
    let mut s = wallet::load()?;
    if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
    let raw = text.trim();
    let key = signers::standard_xpub(raw)?;
    let said = match &raw[..4.min(raw.len())] {
        "zpub" | "vpub" => "zpub", "ypub" | "upub" => "ypub", _ => "",
    }.to_string();
    let kinds: Vec<String> = match said.as_str() {
        "zpub" => vec!["wpkh".into()],
        "ypub" => vec!["shwpkh".into()],
        _ => vec!["pkh".into(), "shwpkh".into(), "wpkh".into(), "tr".into()],
    };
    use std::str::FromStr;
    let x = bitcoin::bip32::Xpub::from_str(&key).map_err(|e| e.to_string())?;
    let network = if x.network == bitcoin::NetworkKind::Main { "mainnet" } else { "signet" };
    if network != s.settings.network { return Err(format!("that key is for {network}, and this app is set to {}", s.settings.network)); }
    if let Some(v) = s.vaults.iter().find(|v| v.watch.as_ref().map(|w| w.key == key).unwrap_or(false)) {
        return Err(format!("this key is already here, as '{}'", v.name));
    }
    // What the key IS, said back: a wallet that exports the wrong node is the commonest way this ends in silence.
    // depth 3 is an account key (m/purpose'/coin'/account'), which is what a watch-only export should be; depth 0 is
    // a master, whose only reachable addresses are m/0/i and m/1/i.
    if x.depth == 0 {
        return Err("that is a MASTER key, not an account key. Hardened levels cannot be derived from a public key, so this app could only watch m/0/i — almost certainly not where your wallet keeps its coins. Export the account key (what a wallet calls its watch-only or xpub export).".into());
    }
    let mut v = Vault { watch: Some(wallet::Watch { key, kinds, said }), spark: None, muun: None, name,
        network: network.to_string(), threshold: 1, keys: Vec::new(), descriptor: String::new(), address: String::new(),
        created: now(), chain_code: None, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0,
        labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(),
        more: Default::default(), labels_synced: 0, seal: Default::default(), ext: None, cost_method: None };
    v.address = wallet::address_at(&v, 0)?;
    s.vaults.push(v.clone());
    wallet::save(&s)?;
    Ok(v)
}

/// A Spark wallet, WATCHED — Wallet of Satoshi's self-custody mode and anything else built on Spark. All it takes is
/// the wallet's own `spark1…` address: its figures are then Spark's word, not the chain's, which the screens say.
#[tauri::command]
async fn spark_restore(name: String, address: String) -> R<Vault> {
    block(move || {
        let mut s = wallet::load()?;
        if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
        let (id, network) = spark::identity_of(&address)?;
        if network != s.settings.network { return Err(format!("that address is a {network} Spark wallet, and this app is set to {}", s.settings.network)); }
        let identity = hex::encode(&id);
        if let Some(v) = s.vaults.iter().find(|v| v.spark.as_ref().map(|x| x.identity == identity).unwrap_or(false)) {
            return Err(format!("this Spark wallet is already here, as '{}'", v.name));
        }
        // asked once here so a wallet that cannot be read says so now, rather than looking empty later
        let balance = spark::balance(&id, &network)?;
        let (list, _) = spark::transfers(&id, &network, 1, 0)?;
        if list.is_empty() && balance > 0 {
            return Err("Spark will not list this wallet's transfers — its privacy setting is on. The balance can be read but the history cannot, so there is nothing here to account with.".into());
        }
        let mut v = Vault { watch: None, spark: Some(wallet::Spark { address: address.trim().to_string(), identity }), muun: None,
            name, network, threshold: 1, keys: Vec::new(), descriptor: String::new(), address: address.trim().to_string(),
            created: now(), chain_code: None, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0,
            labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(),
            more: Default::default(), labels_synced: 0, seal: Default::default(), ext: None,
            // a Spark wallet has no coins to identify, so the method that counts the coins actually spent cannot
            // apply to it; it counts first in, first out, and the screen says which
            cost_method: Some("fifo".into()) };
        v.address = v.spark.as_ref().unwrap().address.clone();
        s.vaults.push(v.clone());
        wallet::save(&s)?;
        Ok(v)
    }).await
}

/// A Spark wallet watched from its SEED WORDS, for an app that shows no Spark code (Wallet of Satoshi shows only
/// Lightning and on-chain). The words are used for one derivation and dropped; the vault keeps the identity public
/// key and the address made from it — the same rule as a Muun recovery code. Mainnet wallets default to account 1,
/// older SDK builds to 0, so both are tried and Spark itself says which one is a wallet with anything in it.
#[tauri::command]
async fn spark_restore_words(name: String, words: String) -> R<Vault> {
    block(move || {
        let s = wallet::load()?;
        if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
        let network = s.settings.network.clone();
        let mut chosen: Option<(u32, Vec<u8>)> = None;
        let mut seen = Vec::new();
        for account in [1u32, 0] {
            let id = spark::identity_from_words(&words, account)?;
            let balance = spark::balance(&id, &network)?;
            let (list, _) = spark::transfers(&id, &network, 1, 0)?;
            seen.push(format!("account {account}: {balance} sats, {}", if list.is_empty() { "no transfers" } else { "has transfers" }));
            if balance > 0 || !list.is_empty() { chosen = Some((account, id)); break; }
        }
        { use zeroize::Zeroize; let mut w = words; w.zeroize(); }   // the words have done their one job
        let (account, id) = chosen.ok_or_else(|| format!("those words derive a Spark identity, but Spark has nothing under it on either account ({}). Either the wallet is empty and has never received, or it was made with a different scheme.", seen.join("; ")))?;
        let _ = account;
        let address = spark::address_of(&id, &network)?;
        spark_vault(s, name, address, hex::encode(&id), network)
    }).await
}

#[tauri::command]
fn seed_wordlist() -> Vec<&'static str> { spark::wordlist() }
#[tauri::command]
fn seed_check(words: String) -> R<usize> { use zeroize::Zeroize; let r = spark::words_check(&words); let mut w = words; w.zeroize(); r }

/// The vault a Spark identity makes, whichever way the identity was found.
fn spark_vault(mut s: Store, name: String, address: String, identity: String, network: String) -> R<Vault> {
    if let Some(v) = s.vaults.iter().find(|v| v.spark.as_ref().map(|x| x.identity == identity).unwrap_or(false)) {
        return Err(format!("this Spark wallet is already here, as '{}'", v.name));
    }
    let v = Vault { watch: None, spark: Some(wallet::Spark { address: address.clone(), identity }), muun: None,
        name, network, threshold: 1, keys: Vec::new(), descriptor: String::new(), address,
        created: now(), chain_code: None, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0,
        labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(),
        more: Default::default(), labels_synced: 0, seal: Default::default(), ext: None, cost_method: Some("fifo".into()) };
    s.vaults.push(v.clone());
    wallet::save(&s)?;
    Ok(v)
}

/// What a kit's key says about itself before any code is typed: enough to tell someone they pasted the wrong thing.
#[tauri::command]
fn muun_check(key1: String, key2: String) -> R<serde_json::Value> {
    let (k1, k2) = (muun::decode(&key1)?, muun::decode(&key2)?);
    if key1.trim() == key2.trim() { return Err("those are the same key twice — the kit has two different ones".into()); }
    Ok(serde_json::json!({ "birthday": k2.birthday.max(k1.birthday), "salted": k2.salt.is_some() }))
}

#[tauri::command]
fn vault_restore(name: String, text: String, names: Vec<String>) -> R<Vault> {
    let mut s = wallet::load()?;
    if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
    if names_extended_keys(&text) {
        let info = ext_for_this_network(&text, &s.settings.network)?;
        // the same wallet written 48h here and 48' there is still the same wallet
        let same = |e: &str| wallet::ext_parse(e).ok().flatten().map(|i| i.canonical == info.canonical).unwrap_or(false);
        if let Some(v) = s.vaults.iter().find(|v| v.ext.as_deref().map(same).unwrap_or(false)) { return Err(format!("this wallet is already here, as '{}'", v.name)); }
        let mut v = ext_vault(name, &info, &names);
        v.address = wallet::address_at(&v, 0)?;
        s.vaults.push(v.clone());
        wallet::save(&s)?;
        return Ok(v);
    }
    let (desc_text, cc) = wallet::split_recovery(&text);
    let (threshold, keys) = wallet::keys_of(&desc_text)?;
    let descriptor = wallet::descriptor_for(threshold, &keys)?;
    let mut v = Vault { watch: None, spark: None, muun: None, name, network: s.settings.network.clone(), threshold,
        keys: keys.iter().enumerate().map(|(i, k)| wallet::KeyRef { name: names.get(i).cloned().unwrap_or_else(|| format!("key {}", i + 1)), serial: 0, pubkey: k.clone(), device: None }).collect(),
        descriptor, address: String::new(), created: now(), chain_code: cc, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default(), ext: None, cost_method: None };
    v.address = wallet::address_at(&v, 0)?;
    s.vaults.push(v.clone());
    wallet::save(&s)?;
    Ok(v)
}

// ---------- a vault of any mix of signers ----------

/// What a signing device handed over, read into its keys; the one a native-segwit multisig wants is marked.
#[derive(Serialize)]
struct DeviceRead { keys: Vec<signers::DeviceKey>, multisig: Option<signers::DeviceKey> }
#[tauri::command]
fn device_key_read(text: String) -> R<DeviceRead> {
    let keys = signers::parse(&text)?;
    let multisig = signers::multisig_key(&keys).cloned();
    Ok(DeviceRead { keys, multisig })
}
/// The same, from a file a device wrote (Coldcard's ccxp-XXXX.json, Passport's export, a text file).
#[tauri::command]
fn device_key_file() -> R<DeviceRead> {
    let path = rfd::FileDialog::new().set_title("Open the key file your signing device wrote").add_filter("Key file", &["json", "txt", "bsms"]).pick_file().ok_or("cancelled")?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    device_key_read(text)
}

#[derive(Deserialize)]
struct SignerIn { name: String, device: String, #[serde(default)] card: Option<String>, #[serde(default)] expression: Option<String>, #[serde(default)] serial: u32 }

/// A vault of any mix of signers: cards as FIXED keys, devices by their extended keys with origins — a standard
/// descriptor that other wallets and every signer in it can derive. (A vault of cards alone keeps the app's private
/// rotation, through vault_create.)
#[tauri::command]
fn vault_create_signers(name: String, threshold: usize, keys: Vec<SignerIn>, network: Option<String>) -> R<Vault> {
    let mut s = wallet::load()?;
    if let Some(n) = network { if n != "mainnet" && n != "signet" { return Err("network must be mainnet or signet".into()); } s.settings.network = n; }
    if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
    let v = signers_vault(name, threshold, &keys, &s.settings.network)?;
    s.vaults.push(v.clone());
    wallet::save(&s)?;
    Ok(v)
}

/// The vault itself, touching no store — so it can be tested exactly.
fn signers_vault(name: String, threshold: usize, keys: &[SignerIn], network: &str) -> R<Vault> {
    use std::str::FromStr;
    if keys.len() < 2 || keys.len() > 15 || threshold < 1 || threshold > keys.len() { return Err("a vault of several signers has 2 to 15 keys, and needs between 1 and all of them".into()); }
    if keys.iter().all(|k| k.card.is_some()) { return Err("every key here is a card: make it as a card vault, which gives each deposit a private fresh address".into()); }
    let mainnet = network == "mainnet";
    let mut exprs = Vec::new();
    for k in keys {
        match (&k.card, &k.expression) {
            (Some(pk), _) => { bitcoin::PublicKey::from_str(pk).map_err(|_| format!("{}: not a public key", k.name))?; exprs.push(pk.to_lowercase()); }
            (None, Some(e)) => {
                let dk = signers::parse(e)?.into_iter().next().ok_or_else(|| format!("{}: no key in it", k.name))?;
                if dk.mainnet() != mainnet { return Err(format!("{}'s key is for {}, and this vault is on {}", k.name, if dk.mainnet() { "mainnet" } else { "a test network" }, network)); }
                exprs.push(format!("{}/<0;1>/*", dk.expression()));
            }
            _ => return Err(format!("{}: neither a card nor a device key", k.name)),
        }
    }
    let mut seen = std::collections::HashSet::new();
    for e in &exprs { if !seen.insert(e.split('/').next().unwrap_or(e).to_string() + e) { return Err("the same key is in this vault twice".into()); } }
    let body = format!("wsh(sortedmulti({threshold},{}))", exprs.join(","));
    let desc = miniscript::Descriptor::<miniscript::DescriptorPublicKey>::from_str(&body).map_err(|e| format!("descriptor: {e}"))?.to_string();
    let info = wallet::ext_parse(&desc)?.ok_or("that made no vault of extended keys")?;
    let mut v = ext_vault(name, &info, &[]);
    // names and devices follow the keys as the person listed them, matched by key rather than position (sortedmulti reorders)
    for kr in v.keys.iter_mut() {
        let src = keys.iter().find(|k| k.card.as_deref().map(|c| c.eq_ignore_ascii_case(&kr.pubkey)).unwrap_or(false)
            || k.expression.as_deref().and_then(|e| signers::parse(e).ok()).and_then(|d| d.into_iter().next()).map(|d| kr.pubkey.ends_with(&format!("]{}", d.xpub))).unwrap_or(false));
        if let Some(src) = src { kr.name = src.name.clone(); kr.device = Some(src.device.clone()); kr.serial = src.serial; }
    }
    v.address = wallet::address_at(&v, 0)?;
    Ok(v)
}

/// A vault of extended keys held by other signers. Each key is named for the person, and keeps its origin — the
/// fingerprint and path a hardware signer matches on — as the "pubkey" shown for it; it is never a card's key.
pub(crate) fn ext_vault(name: String, info: &wallet::ExtInfo, names: &[String]) -> Vault {
    Vault { watch: None, spark: None, muun: None, name, network: info.network.clone(), threshold: info.threshold,
        keys: info.keys.iter().enumerate().map(|(i, k)| wallet::KeyRef { name: names.get(i).cloned().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| if k.fixed { format!("key {} · a card", i + 1) } else { format!("key {} · {}", i + 1, k.fingerprint) }),
            // a fixed key is kept as the bare key, so a card plugged in is recognised as this vault's by the key it holds
            serial: 0, pubkey: if k.fixed { k.xpub.clone() } else { format!("[{}/{}]{}", k.fingerprint, k.path.trim_start_matches("m/").replace('\'', "h"), k.xpub) }, device: if k.fixed { Some("card".into()) } else { None } }).collect(),
        descriptor: info.descriptor.clone(), address: String::new(), created: now(), chain_code: None, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0,
        labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0,
        seal: Default::default(), ext: Some(info.descriptor.clone()), cost_method: None }
}

#[tauri::command]
fn vault_recovery(name: String) -> R<String> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    Ok(wallet::recovery_line(v))
}

/// Rename a vault and/or its keys. Labels only: nothing on the chain or the cards changes.
#[tauri::command]
fn vault_rename(name: String, new_name: String, key_names: Vec<String>) -> R<Store> {
    let mut s = wallet::load()?;
    let new_name = new_name.trim().to_string();
    if new_name.is_empty() { return Err("a vault needs a name".into()); }
    if new_name != name && s.vaults.iter().any(|v| v.name == new_name) { return Err(format!("a vault called '{new_name}' already exists")); }
    let v = s.vaults.iter_mut().find(|v| v.name == name).ok_or("no such vault")?;
    for (i, k) in v.keys.iter_mut().enumerate() {
        if let Some(n) = key_names.get(i) { let n = n.trim(); if !n.is_empty() { k.name = n.to_string(); } }
    }
    v.name = new_name.clone();
    if new_name != name { if let Some(seen) = s.seen.remove(&name) { s.seen.insert(new_name, seen); } }
    wallet::save(&s)?;
    Ok(s)
}

#[tauri::command]
fn vault_forget(name: String) -> R<Store> {
    let mut s = wallet::load()?;
    s.vaults.retain(|v| v.name != name);
    wallet::save(&s)?;
    Ok(s)
}

#[tauri::command]
fn settings_set(app: tauri::AppHandle, network: String, custom: bool, electrum: String, explorer: String, gap: Option<u32>, fiat: Option<String>, prices: Option<String>, vss: Option<String>, tiles: Option<String>, geocode: Option<String>, location: Option<bool>, location_exact: Option<bool>) -> R<Store> {
    let mut s = wallet::load()?;
    if network != "mainnet" && network != "signet" { return Err("network must be mainnet or signet".into()); }
    let gap = gap.unwrap_or(s.settings.gap);
    if !(1..=500).contains(&gap) { return Err("the gap limit must be between 1 and 500".into()); }
    let fiat = fiat.map(|f| { let f = f.trim(); if f.eq_ignore_ascii_case("auto") { "auto".to_string() } else { f.to_uppercase() } }).unwrap_or_else(|| s.settings.fiat.clone());
    if !(fiat.is_empty() || fiat == "auto" || (fiat.len() == 3 && fiat.chars().all(|c| c.is_ascii_uppercase()))) { return Err("the currency is a three-letter code, or empty for none".into()); }
    let prices = prices.map(|p| p.trim().to_string()).unwrap_or_else(|| s.settings.prices.clone());
    if !(prices.is_empty() || prices.starts_with("https://") || prices.starts_with("http://")) { return Err("the price server is a web address".into()); }
    let vss = vss.map(|v| v.trim().trim_end_matches('/').to_string()).unwrap_or_else(|| s.settings.vss.clone());
    if !(vss.is_empty() || vss.starts_with("https://") || vss.starts_with("http://")) { return Err("the label host is a web address, or empty to keep labels on this computer only".into()); }
    let (location, location_exact) = (location.unwrap_or(s.settings.location), location_exact.unwrap_or(s.settings.location_exact));
    let tiles = tiles.map(|v| v.trim().to_string()).unwrap_or_else(|| s.settings.tiles.clone());
    if !(tiles.is_empty() || tiles.starts_with("https://")) { return Err("the map host is an https address of a .pmtiles file, or empty for no maps".into()); }
    s.settings = wallet::Settings { network, custom, electrum: electrum.trim().to_string(), explorer: explorer.trim().to_string(), gap, fiat, prices, vss, location, location_exact, tiles, geocode: geocode.map(|v| v.trim().trim_end_matches('/').to_string()).filter(|v| v.is_empty() || v.starts_with("https://")).unwrap_or_else(|| s.settings.geocode.clone()), offline: s.settings.offline, drill_days: s.settings.drill_days, cost_method: s.settings.cost_method.clone() };
    wallet::save(&s)?;
    location_follow(&app, location);
    Ok(s)
}

#[derive(Serialize)]
struct HistoryRow { txid: String, height: i32, index: u32 }
#[derive(Serialize)]
struct Snapshot { coins: Vec<wallet::CoinAt>, balance: u64, height: u32, history: Vec<HistoryRow>, servers: chain::Servers, receive: String, receive_index: u32, rotating: bool, next_change: u32, gap: u32 }

#[tauri::command]
async fn vault_sync(name: String) -> R<Snapshot> {
    block(move || {
        let s = wallet::load()?;
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        let servers = wallet::servers_for(&s, &v.network);
        // A Spark wallet is not on the chain: its balance comes from an operator, and it has no coins, no addresses
        // and no block height of its own. Everything else on the screen is drawn from this same shape.
        if let Some(sp) = &v.spark {
            let id = hex::decode(&sp.identity).map_err(|_| "this vault's Spark key is damaged")?;
            let balance = spark::balance(&id, &v.network)?;
            return Ok(Snapshot { coins: Vec::new(), balance, height: 0, history: Vec::new(), servers,
                receive: sp.address.clone(), receive_index: 0, rotating: false, next_change: 0, gap: 0 });
        }
        let sc = wallet::scan(&v, &servers, s.settings.gap)?;
        let balance = sc.coins.iter().map(|c| c.value).sum();
        let height = chain::height(&servers).unwrap_or(0);
        let (n, sm, nc, smc) = (sc.next, sc.scan_max, sc.next_change, sc.scan_max_change);
        wallet::update(|s| if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == v.name) { vm.next = n; vm.scan_max = sm; vm.next_change = nc; vm.scan_max_change = smc; })?;
        let receive_index = if wallet::rotates(&v) { sc.next } else { 0 };
        let receive = wallet::address_at(&v, receive_index)?;
        let mut history: Vec<HistoryRow> = sc.history.into_iter().map(|(txid, height, index)| HistoryRow { txid, height, index }).collect();
        history.sort_by_key(|h| if h.height <= 0 { i32::MAX } else { h.height });
        history.dedup_by(|a, b| a.txid == b.txid);
        Ok(Snapshot { coins: sc.coins, balance, height, history, servers, receive, receive_index, rotating: wallet::rotates(&v), next_change: sc.next_change, gap: s.settings.gap })
    }).await
}

#[tauri::command]
async fn fees_get(network: Option<String>) -> R<chain::Fees> {
    block(move || { let s = wallet::load()?; let n = network.unwrap_or_else(|| s.settings.network.clone()); chain::fees(&wallet::servers_for(&s, &n)) }).await
}

#[tauri::command]
async fn servers_test(electrum: String, explorer: String) -> R<String> {
    block(move || {
        let servers = chain::Servers { electrum, explorer };
        let h = chain::height(&servers)?;
        let f = chain::fees(&servers)?;
        Ok(format!("electrum answers at block {h}; explorer fees {:.0}/{:.0}/{:.0} sat/vB", f.slow, f.medium, f.fast))
    }).await
}

/// What a vault of extended keys cannot do yet, said before anything card-shaped is attempted rather than failing
/// deep inside the card arithmetic with a message about public keys.
/// A watched wallet is watched: no screen, and no command, may act as though it could sign.
fn watch_only(v: &Vault) -> R<()> {
    if v.muun.is_some() { return Err(format!("‘{}’ is a watched Muun wallet. Its keys stayed in Muun — nothing here can sign for it, so spend it in the Muun app.", v.name)); }
    Ok(())
}

fn cards_only(v: &Vault, what: &str) -> R<()> {
    if v.ext.is_some() { return Err(format!("{what} works with a vault of YubiKeys only; this vault's keys include other signing devices.")); }
    Ok(())
}

#[tauri::command]
async fn spend_build(name: String, to: String, amount: u64, rate: f64, send_all: bool, coins: Option<Vec<String>>) -> R<wallet::Built> {
    block(move || {
        let s = wallet::load()?;
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        watch_only(&v)?;
        let servers = wallet::servers_for(&s, &v.network);
        let sc = wallet::scan(&v, &servers, s.settings.gap)?;
        let change_index = if wallet::rotates(&v) { wallet::CHANGE | sc.next_change } else { 0 };
        let pick = coins.filter(|c| !c.is_empty());
        let b = wallet::build(&v, &sc.coins, change_index, &to, amount, rate, send_all, pick.as_deref())?;
        let (n, sm, nc, smc, psbt) = (sc.next, sc.scan_max.max(sc.next), sc.next_change, sc.scan_max_change.max(sc.next_change), b.psbt.clone());
        wallet::update(|s| { if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == v.name) { vm.next = n; vm.scan_max = sm; vm.next_change = nc; vm.scan_max_change = smc; } s.last_psbt = Some(psbt); s.last_message = None; })?;
        Ok(b)
    }).await
}

/// An address that belongs to a vault this app holds: the spend stays with the owner, so nothing is disposed of.
#[derive(Serialize)]
struct Owned { vault: String, index: u32, chain: String, this_vault: bool }

#[derive(Serialize)]
struct Preview { choice: wallet::Choice, to_ours: Option<Owned>, coins: Vec<wallet::CoinAt> }

/// What a spend WOULD do, before anything is built: which coins it takes, what leaves, and whether the address is
/// one of this app's own vaults. Send draws its consequences from this; the cost of those coins is the window's to
/// work out, from the same ledger every other screen uses, under the method the taxpayer set.
#[tauri::command]
async fn spend_preview(name: String, to: String, amount: u64, rate: f64, send_all: bool, coins: Option<Vec<String>>) -> R<Preview> {
    block(move || {
        let s = wallet::load()?;
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        watch_only(&v)?;
        let servers = wallet::servers_for(&s, &v.network);
        let sc = wallet::scan(&v, &servers, s.settings.gap)?;
        let change_index = if wallet::rotates(&v) { wallet::CHANGE | sc.next_change } else { 0 };
        let choice = wallet::select(&v, &sc.coins, change_index, &to, amount, rate, send_all, coins.filter(|c| !c.is_empty()).as_deref())?;
        Ok(Preview { choice, to_ours: owner_of(&s, &v, &to), coins: sc.coins })
    }).await
}

/// Which vault an address belongs to, this one or another on the same network — the same tables Activity uses to
/// recognise a move between your own vaults.
fn owner_of(s: &wallet::Store, v: &Vault, to: &str) -> Option<Owned> {
    use std::str::FromStr;
    let spk = bitcoin::Address::from_str(to.trim()).ok()?.assume_checked().script_pubkey();
    let named = |o: &Vault, i: u32| Owned { vault: o.name.clone(), index: i & !wallet::CHANGE, chain: if i & wallet::CHANGE != 0 { "change" } else { "receive" }.into(), this_vault: o.name == v.name };
    if let Some(i) = wallet::ours_table(v).ok().and_then(|f| f(&spk)) { return Some(named(v, i)); }
    s.vaults.iter().filter(|o| o.name != v.name && o.network == v.network)
        .find_map(|o| wallet::ours_table(o).ok().and_then(|f| f(&spk)).map(|i| named(o, i)))
}

#[derive(Serialize)]
struct Signed { psbt: String, signed_inputs: Vec<usize>, status: wallet::SigStatus, signer: String }

/// Sign every input the plugged-in card can, with its PIN; one touch per input.
#[tauri::command]
async fn spend_sign(name: String, reader: String, pin: String) -> R<Signed> {
    block(move || {
        let mut s = wallet::load()?;
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        watch_only(&v)?;
        let mut psbt = s.last_psbt.clone().ok_or("nothing to sign — build a spend first")?;
        let card = cards::list().map_err(|e| e.to_string())?.into_iter().find(|c| c.reader == reader).ok_or("that card is no longer plugged in")?;
        let pub_hex = card.pubkey.ok_or("this card has no signing key")?;
        let key = v.keys.iter().find(|k| k.pubkey == pub_hex).ok_or("this card is not one of the vault's keys")?;
        let todo = wallet::sighashes_for(&v, &psbt, &pub_hex)?;
        if todo.is_empty() { return Err("nothing for this card to sign (already signed, or not its key)".into()); }
        let mut done = Vec::new();
        for t in todo {
            // a rotating vault: the card signs z·T⁻¹ and the signature is lifted back to the child key
            let card_digest = match &t.tweak { Some(tw) => hd::card_digest(&t.digest, tw), None => t.digest };
            let (r, sg) = cards::sign_digest(&reader, &pin, &card_digest).map_err(|e| e.to_string())?;
            let sg = match &t.tweak { Some(tw) => hd::lift_s(&sg, tw), None => sg };
            psbt = wallet::add_signature(&psbt, &t.child, t.input, &r, &sg, &t.digest)?;
            done.push(t.input);
        }
        s.last_psbt = Some(psbt.clone());
        wallet::save(&s)?;
        let status = wallet::status(&psbt, v.threshold)?;
        Ok(Signed { psbt, signed_inputs: done, status, signer: key.name.clone() })
    }).await
}

#[tauri::command]
fn spend_status(name: String) -> R<wallet::SigStatus> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let psbt = s.last_psbt.clone().ok_or("no spend in progress")?;
    wallet::status(&psbt, v.threshold)
}

#[derive(Serialize)]
struct Sent { txid: String, link: String }

#[tauri::command]
async fn spend_broadcast(name: String) -> R<Sent> {
    block(move || {
        let mut s = wallet::load()?;
        let _v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        if s.last_message.is_some() { return Err("this is a signed message, not a spend; finish it on the Messages screen".into()); }
        let psbt = s.last_psbt.clone().ok_or("no spend in progress")?;
        let tx = wallet::finalize(&psbt)?;
        let servers = wallet::servers_for(&s, &_v.network);
        let txid = chain::broadcast(&servers, &tx)?;
        s.last_psbt = None;
        // where and on what this payment was made: written once, now, into its own record
        let (t, fix) = (now(), if s.settings.location { location::fresh(now(), 1800) } else { None });
        let ctx = context::build("made", fix, s.settings.location_exact, t);
        if let Some(vm) = s.vaults.iter_mut().find(|v| v.name == name) { context::stamp(vm, &txid.to_string(), ctx, t); }
        wallet::save(&s)?;
        Ok(Sent { txid: txid.to_string(), link: chain::tx_link(&servers, &txid.to_string()) })
    }).await
}

/// Save the spend in progress to a file the person chooses: the PSBT (binary, BIP 174) or, once
/// every input is fully signed, the raw transaction as hex. Nothing is broadcast.
#[tauri::command]
fn spend_export(name: String, what: String) -> R<String> {
    use bitcoin::consensus::Encodable;
    use std::str::FromStr;
    let s = wallet::load()?;
    let _v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let psbt_b64 = s.last_psbt.clone().ok_or("no spend in progress")?;
    let (bytes, ext, stem) = if what == "tx" {
        let tx = wallet::finalize(&psbt_b64)?;
        let mut raw = Vec::new();
        tx.consensus_encode(&mut raw).map_err(|e| e.to_string())?;
        (hex::encode(raw).into_bytes(), "hex", format!("anytwokeys-{}", &tx.compute_txid().to_string()[..12]))
    } else {
        let psbt = bitcoin::psbt::Psbt::from_str(psbt_b64.trim()).map_err(|e| format!("psbt: {e}"))?;
        let txid = psbt.unsigned_tx.compute_txid().to_string();
        (psbt.serialize(), "psbt", format!("anytwokeys-{}", &txid[..12]))
    };
    let path = rfd::FileDialog::new().set_title(if ext == "psbt" { "Save the PSBT" } else { "Save the signed transaction" })
        .set_file_name(format!("{stem}.{ext}")).add_filter(if ext == "psbt" { "PSBT" } else { "Transaction hex" }, &[ext]).save_file()
        .ok_or("cancelled")?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// The spend in progress as the Sign screen draws it: who has signed, what it pays, what is left.
#[tauri::command]
fn spend_ceremony(name: String) -> R<wallet::Ceremony> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let psbt = s.last_psbt.clone().ok_or("no spend in progress")?;
    // A spend built before 2026-09-23 still carries the empty-path entries that crash a SeedSigner; repair it where
    // it lies, so a spend already staged need not be made again. See wallet::strip_pathless.
    if let Some(fixed) = wallet::without_pathless(&psbt) {
        let keep = fixed.clone();
        wallet::update(move |s| { if s.last_psbt.as_deref() == Some(psbt.as_str()) { s.last_psbt = Some(keep); } })?;
        return wallet::ceremony(v, &fixed);
    }
    wallet::ceremony(v, &psbt)
}

#[derive(Serialize)]
struct QrOut { parts: Vec<String>, single: Option<String>, bytes: usize, bbqr: Vec<String> }

/// UR crypto-psbt codes for the screen: a looping set of parts, and one still code when the PSBT is small enough.
#[tauri::command]
fn spend_qr() -> R<QrOut> {
    use std::str::FromStr;
    let s = wallet::load()?;
    let mut p = bitcoin::psbt::Psbt::from_str(s.last_psbt.as_deref().ok_or("no spend in progress")?.trim()).map_err(|e| format!("psbt: {e}"))?;
    wallet::strip_pathless(&mut p);   // what a device reads is never one of the old, crashing copies
    let raw = p.serialize();
    let single = if raw.len() <= 700 { export::ur_parts(&raw, usize::MAX, false)?.pop() } else { None };
    // BBQr too, for the Coldcard Q, which reads that rather than UR
    Ok(QrOut { parts: export::ur_parts(&raw, 160, true)?, single, bytes: raw.len(), bbqr: export::bbqr_parts(&raw, 'P', 400) })
}

fn sats_text(n: u64) -> String { let s = n.to_string(); let mut o = String::new(); for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { o.push(','); } o.push(c); } o }

/// A printable sheet: what the spend pays, who has signed, the PSBT as UR codes and as base64 text.
#[tauri::command]
fn spend_pdf(name: String) -> R<String> {
    use std::str::FromStr;
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let b64 = s.last_psbt.clone().ok_or("no spend in progress")?;
    let c = wallet::ceremony(v, &b64)?;
    let raw = bitcoin::psbt::Psbt::from_str(b64.trim()).map_err(|e| format!("psbt: {e}"))?.serialize();
    let names = |f: &dyn Fn(&wallet::KeySig) -> bool| { let n: Vec<String> = c.keys.iter().filter(|k| f(k)).map(|k| if k.signed > 0 && k.signed < c.inputs { format!("{} ({} of {} inputs)", k.name, k.signed, c.inputs) } else { k.name.clone() }).collect(); if n.is_empty() { "nobody yet".to_string() } else { n.join(", ") } };
    let left = c.need - c.have;
    let sheet = export::Sheet {
        title: if c.ready { "Fully signed transaction, not yet broadcast (PSBT, BIP 174)".into() } else { "Partially signed transaction (PSBT, BIP 174)".into() },
        facts: vec![("Vault".into(), format!("{} \u{b7} {} of {} \u{b7} {}", v.name, v.threshold, v.keys.len(), v.network)),
            ("Signatures".into(), format!("{} of {} on every input", c.have, c.need)), ("Signed by".into(), names(&|k| k.signed > 0)), ("Not yet".into(), names(&|k| k.signed == 0)),
            ("Transaction".into(), c.txid.clone()), ("Inputs \u{b7} fee".into(), format!("{} \u{b7} {} sats", c.inputs, sats_text(c.fee)))],
        rows: c.pays.iter().enumerate().map(|(i, p)| (if i == 0 { "Pays".to_string() } else { String::new() }, p.address.clone(), format!("{} sats{}", sats_text(p.sats), if p.ours { " \u{b7} back to this vault (change)" } else { "" }))).collect(),
        note: if c.ready { "Every signature is on. Any wallet can finalise and broadcast it. Nothing here is secret, but it shows what the vault pays.".into() }
              else { format!("Needs {left} more signature{}. Load it in Any Two Keys (Send \u{203a} Load a PSBT) or any PSBT wallet. It cannot spend as it is.", if left == 1 { "" } else { "s" }) },
        sturdy: false, code_hint: "Any wallet that reads UR (crypto-psbt) codes can scan these, in any order.".into(), text_heading: "The same PSBT as text (base64, BIP 174)".into(),
        codes: export::ur_parts(&raw, 320, false)?, text: b64.trim().to_string() };
    let bytes = export::pdf(&sheet)?;
    let path = rfd::FileDialog::new().set_title("Save the PSBT as a PDF").set_file_name(format!("anytwokeys-{}.pdf", &c.txid[..12])).add_filter("PDF", &["pdf"]).save_file().ok_or("cancelled")?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// The vault's recovery line split into `n` shares, any `k` of which rebuild it (SSKR, in Gordian Envelopes): one
/// printable sheet per share, into a folder the person picks. A share names only its split and its place in it —
/// never the vault, its keys or its network — because one share alone must say nothing to whoever finds it.
#[tauri::command]
fn vault_shares_pdf(name: String, k: usize, n: usize) -> R<String> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let shares = sskrdesc::split(&wallet::recovery_line(v), k, n)?;
    let dir = rfd::FileDialog::new().set_title(format!("Choose a folder for the {n} share sheets")).pick_folder().ok_or("cancelled")?;
    for (i, sh) in shares.iter().enumerate() {
        let id = sskrdesc::set_of(sh).unwrap_or_default();
        let sheet = export::Sheet {
            title: format!("Recovery share {} of {n} \u{b7} any {k} rebuild the recovery line", i + 1),
            facts: vec![("Share".into(), format!("{} of {n}", i + 1)), ("Needed".into(), format!("any {k} of the {n}")), ("Set".into(), format!("{id} \u{2014} only shares of the same set fit together")), ("Made".into(), export::ymd(now()))],
            rows: vec![], codes: vec![sh.to_uppercase()], sturdy: true,
            code_hint: format!("To restore: Any Two Keys \u{203a} Restore \u{203a} Scan the sheet, and show it {k} shares, one after another, in any order."),
            text_heading: "The same share as text (for engraving; either is enough)".into(), text: sh.to_uppercase(),
            note: format!("SSKR share (Blockchain Commons), in a Gordian Envelope. On its own this share reveals nothing about the vault; any {k} together rebuild its recovery line, which shows every address and balance but cannot spend. Keep the shares in different places.") };
        let path = dir.join(format!("anytwokeys-share-{id}-{}-of-{n}.pdf", i + 1));
        std::fs::write(&path, export::pdf(&sheet)?).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(dir.display().to_string())
}

/// The vault's recovery sheet: its descriptor (and chain code) as a code the Restore screen scans, its keys, and the line as text.
#[tauri::command]
fn vault_pdf(name: String) -> R<String> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let sheet = export::recovery_sheet(v)?;
    let bytes = export::pdf(&sheet)?;
    let safe: String = v.name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let path = rfd::FileDialog::new().set_title("Save the recovery sheet").set_file_name(format!("anytwokeys-recovery-{safe}.pdf")).add_filter("PDF", &["pdf"]).save_file().ok_or("cancelled")?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// Bring a PSBT in: pasted text (base64, hex or UR parts), or a file the person picks when `text` is absent.
#[tauri::command]
fn spend_import(name: String, text: Option<String>) -> R<wallet::Ceremony> {
    let raw = match text { Some(t) => t.into_bytes(), None => {
        let path = rfd::FileDialog::new().set_title("Open a PSBT").add_filter("PSBT", &["psbt", "txt", "base64"]).pick_file().ok_or("cancelled")?;
        std::fs::read(&path).map_err(|e| e.to_string())? } };
    let psbt = export::parse(&raw)?;
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
    let current = if s.last_message.is_some() { None } else { s.last_psbt.clone() };
    let merged = wallet::adopt(&v, psbt, current.as_deref())?;
    let keep = merged.clone();
    wallet::update(move |s| { s.last_psbt = Some(keep); s.last_message = None; })?;
    wallet::ceremony(&v, &merged)
}

#[tauri::command]
fn spend_discard() -> R<()> {
    let mut s = wallet::load()?;
    s.last_psbt = None;
    s.last_message = None;
    wallet::save(&s)
}

// ---------- signed messages ----------
#[derive(Serialize)]
struct AddrRow { index: u32, position: u32, chain: &'static str, address: String, current: bool, future: bool }

/// The vault's addresses: index 0 for a fixed vault; for a rotating one, each chain from 0 to its next
/// unused position plus `ahead` more, so the ones beyond can be seen and handed out early.
#[tauri::command]
fn vault_addresses(name: String, ahead: Option<u32>) -> R<Vec<AddrRow>> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    if !wallet::rotates(v) { return Ok(vec![AddrRow { index: 0, position: 0, chain: "receive", address: wallet::address_at(v, 0)?, current: true, future: false }]); }
    let ahead = ahead.unwrap_or(0).min(200);
    let mut out = Vec::new();
    for (base, chain, next) in [(0u32, "receive", v.next), (wallet::CHANGE, "change", v.next_change)] {
        for p in 0..=next + ahead {
            out.push(AddrRow { index: base | p, position: p, chain, address: wallet::address_at(v, base | p)?, current: p == next, future: p > next });
        }
    }
    Ok(out)
}

/// Sign a message with the plugged-in card's own key: one PIN, one touch. Nothing to do with a vault.
#[tauri::command]
async fn message_sign_key(reader: String, pin: String, message: String) -> R<message::KeySig> {
    block(move || {
        let s = wallet::load()?;
        let card = cards::list().map_err(|e| e.to_string())?.into_iter().find(|c| c.reader == reader).ok_or("that card is no longer plugged in")?;
        let pub_hex = card.pubkey.ok_or("this card has no signing key")?;
        let digest = message::legacy_digest(&message);
        let (r, sg) = cards::sign_digest(&reader, &pin, &digest).map_err(|e| e.to_string())?;
        message::key_signature(&pub_hex, wallet::network_of(&s.settings.network), &message, &r, &sg)
    }).await
}

/// Stage a BIP 322 proof for one of the vault's addresses. It then goes through the Sign screen like a spend.
#[tauri::command]
fn message_stage(name: String, index: u32, message: String) -> R<wallet::StagedMessage> {
    let mut s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone(); cards_only(&v, "Signing a message")?;
    if message.trim().is_empty() { return Err("write the message first".into()); }
    let d = wallet::parse(&wallet::descriptor_at(&v, index)?)?.at_derivation_index(0).map_err(|e| e.to_string())?;
    let spk = d.script_pubkey();
    let ws = if v.keys.len() > 1 { Some(d.explicit_script().map_err(|e| e.to_string())?) } else { None };
    let psbt = message::to_sign_psbt(&spk, ws, &message)?;
    let staged = wallet::StagedMessage { vault: v.name.clone(), address: wallet::address_at(&v, index)?, message, index };
    if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == v.name) { if wallet::is_change(index) { vm.scan_max_change = vm.scan_max_change.max(wallet::position(index)); } else { vm.scan_max = vm.scan_max.max(index); } }
    s.last_psbt = Some(psbt.to_string());
    s.last_message = Some(staged.clone());
    wallet::save(&s)?;
    Ok(staged)
}

#[derive(Serialize)]
struct MessageDone { address: String, message: String, signature: String }

/// Every signature is in: turn the finished proof into the base64 BIP 322 signature and clear it.
#[tauri::command]
fn message_finish(name: String) -> R<MessageDone> {
    let mut s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
    let st = s.last_message.clone().ok_or("no message is staged")?;
    let psbt = s.last_psbt.clone().ok_or("no message is staged")?;
    let tx = wallet::finalize(&psbt)?;
    let signature = message::encode_witness(&tx)?;
    let check = message::verify(&st.address, &st.message, &signature, wallet::network_of(&v.network))?;
    if !check.ok { return Err(format!("the finished proof does not verify: {}", check.detail)); }
    s.last_psbt = None;
    s.last_message = None;
    wallet::save(&s)?;
    Ok(MessageDone { address: st.address, message: st.message, signature })
}

#[tauri::command]
fn message_verify(address: String, message: String, signature: String) -> R<message::Verified> {
    let s = wallet::load()?;
    message::verify(&address, &message, &signature, wallet::network_of(&s.settings.network))
}

#[tauri::command]
fn notify_test(app: tauri::AppHandle) -> R<String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification().builder().title("Any Two Keys").body("Notifications reach you. Arrivals, spends and confirmations will look like this.").show().map_err(|e| format!("macOS refused the notification: {e}"))?;
    // show() hands the notification to macOS on a spawned task and drops the result: "sent" is all we can say.
    watch::log("notify_test: sent");
    Ok("Sent. If no banner appeared, look in Notification Centre (click the clock): a Focus such as Do Not Disturb delivers it there silently. To get banners during a Focus, add Any Two Keys under System Settings › Focus › Allowed Apps. If it is not in Notification Centre either, allow it under System Settings › Notifications › Any Two Keys.".into())
}

// ---------- the seal: the recovery record split across the cards (seal.rs) ----------
fn seal_record(v: &Vault) -> seal::Record {
    seal::Record { network: v.network.clone(), threshold: v.threshold, pubkeys: v.keys.iter().map(|k| k.pubkey.clone()).collect(), chain_code: v.chain_code.clone(),
        name: v.name.clone(), key_names: v.keys.iter().map(|k| k.name.clone()).collect() }
}
fn seal_id(v: &Vault) -> String {
    seal::vault_id(&v.network, v.threshold, &v.keys.iter().map(|k| k.pubkey.clone()).collect::<Vec<_>>(), v.chain_code.as_deref())
}
fn points(es: &[&seal::Entry]) -> R<Vec<(u8, Vec<u8>)>> {
    es.iter().map(|e| Ok((e.x, hex::decode(&e.share).map_err(|e| e.to_string())?))).collect()
}
/// The first k-subset of `es` that rebuilds a record, with all n shares re-made from it.
fn seal_combine(es: &[&seal::Entry]) -> Option<(seal::Record, Vec<String>)> {
    let k = es.first()?.k as usize;
    if k == 0 || es.len() < k { return None; }
    let mut pick: Vec<usize> = (0..k).collect();
    loop {
        let chosen: Vec<&seal::Entry> = pick.iter().map(|i| es[*i]).collect();
        if let Ok(pts) = points(&chosen) {
            if let Ok(rec) = seal::interpolate(&pts, 0).and_then(|b| seal::decode(&b)) {
                let all = (1..=rec.pubkeys.len() as u8).map(|x| seal::interpolate(&pts, x).map(hex::encode)).collect::<Result<Vec<_>, _>>().ok()?;
                return Some((rec, all));
            }
        }
        // next combination of k indexes out of es.len()
        let mut i = k;
        loop { if i == 0 { return None; } i -= 1; if pick[i] != i + es.len() - k { break; } }
        pick[i] += 1;
        for j in i + 1..k { pick[j] = pick[j - 1] + 1; }
    }
}

/// Plugged-in cards. "No reader" is a real empty answer; any other failure must not pass for one, or a vault
/// whose cards could not be read would be given a second, incompatible set of shares.
fn seal_cards() -> R<Vec<cards::Sealed>> {
    match cards::read_sealed() {
        Ok(c) => Ok(c),
        Err(cards::CardError::Pcsc(pcsc::Error::NoReadersAvailable)) | Err(cards::CardError::Pcsc(pcsc::Error::NoService)) => Ok(vec![]),
        Err(e) => Err(format!("could not read the keys: {e}")),
    }
}

/// The vault's shares, made once. A vault restored from paper on a second computer has none: adopt the ones
/// already on its cards when k of them are plugged in, rather than sealing a second, incompatible set.
fn seal_shares(v: &Vault, cards: &[cards::Sealed]) -> R<Option<Vec<String>>> {
    if !v.seal.shares.is_empty() { return Ok(Some(v.seal.shares.clone())); }
    let id = seal_id(v);
    let on_cards: Vec<seal::Entry> = cards.iter().filter_map(|c| seal::parse_container(&c.container).ok()).flatten().filter(|e| e.id == id).collect();
    if !on_cards.is_empty() {
        // locked shares cannot be read without their PINs: adopting those goes through Restore › From the keys
        let on_cards: Vec<seal::Entry> = on_cards.into_iter().filter(|e| !e.locked).collect();
        return match seal_combine(&on_cards.iter().collect::<Vec<_>>()) {
            Some((rec, all)) if rec.pubkeys == seal_record(v).pubkeys => Ok(Some(all)),
            _ => Ok(None),   // cards carry an earlier sealing we cannot rebuild yet
        };
    }
    use bitcoin::secp256k1::rand::RngCore;
    let secret = seal::encode(&seal_record(v))?;
    let mut random = vec![0u8; (v.threshold - 1) * secret.len()];
    bitcoin::secp256k1::rand::rngs::OsRng.fill_bytes(&mut random);
    let shares = seal::split(&secret, v.threshold, v.keys.len(), &random)?;
    // never hand out shares that do not come back: rebuild from the last k before trusting them
    let last: Vec<(u8, Vec<u8>)> = (v.keys.len() - v.threshold..v.keys.len()).map(|i| (i as u8 + 1, shares[i].clone())).collect();
    if seal::decode(&seal::interpolate(&last, 0)?)? != seal_record(v) { return Err("the split did not rebuild; nothing was written".into()); }
    Ok(Some(shares.iter().map(hex::encode).collect()))
}

#[derive(Serialize)]
struct SealKey { name: String, pubkey: String, here: bool, state: String }
#[derive(Serialize)]
struct SealStatus { id: String, threshold: usize, keys: Vec<SealKey>, sealed: usize, open: usize, earlier: bool }

#[tauri::command]
async fn seal_status(name: String) -> R<SealStatus> {
    block(move || {
        let s = wallet::load()?;
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone(); cards_only(&v, "Sealing")?;
        let cards = seal_cards()?;
        let shares = seal_shares(&v, &cards)?;
        let id = seal_id(&v);
        let mut sealed = v.seal.sealed.clone();
        let keys: Vec<SealKey> = v.keys.iter().enumerate().map(|(i, k)| {
            let card = cards.iter().find(|c| c.pubkey.as_deref() == Some(k.pubkey.as_str()));
            let state = match card {
                None => if sealed.contains(&k.pubkey) { "sealed" } else { "unsealed" },
                Some(c) => match seal::parse_container(&c.container) {
                    Err(_) => "foreign",
                    Ok(es) => match (es.iter().find(|e| e.id == id), &shares) {
                        // a locked share cannot be compared without the PIN; its place and vault id are what can be checked
                        (Some(e), _) if e.locked && e.x as usize == i + 1 => { if !sealed.contains(&k.pubkey) { sealed.push(k.pubkey.clone()); } "sealed" }
                        (Some(e), Some(sh)) if e.share == sh[i] && e.x as usize == i + 1 => { if !sealed.contains(&k.pubkey) { sealed.push(k.pubkey.clone()); } "open" }
                        (Some(_), _) => { sealed.retain(|p| p != &k.pubkey); "other" }
                        (None, _) => { sealed.retain(|p| p != &k.pubkey); "unsealed" }
                    },
                },
            };
            SealKey { name: k.name.clone(), pubkey: k.pubkey.clone(), here: card.is_some(), state: state.into() }
        }).collect();
        let id2 = id.clone();
        let adopted = v.seal.shares.is_empty() && cards.iter().filter_map(|c| seal::parse_container(&c.container).ok()).flatten().any(|e| e.id == id2 && !e.locked);
        let shares_keep = if adopted { shares.clone() } else { None };   // fresh shares are saved by the first successful write, not by looking
        if sealed != v.seal.sealed || shares_keep.is_some() {
            let (nm, sh, sd) = (v.name.clone(), shares_keep, sealed.clone());
            wallet::update(move |s| if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == nm) { if vm.seal.shares.is_empty() { if let Some(sh) = sh { vm.seal.shares = sh; } } vm.seal.sealed = sd; })?;
        }
        Ok(SealStatus { id, threshold: v.threshold, sealed: keys.iter().filter(|k| k.state == "sealed" || k.state == "open").count(), open: keys.iter().filter(|k| k.state == "open").count(), earlier: shares.is_none(), keys })
    }).await
}

/// Write this vault's share onto the card in `reader`, LOCKED under a secret the card only gives up after its user PIN.
/// `pin` is that PIN (it makes or reads the secret); `admin_pin` writes the certificate object. `replace` overwrites a share
/// from a different sealing. An unlocked share of the same sealing is simply rewritten locked.
#[tauri::command]
async fn seal_write(name: String, reader: String, admin_pin: String, pin: String, replace: bool) -> R<String> {
    block(move || {
        let s = wallet::load()?;
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone(); cards_only(&v, "Sealing")?;
        let cards = seal_cards()?;
        let card = cards.iter().find(|c| c.reader == reader).ok_or("that key is no longer plugged in")?;
        let i = v.keys.iter().position(|k| Some(k.pubkey.as_str()) == card.pubkey.as_deref()).ok_or("that key is not one of this vault's")?;
        let mut v2 = v.clone();
        if replace && seal_shares(&v, &cards)?.is_none() { v2.seal.shares.clear(); }
        let shares = match seal_shares(&v2, if replace { &[] } else { &cards })? { Some(sh) => sh, None => return Err("This vault's cards carry shares from an earlier sealing. Plug in enough of them together to adopt it, or reseal every card.".into()) };
        let id = seal_id(&v);
        let mut entries = seal::parse_container(&card.container)?;
        if pin.is_empty() { return Err("enter this key's own PIN as well: it locks the share, so that nobody who only HOLDS the key can read it".into()); }
        if let Some(e) = entries.iter().find(|e| e.id == id) {
            if e.locked && !replace { return Ok("already sealed and locked".into()); }
            if !e.locked && e.share != shares[i] && !replace { return Err("This key already carries a different share of this vault. Reseal every card to replace it.".into()); }
        }
        let key = cards::seal_key(&reader, &pin, true).map_err(|e| e.to_string())?.ok_or("the key would not keep a secret for the seal")?;
        if let Some(pk) = &card.pubkey { learn_card(pk, card.serial, &key); }   // a sealed key can open this app too
        use bitcoin::secp256k1::rand::RngCore;
        let mut nonce = [0u8; 8];
        bitcoin::secp256k1::rand::rngs::OsRng.fill_bytes(&mut nonce);
        let plain = hex::decode(&shares[i]).map_err(|e| e.to_string())?;
        let body = seal::lock_share(&key, &id, v.threshold as u8, i as u8 + 1, &plain, nonce)?;
        // never write a lock that has not been seen to open
        if seal::unlock_share(&key, &id, v.threshold as u8, i as u8 + 1, &body)? != plain { return Err("the lock did not open again; nothing was written".into()); }
        entries.retain(|e| e.id != id);
        entries.push(seal::Entry { id, k: v.threshold as u8, x: i as u8 + 1, share: hex::encode(body), locked: true });
        let data = seal::build_container(&entries)?;
        if data.len() > card.room { return Err(format!("this key has no room left ({} of {} bytes)", data.len(), card.room)); }
        cards::write_sealed(&reader, &admin_pin, &card.container, &data).map_err(|e| e.to_string())?;
        let (nm, pk) = (v.name.clone(), v.keys[i].pubkey.clone());
        wallet::update(move |s| if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == nm) {
            if vm.seal.shares != shares { vm.seal.shares = shares; vm.seal.sealed.clear(); }
            if !vm.seal.sealed.contains(&pk) { vm.seal.sealed.push(pk); }
        })?;
        Ok(format!("sealed and locked, {} bytes on the key", data.len()))
    }).await
}

#[derive(Serialize, Deserialize, Clone)]
struct Found { pubkey: String, serial: u32, entry: seal::Entry }
#[derive(Serialize)]
struct SealRead { cards: usize, foreign: Vec<u32>, found: Vec<Found> }

/// Every share on every plugged-in card. No PIN: the certificate object is readable by anyone holding the card.
#[tauri::command]
async fn seal_read() -> R<SealRead> {
    block(|| {
        let cards = seal_cards()?;
        let mut out = SealRead { cards: cards.len(), foreign: vec![], found: vec![] };
        for c in &cards {
            match seal::parse_container(&c.container) {
                Err(_) => out.foreign.push(c.serial),
                Ok(es) => for e in es { out.found.push(Found { pubkey: c.pubkey.clone().unwrap_or_default(), serial: c.serial, entry: e }); },
            }
        }
        Ok(out)
    }).await
}

#[derive(Serialize)]
struct Candidate { id: String, have: usize, locked: usize, need: usize, record: Option<seal::Record>, line: Option<String>, address: Option<String>, known_as: Option<String> }

fn seal_vault(rec: &seal::Record, name: String, shares: Vec<String>, sealed: Vec<String>) -> R<Vault> {
    let descriptor = wallet::descriptor_for(rec.threshold, &rec.pubkeys)?;
    let mut v = Vault { watch: None, spark: None, muun: None, name, network: rec.network.clone(), threshold: rec.threshold,
        keys: rec.pubkeys.iter().zip(&rec.key_names).map(|(p, n)| wallet::KeyRef { name: n.clone(), serial: 0, pubkey: p.clone(), device: None }).collect(),
        descriptor, address: String::new(), created: now(), chain_code: rec.chain_code.clone(), next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0,
        seal: wallet::Seal { shares, sealed }, ext: None, cost_method: None };
    v.address = wallet::address_at(&v, 0)?;
    Ok(v)
}

/// A share must sit on the card whose key holds that position in the record, or it was copied from elsewhere.
fn seal_check(rec: &seal::Record, found: &[Found]) -> R<()> {
    for f in found { if rec.pubkeys.get(f.entry.x as usize - 1) != Some(&f.pubkey) { return Err(format!("the share on key {} does not belong to that key", f.serial)); } }
    Ok(())
}

/// What the shares collected so far (possibly over several card swaps) can rebuild.
#[tauri::command]
fn seal_preview(found: Vec<Found>) -> R<Vec<Candidate>> {
    let s = wallet::load()?;
    let mut ids: Vec<String> = found.iter().map(|f| f.entry.id.clone()).collect();
    ids.sort(); ids.dedup();
    Ok(ids.into_iter().map(|id| {
        let all: Vec<&Found> = found.iter().filter(|f| f.entry.id == id).collect();
        let need = all[0].entry.k as usize;
        let locked = all.iter().filter(|f| f.entry.locked).count();
        let mine: Vec<&Found> = all.into_iter().filter(|f| !f.entry.locked).collect();
        let built = seal_combine(&mine.iter().map(|f| &f.entry).collect::<Vec<_>>());
        let known_as = s.vaults.iter().find(|v| seal_id(v) == id).map(|v| v.name.clone());
        match built.and_then(|(rec, sh)| seal_vault(&rec, rec.name.clone(), sh, vec![]).ok().map(|v| (rec, v))) {
            Some((rec, v)) => Candidate { id, have: mine.len(), locked, need, line: Some(wallet::recovery_line(&v)), address: Some(v.address.clone()), record: Some(rec), known_as },
            None => Candidate { id, have: mine.len(), locked, need, record: None, line: None, address: None, known_as },
        }
    }).collect())
}

// ------------------------------------------------------------------- lock --
// What this computer keeps is sealed (lockbox.rs). These are the ways in, and the settings for them.

/// Location Services are listened to only while the setting is on. The manager lives on the main thread.
fn location_follow(app: &tauri::AppHandle, on: bool) { let _ = app.run_on_main_thread(move || if on { location::start() } else { location::stop() }); }

/// After the app opens (the setting is inside the sealed store, so not before): start listening if the person asked for it.
#[tauri::command]
fn location_status(app: tauri::AppHandle) -> R<String> {
    let on = wallet::load()?.settings.location;
    if on && location::state() == "off" { location_follow(&app, true); return Ok("asking".into()); }
    Ok(if on { location::state() } else { "off".into() })
}

#[derive(Serialize)]
struct Here { lat: f64, lon: f64, acc: f64, name: Option<String> }

/// Where this Mac is now, for the one transaction the person is looking at — asked for by a tap, so it asks Location
/// Services once even when places are not recorded automatically, and stops listening again straight after. The
/// name is worked out here from the built-in list of towns: nothing goes to any server.
#[tauri::command]
async fn location_now(app: tauri::AppHandle) -> R<Here> {
    let following = wallet::load()?.settings.location;
    // Places on and listening: its last fix is right to within 200 m however old (it reports on movement), so it
    // answers at once. Waiting for a NEW one, as this did, waited for a report that comes only after moving.
    if following && location::state() == "allowed" { if let Some(f) = location::last() {
        let _ = app.run_on_main_thread(location::nudge_main);   // and freshen it for next time
        return Ok(Here { lat: f.lat, lon: f.lon, acc: f.acc, name: context::place(f.lat, f.lon) });
    } }
    let started = location::fresh(now(), 120).is_none() && location::state() == "off";
    if started { location_follow(&app, true); } else { let _ = app.run_on_main_thread(location::nudge_main); }   // one reading now, not after 200 m
    // the system answers on the main thread; this waits for it off the main thread
    let fix = block(move || -> R<Option<context::Fix>> {
        let t = std::time::Instant::now();
        Ok(loop {
            if let Some(f) = location::fresh(now(), 120) { break Some(f); }
            let st = location::state();
            if st == "denied" || st.starts_with("unavailable") || t.elapsed() > std::time::Duration::from_secs(15) { break None; }
            std::thread::sleep(std::time::Duration::from_millis(250));
        })
    }).await?;
    let st = location::state();
    // while macOS is still asking the person, keep listening: stopping now would lose their answer
    if started && !following && st != "asking" { location_follow(&app, false); }
    let f = fix.ok_or_else(|| match st.as_str() {
        "denied" => "Location Services are off for Any Two Keys. Turn them on in System Settings › Privacy & Security › Location Services, then try again.".to_string(),
        "asking" => "macOS is asking whether Any Two Keys may use your location. Answer its prompt (it may be behind this window), then tap the button again.".to_string(),
        s if s.starts_with("unavailable") => "This computer cannot say where it is.".to_string(),
        _ => format!("Location Services let Any Two Keys ask, but no position came within 15 seconds{}. Check that Location Services are on for this Mac (the switch at the top of System Settings › Privacy & Security › Location Services) and that Wi-Fi is on — a Mac finds itself from nearby networks — then try again.",
            location::last_error().map(|c| format!(" (Core Location error {c})")).unwrap_or_default()),
    })?;
    Ok(Here { lat: f.lat, lon: f.lon, acc: f.acc, name: context::place(f.lat, f.lon) })
}

/// What this computer would record now, for the screen that asks whether to keep places: the NAME only.
#[tauri::command]
fn location_preview() -> R<Option<String>> { Ok(location::fresh(now(), 1800).and_then(|f| context::place(f.lat, f.lon))) }

/// The picture for a place: tiles from OUR host, fetched here (never by the window), each kept sealed once fetched so a
/// place is asked for once; drawn here, handed over as a PNG. `Ok(None)`: no map to be had just now — the screen simply
/// shows none, because a map is a nicety and its absence is not an error worth a message.
#[derive(serde::Serialize)]
struct MapPicture { png: String, metres_per_px: f64, width: u32 }

#[tauri::command]
async fn tx_map(lat: f64, lon: f64, dark: bool) -> R<Option<MapPicture>> {
    block(move || {
        use bitcoin::base64::{self, Engine};
        let Some(png) = map_png(lat, lon, dark)? else { return Ok(None) };
        Ok(Some(MapPicture { png: format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)), metres_per_px: map::metres_per_px(), width: map::W }))
    }).await
}

/// The picture for a place, drawn here from tiles kept sealed. `Ok(None)`: no map to be had just now.
fn map_png(lat: f64, lon: f64, dark: bool) -> R<Option<Vec<u8>>> {
    {
        if !(-85.0..=85.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) { return Ok(None); }
        let url = std::env::var("A2K_TILES").ok().unwrap_or_else(|| wallet::load().map(|s| s.settings.tiles).unwrap_or_default());
        if url.is_empty() || !lockbox::sealed_mode() { return Ok(None); }
        let (x, y, fx, fy) = map::locate(lat, lon, map::ZOOM);
        let src = map::Http { url: url.clone() };
        static HEAD: std::sync::Mutex<Option<(String, Vec<u8>)>> = std::sync::Mutex::new(None);
        let mut head: Option<Vec<u8>> = HEAD.lock().ok().and_then(|g| g.clone()).filter(|(u, _)| *u == url).map(|(_, h)| h);
        let dir = lockbox::dir().join("tiles"); std::fs::create_dir_all(&dir).ok();
        let mut tiles = Vec::new();
        for (dx, dy) in map::needed(lat, fx, fy) {
            let (tx, ty) = (x as i64 + dx as i64, y as i64 + dy as i64);
            if tx < 0 || ty < 0 || tx >= (1i64 << map::ZOOM) || ty >= (1i64 << map::ZOOM) { continue; }
            let path = dir.join(lockbox::blob_name(&format!("tile:{}/{tx}/{ty}", map::ZOOM))?);
            let bytes = match lockbox::read_sealed(&path, "tile") { Ok(Some(b)) => Some(b), _ => {
                if head.is_none() { use map::Source; match src.range(0, 16384) { Ok(h) => { if let Ok(mut g) = HEAD.lock() { *g = Some((url.clone(), h.clone())); } head = Some(h); } Err(_) => return Ok(None) } }
                match map::tile(&src, head.as_ref().unwrap(), map::ZOOM, tx as u32, ty as u32) { Ok(Some(b)) => { let _ = lockbox::write_sealed(&path, "tile", &b); Some(b) } Ok(None) => None, Err(_) => return Ok(None) } } };
            if let Some(b) = bytes { tiles.push((dx, dy, b)); }
        }
        if tiles.is_empty() { return Ok(None); }
        Ok(Some(map::draw(lat, &tiles, fx, fy, dark)?))
    }
}

/// A printable record of ONE transaction: what it was, what it was worth, your label, note, place and pictures, and
/// the chain's own facts under them. Everything on it is already on the screen; nothing is fetched that the screen
/// would not fetch. The pictures are embedded, so the file stands on its own.
#[tauri::command]
async fn tx_pdf(name: String, txid: String, height: i32, ccy: String) -> R<String> {
    if !(ccy.is_empty() || (ccy.len() == 3 && ccy.chars().all(|c| c.is_ascii_uppercase()))) { return Err("the currency is a three-letter code".into()); }
    let bytes = block({ let (name, txid, ccy) = (name.clone(), txid.clone(), ccy.clone()); move || tx_sheet(&name, &txid, height, &ccy) }).await?;
    let short: String = txid.chars().take(8).collect();
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let path = rfd::FileDialog::new().set_title("Save this transaction").set_file_name(format!("anytwokeys-{safe}-{short}.pdf")).add_filter("PDF", &["pdf"]).save_file().ok_or("cancelled")?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

pub(crate) fn tx_sheet(name: &str, txid: &str, height: i32, ccy: &str) -> R<Vec<u8>> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
    let servers = wallet::servers_for(&s, &v.network);
    let d = activity::detail(&v, &servers, txid, height)?;
    let prices = if s.settings.custom { s.settings.prices.clone() } else { activity::PRICES.to_string() };
    let fiat_ok = v.network == "mainnet" && !ccy.is_empty() && !prices.is_empty();
    let (q_then, q_now) = if fiat_ok { (activity::quote_at(&prices, ccy, d.time), activity::quote_at(&prices, ccy, None)) } else { (None, None) };
    let (title, subtitle, blocks) = tx_blocks(&v, &d, &s.contacts, ccy, &q_then, &q_now, true);
    export::doc(&title, &subtitle, &blocks)
}

/// Everything that goes on the sheet, worked out from what this computer already knows: no network, no clock beyond
/// the printing date, so it can be tested as it will be printed. `with_pictures`: read and embed the files.
pub(crate) fn tx_blocks(v: &wallet::Vault, d: &activity::Detail, contacts: &[wallet::Contact], ccy: &str,
    q_then: &Option<activity::Quote>, q_now: &Option<activity::Quote>, with_pictures: bool) -> (String, String, Vec<export::Block>) {
    use export::Block;
    let txid = &d.txid;
    let key = format!("tx:{txid}");
    let (label, note) = (v.labels.get(&key).cloned().unwrap_or_default(), v.notes.get(&key).cloned().unwrap_or_default());
    let ctx = v.more.get(&key).and_then(|m| m.get("ctx")).filter(|c| c.get("removed").is_none()).cloned();

    let sats = |n: u64| { let t = n.to_string(); let mut o = String::new(); for (i, c) in t.chars().enumerate() { if i > 0 && (t.len() - i) % 3 == 0 { o.push(','); } o.push(c); } o };
    let money = |q: &Option<activity::Quote>| q.as_ref().map(|q| format!("{}{:.2} {ccy}", if d.net < 0 { "−" } else { "" }, (d.net.unsigned_abs() as f64) / 1e8 * q.price));

    let when = d.time.map(|t| format!("{} UTC", chrono_ish(t as u64))).unwrap_or_else(|| "not in a block yet".into());
    let conf = if d.height > 0 { format!("confirmed · {} block{} deep", sats((d.tip as i64 - d.height as i64 + 1).max(1) as u64), if d.tip as i64 - d.height as i64 + 1 == 1 { "" } else { "s" }) } else { "in the mempool".into() };
    let mut facts = vec![
        ("Amount".to_string(), format!("{}{} sats", if d.net > 0 { "+" } else if d.net < 0 { "−" } else { "" }, sats(d.net.unsigned_abs()))),
    ];
    if let Some(m) = money(&q_then) { facts.push(("Worth then".into(), format!("{m}  ({})", q_then.as_ref().unwrap().basis))); }
    if let Some(m) = money(&q_now) { facts.push(("Worth now".into(), format!("{m}  ({})", q_now.as_ref().unwrap().basis))); }
    facts.push(("When".into(), when));
    facts.push(("Status".into(), conf));
    facts.push(("Vault".into(), format!("{} · {} · {} of {} keys", v.name, v.network, v.threshold, v.keys.len())));

    let book: Vec<&wallet::Contact> = contacts.iter().filter(|c| c.network == v.network).collect();
    let who = |a: &Option<String>| a.as_ref().and_then(|a| book.iter().find(|c| c.address.eq_ignore_ascii_case(a)).map(|c| format!("  ({})", c.name))).unwrap_or_default();
    let place_of = |i: u32| if i >= wallet::CHANGE { format!("  ({} · change #{})", v.name, i - wallet::CHANGE) } else { format!("  ({} · receive #{})", v.name, i) };
    let line = |a: &Option<String>, ours: Option<u32>, n: u64| format!("{}{}  {} sats", a.clone().unwrap_or_else(|| "no address".into()), ours.map(place_of).unwrap_or_else(|| who(a)), sats(n));
    let ours_in: Vec<&activity::TxInput> = d.inputs.iter().filter(|i| i.ours.is_some()).collect();
    let (foreign_out, ours_out): (Vec<_>, Vec<_>) = d.outputs.iter().partition(|o| o.ours.is_none());
    if ours_in.is_empty() {
        facts.push(("From".into(), format!("outside this vault · {} input{}", d.inputs.len(), if d.inputs.len() == 1 { "" } else { "s" })));
        facts.push(("To".into(), ours_out.iter().map(|o| line(&o.address, o.ours, o.sats)).collect::<Vec<_>>().join("; ")));
        facts.push(("Fee".into(), d.fee.map(|f| format!("{} sats, paid by the sender", sats(f))).unwrap_or_else(|| "paid by the sender".into())));
    } else {
        facts.push(("To".into(), if foreign_out.is_empty() { "nowhere outside this vault: the coins moved between its own addresses".into() } else { foreign_out.iter().map(|o| line(&o.address, o.ours, o.sats)).collect::<Vec<_>>().join("; ") }));
        if !ours_out.is_empty() { facts.push(("Change".into(), ours_out.iter().map(|o| line(&o.address, o.ours, o.sats)).collect::<Vec<_>>().join("; "))); }
        facts.push(("Fee".into(), match (d.fee, d.fee_rate) { (Some(f), Some(r)) => format!("{} sats · {r:.2} sat/vB", sats(f)), (Some(f), None) => format!("{} sats", sats(f)), _ => "not known".into() }));
        let signers: Vec<String> = { let mut v: Vec<String> = ours_in.iter().flat_map(|i| i.signers.clone()).collect(); v.dedup(); v.dedup(); v };
        facts.push(("Signed by".into(), if signers.is_empty() { "could not be read from the signatures".into() } else { format!("{}  (read from the signatures on the transaction)", signers.join(" and ")) }));
    }

    let title = if label.is_empty() { format!("{} · {}{} sats", d.time.map(|t| chrono_ish(t as u64)).unwrap_or_else(|| "Unconfirmed".into()), if d.net > 0 { "+" } else if d.net < 0 { "−" } else { "" }, sats(d.net.unsigned_abs())) } else { label.clone() };
    let mut blocks = vec![Block::Facts(facts)];
    // the label is the title already; printing it twice is noise
    if !note.trim().is_empty() { blocks.push(Block::Heading("Your note".into())); blocks.push(Block::Para(note)); }

    // where and on what, with the map as it is drawn on screen
    if let Some(c) = &ctx {
        let str_of = |k: &str| c.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let mut kv = Vec::new();
        let how = match c.get("how").and_then(|h| h.as_str()) { Some("made") => "Made on", Some("seen") => "First seen on", _ => "Noted on" };
        if let Some(p) = str_of("place") { kv.push(("Place".to_string(), p)); }
        if let Some(g) = c.get("geo") { if let (Some(la), Some(lo)) = (g.get("lat").and_then(|x| x.as_f64()), g.get("lon").and_then(|x| x.as_f64())) {
            kv.push(("Position".into(), format!("{la:.3}, {lo:.3}{}", g.get("acc").and_then(|a| a.as_f64()).filter(|a| *a > 0.0).map(|a| format!("  (to about {a:.0} m)")).unwrap_or_default())));
        } }
        if let Some(dev) = c.get("device") {
            let d2 = |k: &str| dev.get(k).and_then(|x| x.as_str()).unwrap_or("");
            kv.push((how.to_string(), format!("{}  ({})", d2("name"), [d2("model"), d2("os"), &format!("Any Two Keys {}", d2("app"))].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · "))));
        }
        if c.get("how").and_then(|h| h.as_str()) == Some("seen") { kv.push((String::new(), "A deposit happens wherever the sender was; this is only where this computer was when it noticed.".into())); }
        if !kv.is_empty() { blocks.push(Block::Heading("Where and on what".into())); blocks.push(Block::Facts(kv)); }
        if let Some(g) = c.get("geo").filter(|_| with_pictures) { if let (Some(la), Some(lo)) = (g.get("lat").and_then(|x| x.as_f64()), g.get("lon").and_then(|x| x.as_f64())) {
            if let Ok(Some(png)) = map_png(la, lo, false) { if let Ok((j, w, h)) = export::jpeg_for_print(&png, 1200) {
                blocks.push(Block::Jpeg { data: j, w, h, caption: "Map data © OpenStreetMap contributors. The place is marked at the centre.".into(), wide: false });
            } }
        } }
    }

    blocks.extend(file_blocks(v, &key, with_pictures));


    blocks.push(Block::Rule);
    blocks.push(Block::Heading("The chain's own facts".into()));
    blocks.push(Block::Facts(vec![
        ("Transaction".into(), d.txid.clone()),
        ("Witness id".into(), d.wtxid.clone()),
        ("Block".into(), match (&d.block, d.position) { (Some(b), Some(p)) => format!("{} · {}  (position {})", sats(d.height as u64), b, sats(p as u64)), (Some(b), None) => format!("{} · {}", sats(d.height as u64), b), _ => "not in a block yet".into() }),
        ("Inclusion".into(), if d.proof == "proved" { format!("proved: this computer's node gave a merkle path that hashes to the root in block {}'s header", sats(d.height as u64)) } else { d.proof.clone() }),
        ("Size".into(), format!("{} B · {} vB · {} WU", sats(d.size as u64), sats(d.vsize as u64), sats(d.weight))),
        ("Replaceable".into(), if d.rbf { "signalled (RBF)".into() } else { "not signalled".to_string() }),
    ]));
    blocks.push(Block::Heading("The transaction itself".into()));
    blocks.push(Block::Mono(d.hex.clone()));
    blocks.push(Block::Para("Anyone can check this record: the transaction above is the one the chain holds, and its id is printed with it.".into()));

    (title, format!("{} · printed {}", v.name, export::ymd(now())), blocks)
}

/// The pictures kept with a record, printed, and a line for every other file — for any record's sheet.
pub(crate) fn file_blocks(v: &wallet::Vault, key: &str, with_pictures: bool) -> Vec<export::Block> {
    use export::Block;
    let mut blocks = Vec::new();
    let files = v.files.get(key).cloned().unwrap_or_default();
    let (mut shown, mut listed) = (false, Vec::new());
    for e in &files {
        let (nm, mime, hash) = (e.get("name").and_then(|x| x.as_str()).unwrap_or("file").to_string(), e.get("mime").and_then(|x| x.as_str()).unwrap_or("").to_string(), e.get("sha256").and_then(|x| x.as_str()).unwrap_or("").to_string());
        let size = e.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
        let here = with_pictures && attach::have(&hash);
        if mime.starts_with("image/") && here {
            if let Ok(bytes) = attach::read(&hash) { if let Ok((j, w, h)) = export::jpeg_for_print(&bytes, 1400) {
                if !shown { blocks.push(Block::Heading("Pictures".into())); shown = true; }
                blocks.push(Block::Jpeg { data: j, w, h, caption: nm.clone(), wide: true });
                continue;
            } }
        }
        listed.push(format!("{nm} · {} KB{}", size / 1024, if here || !with_pictures { "" } else { " · not on this computer" }));
    }
    if !listed.is_empty() { blocks.push(Block::Heading("Other files kept with this".into())); blocks.push(Block::Para(listed.join("\n"))); }
    blocks
}

/// One block of a report the window lays out: facts, a heading, a paragraph, or a chart.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum BlockIn { Facts { rows: Vec<(String, String)> }, Heading { text: String }, Para { text: String }, Chart { chart: ChartIn } }

/// A printable report of a whole vault: what the screen worked out (its figures, charts, coins and every
/// transaction, passed in so the sheet says what the screen says), laid out as a PDF. The recovery line is not on
/// it — that is the recovery sheet's job, and a report is shared more freely.
#[tauri::command]
async fn report_pdf(title: String, subtitle: String, blocks: Vec<BlockIn>, file: String) -> R<String> {
    let bytes = block(move || -> R<Vec<u8>> {
        use export::Block;
        let out: Vec<Block> = blocks.into_iter().map(|b| match b {
            BlockIn::Facts { rows } => Block::Facts(rows), BlockIn::Heading { text } => Block::Heading(text), BlockIn::Para { text } => Block::Para(text),
            BlockIn::Chart { chart: c } => Block::Chart { title: c.title, series: c.series.into_iter().map(|s| (s.name, if s.color == "cost" { export::COST_RGB } else { export::VALUE_RGB }, s.pts, s.step)).collect(),
                lo: c.lo, hi: c.hi, lo_label: c.lo_label, hi_label: c.hi_label, from_label: c.from_label, to_label: c.to_label },
        }).collect();
        export::doc(&title, &subtitle, &out)
    }).await?;
    let safe: String = file.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' }).collect();
    let path = rfd::FileDialog::new().set_title("Save the report").set_file_name(format!("anytwokeys-{safe}.pdf")).add_filter("PDF", &["pdf"]).save_file().ok_or("cancelled")?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// An address's balance over time, as the flow of each transaction that touched it (see activity::address_flow).
#[tauri::command]
async fn address_flow(name: String, index: u32, history: Vec<(String, i32)>) -> R<Vec<activity::Flow>> {
    block(move || {
        let s = wallet::load()?;
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        activity::address_flow(&v, &wallet::servers_for(&s, &v.network), &history, index)
    }).await
}

#[derive(Deserialize)]
struct SeriesIn { name: String, color: String, pts: Vec<(f64, f64)>, #[serde(default)] step: bool }
#[derive(Deserialize)]
struct ChartIn { title: String, series: Vec<SeriesIn>, lo: f64, hi: f64, lo_label: String, hi_label: String, from_label: String, to_label: String }

/// A printable record of one coin or one address: the figures the screen worked out (cost, worth, gain — passed in,
/// so the sheet says exactly what the screen says), its chart, and everything kept with it: label, note, pictures,
/// files. `key` is the record's own key ("output:txid:vout" or "addr:…").
#[tauri::command]
async fn record_pdf(name: String, key: String, title: String, subtitle: String, facts: Vec<(String, String)>, sections: Vec<(String, Vec<(String, String)>)>, charts: Vec<ChartIn>, file: String) -> R<String> {
    if !(key.starts_with("output:") || key.starts_with("addr:")) { return Err("that is not a coin or an address".into()); }
    let bytes = block(move || -> R<Vec<u8>> {
        use export::Block;
        let s = wallet::load()?;
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        let mut blocks = vec![Block::Facts(facts)];
        for c in charts {
            let series = c.series.into_iter().map(|s| (s.name, if s.color == "cost" { export::COST_RGB } else { export::VALUE_RGB }, s.pts, s.step)).collect();
            blocks.push(Block::Chart { title: c.title, series, lo: c.lo, hi: c.hi, lo_label: c.lo_label, hi_label: c.hi_label, from_label: c.from_label, to_label: c.to_label });
        }
        for (h, kv) in sections { if !kv.is_empty() { blocks.push(Block::Heading(h)); blocks.push(Block::Facts(kv)); } }
        let note = v.notes.get(&key).cloned().unwrap_or_default();
        if !note.trim().is_empty() { blocks.push(Block::Heading("Your note".into())); blocks.push(Block::Para(note)); }
        blocks.extend(file_blocks(&v, &key, true));
        export::doc(&title, &subtitle, &blocks)
    }).await?;
    let safe: String = file.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' }).collect();
    let path = rfd::FileDialog::new().set_title("Save this record").set_file_name(format!("anytwokeys-{safe}.pdf")).add_filter("PDF", &["pdf"]).save_file().ok_or("cancelled")?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// "Thu 18 Sep 2026, 18:26" for a Unix time, UTC. Enough for a printed record; no date crate for one line.
fn chrono_ish(t: u64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MON: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let ymd = export::ymd(t);
    let (y, m, d) = (&ymd[..4], ymd[5..7].parse::<usize>().unwrap_or(1), &ymd[8..10]);
    format!("{} {} {} {}, {:02}:{:02}", DAYS[((t / 86_400) % 7) as usize], d.trim_start_matches('0'), MON[m - 1], y, (t / 3600) % 24, (t / 60) % 60)
}

/// Search-as-you-type for a place, on OUR server only (places.rs). Ranked near this computer when Places is on —
/// by a position rounded to about 11 km.
#[tauri::command]
async fn place_search(q: String) -> R<Vec<places::Hit>> {
    block(move || {
        let s = wallet::load()?;
        let near = if s.settings.location { location::fresh(now(), 3600).map(|f| (f.lat, f.lon)) } else { None };
        places::search(&s.settings.geocode, &q, near)
    }).await
}

/// What a spend OUT of the vaults was: a sale, something bought, a gift, or a move to a wallet of their own that
/// this app does not hold. Only the last changes the arithmetic — it is not a disposal, so no gain is realised and
/// the cost leaves the books with the coins. The others are all disposals; they record WHY, which is what an
/// accountant needs and what this app cannot infer from the chain.
#[tauri::command]
async fn disposal_set(name: String, txid: String, kind: String) -> R<Store> {
    const KINDS: [&str; 5] = ["sold", "spent", "gift", "self", ""];
    if !KINDS.contains(&kind.as_str()) { return Err(format!("a disposal is one of {}", KINDS[..4].join(", "))); }
    let t = now();
    let (nm, id, k) = (name.clone(), txid.clone(), kind.clone());
    wallet::update(move |s| if let Some(v) = s.vaults.iter_mut().find(|v| v.name == nm) {
        let key = format!("tx:{id}");
        let rec = v.more.entry(key.clone()).or_default();
        if k.is_empty() { rec.remove("disposal"); } else { rec.insert("disposal".into(), serde_json::json!({ "kind": k, "t": t })); }
        v.label_meta.insert(key, t);
    })?;
    let _ = labels_sync(name).await;
    Ok(wallet::load()?)
}

/// The person's own say over a transaction's place: take it off, name it themselves, or choose it from the search
/// (which brings its position with it, so the record gets a map).
#[tauri::command]
fn ctx_set(name: String, txid: String, remove: bool, place: Option<String>, lat: Option<f64>, lon: Option<f64>) -> R<Store> {
    let t = now();
    wallet::update(|s| if let Some(v) = s.vaults.iter_mut().find(|v| v.name == name) {
        let key = format!("tx:{txid}");
        let rec = v.more.entry(key.clone()).or_default();
        if remove { rec.insert("ctx".into(), serde_json::json!({ "removed": true, "t": t })); }
        else if let Some(p) = place.as_ref().map(|p| p.trim().chars().take(160).collect::<String>()).filter(|p| !p.is_empty()) {
            let mut ctx = rec.get("ctx").cloned().filter(|c| c.get("removed").is_none()).unwrap_or_else(|| context::build("noted", None, false, t));
            ctx["place"] = p.into(); ctx["named"] = true.into();
            match (lat, lon) {
                (Some(la), Some(lo)) if (-90.0..=90.0).contains(&la) && (-180.0..=180.0).contains(&lo) => { let r = |v: f64| if s.settings.location_exact { (v * 1e6).round() / 1e6 } else { (v * 1e3).round() / 1e3 }; ctx["geo"] = serde_json::json!({ "lat": r(la), "lon": r(lo), "acc": 0 }); ctx["picked"] = true.into(); }
                _ => { if let Some(o) = ctx.as_object_mut() { o.remove("geo"); o.remove("picked"); } }   // the person's own word for the place replaces any coordinates
            }
            rec.insert("ctx".into(), ctx);
        }
        v.label_meta.insert(key, t);   // an edit: it has to win over the copy on the host
    })
}

#[derive(serde::Serialize)]
struct LockCard { pubkey: String, name: String, added: u64 }
#[derive(serde::Serialize)]
struct LockStatus { sealed: bool, unlocked: bool, finished: bool, touch: bool, idle_minutes: u32, with_screen: bool, cards: Vec<LockCard>, moved: Option<lockbox::Migration> }

fn lock_state(moved: Option<lockbox::Migration>) -> R<LockStatus> {
    let s = lockbox::slots()?;
    Ok(LockStatus { sealed: lockbox::sealed_mode(), unlocked: lockbox::unlocked(), finished: s.as_ref().map(|s| s.migrated).unwrap_or(false), touch: s.as_ref().map(|s| s.touch).unwrap_or(true),
        idle_minutes: s.as_ref().map(|s| s.idle_minutes).unwrap_or(0), with_screen: s.as_ref().map(|s| s.with_screen).unwrap_or(true), cards: s.map(|s| s.cards.into_iter().map(|c| LockCard { pubkey: c.pubkey, name: c.name, added: c.added }).collect()).unwrap_or_default(), moved })
}

#[tauri::command]
fn lock_status() -> R<LockStatus> { lock_state(None) }

/// The first run of a build that seals its files: make the key, put it in the Keychain, seal everything, prove each
/// sealed copy before its readable one goes. Also finishes a move that was interrupted.
#[tauri::command]
async fn lock_start() -> R<LockStatus> {
    block(|| { let m = lockbox::migrate()?; attach::clear_opened(); lock_state(Some(m)) }).await
}

#[tauri::command]
async fn lock_open_mac() -> R<LockStatus> {
    block(|| { lockbox::unlock_mac("open your vaults")?; let m = lockbox::migrate()?; attach::clear_opened(); lock_state(Some(m)) }).await
}

/// The plugged-in key and its PIN. A wrong PIN costs one of the key's tries, as it would when signing.
#[tauri::command]
async fn lock_open_card(reader: String, pin: String) -> R<LockStatus> {
    block(move || {
        let card = cards::list().map_err(|e| e.to_string())?.into_iter().find(|c| c.reader == reader).ok_or("that key is no longer plugged in")?;
        let pubkey = card.pubkey.clone().ok_or("that key has no signing key on it")?;
        let slots = lockbox::slots()?.ok_or("this computer has nothing sealed yet")?;
        // the PIN is only spent on a key that can actually open this app
        if !slots.cards.iter().any(|c| c.pubkey == pubkey) { return Err("that key has not been added as a way to open this app".into()); }
        let secret = cards::seal_key(&reader, &pin, false).map_err(|e| e.to_string())?.ok_or("that key no longer holds the secret it was added with")?;
        lockbox::unlock_card(&pubkey, &secret)?;
        let m = lockbox::migrate()?; attach::clear_opened();
        lock_state(Some(m))
    }).await
}

#[tauri::command]
fn lock_now() -> R<LockStatus> { lockbox::lock(); lock_state(None) }

/// Let the plugged-in key open this app. Its secret is made if the key has none yet (the same one a seal will use).
#[tauri::command]
async fn lock_add_card(reader: String, pin: String) -> R<LockStatus> {
    block(move || {
        lockbox::key()?;
        let card = cards::list().map_err(|e| e.to_string())?.into_iter().find(|c| c.reader == reader).ok_or("that key is no longer plugged in")?;
        let pubkey = card.pubkey.clone().ok_or("that key has no signing key on it yet")?;
        let secret = cards::seal_key(&reader, &pin, true).map_err(|e| e.to_string())?.ok_or("the key would not keep a secret")?;
        lockbox::add_card(&pubkey, &card_name(&pubkey, card.serial), &secret, now())?;
        lock_state(None)
    }).await
}

#[tauri::command]
fn lock_remove_card(pubkey: String) -> R<LockStatus> {
    lockbox::key()?;
    let mut s = lockbox::slots()?.ok_or("nothing is sealed yet")?;
    s.cards.retain(|c| c.pubkey != pubkey);
    lockbox::save_slots(&s)?;
    lock_state(None)
}

/// Touch ID on or off, and the idle lock. Turning the prompt OFF is itself something only the person at the Mac may do.
#[tauri::command]
async fn lock_settings(touch: Option<bool>, idle_minutes: Option<u32>, with_screen: Option<bool>) -> R<LockStatus> {
    block(move || {
        lockbox::key()?;
        let mut s = lockbox::slots()?.ok_or("nothing is sealed yet")?;
        if let Some(t) = touch { if s.touch && !t { lockbox::mac::presence("stop asking for Touch ID when Any Two Keys opens")?; } s.touch = t; }
        if let Some(m) = idle_minutes { s.idle_minutes = m.min(24 * 60); }
        if let Some(w) = with_screen { s.with_screen = w; }
        lockbox::save_slots(&s)?;
        lock_state(None)
    }).await
}

/// What a key is called here: its name in any vault that has it, else its serial.
fn card_name(pubkey: &str, serial: u32) -> String {
    wallet::load().ok().and_then(|s| s.vaults.iter().flat_map(|v| v.keys.iter()).find(|k| k.pubkey == pubkey).map(|k| k.name.clone())).unwrap_or_else(|| format!("key {serial}"))
}
/// A key's secret has just been read for another reason (sealing, restoring): let that key open this app too.
fn learn_card(pubkey: &str, serial: u32, secret: &[u8; 32]) {
    if !lockbox::sealed_mode() || !lockbox::unlocked() { return; }
    if lockbox::slots().ok().flatten().map(|s| s.cards.iter().any(|c| c.pubkey == pubkey && lockbox::unwrap_from_card(&s.id, c, secret).is_ok())).unwrap_or(true) { return; }
    let _ = lockbox::add_card(pubkey, &card_name(pubkey, serial), secret, now());
}

/// Open the locked shares on one key with its PIN. A wrong PIN costs one of the key's tries, as it would when signing.
#[tauri::command]
async fn seal_unlock(reader: String, pin: String) -> R<Vec<Found>> {
    block(move || {
        let cards = seal_cards()?;
        let card = cards.iter().find(|c| c.reader == reader).ok_or("that key is no longer plugged in")?;
        let entries = seal::parse_container(&card.container)?;
        let key = cards::seal_key(&reader, &pin, false).map_err(|e| e.to_string())?.ok_or("this key holds no secret for a seal: its shares were not locked by this app")?;
        if let Some(pk) = &card.pubkey { learn_card(pk, card.serial, &key); }
        entries.into_iter().filter(|e| e.locked).map(|e| {
            let plain = seal::unlock_share(&key, &e.id, e.k, e.x, &hex::decode(&e.share).map_err(|e| e.to_string())?)?;
            Ok(Found { pubkey: card.pubkey.clone().unwrap_or_default(), serial: card.serial, entry: seal::Entry { share: hex::encode(plain), locked: false, ..e } })
        }).collect()
    }).await
}

/// A vault this computer already has (restored from paper, say) learns its shares from its own keys, once they are open.
#[tauri::command]
fn seal_adopt(name: String, found: Vec<Found>) -> R<Store> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
    let id = seal_id(&v);
    let mine: Vec<Found> = found.into_iter().filter(|f| f.entry.id == id && !f.entry.locked).collect();
    let (rec, shares) = seal_combine(&mine.iter().map(|f| &f.entry).collect::<Vec<_>>()).ok_or("not enough open shares yet")?;
    seal_check(&rec, &mine)?;
    if rec.pubkeys != seal_record(&v).pubkeys { return Err("those shares describe a different vault".into()); }
    let sealed: Vec<String> = mine.iter().map(|f| f.pubkey.clone()).collect();
    wallet::update(move |s| if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == name) { vm.seal.shares = shares; for p in sealed { if !vm.seal.sealed.contains(&p) { vm.seal.sealed.push(p); } } })
}

#[tauri::command]
fn seal_restore(id: String, name: String, found: Vec<Found>) -> R<Vault> {
    let mine: Vec<Found> = found.into_iter().filter(|f| f.entry.id == id && !f.entry.locked).collect();
    let (rec, shares) = seal_combine(&mine.iter().map(|f| &f.entry).collect::<Vec<_>>()).ok_or("not enough keys yet to rebuild this vault")?;
    seal_check(&rec, &mine)?;
    if seal::vault_id(&rec.network, rec.threshold, &rec.pubkeys, rec.chain_code.as_deref()) != id { return Err("the record does not match the vault it was filed under".into()); }
    let name = if name.trim().is_empty() { rec.name.clone() } else { name.trim().to_string() };
    let sealed = mine.iter().filter(|f| shares.get(f.entry.x as usize - 1) == Some(&f.entry.share)).map(|f| f.pubkey.clone()).collect();
    let mut v = seal_vault(&rec, name.clone(), shares, sealed)?;
    for f in &mine { if let Some(k) = v.keys.iter_mut().find(|k| k.pubkey == f.pubkey) { k.serial = f.serial; } }
    let v2 = v.clone();
    let mut clash = false;
    wallet::update(|s| { if s.vaults.iter().any(|x| x.name == name) { clash = true; } else { s.settings.network = v2.network.clone(); s.vaults.push(v2); } })?;
    if clash { return Err(format!("a vault called '{name}' already exists")); }
    Ok(v)
}

// ---------- labels (BIP 329) ----------
#[tauri::command]
async fn label_set(name: String, key: String, label: String) -> R<Store> {
    if !(key.starts_with("addr:") || key.starts_with("tx:") || key.starts_with("out:")) { return Err("label key must be addr:, tx: or out:".into()); }
    let nm = name.clone();
    wallet::update(|s| if let Some(v) = s.vaults.iter_mut().find(|v| v.name == nm) {
        let label = label.trim().to_string();
        v.label_meta.insert(key.clone(), now());
        if label.is_empty() { v.labels.remove(&key); } else { v.labels.insert(key, label); }
    })?;
    let _ = labels_sync(name).await;   // best effort; the status line on Home reports a host that could not be reached
    Ok(wallet::load()?)
}

/// The long note on a transaction, an address or a coin. It rides in the same synced record as the label.
#[tauri::command]
async fn note_set(name: String, key: String, note: String) -> R<Store> {
    if !(key.starts_with("addr:") || key.starts_with("tx:") || key.starts_with("out:")) { return Err("note key must be addr:, tx: or out:".into()); }
    if note.chars().count() > 20_000 { return Err("a note holds up to 20,000 characters".into()); }
    let nm = name.clone();
    wallet::update(|s| if let Some(v) = s.vaults.iter_mut().find(|v| v.name == nm) {
        let note = note.trim_end().to_string();
        v.label_meta.insert(key.clone(), now());
        if note.trim().is_empty() { v.notes.remove(&key); } else { v.notes.insert(key, note); }
    })?;
    let _ = labels_sync(name).await;
    Ok(wallet::load()?)
}

// ---------- attachments: files kept with a record (attach.rs) ----------
#[derive(Serialize)]
struct Attached { store: Store, added: usize, notes: Vec<String>, errors: Vec<String> }

fn record_key_ok(key: &str) -> R<()> { if key.starts_with("addr:") || key.starts_with("tx:") || key.starts_with("out:") { Ok(()) } else { Err("the record key must be addr:, tx: or out:".into()) } }

async fn attach_into(name: String, key: String, items: Vec<(String, Vec<u8>)>, keep_original: bool, mut errors: Vec<String>) -> R<Attached> {
    record_key_ok(&key)?;
    let (mut entries, mut notes) = (Vec::new(), Vec::new());
    for (fname, bytes) in items {
        match attach::add(&fname, &bytes, keep_original, now()) { Ok(a) => { if let Some(n) = a.note { notes.push(n); } entries.push(a.entry); } Err(e) => errors.push(e) }
    }
    let added = entries.len();
    if added > 0 {
        let nm = name.clone();
        wallet::update(move |s| if let Some(v) = s.vaults.iter_mut().find(|v| v.name == nm) {
            let list = v.files.entry(key.clone()).or_default();
            for e in entries { if !list.iter().any(|x| x.get("sha256") == e.get("sha256")) { list.push(e); } }   // the same file twice is one file
            v.label_meta.insert(key, now());
        })?;
        // the record and then the bytes travel in the background: a 25 MB receipt must not hold the screen while it goes up
        tauri::async_runtime::spawn(async move { let _ = labels_sync(name).await; });
    }
    Ok(Attached { store: wallet::load()?, added, notes, errors })
}

fn read_capped(path: &std::path::Path) -> Result<(String, Vec<u8>), String> {
    let shown = attach::safe_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let meta = std::fs::metadata(path).map_err(|e| format!("{shown}: {e}"))?;
    if !meta.is_file() { return Err(format!("{shown} is a folder; drop the files inside it")); }
    if meta.len() > attach::MAX_FILE { return Err(format!("{shown} is larger than 25 MB")); }
    Ok((shown.clone(), std::fs::read(path).map_err(|e| format!("{shown}: {e}"))?))
}

/// Files dropped on the window arrive as PATHS (Tauri reads the drop, the webview never sees the bytes).
#[tauri::command]
async fn attach_paths(name: String, key: String, paths: Vec<String>, keep_original: bool) -> R<Attached> {
    let (mut items, mut errors) = (Vec::new(), Vec::new());
    for p in paths.iter().take(40) { match read_capped(std::path::Path::new(p)) { Ok(i) => items.push(i), Err(e) => errors.push(e) } }
    attach_into(name, key, items, keep_original, errors).await
}

#[tauri::command]
async fn attach_pick(name: String, key: String, keep_original: bool) -> R<Attached> {
    let picked = rfd::FileDialog::new().set_title("Add files to this record").pick_files().ok_or("cancelled")?;
    attach_paths(name, key, picked.into_iter().map(|p| p.to_string_lossy().into_owned()).collect(), keep_original).await
}

/// A picture pasted from the clipboard comes through the window as base64.
#[tauri::command]
async fn attach_bytes(name: String, key: String, filename: String, base64: String, keep_original: bool) -> R<Attached> {
    use bitcoin::base64::Engine;
    let bytes = bitcoin::base64::engine::general_purpose::STANDARD.decode(base64.trim()).map_err(|_| "that paste could not be read".to_string())?;
    attach_into(name, key, vec![(filename, bytes)], keep_original, vec![]).await
}

#[tauri::command]
async fn attach_remove(name: String, key: String, sha256: String) -> R<Store> {
    record_key_ok(&key)?;
    let nm = name.clone();
    let mut gone: Vec<String> = Vec::new();
    let st = wallet::update(|s| if let Some(v) = s.vaults.iter_mut().find(|v| v.name == nm) {
        if let Some(list) = v.files.get_mut(&key) {
            for e in list.iter().filter(|e| e.get("sha256").and_then(|x| x.as_str()) == Some(&sha256)) { for f in ["sha256", "thumb"] { if let Some(h) = e.get(f).and_then(|x| x.as_str()) { gone.push(h.to_string()); } } }
            list.retain(|e| e.get("sha256").and_then(|x| x.as_str()) != Some(&sha256));
            if list.is_empty() { v.files.remove(&key); }
            v.label_meta.insert(key.clone(), now());
        }
    })?;
    // bytes go only when no record of ANY vault on this computer still names them
    let in_use: std::collections::HashSet<String> = st.vaults.iter().flat_map(|v| v.files.values().flatten()).flat_map(|e| ["sha256", "thumb"].into_iter().filter_map(|f| e.get(f).and_then(|x| x.as_str()).map(String::from)).collect::<Vec<_>>()).collect();
    attach::sweep(&gone, &in_use);
    // off the host too, once no record of THIS vault names them (the record itself travels with the sync below)
    if let Some(v) = st.vaults.iter().find(|v| v.name == name) {
        let mine: std::collections::HashSet<&str> = v.files.values().flatten().flat_map(|e| ["sha256", "thumb"].into_iter().filter_map(|f| e.get(f).and_then(|x| x.as_str()))).collect();
        if let (false, Some(root)) = (st.settings.vss.is_empty(), labels::root_from_shares(&v.seal.shares)) {
            let keys = labels::keys(&root);
            for h in gone.iter().filter(|h| !mine.contains(h.as_str())) { let _ = labels::att_delete(&st.settings.vss, &keys, h).await; }
        }
    }
    let _ = labels_sync(name).await;
    Ok(wallet::load()?)
}

#[tauri::command]
fn attach_open(sha256: String, name: String, mime: String) -> R<String> { attach::open(&sha256, &name, &mime).map(String::from) }

/// Which of these are on this computer (a record can arrive from another computer before its bytes do).
#[tauri::command]
fn attach_have(hashes: Vec<String>) -> Vec<bool> { hashes.iter().map(|h| attach::have(h)).collect() }

#[tauri::command]
fn attach_thumb(sha256: String) -> R<String> {
    use bitcoin::base64::Engine;
    Ok(format!("data:image/jpeg;base64,{}", bitcoin::base64::engine::general_purpose::STANDARD.encode(attach::small(&sha256)?)))
}

#[derive(Serialize, Clone)]
struct LabelsStatus { backed_up: bool, host: String, count: usize, synced: u64, note: String }

/// Pull, merge and push this vault's labels with the label host (labels.rs). Cheap when nothing changed.
#[tauri::command]
async fn labels_sync(name: String) -> R<LabelsStatus> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
    let host = s.settings.vss.clone();
    let count = v.labels.len();
    if host.is_empty() { return Ok(LabelsStatus { backed_up: false, host, count, synced: v.labels_synced, note: "Labels stay on this computer: no label host is set (App settings).".into() }); }
    let root = match labels::root_from_shares(&v.seal.shares) { Some(r) => r, None => return Ok(LabelsStatus { backed_up: false, host, count, synced: 0, note: "Seal this vault's keys to back its labels up: they are encrypted so that any two keys bring them back with the vault.".into() }) };
    let keys = labels::keys(&root);
    let local = labels::Envelope::from_vault(&v.labels, &v.label_meta, &v.notes, &v.files, &v.more);
    match labels::sync(&host, &keys, &local).await {
        Ok((merged, _wrote)) => {
            let t = now();
            wallet::update(|s| if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == name) {
                for (k, r) in &merged.records {
                    let mine_t = *vm.label_meta.get(k).unwrap_or(&0);
                    if r.t >= mine_t {
                        vm.label_meta.insert(k.clone(), r.t);
                        if r.label.is_empty() { vm.labels.remove(k); } else { vm.labels.insert(k.clone(), r.label.clone()); }
                        if r.note.is_empty() { vm.notes.remove(k); } else { vm.notes.insert(k.clone(), r.note.clone()); }
                        if r.files.is_empty() { vm.files.remove(k); } else { vm.files.insert(k.clone(), r.files.clone()); }
                        if r.more.is_empty() { vm.more.remove(k); } else { vm.more.insert(k.clone(), r.more.clone()); }
                    }
                }
                vm.labels_synced = t;
            })?;
            let n = merged.records.values().filter(|r| !r.label.is_empty()).count();
            // the files the records name: whatever this computer holds and the host lacks goes up, within the vault's allowance
            let note = match attachments_up(&host, &keys, &merged).await { Ok(n) => n, Err(e) => format!("Labels are backed up; attachments are not yet: {e}") };
            Ok(LabelsStatus { backed_up: true, host, count: n, synced: t, note })
        }
        Err(e) => Ok(LabelsStatus { backed_up: false, host, count, synced: v.labels_synced, note: e }),
    }
}

/// How much of a vault's attachments the label host will be asked to hold.
const ATT_QUOTA: u64 = 250 * 1024 * 1024;

fn att_entries(env: &labels::Envelope) -> Vec<(String, u64)> {
    let mut seen = std::collections::HashSet::new(); let mut out = Vec::new();
    for e in env.records.values().flat_map(|r| r.files.iter()) {
        let size = e.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
        for (f, sz) in [("sha256", size), ("thumb", 1)] { if let Some(h) = e.get(f).and_then(|x| x.as_str()) { if seen.insert(h.to_string()) { out.push((h.to_string(), sz)); } } }
    }
    out
}

async fn attachments_up(host: &str, keys: &labels::Keys, env: &labels::Envelope) -> Result<String, String> {
    let entries = att_entries(env);
    if entries.is_empty() { return Ok(String::new()); }
    let present = labels::att_present(host, keys).await?;
    let (mut total, mut skipped) = (0u64, 0usize);
    for (hash, size) in &entries {
        total += size;
        if total > ATT_QUOTA { skipped += 1; continue; }
        let h = hash.clone();
        let Ok(bytes) = block(move || attach::read(&h)).await else { continue };   // added on another computer and not fetched here: not ours to send
        labels::att_upload(host, keys, hash, &bytes, &present).await?;
    }
    Ok(if skipped > 0 { format!("{skipped} attachment{} stayed on this computer only: this vault's backup allowance is 250 MB.", if skipped == 1 { "" } else { "s" }) } else { String::new() })
}

/// Bring a file that was added on another computer here: fetched from the label host, opened with the vault's keys,
/// and kept only if it hashes to what the record names.
#[tauri::command]
async fn attach_fetch(name: String, sha256: String, size: u64) -> R<bool> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    if !v.files.values().flatten().any(|e| ["sha256", "thumb"].iter().any(|f| e.get(*f).and_then(|x| x.as_str()) == Some(sha256.as_str()))) { return Err("no record of this vault names that file".into()); }
    if attach::have(&sha256) { return Ok(true); }
    if s.settings.vss.is_empty() { return Err("no label host is set, so there is nowhere to fetch it from".into()); }
    let root = labels::root_from_shares(&v.seal.shares).ok_or("this vault is not sealed, so it has no backup to fetch from")?;
    let bytes = labels::att_download(&s.settings.vss, &labels::keys(&root), &sha256, size.clamp(1, attach::MAX_FILE) as usize).await?;
    block(move || attach::keep(&sha256, &bytes)).await?;
    Ok(true)
}

#[tauri::command]
fn labels_export(name: String) -> R<String> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let mut lines: Vec<String> = Vec::new();
    let mut keys: Vec<&String> = v.labels.keys().collect(); keys.sort();
    for k in keys {
        let (t, r) = k.split_once(':').unwrap_or(("", ""));
        let t = match t { "addr" => "addr", "tx" => "tx", "out" => "output", _ => continue };
        lines.push(serde_json::json!({ "type": t, "ref": r, "label": v.labels[k] }).to_string());
    }
    if lines.is_empty() { return Err("this vault has no labels yet".into()); }
    let path = rfd::FileDialog::new().set_title("Save labels (BIP 329)").set_file_name(format!("{}-labels.jsonl", v.name.replace(' ', "-"))).add_filter("BIP 329 labels", &["jsonl", "json"]).save_file().ok_or("cancelled")?;
    std::fs::write(&path, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
async fn labels_import(name: String) -> R<usize> {
    let path = rfd::FileDialog::new().set_title("Open a labels file (BIP 329)").add_filter("BIP 329 labels", &["jsonl", "json", "txt"]).pick_file().ok_or("cancelled")?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut got: Vec<(String, String)> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let j: serde_json::Value = match serde_json::from_str(line) { Ok(j) => j, Err(_) => continue };
        let (t, r, lab) = (j["type"].as_str().unwrap_or(""), j["ref"].as_str().unwrap_or(""), j["label"].as_str().unwrap_or(""));
        let key = match t { "addr" => format!("addr:{r}"), "tx" => format!("tx:{r}"), "output" => format!("out:{r}"), _ => continue };
        if !r.is_empty() && !lab.trim().is_empty() { got.push((key, lab.trim().to_string())); }
    }
    let n = got.len();
    if n == 0 { return Err("no addr, tx or output labels in that file".into()); }
    let nm = name.clone();
    wallet::update(|s| if let Some(v) = s.vaults.iter_mut().find(|v| v.name == nm) { let t = now(); for (k, l) in got { v.label_meta.insert(k.clone(), t); v.labels.insert(k, l); } })?;
    let _ = labels_sync(name).await;
    Ok(n)
}

// ---------- BSMS (BIP 129) ----------
#[tauri::command]
fn bsms_export(name: String) -> R<String> {
    let s = wallet::load()?;
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let text = bsms_record(v)?;
    let path = rfd::FileDialog::new().set_title("Save the vault as a BSMS descriptor record").set_file_name(format!("{}.bsms", v.name.replace(' ', "-"))).add_filter("BSMS", &["bsms", "txt"]).save_file().ok_or("cancelled")?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[derive(Debug, Serialize)]
struct BsmsIn { descriptor: String, first_address: String }

/// A BSMS 1.0 descriptor record (BIP 129): version, descriptor, path restrictions, first address. For a vault of
/// extended keys the descriptor is the TEMPLATE — each key ends `/**` — with the restriction line `/0/*,/1/*` and the
/// first address of the first restricted path; "No path restrictions" is only for a descriptor that has none.
pub fn bsms_record(v: &Vault) -> R<String> {
    let first = wallet::address_at(v, 0)?;
    match &v.ext {
        Some(ext) => {
            let body = ext.split('#').next().unwrap_or(ext);
            if !body.contains("/<0;1>/*") { return Err("this vault's descriptor is not written with <0;1>/*, so it cannot be put in a BSMS template".into()); }
            Ok(format!("BSMS 1.0\n{}\n/0/*,/1/*\n{first}\n", body.replace("/<0;1>/*", "/**")))
        }
        None => Ok(format!("BSMS 1.0\n{}\nNo path restrictions\n{first}\n", v.descriptor)),
    }
}

/// Read a BSMS record into the text the Restore screen reads — and CHECK its fourth line: the first address is there
/// so both sides can confirm they derived the same wallet, and a record whose address this app does not derive is
/// refused rather than restored into a vault that watches the wrong addresses.
pub fn bsms_read(text: &str, network: &str) -> R<BsmsIn> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 || !lines[0].starts_with("BSMS") { return Err("not a BSMS descriptor record (first line should be 'BSMS 1.0')".into()); }
    let (desc, restrictions, claimed) = (lines[1], lines.get(2).copied().unwrap_or(""), lines.get(3).copied().unwrap_or(""));
    if names_extended_keys(desc) {
        let restricted = restrictions.replace(' ', "");
        if !(restricted == "/0/*,/1/*" || restrictions.eq_ignore_ascii_case("No path restrictions")) {
            return Err(format!("this record restricts its keys to {restrictions:?}; this app reads wallets of receive and change, /0/*,/1/*"));
        }
        let descriptor = desc.split('#').next().unwrap_or(desc).replace("/**", "/<0;1>/*");
        let info = ext_for_this_network(&descriptor, network)?;
        let mut v = ext_vault(String::new(), &info, &[]);
        let derived = wallet::address_at(&v, 0)?; v.address = derived.clone();
        if !claimed.is_empty() && claimed != derived { return Err(format!("the record says its first address is {claimed}, but these keys give {derived}: the record and its keys do not agree, so it was not read")); }
        return Ok(BsmsIn { descriptor: info.descriptor, first_address: derived });
    }
    let desc = desc.replace("/**", "");
    let (threshold, keys) = wallet::keys_of(&desc)?;
    let descriptor = wallet::descriptor_for(threshold, &keys)?;
    Ok(BsmsIn { descriptor, first_address: claimed.to_string() })
}

/// Read a BSMS 1.0 descriptor record from a file.
#[tauri::command]
fn bsms_import() -> R<BsmsIn> {
    let path = rfd::FileDialog::new().set_title("Open a BSMS descriptor record").add_filter("BSMS", &["bsms", "txt"]).pick_file().ok_or("cancelled")?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let s = wallet::load()?;
    bsms_read(&text, &s.settings.network)
}

/// The same wallet, written the way another wallet writes it. A vault's descriptor is kept exactly as it was pasted, and
/// one pasted before that was true is stored in the canonical form (48' rather than 48h — and so a different checksum
/// from the one Sparrow or the SeedSigner shows). This replaces the TEXT only, and only when it is the same wallet.
#[tauri::command]
fn vault_ext_rewrite(name: String, text: String) -> R<Store> {
    let mut s = wallet::load()?;
    let v = s.vaults.iter_mut().find(|v| v.name == name).ok_or("no such vault")?;
    let stored = v.ext.clone().ok_or("this vault's keys are cards; its recovery line is its own")?;
    let (old, new) = (wallet::ext_parse(&stored)?.ok_or("stored descriptor")?, wallet::ext_parse(&text)?.ok_or("that names no extended keys")?);
    if old.canonical != new.canonical { return Err("that is a different wallet from this vault's, so it was not put in its place".into()); }
    if !text.trim().contains('#') { return Err("paste it with its checksum (the #… at the end), exactly as the other wallet shows it".into()); }
    v.ext = Some(new.descriptor.clone()); v.descriptor = new.descriptor;
    wallet::save(&s)?;
    Ok(s)
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}


// ------------------------------------------------------------------ watcher --
mod watch {
    use crate::{chain, wallet};
    use tauri::{AppHandle, Emitter};
    use tauri_plugin_notification::NotificationExt;

    /// `kind`: "in" | "out" | "tx" (no net change) | "confirmed" — the window draws each differently
    #[derive(Clone, serde::Serialize)]
    pub struct Event { pub vault: String, pub kind: &'static str, pub title: String, pub body: String, pub txid: String }

    /// One pass over every vault: what changed since last time, as notifications.
    pub fn log(line: &str) {
        use std::io::Write;
        let p = wallet::store_path().with_file_name("watch.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
            let _ = writeln!(f, "{} {line}", chrono_now());
            if f.metadata().map(|m| m.len() > 2_000_000).unwrap_or(false) { let _ = std::fs::write(&p, ""); }
        }
    }
    fn chrono_now() -> String {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let (h, m, sec) = (((t + 10 * 3600) / 3600) % 24, (t / 60) % 60, t % 60); // Brisbane, UTC+10, no DST
        format!("{h:02}:{m:02}:{sec:02}")
    }

    pub fn pass() -> Vec<Event> {
        // a store that cannot be read (damaged, or locked) means no pass at all: nothing is compared, nothing is saved
        let s0 = match wallet::load() { Ok(s) => s, Err(e) => { if e != crate::lockbox::LOCKED { log(&format!("pass skipped: {e}")); } return Vec::new() } };
        let mut out = Vec::new();
        let vaults = s0.vaults.clone();
        for (n, v) in vaults.iter().enumerate() {
            let tag = format!("vault {}", n + 1);   // the log is readable by anyone who can read the disk: no names, amounts or ids in it
            let servers = wallet::servers_for(&s0, &v.network);
            let sc = match wallet::scan(v, &servers, s0.settings.gap) { Ok(sc) => sc, Err(e) => { log(&format!("{tag}: scan failed: {e}")); continue } };
            // re-read after the slow part, so a sync that saved meanwhile is not erased
            let mut s = match wallet::load() { Ok(s) => s, Err(e) => { log(&format!("pass stopped: {e}")); return out } };
            let seen = s.seen.entry(v.name.clone()).or_default();
            let now_txs: std::collections::HashMap<String, i32> = sc.history.iter().map(|(t, h, _)| (t.clone(), *h)).collect();
            let now_coins: std::collections::HashMap<String, u64> = sc.coins.iter().map(|c| (format!("{}:{}", c.txid, c.vout), c.value)).collect();
            // a confirmed transaction does not vanish; a scan that lost one came from a server that answered badly
            if let Some(gone) = seen.txs.iter().find(|(t, h)| **h > 0 && !now_txs.contains_key(*t)) {
                let _ = gone; log(&format!("{tag}: scan lost a confirmed transaction; ignoring this pass"));
                continue;
            }
            let before = out.len();
            if seen.primed {
                for (txid, h) in &now_txs {
                    let gained: u64 = now_coins.iter().filter(|(k, _)| k.starts_with(txid.as_str()) && !seen.coins.contains_key(*k)).map(|(_, v)| v).sum();
                    match seen.txs.get(txid) {
                        None => {
                            let lost: u64 = seen.coins.iter().filter(|(k, _)| !now_coins.contains_key(*k)).map(|(_, v)| v).sum();
                            let net = gained as i64 - lost as i64;
                            let short = format!("{}…{}", &txid[..8], &txid[txid.len() - 6..]);
                            if net == 0 {
                                out.push(Event { vault: v.name.clone(), kind: "tx", txid: txid.clone(), title: format!("{}: new transaction", v.name), body: format!("{short} · {}", if *h > 0 { "confirmed" } else { "in the mempool" }) });
                            } else if net > 0 {
                                out.push(Event { vault: v.name.clone(), kind: "in", txid: txid.clone(), title: format!("{}: {} sats arrived", v.name, fmt(net as u64)), body: format!("{short} · {}", if *h > 0 { "confirmed" } else { "waiting for a confirmation" }) });
                            } else {
                                out.push(Event { vault: v.name.clone(), kind: "out", txid: txid.clone(), title: format!("{}: {} sats sent", v.name, fmt((-net) as u64)), body: format!("{short} · {}", if *h > 0 { "confirmed" } else { "in the mempool" }) });
                            }
                        }
                        Some(prev) if *prev <= 0 && *h > 0 => {
                            out.push(Event { vault: v.name.clone(), kind: "confirmed", txid: txid.clone(), title: format!("{}: confirmed", v.name), body: format!("{}…{} is in block {}", &txid[..8], &txid[txid.len() - 6..], fmt(*h as u64)) });
                        }
                        _ => {}
                    }
                }
            }
            let primed_before = seen.primed;
            let seen_now = wallet::Seen { txs: now_txs, coins: now_coins, primed: true };
            let (n, sm, nc, smc) = (sc.next, sc.scan_max, sc.next_change, sc.scan_max_change);
            let name = v.name.clone();
            let fresh: Vec<String> = out[before..].iter().filter(|e| e.kind != "confirmed").map(|e| e.txid.clone()).collect();
            let r = wallet::update(move |s| { s.seen.insert(name.clone(), seen_now);
                // a transaction first noticed from here: say so, and where "here" was, unless it was made here (then it already says)
                let (t, fix, exact) = (super::now(), if s.settings.location { crate::location::fresh(super::now(), 1800) } else { None }, s.settings.location_exact);
                if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == name) { for txid in &fresh { crate::context::stamp(vm, txid, crate::context::build("seen", fix, exact, t), t); } } if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == name) { vm.next = n; vm.scan_max = sm; vm.next_change = nc; vm.scan_max_change = smc; } });
            log(&format!("{tag}: {} coins, {} txs, primed_before={primed_before}, events={}, save={}", sc.coins.len(), sc.history.len(), out.len() - before, r.as_ref().map(|_| "ok").unwrap_or("FAILED")));
        }
        out
    }

    fn fmt(n: u64) -> String {
        let s = n.to_string();
        let mut out = String::new();
        for (i, ch) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { out.push(','); } out.push(ch); }
        out
    }

    pub fn start(app: AppHandle) {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(8));
            loop {
                if crate::net::offline() { std::thread::sleep(std::time::Duration::from_secs(20)); continue; }   // nothing goes out while offline, least of all on a timer
                let events = pass();
                for e in &events {
                    let r = app.notification().builder().title(&e.title).body(&e.body).show();
                    log(&format!("notify ({}): {}", e.kind, match &r { Ok(_) => "sent".to_string(), Err(e) => format!("FAILED {e}") }));
                    let _ = app.emit("vault-changed", &e.vault);
                    let _ = app.emit("vault-event", e);   // the window shows its own notice too: a Focus can silence the system one
                }
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            attach::clear_opened();
            net::attach(app.handle().clone());
            if let Ok(s) = wallet::load() { net::seed(s.settings.offline); }   // a store that cannot be read yet is seeded by the window's first store_get
            watch::start(app.handle().clone());
            // when the Mac's screen locks, so does this: the key leaves memory and the window is told to forget what it knew
            let h = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                if lockbox::mac::screen_locked() && lockbox::sealed_mode() && std::env::var_os("A2K_STORE_KEY").is_none() && lockbox::unlocked() && lockbox::slots().ok().flatten().map(|s| s.with_screen).unwrap_or(true) {
                    lockbox::lock(); use tauri::Emitter; let _ = h.emit("app-locked", ());
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![device_key_read, device_key_file, vault_create_signers, net_offline, drill_every, cost_method_set, vault_cost_method, vault_ext_rewrite, cards_other, prices_series, location_status, location_preview, location_now, place_search, ctx_set, tx_map, tx_pdf, lock_status, lock_start, lock_open_mac, lock_open_card, lock_now, lock_add_card, lock_remove_card, lock_settings, open_url, vault_activity, tx_detail, node_status, device_locale, contact_save, contact_delete, contact_used, vault_receive_address, vault_key_seen, key_form_seen, vault_pdf, vault_shares_pdf, record_pdf, address_flow, report_pdf, notify_test, spend_ceremony, spend_qr, spend_pdf, spend_import, seal_status, seal_write, seal_read, seal_unlock, seal_adopt, seal_preview, seal_restore, label_set, note_set, attach_paths, attach_pick, attach_bytes, attach_remove, attach_open, attach_have, attach_thumb, attach_fetch, labels_sync, labels_export, labels_import, bsms_export, bsms_import, vault_addresses, message_sign_key, message_stage, message_finish, message_verify, 
            cards_list, card_generate, store_get, vault_create, descriptor_parse, vault_restore, vault_recovery, vault_rename, vault_forget, spend_preview, disposal_set, muun_restore, muun_check, spark_restore, spark_restore_words, seed_wordlist, seed_check, xpub_restore,
            settings_set, vault_sync, fees_get, servers_test, spend_build, spend_sign, spend_status, spend_broadcast, spend_export, spend_discard
        ])
        .build(tauri::generate_context!())
        .expect("error while running Any Two Keys")
        .run(|_, event| { if let tauri::RunEvent::Exit = event { lockbox::lock(); } });   // readable copies handed to a viewer go when the app does
}

#[cfg(test)]
mod sheet_tests {
    use super::*;

    const KEY: &str = concat!("tx:", "abababababababababababababababababababababababababababababababab");

    fn vault() -> wallet::Vault {
        let mut v: wallet::Vault = serde_json::from_value(serde_json::json!({
            "name": "mineracks vault", "network": "mainnet", "threshold": 2, "descriptor": "wsh(multi(2,…))", "address": "bc1q…", "created": 1_789_000_000,
            "keys": [{ "name": "laptop", "serial": 1, "pubkey": "02aa" }, { "name": "keyring", "serial": 2, "pubkey": "02bb" }, { "name": "off-site", "serial": 3, "pubkey": "02cc" }],
        })).unwrap();
        v.labels.insert(KEY.into(), "Lets check if labelling still works".into());
        v.notes.insert(KEY.into(), "Ordered 17 Sep, invoice 4471.\nAsk Sam about the warranty.".into());
        v.files.insert(KEY.into(), vec![
            serde_json::json!({ "name": "invoice-4471.pdf", "mime": "application/pdf", "size": 217_344, "sha256": "ab".repeat(32), "t": 1 }),
        ]);
        v.more.insert(KEY.into(), serde_json::from_value(serde_json::json!({ "ctx": {
            "t": 1_789_600_000u64, "how": "made", "place": "445 Upper Edward Street, Spring Hill, Brisbane, Queensland 4000, Australia",
            "geo": { "lat": -27.462, "lon": 153.025, "acc": 110 },
            "device": { "name": "Piers’s MacBook PRO", "model": "MacBookPro17,1", "os": "macOS 27.0", "app": "0.1.0" } } })).unwrap());
        v
    }

    fn spend() -> activity::Detail {
        serde_json::from_value(serde_json::json!({
            "txid": "ab".repeat(32), "wtxid": "cd".repeat(32), "version": 2, "locktime": 0, "size": 380, "vsize": 189, "weight": 756, "rbf": true,
            "net": -5_190i64, "fee": 190u64, "fee_rate": 1.0053, "height": 967_531, "tip": 967_600, "block": "0000000000000000000112ab", "time": 1_789_600_120u32,
            "proof": "proved", "position": 1_757,
            "inputs": [{ "outpoint": "ee:1", "address": "bc1qvault", "sats": 20_000u64, "ours": 0, "sequence": 4_294_967_293u32, "witness": 4, "signers": ["laptop", "off-site"] }],
            "outputs": [{ "n": 0, "address": "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4", "sats": 5_000u64, "kind": "P2WPKH", "ours": null },
                        { "n": 1, "address": "bc1qvaultchange", "sats": 14_810u64, "kind": "P2WSH", "ours": 2_147_483_648u32 }],
            "hex": "0200000000010".repeat(60)
        })).unwrap()
    }

    /// The date on a printed sheet, against what Python's own UTC formatting gives for the same instants.
    #[test]
    fn the_date_on_a_sheet_is_the_right_one() {
        for (t, want) in [(1_789_600_120u64, "Wed 16 Sep 2026, 23:08"), (1_758_286_000, "Fri 19 Sep 2025, 12:46"), (0, "Thu 1 Jan 1970, 00:00"), (951_782_400, "Tue 29 Feb 2000, 00:00")] {
            assert_eq!(chrono_ish(t), want);
        }
    }

    /// The sheet a person will actually print, built without a network and read back by pypdf in `test/pdf-check.py`.
    #[test]
    fn a_transaction_sheet_says_what_the_screen_says() {
        let (v, d) = (vault(), spend());
        let contacts = vec![wallet::Contact { name: "hardware shop".into(), address: "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".into(), network: "mainnet".into(), added: 1, used: 2, last_used: 1 }];
        let q = Some(activity::Quote { price: 164_500.0, basis: "hourly 2026-09-18 08:00 UTC".into() });
        let (title, subtitle, blocks) = tx_blocks(&v, &d, &contacts, "AUD", &q, &q, false);
        assert_eq!(title, "Lets check if labelling still works");
        assert!(subtitle.starts_with("mineracks vault · printed 20"));
        let flat = format!("{:?}", blocks.iter().map(|b| match b {
            export::Block::Facts(kv) => kv.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("|"),
            export::Block::Heading(h) => format!("#{h}"), export::Block::Para(p) => p.clone(), export::Block::Mono(m) => format!("hex:{}", m.len()),
            export::Block::Jpeg { caption, .. } => format!("img:{caption}"), export::Block::Rule => "---".into(), export::Block::Chart { title, .. } => format!("chart:{title}") }).collect::<Vec<_>>());
        for want in [
            "Amount=−5,190 sats", "Worth then=−8.54 AUD  (hourly 2026-09-18 08:00 UTC)",   // 5,190 sats at 164,500/BTC
            "To=bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4  (hardware shop)",        // the address book names it
            "Change=bc1qvaultchange  (mineracks vault · change #0)",
            "Fee=190 sats · 1.01 sat/vB", "Signed by=laptop and off-site",
            "Status=confirmed · 70 blocks deep", "When=Wed 16 Sep 2026, 23:08 UTC",
            "#Where and on what", "445 Upper Edward Street", "Position=-27.462, 153.025  (to about 110 m)", "Made on=Piers’s MacBook PRO",
            "#Your note", "Ask Sam about the warranty.", "invoice-4471.pdf · 212 KB",
            "Inclusion=proved", "position 1,757", "hex:780",
        ] { assert!(flat.contains(want), "the sheet does not say {want:?}\n{flat}"); }
        assert!(!flat.contains("#Label"), "the label is the title; it is not printed twice");

        // a deposit reads the other way round, and says where the money did NOT happen
        let mut dep = spend();
        dep.net = 9_171; dep.fee = Some(188); dep.fee_rate = None;
        dep.inputs[0].ours = None; dep.inputs[0].signers.clear();
        dep.outputs[0].ours = Some(0); dep.outputs[1].ours = Some(0);
        let mut v2 = vault();
        v2.more.get_mut(KEY).unwrap().get_mut("ctx").unwrap()["how"] = "seen".into();
        let (_, _, b2) = tx_blocks(&v2, &dep, &[], "AUD", &q, &q, false);
        let f2 = format!("{:?}", b2.iter().map(|b| match b { export::Block::Facts(kv) => kv.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("|"), export::Block::Para(p) => p.clone(), _ => String::new() }).collect::<Vec<_>>());
        assert!(f2.contains("Amount=+9,171 sats") && f2.contains("From=outside this vault · 1 input") && f2.contains("Fee=188 sats, paid by the sender"), "{f2}");
        assert!(f2.contains("First seen on=") && f2.contains("wherever the sender was"), "a deposit must not claim it was made here: {f2}");
        assert!(!f2.contains("Signed by"), "a deposit was not signed by our keys");

        // no label, no note, no place, no money: still a sheet, and its title says which transaction
        let bare: wallet::Vault = serde_json::from_value(serde_json::json!({ "name": "signet test vault", "network": "signet", "threshold": 2, "keys": [], "descriptor": "", "address": "", "created": 0 })).unwrap();
        let (t3, _, b3) = tx_blocks(&bare, &spend(), &[], "", &None, &None, false);
        assert_eq!(t3, "Wed 16 Sep 2026, 23:08 · −5,190 sats");
        let f3 = format!("{b3:?}").replace("Jpeg { data: [", "Jpeg { data: [… ");
        assert!(!f3.contains("Worth") && !f3.contains("Where and on what") && !f3.contains("Your note"), "nothing is invented for an empty record: {f3}");

        let pdf = export::doc(&title, &subtitle, &blocks).unwrap();
        let dir = std::env::temp_dir().join("a2k-pdf"); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sheet.pdf"), &pdf).unwrap();
    }
}

#[cfg(test)]
mod bsms_tests {
    use super::*;
    fn fixture(k: &str) -> String { let f: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/ext-descriptors.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap(); f[k]["descriptor"].as_str().unwrap().to_string() }
    fn ext(desc: &str) -> Vault { let i = wallet::ext_parse(desc).unwrap().unwrap(); let mut v = ext_vault("t".into(), &i, &[]); v.address = wallet::address_at(&v, 0).unwrap(); v }

    #[test]
    fn a_bsms_record_of_extended_keys_is_the_bip129_template_and_reads_back_to_the_same_wallet() {
        let v = ext(&fixture("sortedmulti"));
        let rec = bsms_record(&v).unwrap();
        let lines: Vec<&str> = rec.lines().collect();
        assert_eq!(lines[0], "BSMS 1.0");
        assert!(lines[1].contains("/**") && !lines[1].contains("<0;1>") && !lines[1].contains('#'), "a template: each key ends /**");
        assert_eq!(lines[2], "/0/*,/1/*", "restricted to receive and change — never 'No path restrictions' for a wallet that has them");
        assert_eq!(lines[3], wallet::address_at(&v, 0).unwrap());
        let back = bsms_read(&rec, "mainnet").unwrap();
        let (a, b) = (wallet::ext_parse(&back.descriptor).unwrap().unwrap(), wallet::ext_parse(&fixture("sortedmulti")).unwrap().unwrap());
        assert_eq!(a.canonical, b.canonical, "the record reads back as the very same wallet");
    }
    #[test]
    fn a_record_whose_address_its_keys_do_not_give_is_refused() {
        let rec = bsms_record(&ext(&fixture("sortedmulti"))).unwrap();
        let other = wallet::address_at(&ext(&fixture("multi")), 0).unwrap();
        let lie = rec.replace(rec.lines().nth(3).unwrap(), &other);
        assert!(bsms_read(&lie, "mainnet").unwrap_err().contains("do not agree"));
    }
}

#[cfg(test)]
mod signers_vault_tests {
    use super::*;
    fn mixed() -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/mixed-descriptor.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap() }
    fn keys(order: [usize; 3]) -> Vec<SignerIn> {
        let f = mixed(); let cards: Vec<String> = f["cards"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
        let x = f["xkey"].as_str().unwrap().to_string();
        let all = vec![
            SignerIn { name: "seedsigner".into(), device: "seedsigner".into(), card: None, expression: Some(x), serial: 0 },
            SignerIn { name: "card one".into(), device: "yubikey".into(), card: Some(cards[0].clone()), expression: None, serial: 111 },
            SignerIn { name: "card two".into(), device: "nitrokey3".into(), card: Some(cards[1].clone()), expression: None, serial: 222 }];
        order.iter().map(|&i| SignerIn { name: all[i].name.clone(), device: all[i].device.clone(), card: all[i].card.clone(), expression: all[i].expression.clone(), serial: all[i].serial }).collect()
    }
    #[test]
    fn any_order_of_a_device_and_two_cards_makes_the_vault_embit_derives_and_each_name_stays_with_its_key() {
        let f = mixed();
        for order in [[0, 1, 2], [2, 0, 1], [1, 2, 0]] {
            let v = signers_vault("m".into(), 2, &keys(order), "mainnet").unwrap();
            for (chain, base) in [("recv", 0u32), ("chg", wallet::CHANGE)] {
                for (i, want) in f[chain].as_array().unwrap().iter().enumerate() { assert_eq!(wallet::address_at(&v, base | i as u32).unwrap(), want.as_str().unwrap(), "{order:?} {chain} {i}"); }
            }
            let by = |n: &str| v.keys.iter().find(|k| k.name == n).unwrap().clone();
            assert_eq!((by("card one").pubkey, by("card one").device.as_deref(), by("card one").serial), (f["cards"][0].as_str().unwrap().to_string(), Some("yubikey"), 111), "{order:?}");
            assert_eq!(by("card two").device.as_deref(), Some("nitrokey3"));
            assert!(by("seedsigner").pubkey.starts_with("[c9ad277c/48h/0h/0h/2h]xpub"), "{}", by("seedsigner").pubkey);
        }
    }
    /// One made-up coin at receive index 3 of the mixed vault; the spend pays 30,000 sats away with change to change index 2.
    fn mixed_spend() -> (Vault, wallet::Built) {
        let v = signers_vault("m".into(), 2, &keys([0, 1, 2]), "mainnet").unwrap();
        let coin = wallet::CoinAt { txid: "aa".repeat(32), vout: 1, value: 100_000, height: 900_000, index: 3 };
        let to = "bc1q08zdgswgey4kqeau8af0v7ak8e72aqdpnhhsc45uqdush8qs44zs4zwyh7";   // another wallet's (ext-descriptors.json, sortedmulti receive 0)
        let b = wallet::build(&v, &[coin], wallet::CHANGE | 2, to, 30_000, 2.0, false, None).unwrap();
        (v, b)
    }
    /// Writes the unsigned spend for the fixture generator (tools/mixed-sign-fixture.py) to sign with embit.
    #[test] #[ignore]
    fn emit_mixed_spend() { println!("PSBT {}", mixed_spend().1.psbt); }

    /// The whole round trip a SeedSigner makes, with embit — the library SeedSigner signs with — standing in for it:
    /// our PSBT tells it enough to find its key and see change as change; it hands back only its signature; a card
    /// signs too; the app merges both, checks each against its own sighash, and finalises.
    #[test]
    fn a_spend_signed_by_the_seedsigner_and_a_card_merges_checks_and_finalises() {
        use std::str::FromStr;
        let f: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/mixed-signed.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
        let (v, b) = mixed_spend();
        assert_eq!(b.psbt, f["unsigned"].as_str().unwrap(), "the spend is built exactly as the fixture was signed");
        let p = bitcoin::psbt::Psbt::from_str(&b.psbt).unwrap();
        let fp = bitcoin::bip32::Fingerprint::from_str("c9ad277c").unwrap();
        assert!(p.inputs[0].bip32_derivation.values().any(|(f, path)| *f == fp && path.to_string() == "48'/0'/0'/2'/0/3"), "the device finds its key at its full path");
        assert!(p.outputs[1].bip32_derivation.values().any(|(f, path)| *f == fp && path.to_string() == "48'/0'/0'/2'/1/2"), "change carries the device's path, so it can see it is change");
        assert!(p.outputs[0].bip32_derivation.is_empty(), "the payment is not ours");
        // the device's answer, minimal as a SeedSigner's is: no witness UTXO, no script
        let dev = export::parse(f["device_minimal"].as_str().unwrap().as_bytes()).unwrap();
        assert!(dev.inputs[0].witness_utxo.is_none() && dev.inputs[0].partial_sigs.len() == 1);
        let merged = wallet::adopt(&v, dev.clone(), Some(&b.psbt)).unwrap();
        let c = wallet::ceremony(&v, &merged).unwrap();
        assert_eq!((c.have, c.ready), (1, false));
        assert_eq!(c.keys.iter().find(|k| k.name == "seedsigner").unwrap().signed, 1, "the signature is the device's, by name");
        // alone, with nothing in progress, a minimal answer cannot be checked and is refused rather than trusted
        assert!(wallet::adopt(&v, dev, None).is_err());
        let card = export::parse(f["card_one"].as_str().unwrap().as_bytes()).unwrap();
        let both = wallet::adopt(&v, card, Some(&merged)).unwrap();
        let c = wallet::ceremony(&v, &both).unwrap();
        assert_eq!((c.have, c.need, c.ready), (2, 2, true));
        assert_eq!(c.keys.iter().filter(|k| k.signed == 1).map(|k| k.name.as_str()).collect::<Vec<_>>(), ["seedsigner", "card one"]);
        let tx = wallet::finalize(&both).unwrap();
        assert_eq!(bitcoin::consensus::encode::serialize_hex(&tx), f["final_tx"].as_str().unwrap_or(""), "the finished transaction is byte-for-byte what embit expects");
    }
    /// A card's key is derived from nothing, so miniscript gives it an origin with an EMPTY path. A SeedSigner reads
    /// the path of the FIRST key on a change output two levels from its end — `"m".split("/")[-2]` — and threw
    /// `IndexError: list index out of range` on Piers's bench (2026-09-23), stopping before it could show the spend.
    /// Keys with no derivation are left out; every derivation that survives is one a signer can act on.
    #[test]
    fn no_key_is_given_a_derivation_it_does_not_have_so_a_device_can_read_every_path_it_finds() {
        use std::str::FromStr;
        let p = bitcoin::psbt::Psbt::from_str(&mixed_spend().1.psbt).unwrap();
        let every = p.inputs.iter().map(|i| &i.bip32_derivation).chain(p.outputs.iter().map(|o| &o.bip32_derivation));
        let mut seen = 0;
        for d in every {
            for (key, (_, path)) in d {
                assert!(!path.is_empty(), "{key} carries an empty path — the device reads \"m\" and crashes");
                // what psbt_views.py:123 does with it: the last two levels must be there to read
                assert!(path.len() >= 2 && matches!(path[path.len() - 2].to_string().as_str(), "0" | "1"), "{key}: {path}");
                seen += 1;
            }
        }
        assert_eq!(seen, 2, "one derivation on the input and one on change — the device's key, and no other");
    }
    /// A spend built before the fix is still in Piers's store, and it is the PSBT the device reads — so it is
    /// repaired where it lies (`spend_ceremony`, `spend_qr`) rather than rebuilt. The signatures already on it must
    /// survive that: `bip32_derivation` is covered by no sighash.
    #[test]
    fn a_spend_staged_before_the_fix_is_repaired_where_it_lies_and_keeps_its_signatures() {
        use std::str::FromStr;
        let f: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/mixed-signed.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
        let (v, b) = mixed_spend();
        // put the empty paths back, as the old build wrote them
        let mut old = bitcoin::psbt::Psbt::from_str(&b.psbt).unwrap();
        let card = bitcoin::PublicKey::from_str(&v.keys.iter().find(|k| k.serial == 111).unwrap().pubkey).unwrap();
        let none = (bitcoin::bip32::Fingerprint::from_str("d2e05f32").unwrap(), bitcoin::bip32::DerivationPath::master());
        old.outputs[1].bip32_derivation.insert(card.inner, none.clone());
        old.inputs[0].bip32_derivation.insert(card.inner, none);
        let paths: Vec<String> = old.outputs[1].bip32_derivation.values().map(|(_, p)| p.to_string()).collect();
        assert!(paths.iter().any(|p| p.is_empty()), "the old shape: an empty path on change — {paths:?}");
        assert_eq!(paths[0], "", "and it sorts FIRST, which is the one the device reads");
        let staged = wallet::adopt(&v, export::parse(f["device_minimal"].as_str().unwrap().as_bytes()).unwrap(), Some(&old.to_string())).unwrap();
        let before = wallet::ceremony(&v, &staged).unwrap();
        let fixed = wallet::without_pathless(&staged).expect("it had entries to drop");
        let after = wallet::ceremony(&v, &fixed).unwrap();
        assert_eq!((before.have, after.have), (1, 1), "the signature already on it survives the repair");
        let p = bitcoin::psbt::Psbt::from_str(&fixed).unwrap();
        for d in p.inputs.iter().map(|i| &i.bip32_derivation).chain(p.outputs.iter().map(|o| &o.bip32_derivation)) {
            for (_, (_, path)) in d { assert!(path.len() >= 2, "{path} would still crash the device"); }
        }
        assert!(wallet::without_pathless(&fixed).is_none(), "a clean spend is left alone, so nothing is rewritten on every poll");
    }
    /// SeedSigner will not verify change until it has shown the wallet's cosigner fingerprints, and a fixed key has
    /// no origin to take one from: `hexlify(None)` → `TypeError: a bytes-like object is required, not 'NoneType'`
    /// (`seed_views.py:2067`, on the bench 2026-09-23). The line we SHOW gives each fixed key the origin it implies.
    /// It must remain the very same wallet: same addresses, same vault when read back.
    #[test]
    fn the_line_shown_to_a_device_names_every_key_and_is_still_the_same_wallet() {
        use std::str::FromStr;
        let v = signers_vault("m".into(), 2, &keys([0, 1, 2]), "mainnet").unwrap();
        let stored = v.ext.clone().unwrap();
        let shown = wallet::recovery_line(&v);
        assert!(stored.contains(",02") || stored.contains(",03"), "as stored, a card's key is written plainly: {stored}");
        assert!(!shown.contains(",02") && !shown.contains(",03"), "as shown, every key carries an origin: {shown}");
        assert_eq!(shown.matches('[').count(), 3, "one origin per key, including both cards: {shown}");
        // the fingerprint given to a fixed key is the one a master holding exactly that key would have
        for k in v.keys.iter().filter(|k| k.serial > 0) {
            let pk = bitcoin::PublicKey::from_str(&k.pubkey).unwrap();
            let want = format!("[{}]{}", hex::encode(&pk.pubkey_hash()[..4]), k.pubkey);
            assert!(shown.contains(&want), "{} should appear as {want} in {shown}", k.name);
        }
        let back = wallet::ext_parse(&shown).unwrap().unwrap();
        assert_eq!(back.canonical, wallet::ext_parse(&stored).unwrap().unwrap().canonical, "read back, it is the same vault, not a new one");
        let same = ext_vault("shown".into(), &back, &[]);
        for i in [0u32, 1, 2, wallet::CHANGE, wallet::CHANGE | 1] {
            assert_eq!(wallet::address_at(&same, i).unwrap(), wallet::address_at(&v, i).unwrap(), "address {i} must not move");
        }
    }
    /// Send shows what a spend would do before anything is built. If it worked that out separately from `build`,
    /// the two would drift and the screen would describe a different spend from the one signed — so they are the
    /// same function, and this holds them to it.
    #[test]
    fn what_send_shows_is_the_very_choice_that_gets_built() {
        let v = signers_vault("m".into(), 2, &keys([0, 1, 2]), "mainnet").unwrap();
        let coins: Vec<wallet::CoinAt> = [(9_430u64, 3u32), (2_000, 4), (50_000, 5)].iter().enumerate()
            .map(|(i, (value, index))| wallet::CoinAt { txid: format!("{:02x}", i + 0xa0).repeat(32), vout: 1, value: *value, height: 900_000, index: *index }).collect();
        let to = "bc1q08zdgswgey4kqeau8af0v7ak8e72aqdpnhhsc45uqdush8qs44zs4zwyh7";
        for (amount, all) in [(2_000u64, false), (30_000, false), (0, true)] {
            let c = wallet::select(&v, &coins, wallet::CHANGE | 2, to, amount, 2.0, all, None).unwrap();
            let b = wallet::build(&v, &coins, wallet::CHANGE | 2, to, amount, 2.0, all, None).unwrap();
            assert_eq!((c.chosen.len(), c.fee, c.change, c.vsize), (b.inputs, b.fee, b.change, b.vsize), "amount {amount}");
            assert_eq!(c.chosen, b.chosen, "the very same coins, in the same order");
            // what leaves is the payment and the fee; the change is still yours, so it is not part of it
            assert_eq!(c.leaving, c.send + c.fee, "amount {amount}");
            assert_eq!(c.spending, c.leaving + c.change, "every sat of the coins taken is accounted for");
            if !all { assert_eq!(c.send, amount); }
        }
    }
    /// A watched Muun vault answers with Muun's own addresses. This pins the wiring that a descriptor would
    /// normally carry: which scheme a position means, and which chain is which — Muun counts `m/1h/1h/0` as CHANGE
    /// and `m/1h/1h/1` as the addresses it hands out, the opposite way round from everyone else.
    #[test]
    fn a_watched_muun_vault_derives_the_addresses_muun_itself_derives() {
        let mut v = signers_vault("m".into(), 2, &keys([0, 1, 2]), "mainnet").unwrap();
        v.ext = None;
        v.keys.clear();
        v.network = "regtest".into();
        v.muun = Some(wallet::Muun {
            user: "tpubDBf5wCeqg3KrLJiXaveDzD5JtFJ1ss9NVvFMx4RYS73SjwPEEawcAQ7V1B5DGM4gunWDeYNrnkc49sUaf7mS1wUKiJJQD6WEctExUQoLvrg".into(),
            cosign: "tpubDB22PFkUaHoB7sgxh7exCivV5rAevVSzbB8WkFCCdbHq39r8xnYexiot4NGbi8PM6E1ySVeaHsoDeMYb6EMndpFrzVmuX8iQNExzwNpU61B".into(),
            birthday: 0, fp_user: String::new(), fp_cosign: String::new() });
        // scheme 2 of [v2, v3, v4, v5] is v4; position 2 on the chain Muun hands out
        let v4 = (2u32 << wallet::SCHEME_SHIFT) | 2;
        assert_eq!(wallet::address_at(&v, v4).unwrap(), "bcrt1qrs3vk4dzv70syck2qdz3g06tgckq4pftenuk5p77st9glnskpvtqe2tvvk");
        let v5 = (3u32 << wallet::SCHEME_SHIFT) | 17;
        assert_eq!(wallet::address_at(&v, v5).unwrap(), "bcrt1pvqngr85tm8hmsv2hjyrejlpsy7u65f7vke8mmrxnyuj3aj3xsapqvh8yrf");
        // the change flag reaches Muun's OTHER chain, so change is watched and not read as a payment away
        let chg = wallet::address_at(&v, v4 | wallet::CHANGE).unwrap();
        assert_ne!(chg, wallet::address_at(&v, v4).unwrap());
        let (user, cosign) = (v.muun.clone().unwrap().user, v.muun.clone().unwrap().cosign);
        use std::str::FromStr;
        assert_eq!(chg, muun::address_at(&bitcoin::bip32::Xpub::from_str(&user).unwrap(), &bitcoin::bip32::Xpub::from_str(&cosign).unwrap(),
                   muun::Scheme::V4, 0, 2, bitcoin::Network::Regtest).unwrap().to_string(), "our change flag is Muun's chain 0");
        // and it cannot be spent from here, however it is asked — refused by the arithmetic itself, not only by a
        // guard on a command, so no new command can forget to ask
        assert!(wallet::descriptor_at(&v, 0).unwrap_err().contains("watched"));
        let coin = wallet::CoinAt { txid: "aa".repeat(32), vout: 0, value: 100_000, height: 900_000, index: v4 };
        assert!(wallet::select(&v, &[coin], v4 | wallet::CHANGE, "bc1q08zdgswgey4kqeau8af0v7ak8e72aqdpnhhsc45uqdush8qs44zs4zwyh7", 1_000, 2.0, false, None).is_err(),
                "a watched wallet cannot build a spend");
        // but it MUST still be readable: the whole point is watching it. Every scheme's scripts are recognised as
        // ours, on both chains — this is what a guard on the wrong function broke once (2026-09-24, on the bench).
        let table = wallet::ours_table(&v).unwrap();
        for scheme in 0..wallet::MUUN_SCHEMES.len() as u32 {
            for chain in [0, wallet::CHANGE] {
                let idx = (scheme << wallet::SCHEME_SHIFT) | chain;
                let spk = wallet::script_at(&v, idx).unwrap();
                assert_eq!(table(&spk), Some(idx), "scheme {scheme} chain {chain} must be recognised as ours");
            }
        }
    }
    /// A wallet watched from one exported key, against BIP 84's own published vectors (fetched from the BIP, not
    /// recalled): the same account key gives the receive and change addresses the standard states. A plain xpub is
    /// watched for every kind at once, so a key exported without its script type still finds its coins.
    #[test]
    fn a_wallet_watched_from_an_exported_key_gives_the_addresses_the_standard_states() {
        const ZPUB: &str = "zpub6rFR7y4Q2AijBEqTUquhVz398htDFrtymD9xYYfG1m4wAcvPhXNfE3EfH1r1ADqtfSdVCToUG868RvUUkgDKf31mGDtKsAYz2oz2AGutZYs";
        let key = signers::standard_xpub(ZPUB).unwrap();
        let mut v = signers_vault("m".into(), 2, &keys([0, 1, 2]), "mainnet").unwrap();
        v.ext = None; v.keys.clear(); v.threshold = 1;
        v.watch = Some(wallet::Watch { key: key.clone(), kinds: vec!["wpkh".into()], said: "zpub".into() });
        assert_eq!(wallet::address_at(&v, 0).unwrap(), "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu", "BIP 84 first receiving address");
        assert_eq!(wallet::address_at(&v, wallet::CHANGE).unwrap(), "bc1q8c6fshw2dlwun7ekn9qwf37cu2rn755upcp6el", "BIP 84 first change address");
        // a plain xpub says nothing about its script type, so every kind is watched and each gives its own address
        v.watch = Some(wallet::Watch { key, kinds: vec!["pkh".into(), "shwpkh".into(), "wpkh".into(), "tr".into()], said: String::new() });
        assert_eq!(wallet::schemes(&v), 4);
        let at = |k: u32| wallet::address_at(&v, k << wallet::SCHEME_SHIFT).unwrap();
        assert!(at(0).starts_with('1'), "legacy: {}", at(0));
        assert!(at(1).starts_with('3'), "wrapped segwit: {}", at(1));
        assert_eq!(at(2), "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu", "the same key, read as BIP 84");
        assert!(at(3).starts_with("bc1p"), "taproot: {}", at(3));
        let table = wallet::ours_table(&v).unwrap();
        for k in 0..4u32 { let spk = wallet::script_at(&v, k << wallet::SCHEME_SHIFT).unwrap(); assert_eq!(table(&spk), Some(k << wallet::SCHEME_SHIFT), "kind {k} is recognised as ours"); }
        // and the scan must WALK every one of them. It did not: a wallet watched from a plain xpub was scanned for
        // the legacy kind alone, so one holding 153,181 sats read as empty (2026-09-25).
        assert_eq!(wallet::scan_bases(&v), vec![0, 1 << wallet::SCHEME_SHIFT, 2 << wallet::SCHEME_SHIFT, 3 << wallet::SCHEME_SHIFT]);
        v.watch = None;
        assert_eq!(wallet::scan_bases(&v), vec![0], "an ordinary vault is the single walk it always was");
    }
    #[test]
    fn it_refuses_what_it_should() {
        let k = keys([0, 1, 2]);
        assert!(signers_vault("m".into(), 2, &k, "signet").unwrap_err().contains("is for mainnet"), "a mainnet key in a test-network vault");
        let cards_only: Vec<SignerIn> = k.into_iter().filter(|x| x.card.is_some()).collect();
        assert!(signers_vault("m".into(), 2, &cards_only, "mainnet").unwrap_err().contains("every key here is a card"));
        let mut twice = keys([0, 1, 2]); twice[2].card = twice[1].card.clone();
        assert!(signers_vault("m".into(), 2, &twice, "mainnet").is_err(), "the same key twice");
    }
}


#[cfg(test)]
mod perf_probe {
    use super::*;
    /// The public "abandon … about" test wallet (BIP 84), which has real history on mainnet: its descriptor was derived
    /// with embit from the mnemonic, not recalled.
    const ABANDON: &str = "wpkh([73c5da0a/84h/0h/0h]xpub6CatWdiZiodmUeTDp8LT5or8nmbKNcuyvz7WyksVFkKB4RHwCD3XyuvPEbvqAQY3rAPshWcMLoP2fMFMKHPJ4ZeZXYVUhLv1VMrjPC7PW6V/<0;1>/*)";
    /// The scan as it was: one history request, and one unspent request per used address, address by address.
    fn scan_one_by_one(v: &Vault, servers: &chain::Servers, gap: u32) -> wallet::Scanned {
        let mut coins = Vec::new(); let mut history = Vec::new();
        let mut chain_ = |base: u32| {
            let (mut last, mut next, mut i) = (None::<u32>, None::<u32>, 0u32);
            loop {
                let spk = wallet::script_of(&wallet::descriptor_at(v, base | i).unwrap()).unwrap();
                let h = chain::history(servers, &spk).unwrap();
                if !h.is_empty() { last = Some(i); for it in h { history.push((it.txid, it.height, base | i)); } for c in chain::coins(servers, &spk).unwrap() { coins.push(wallet::CoinAt { txid: c.txid, vout: c.vout, value: c.value, height: c.height, index: base | i }); } }
                else if next.is_none() { next = Some(i); }
                if i >= last.map(|u| u + gap).unwrap_or(gap - 1) { break; }
                i += 1;
            }
            (next.unwrap_or(i + 1), i)
        };
        let (next, scan_max) = chain_(0); let (next_change, scan_max_change) = chain_(wallet::CHANGE);
        wallet::Scanned { coins, history, next, scan_max, next_change, scan_max_change }
    }
    #[test] #[ignore]
    fn the_batched_scan_finds_exactly_what_the_one_by_one_scan_found_on_a_real_wallet() {
        let info = wallet::ext_parse(ABANDON).unwrap().unwrap();
        let v = ext_vault("abandon".into(), &info, &[]);
        let servers = chain::defaults(bitcoin::Network::Bitcoin);
        let t = std::time::Instant::now(); let old = scan_one_by_one(&v, &servers, 20); let t_old = t.elapsed();
        let t = std::time::Instant::now(); let new = wallet::scan(&v, &servers, 20).unwrap(); let t_new = t.elapsed();
        let t = std::time::Instant::now(); let again = wallet::scan(&v, &servers, 20).unwrap(); let t_again = t.elapsed();
        let key = |s: &wallet::Scanned| { let mut c: Vec<_> = s.coins.iter().map(|c| (c.txid.clone(), c.vout, c.value, c.index)).collect(); c.sort(); (s.history.clone(), c, s.next, s.scan_max, s.next_change, s.scan_max_change) };
        assert_eq!(key(&old), key(&new), "the same addresses, history, coins and next positions");
        assert_eq!(key(&new), key(&again));
        let hist: Vec<(String, i32)> = { let mut h: Vec<(String, i32)> = new.history.iter().map(|(t, h, _)| (t.clone(), *h)).collect(); h.sort_by_key(|x| if x.1 <= 0 { i32::MAX } else { x.1 }); h.dedup_by(|a, b| a.0 == b.0); h };
        // the chain cache, under a throwaway key in a throwaway folder
        let dir = std::env::temp_dir().join(format!("a2k-perf-{}", std::process::id())); let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::set_var("A2K_DATA_DIR", &dir); std::env::set_var("A2K_STORE_KEY", "11".repeat(32)); }
        let t = std::time::Instant::now(); let a = activity::build(&v, &servers, &hist, "", "").unwrap(); let t_act = t.elapsed();
        let t = std::time::Instant::now(); let b = activity::build(&v, &servers, &hist, "", "").unwrap(); let t_warm = t.elapsed();
        assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap(), "the cache answers exactly what the network did");
        // with money: hourly prices for every transaction, and today's
        let t = std::time::Instant::now(); let f1 = activity::build(&v, &servers, &hist, "AUD", activity::PRICES).unwrap(); let t_f1 = t.elapsed();
        // the background fill works at the service's pace; wait for it, then open again
        let t = std::time::Instant::now(); let mut filled = 0;
        while t.elapsed() < std::time::Duration::from_secs(240) { std::thread::sleep(std::time::Duration::from_secs(5)); let n = activity::build(&v, &servers, &hist, "AUD", activity::PRICES).unwrap(); filled = n.rows.iter().filter(|r| r.quote.is_some()).count(); if n.note.as_deref().map_or(true, |x| !x.contains("background")) { break; } }
        eprintln!("PERF background fill done in {:?} ({filled} priced)", t.elapsed());
        let t = std::time::Instant::now(); let f2 = activity::build(&v, &servers, &hist, "AUD", activity::PRICES).unwrap(); let t_f2 = t.elapsed();
        let priced = |a: &activity::Activity| a.rows.iter().filter(|r| r.quote.is_some()).count();
        eprintln!("PERF with AUD: cold {t_f1:?} ({} of {} priced, note {:?}) · again {t_f2:?} ({} priced, note {:?})", priced(&f1), f1.rows.len(), f1.note, priced(&f2), f2.note);
        eprintln!("PERF activity cold {t_act:?} · warm (from the chain cache) {t_warm:?} · cache file {} bytes", std::fs::metadata(dir.join("chain.db")).map(|m| m.len()).unwrap_or(0));
        eprintln!("PERF {} history items, {} coins, recv next {} / change next {} · one-by-one {t_old:?} · batched {t_new:?} · again {t_again:?} · activity of {} txs {t_act:?}",
            new.history.len(), new.coins.len(), new.next, new.next_change, a.rows.len());
    }
}

#[cfg(test)]
mod shares_tests {
    use super::*;
    /// A vault's recovery line → shares → any two of them through Restore's own parser → the same vault: its kind,
    /// its keys, its first address. For a vault of extended keys, and for a card vault, whose line carries cc=.
    #[test]
    fn a_split_recovery_line_restores_the_same_vault_from_any_two_shares() {
        let f: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/ext-descriptors.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
        let info = wallet::ext_parse(f["sortedmulti"]["descriptor"].as_str().unwrap()).unwrap().unwrap();
        let mut ext = ext_vault("x".into(), &info, &[]); ext.address = wallet::address_at(&ext, 0).unwrap();
        let cards = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe", "038410f240613b33fe73d968f331ca82eb1fbd018c6189f89f8c5f9b9fb26f6a47"];
        let d = wallet::descriptor_for(2, &cards.iter().map(|c| c.to_string()).collect::<Vec<_>>()).unwrap();
        let card = Vault { watch: None, spark: None, muun: None, name: "c".into(), network: "mainnet".into(), threshold: 2, keys: cards.iter().map(|k| wallet::KeyRef { name: String::new(), serial: 0, pubkey: k.to_string(), device: None }).collect(),
            descriptor: d, address: String::new(), created: 0, chain_code: Some("ab".repeat(32)), next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default(), ext: None, cost_method: None };
        let dir = std::env::temp_dir().join(format!("a2k-shares-{}", std::process::id())); let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::set_var("A2K_DATA_DIR", &dir); }
        let mut s = wallet::Store::default(); s.settings.network = "mainnet".into(); wallet::save(&s).unwrap();
        for v in [&ext, &card] {
            let line = wallet::recovery_line(v);
            let sh = sskrdesc::split(&line, 2, 3).unwrap();
            let p = descriptor_parse(format!("{}\n{}", sh[2], sh[0])).unwrap();
            assert_eq!(p.address, wallet::address_at(v, 0).unwrap(), "{}", v.name);
            assert_eq!(p.threshold, 2);
            assert!(descriptor_parse(sh[1].clone()).err().unwrap().contains("1 of 2 shares"));
            // the sheet builds, its code dense but within what a sturdy QR holds
            let sheet = export::Sheet { title: "t".into(), facts: vec![], rows: vec![], codes: vec![sh[0].to_uppercase()], sturdy: true, code_hint: String::new(), text_heading: String::new(), text: sh[0].to_uppercase(), note: String::new() };
            assert!(export::pdf(&sheet).is_ok(), "a share's sheet with its QR");
        }
    }
}

#[cfg(test)]
mod record_sheet_tests {
    use super::*;
    #[test]
    fn a_record_sheet_with_a_chart_is_a_pdf_that_carries_its_figures() {
        use export::Block;
        let blocks = vec![Block::Facts(vec![("Cost basis".into(), "$10.00".into()), ("Worth now".into(), "$12.00".into())]),
            Block::Chart { title: "Worth and cost basis, in AUD".into(), series: vec![("Worth".into(), export::VALUE_RGB, vec![(0.0, 10.0), (86400.0, 11.0), (172800.0, 12.0)], false), ("Cost basis".into(), export::COST_RGB, vec![(0.0, 10.0), (172800.0, 10.0)], true)],
                lo: 0.0, hi: 13.0, lo_label: "$0".into(), hi_label: "$13".into(), from_label: "1 Jan".into(), to_label: "3 Jan".into() },
            Block::Heading("Your note".into()), Block::Para("change from the car".into())];
        let pdf = export::doc("Coin 758adfd2…:0", "demo vault · mainnet", &blocks).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(pdf.starts_with(b"%PDF-1.4") && text.contains("%%EOF"));
        // fact labels print in capitals
        for want in ["(COST BASIS)", "($10.00)", "(WORTH NOW)", "($12.00)", "(Worth)", "($13)", "(change from the car)", "RG"] { assert!(text.contains(want), "{want}"); }
        // the worth line is drawn as a path: a move and at least two lines
        assert!(text.contains(" m ") && text.matches(" l ").count() >= 3);
    }
}

#[cfg(test)]
mod transfer_tests {
    use super::*;
    /// A payment from one of this person's vaults to another is recognised from either end: the output is marked as
    /// the other vault's (with its address position), and the transaction names that vault as its other end.
    #[test]
    fn a_payment_into_another_of_this_persons_vaults_is_marked_as_a_transfer() {
        let f: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/ext-descriptors.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
        let mk = |k: &str, n: &str| { let i = wallet::ext_parse(f[k]["descriptor"].as_str().unwrap()).unwrap().unwrap(); ext_vault(n.into(), &i, &[]) };
        let (a, b) = (mk("sortedmulti", "spending vault"), mk("multi", "savings vault"));
        let mut s = wallet::Store::default(); s.vaults = vec![a.clone(), b.clone()];
        let to_b = f["multi"]["recv"][0].as_str().unwrap();
        let mut d: activity::Detail = serde_json::from_value(serde_json::json!({ "txid": "ab".repeat(32), "wtxid": "ab".repeat(32), "version": 2, "locktime": 0, "size": 1, "vsize": 1, "weight": 4, "rbf": false,
            "net": -30000, "fee": 200, "fee_rate": 1.0, "height": 1, "tip": 1, "block": null, "time": null, "proof": "unconfirmed", "position": null, "hex": "",
            "inputs": [{ "outpoint": "cd".repeat(32) + ":0", "address": f["sortedmulti"]["recv"][0], "sats": 30200, "ours": 0, "sequence": 0, "witness": 4, "signers": [] }],
            "outputs": [{ "n": 0, "address": to_b, "sats": 30000, "kind": "P2WSH", "ours": null }, { "n": 1, "address": "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4", "sats": 1, "kind": "P2WPKH", "ours": null }] })).unwrap();
        mark_peers(&s, &a, &mut d);
        assert_eq!(d.outputs[0].peer, Some(("savings vault".to_string(), 0)), "the other vault's address, at its position");
        assert_eq!(d.outputs[1].peer, None, "a stranger's address is nobody's vault");
        assert_eq!(d.peers, vec!["savings vault".to_string()]);
        // a vault on another network is never a transfer partner, even with the same keys
        let mut c = b.clone(); c.name = "signet twin".into(); c.network = "signet".into(); s.vaults.push(c);
        let mut d2 = d; d2.peers.clear(); for o in d2.outputs.iter_mut() { o.peer = None; }
        mark_peers(&s, &a, &mut d2);
        assert_eq!(d2.peers, vec!["savings vault".to_string()]);
    }
}

#[cfg(test)]
mod report_tests {
    use super::*;
    /// The blocks the window sends, as JSON, become a PDF: facts, a heading, a paragraph, a chart.
    #[test]
    fn a_report_laid_out_by_the_window_prints() {
        let blocks: Vec<BlockIn> = serde_json::from_value(serde_json::json!([
            { "kind": "facts", "rows": [["Holding", "9,430 sats in 1 coin"], ["Worth now", "$10.77"]] },
            { "kind": "chart", "chart": { "title": "Balance, in sats", "lo": 0.0, "hi": 21000.0, "lo_label": "0 sats", "hi_label": "21,000 sats", "from_label": "17/09/2026", "to_label": "22/09/2026",
                "series": [{ "name": "Balance", "color": "value", "step": true, "pts": [[1.0, 0.0], [2.0, 20000.0], [3.0, 9430.0]] }] } },
            { "kind": "heading", "text": "Transactions (4)" },
            { "kind": "para", "text": "The recovery line is not in this report." }])).unwrap();
        let out: Vec<export::Block> = blocks.into_iter().map(|b| match b {
            BlockIn::Facts { rows } => export::Block::Facts(rows), BlockIn::Heading { text } => export::Block::Heading(text), BlockIn::Para { text } => export::Block::Para(text),
            BlockIn::Chart { chart: c } => export::Block::Chart { title: c.title, series: c.series.into_iter().map(|s| (s.name, export::VALUE_RGB, s.pts, s.step)).collect(), lo: c.lo, hi: c.hi, lo_label: c.lo_label, hi_label: c.hi_label, from_label: c.from_label, to_label: c.to_label } }).collect();
        let pdf = export::doc("demo vault", "Vault report", &out).unwrap();
        if let Ok(o) = std::env::var("A2K_REPORT_OUT") { std::fs::write(o, &pdf).unwrap(); }   // to look at it
        let text = String::from_utf8_lossy(&pdf);
        for want in ["(HOLDING)", "(9,430 sats in 1 coin)", "(BALANCE, IN SATS)", "(TRANSACTIONS \\(4\\))", "(The recovery line is not in this report.)"] { assert!(text.contains(want), "{want}"); }
    }
}
