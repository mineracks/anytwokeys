mod activity;
mod cards;
mod chain;
mod export;
mod hd;
mod labels;
mod message;
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
    let mut s = wallet::load();
    let v = s.vaults.iter_mut().find(|v| v.name == name).ok_or("no such vault")?;
    let mut changed = false;
    for k in v.keys.iter_mut() { if k.pubkey == pubkey && k.serial == 0 && serial != 0 { k.serial = serial; changed = true; } }
    if changed { wallet::save(&s)?; }
    Ok(s)
}

#[tauri::command]
fn key_form_seen(pubkey: String, form: u8, nfc: bool) -> R<Store> {
    // one byte per key: the form factor, 0x40 = NFC was asked, 0x80 = the key has NFC
    wallet::update(move |s| { s.forms.insert(pubkey, (form & 0x0F) | 0x40 | if nfc { 0x80 } else { 0 }); })
}

/// When each transaction happened, what it moved, and what that was worth. Separate from the sync so the balance never waits on it.
#[tauri::command]
async fn vault_activity(name: String, history: Vec<(String, i32)>, ccy: String) -> R<activity::Activity> {
    block(move || {
        let s = wallet::load();
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        let servers = wallet::servers_for(&s, &v.network);
        // our price service goes with our servers; someone running their own names their own, or has none
        let prices = if s.settings.custom { s.settings.prices.clone() } else { activity::PRICES.to_string() };
        // `ccy` is what the screen resolved the setting to ("auto" is worked out from the device there); empty = no fiat
        if !(ccy.is_empty() || (ccy.len() == 3 && ccy.chars().all(|c| c.is_ascii_uppercase()))) { return Err("the currency is a three-letter code".into()); }
        activity::build(&v, &servers, &history, &ccy, &prices)
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
    let s = wallet::load();
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    wallet::address_at(v, if v.chain_code.is_some() { v.next } else { 0 })
}

#[derive(Serialize)]
struct NodeStatus { electrum: String, height: u32, ms: u128, ours: bool }

/// Does the Electrum server for this network answer right now, and how fast? Opens a fresh connection on purpose:
/// a light that stays green on a cached answer is worse than no light.
#[tauri::command]
async fn node_status(network: String) -> R<NodeStatus> {
    block(move || {
        let s = wallet::load();
        let servers = wallet::servers_for(&s, &network);
        let ours = servers.electrum == chain::defaults(wallet::network_of(&network)).electrum;
        let t = std::time::Instant::now();
        let height = chain::height(&servers)?;
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
        let s = wallet::load();
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        activity::detail(&v, &wallet::servers_for(&s, &v.network), &txid, height)
    }).await
}

#[tauri::command]
fn store_get() -> Store {
    wallet::load()
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
    let mut s = wallet::load();
    if let Some(n) = network { if n != "mainnet" && n != "signet" { return Err("network must be mainnet or signet".into()); } s.settings.network = n; }
    if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
    if threshold < 1 || threshold > keys.len() || keys.len() > 15 { return Err("threshold must be between 1 and the number of keys, and at most 15 keys".into()); }
    let pubs: Vec<String> = keys.iter().map(|k| k.pubkey.to_lowercase()).collect();
    let descriptor = wallet::descriptor_for(threshold, &pubs)?;
    let network = wallet::network_of(&s.settings.network);
    let mut v = Vault { name, network: s.settings.network.clone(), threshold,
        keys: keys.into_iter().map(|k| wallet::KeyRef { name: k.name, serial: k.serial, pubkey: k.pubkey.to_lowercase() }).collect(),
        descriptor, address: String::new(), created: now(),
        chain_code: if rotating.unwrap_or(true) { Some(fresh_chain_code()) } else { None }, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), labels_synced: 0, seal: Default::default() };
    v.address = wallet::address_at(&v, 0)?;
    let _ = network;
    s.vaults.push(v.clone());
    wallet::save(&s)?;
    Ok(v)
}

#[derive(Serialize)]
struct Parsed { threshold: usize, keys: Vec<String>, descriptor: String, address: String, rotating: bool }

#[tauri::command]
fn descriptor_parse(text: String) -> R<Parsed> {
    let s = wallet::load();
    let (desc_text, cc) = wallet::split_recovery(&text);
    let (threshold, keys) = wallet::keys_of(&desc_text)?;
    let descriptor = wallet::descriptor_for(threshold, &keys)?;
    let tmp = Vault { name: String::new(), network: s.settings.network.clone(), threshold, keys: keys.iter().map(|k| wallet::KeyRef { name: String::new(), serial: 0, pubkey: k.clone() }).collect(),
        descriptor: descriptor.clone(), address: String::new(), created: 0, chain_code: cc.clone(), next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), labels_synced: 0, seal: Default::default() };
    let address = wallet::address_at(&tmp, 0)?;
    Ok(Parsed { threshold, keys, descriptor: wallet::recovery_line(&tmp), address, rotating: cc.is_some() })
}

#[tauri::command]
fn vault_restore(name: String, text: String, names: Vec<String>) -> R<Vault> {
    let mut s = wallet::load();
    if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
    let (desc_text, cc) = wallet::split_recovery(&text);
    let (threshold, keys) = wallet::keys_of(&desc_text)?;
    let descriptor = wallet::descriptor_for(threshold, &keys)?;
    let mut v = Vault { name, network: s.settings.network.clone(), threshold,
        keys: keys.iter().enumerate().map(|(i, k)| wallet::KeyRef { name: names.get(i).cloned().unwrap_or_else(|| format!("key {}", i + 1)), serial: 0, pubkey: k.clone() }).collect(),
        descriptor, address: String::new(), created: now(), chain_code: cc, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), labels_synced: 0, seal: Default::default() };
    v.address = wallet::address_at(&v, 0)?;
    s.vaults.push(v.clone());
    wallet::save(&s)?;
    Ok(v)
}

#[tauri::command]
fn vault_recovery(name: String) -> R<String> {
    let s = wallet::load();
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    Ok(wallet::recovery_line(v))
}

/// Rename a vault and/or its keys. Labels only: nothing on the chain or the cards changes.
#[tauri::command]
fn vault_rename(name: String, new_name: String, key_names: Vec<String>) -> R<Store> {
    let mut s = wallet::load();
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
    let mut s = wallet::load();
    s.vaults.retain(|v| v.name != name);
    wallet::save(&s)?;
    Ok(s)
}

#[tauri::command]
fn settings_set(network: String, custom: bool, electrum: String, explorer: String, gap: Option<u32>, fiat: Option<String>, prices: Option<String>, vss: Option<String>) -> R<Store> {
    let mut s = wallet::load();
    if network != "mainnet" && network != "signet" { return Err("network must be mainnet or signet".into()); }
    let gap = gap.unwrap_or(s.settings.gap);
    if !(1..=500).contains(&gap) { return Err("the gap limit must be between 1 and 500".into()); }
    let fiat = fiat.map(|f| { let f = f.trim(); if f.eq_ignore_ascii_case("auto") { "auto".to_string() } else { f.to_uppercase() } }).unwrap_or_else(|| s.settings.fiat.clone());
    if !(fiat.is_empty() || fiat == "auto" || (fiat.len() == 3 && fiat.chars().all(|c| c.is_ascii_uppercase()))) { return Err("the currency is a three-letter code, or empty for none".into()); }
    let prices = prices.map(|p| p.trim().to_string()).unwrap_or_else(|| s.settings.prices.clone());
    if !(prices.is_empty() || prices.starts_with("https://") || prices.starts_with("http://")) { return Err("the price server is a web address".into()); }
    let vss = vss.map(|v| v.trim().trim_end_matches('/').to_string()).unwrap_or_else(|| s.settings.vss.clone());
    if !(vss.is_empty() || vss.starts_with("https://") || vss.starts_with("http://")) { return Err("the label host is a web address, or empty to keep labels on this computer only".into()); }
    s.settings = wallet::Settings { network, custom, electrum: electrum.trim().to_string(), explorer: explorer.trim().to_string(), gap, fiat, prices, vss };
    wallet::save(&s)?;
    Ok(s)
}

#[derive(Serialize)]
struct HistoryRow { txid: String, height: i32, index: u32 }
#[derive(Serialize)]
struct Snapshot { coins: Vec<wallet::CoinAt>, balance: u64, height: u32, history: Vec<HistoryRow>, servers: chain::Servers, receive: String, receive_index: u32, rotating: bool, next_change: u32, gap: u32 }

#[tauri::command]
async fn vault_sync(name: String) -> R<Snapshot> {
    block(move || {
        let s = wallet::load();
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        let servers = wallet::servers_for(&s, &v.network);
        let sc = wallet::scan(&v, &servers, s.settings.gap)?;
        let balance = sc.coins.iter().map(|c| c.value).sum();
        let height = chain::height(&servers).unwrap_or(0);
        let (n, sm, nc, smc) = (sc.next, sc.scan_max, sc.next_change, sc.scan_max_change);
        wallet::update(|s| if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == v.name) { vm.next = n; vm.scan_max = sm; vm.next_change = nc; vm.scan_max_change = smc; })?;
        let receive_index = if v.chain_code.is_some() { sc.next } else { 0 };
        let receive = wallet::address_at(&v, receive_index)?;
        let mut history: Vec<HistoryRow> = sc.history.into_iter().map(|(txid, height, index)| HistoryRow { txid, height, index }).collect();
        history.sort_by_key(|h| if h.height <= 0 { i32::MAX } else { h.height });
        history.dedup_by(|a, b| a.txid == b.txid);
        Ok(Snapshot { coins: sc.coins, balance, height, history, servers, receive, receive_index, rotating: v.chain_code.is_some(), next_change: sc.next_change, gap: s.settings.gap })
    }).await
}

#[tauri::command]
async fn fees_get(network: Option<String>) -> R<chain::Fees> {
    block(move || { let s = wallet::load(); let n = network.unwrap_or_else(|| s.settings.network.clone()); chain::fees(&wallet::servers_for(&s, &n)) }).await
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

#[tauri::command]
async fn spend_build(name: String, to: String, amount: u64, rate: f64, send_all: bool, coins: Option<Vec<String>>) -> R<wallet::Built> {
    block(move || {
        let s = wallet::load();
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        let servers = wallet::servers_for(&s, &v.network);
        let sc = wallet::scan(&v, &servers, s.settings.gap)?;
        let change_index = if v.chain_code.is_some() { wallet::CHANGE | sc.next_change } else { 0 };
        let pick = coins.filter(|c| !c.is_empty());
        let b = wallet::build(&v, &sc.coins, change_index, &to, amount, rate, send_all, pick.as_deref())?;
        let (n, sm, nc, smc, psbt) = (sc.next, sc.scan_max.max(sc.next), sc.next_change, sc.scan_max_change.max(sc.next_change), b.psbt.clone());
        wallet::update(|s| { if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == v.name) { vm.next = n; vm.scan_max = sm; vm.next_change = nc; vm.scan_max_change = smc; } s.last_psbt = Some(psbt); s.last_message = None; })?;
        Ok(b)
    }).await
}

#[derive(Serialize)]
struct Signed { psbt: String, signed_inputs: Vec<usize>, status: wallet::SigStatus, signer: String }

/// Sign every input the plugged-in card can, with its PIN; one touch per input.
#[tauri::command]
async fn spend_sign(name: String, reader: String, pin: String) -> R<Signed> {
    block(move || {
        let mut s = wallet::load();
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
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
    let s = wallet::load();
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let psbt = s.last_psbt.clone().ok_or("no spend in progress")?;
    wallet::status(&psbt, v.threshold)
}

#[derive(Serialize)]
struct Sent { txid: String, link: String }

#[tauri::command]
async fn spend_broadcast(name: String) -> R<Sent> {
    block(move || {
        let mut s = wallet::load();
        let _v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        if s.last_message.is_some() { return Err("this is a signed message, not a spend; finish it on the Messages screen".into()); }
        let psbt = s.last_psbt.clone().ok_or("no spend in progress")?;
        let tx = wallet::finalize(&psbt)?;
        let servers = wallet::servers_for(&s, &_v.network);
        let txid = chain::broadcast(&servers, &tx)?;
        s.last_psbt = None;
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
    let s = wallet::load();
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
    let s = wallet::load();
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    wallet::ceremony(v, s.last_psbt.as_deref().ok_or("no spend in progress")?)
}

#[derive(Serialize)]
struct QrOut { parts: Vec<String>, single: Option<String>, bytes: usize }

/// UR crypto-psbt codes for the screen: a looping set of parts, and one still code when the PSBT is small enough.
#[tauri::command]
fn spend_qr() -> R<QrOut> {
    use std::str::FromStr;
    let s = wallet::load();
    let raw = bitcoin::psbt::Psbt::from_str(s.last_psbt.as_deref().ok_or("no spend in progress")?.trim()).map_err(|e| format!("psbt: {e}"))?.serialize();
    let single = if raw.len() <= 700 { export::ur_parts(&raw, usize::MAX, false)?.pop() } else { None };
    Ok(QrOut { parts: export::ur_parts(&raw, 160, true)?, single, bytes: raw.len() })
}

fn sats_text(n: u64) -> String { let s = n.to_string(); let mut o = String::new(); for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { o.push(','); } o.push(c); } o }

/// A printable sheet: what the spend pays, who has signed, the PSBT as UR codes and as base64 text.
#[tauri::command]
fn spend_pdf(name: String) -> R<String> {
    use std::str::FromStr;
    let s = wallet::load();
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

/// The vault's recovery sheet: its descriptor (and chain code) as a code the Restore screen scans, its keys, and the line as text.
#[tauri::command]
fn vault_pdf(name: String) -> R<String> {
    let s = wallet::load();
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
    let s = wallet::load();
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
    let current = if s.last_message.is_some() { None } else { s.last_psbt.clone() };
    let merged = wallet::adopt(&v, psbt, current.as_deref())?;
    let keep = merged.clone();
    wallet::update(move |s| { s.last_psbt = Some(keep); s.last_message = None; })?;
    wallet::ceremony(&v, &merged)
}

#[tauri::command]
fn spend_discard() -> R<()> {
    let mut s = wallet::load();
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
    let s = wallet::load();
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    if v.chain_code.is_none() { return Ok(vec![AddrRow { index: 0, position: 0, chain: "receive", address: wallet::address_at(v, 0)?, current: true, future: false }]); }
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
        let s = wallet::load();
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
    let mut s = wallet::load();
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
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
    let mut s = wallet::load();
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
    let s = wallet::load();
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
        let s = wallet::load();
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
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
        let s = wallet::load();
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
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
    let mut v = Vault { name, network: rec.network.clone(), threshold: rec.threshold,
        keys: rec.pubkeys.iter().zip(&rec.key_names).map(|(p, n)| wallet::KeyRef { name: n.clone(), serial: 0, pubkey: p.clone() }).collect(),
        descriptor, address: String::new(), created: now(), chain_code: rec.chain_code.clone(), next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), labels_synced: 0,
        seal: wallet::Seal { shares, sealed } };
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
    let s = wallet::load();
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

/// Open the locked shares on one key with its PIN. A wrong PIN costs one of the key's tries, as it would when signing.
#[tauri::command]
async fn seal_unlock(reader: String, pin: String) -> R<Vec<Found>> {
    block(move || {
        let cards = seal_cards()?;
        let card = cards.iter().find(|c| c.reader == reader).ok_or("that key is no longer plugged in")?;
        let entries = seal::parse_container(&card.container)?;
        let key = cards::seal_key(&reader, &pin, false).map_err(|e| e.to_string())?.ok_or("this key holds no secret for a seal: its shares were not locked by this app")?;
        entries.into_iter().filter(|e| e.locked).map(|e| {
            let plain = seal::unlock_share(&key, &e.id, e.k, e.x, &hex::decode(&e.share).map_err(|e| e.to_string())?)?;
            Ok(Found { pubkey: card.pubkey.clone().unwrap_or_default(), serial: card.serial, entry: seal::Entry { share: hex::encode(plain), locked: false, ..e } })
        }).collect()
    }).await
}

/// A vault this computer already has (restored from paper, say) learns its shares from its own keys, once they are open.
#[tauri::command]
fn seal_adopt(name: String, found: Vec<Found>) -> R<Store> {
    let s = wallet::load();
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
    Ok(wallet::load())
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
    Ok(wallet::load())
}

#[derive(Serialize, Clone)]
struct LabelsStatus { backed_up: bool, host: String, count: usize, synced: u64, note: String }

/// Pull, merge and push this vault's labels with the label host (labels.rs). Cheap when nothing changed.
#[tauri::command]
async fn labels_sync(name: String) -> R<LabelsStatus> {
    let s = wallet::load();
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
    let host = s.settings.vss.clone();
    let count = v.labels.len();
    if host.is_empty() { return Ok(LabelsStatus { backed_up: false, host, count, synced: v.labels_synced, note: "Labels stay on this computer: no label host is set (App settings).".into() }); }
    let root = match labels::root_from_shares(&v.seal.shares) { Some(r) => r, None => return Ok(LabelsStatus { backed_up: false, host, count, synced: 0, note: "Seal this vault's keys to back its labels up: they are encrypted so that any two keys bring them back with the vault.".into() }) };
    let keys = labels::keys(&root);
    let local = labels::Envelope::from_vault(&v.labels, &v.label_meta, &v.notes, &v.files);
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
                    }
                }
                vm.labels_synced = t;
            })?;
            let n = merged.records.values().filter(|r| !r.label.is_empty()).count();
            Ok(LabelsStatus { backed_up: true, host, count: n, synced: t, note: String::new() })
        }
        Err(e) => Ok(LabelsStatus { backed_up: false, host, count, synced: v.labels_synced, note: e }),
    }
}

#[tauri::command]
fn labels_export(name: String) -> R<String> {
    let s = wallet::load();
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
    let s = wallet::load();
    let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let first = wallet::address_at(v, 0)?;
    let text = format!("BSMS 1.0\n{}\nNo path restrictions\n{first}\n", v.descriptor);
    let path = rfd::FileDialog::new().set_title("Save the vault as a BSMS descriptor record").set_file_name(format!("{}.bsms", v.name.replace(' ', "-"))).add_filter("BSMS", &["bsms", "txt"]).save_file().ok_or("cancelled")?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[derive(Serialize)]
struct BsmsIn { descriptor: String, first_address: String }

/// Read a BSMS 1.0 descriptor record. Only fixed public keys can come back: the cards hold no xpubs.
#[tauri::command]
fn bsms_import() -> R<BsmsIn> {
    let path = rfd::FileDialog::new().set_title("Open a BSMS descriptor record").add_filter("BSMS", &["bsms", "txt"]).pick_file().ok_or("cancelled")?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 || !lines[0].starts_with("BSMS") { return Err("not a BSMS descriptor record (first line should be 'BSMS 1.0')".into()); }
    let desc = lines[1].replace("/**", "");
    if desc.contains("pub") && (desc.contains("xpub") || desc.contains("tpub") || desc.contains('[')) { return Err("this record uses extended keys with derivation paths; YubiKeys hold single keys, so only fixed-key records can be restored here".into()); }
    let (threshold, keys) = wallet::keys_of(&desc)?;
    let descriptor = wallet::descriptor_for(threshold, &keys)?;
    let first_address = lines.get(3).unwrap_or(&"").to_string();
    Ok(BsmsIn { descriptor, first_address })
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
        let s0 = wallet::load();
        let mut out = Vec::new();
        let vaults = s0.vaults.clone();
        for v in &vaults {
            let servers = wallet::servers_for(&s0, &v.network);
            let sc = match wallet::scan(v, &servers, s0.settings.gap) { Ok(sc) => sc, Err(e) => { log(&format!("{}: scan failed: {e}", v.name)); continue } };
            // re-read after the slow part, so a sync that saved meanwhile is not erased
            let mut s = wallet::load();
            let seen = s.seen.entry(v.name.clone()).or_default();
            let now_txs: std::collections::HashMap<String, i32> = sc.history.iter().map(|(t, h, _)| (t.clone(), *h)).collect();
            let now_coins: std::collections::HashMap<String, u64> = sc.coins.iter().map(|c| (format!("{}:{}", c.txid, c.vout), c.value)).collect();
            // a confirmed transaction does not vanish; a scan that lost one came from a server that answered badly
            if let Some(gone) = seen.txs.iter().find(|(t, h)| **h > 0 && !now_txs.contains_key(*t)) {
                log(&format!("{}: scan lost confirmed {}; ignoring this pass", v.name, &gone.0[..8]));
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
            let r = wallet::update(move |s| { s.seen.insert(name.clone(), seen_now); if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == name) { vm.next = n; vm.scan_max = sm; vm.next_change = nc; vm.scan_max_change = smc; } });
            log(&format!("{}: {} coins, {} txs, primed_before={primed_before}, events={}, save={}", v.name, sc.coins.len(), sc.history.len(), out.len() - before, r.as_ref().map(|_| "ok").unwrap_or("FAILED")));
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
                let events = pass();
                for e in &events {
                    let r = app.notification().builder().title(&e.title).body(&e.body).show();
                    log(&format!("notify '{}': {}", e.title, match &r { Ok(_) => "sent".to_string(), Err(e) => format!("FAILED {e}") }));
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
        .setup(|app| { watch::start(app.handle().clone()); Ok(()) })
        .invoke_handler(tauri::generate_handler![open_url, vault_activity, tx_detail, node_status, device_locale, contact_save, contact_delete, contact_used, vault_receive_address, vault_key_seen, key_form_seen, vault_pdf, notify_test, spend_ceremony, spend_qr, spend_pdf, spend_import, seal_status, seal_write, seal_read, seal_unlock, seal_adopt, seal_preview, seal_restore, label_set, note_set, labels_sync, labels_export, labels_import, bsms_export, bsms_import, vault_addresses, message_sign_key, message_stage, message_finish, message_verify, 
            cards_list, card_generate, store_get, vault_create, descriptor_parse, vault_restore, vault_recovery, vault_rename, vault_forget,
            settings_set, vault_sync, fees_get, servers_test, spend_build, spend_sign, spend_status, spend_broadcast, spend_export, spend_discard
        ])
        .run(tauri::generate_context!())
        .expect("error while running Any Two Keys");
}
