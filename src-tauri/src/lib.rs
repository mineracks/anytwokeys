mod cards;
mod chain;
mod hd;
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
        chain_code: if rotating.unwrap_or(true) { Some(fresh_chain_code()) } else { None }, next: 0, scan_max: 0 };
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
        descriptor: descriptor.clone(), address: String::new(), created: 0, chain_code: cc.clone(), next: 0, scan_max: 0 };
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
        descriptor, address: String::new(), created: now(), chain_code: cc, next: 0, scan_max: 0 };
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

#[tauri::command]
fn vault_forget(name: String) -> R<Store> {
    let mut s = wallet::load();
    s.vaults.retain(|v| v.name != name);
    wallet::save(&s)?;
    Ok(s)
}

#[tauri::command]
fn settings_set(network: String, custom: bool, electrum: String, explorer: String) -> R<Store> {
    let mut s = wallet::load();
    if network != "mainnet" && network != "signet" { return Err("network must be mainnet or signet".into()); }
    s.settings = wallet::Settings { network, custom, electrum: electrum.trim().to_string(), explorer: explorer.trim().to_string() };
    wallet::save(&s)?;
    Ok(s)
}

#[derive(Serialize)]
struct HistoryRow { txid: String, height: i32, index: u32 }
#[derive(Serialize)]
struct Snapshot { coins: Vec<wallet::CoinAt>, balance: u64, height: u32, history: Vec<HistoryRow>, servers: chain::Servers, receive: String, receive_index: u32, rotating: bool }

#[tauri::command]
async fn vault_sync(name: String) -> R<Snapshot> {
    block(move || {
        let mut s = wallet::load();
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        let servers = wallet::servers_for(&s, &v.network);
        let sc = wallet::scan(&v, &servers)?;
        let balance = sc.coins.iter().map(|c| c.value).sum();
        let height = chain::height(&servers).unwrap_or(0);
        if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == v.name) { vm.next = sc.next; vm.scan_max = sc.scan_max; }
        wallet::save(&s)?;
        let receive_index = if v.chain_code.is_some() { sc.next } else { 0 };
        let receive = wallet::address_at(&v, receive_index)?;
        let mut history: Vec<HistoryRow> = sc.history.into_iter().map(|(txid, height, index)| HistoryRow { txid, height, index }).collect();
        history.sort_by_key(|h| if h.height <= 0 { i32::MAX } else { h.height });
        history.dedup_by(|a, b| a.txid == b.txid);
        Ok(Snapshot { coins: sc.coins, balance, height, history, servers, receive, receive_index, rotating: v.chain_code.is_some() })
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
async fn spend_build(name: String, to: String, amount: u64, rate: f64, send_all: bool) -> R<wallet::Built> {
    block(move || {
        let mut s = wallet::load();
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        let servers = wallet::servers_for(&s, &v.network);
        let sc = wallet::scan(&v, &servers)?;
        let change_index = if v.chain_code.is_some() { sc.next } else { 0 };
        let b = wallet::build(&v, &sc.coins, change_index, &to, amount, rate, send_all)?;
        if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == v.name) { vm.next = sc.next; vm.scan_max = sc.scan_max.max(sc.next); }
        s.last_psbt = Some(b.psbt.clone());
        wallet::save(&s)?;
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

#[tauri::command]
fn spend_discard() -> R<()> {
    let mut s = wallet::load();
    s.last_psbt = None;
    wallet::save(&s)
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}


// ------------------------------------------------------------------ watcher --
mod watch {
    use crate::{chain, wallet};
    use tauri::{AppHandle, Emitter};
    use tauri_plugin_notification::NotificationExt;

    pub struct Event { pub vault: String, pub title: String, pub body: String }

    /// One pass over every vault: what changed since last time, as notifications.
    pub fn pass() -> Vec<Event> {
        let mut s = wallet::load();
        let mut out = Vec::new();
        let vaults = s.vaults.clone();
        for v in &vaults {
            let servers = wallet::servers_for(&s, &v.network);
            let Ok(sc) = wallet::scan(v, &servers) else { continue };
            let seen = s.seen.entry(v.name.clone()).or_default();
            let now_txs: std::collections::HashMap<String, i32> = sc.history.iter().map(|(t, h, _)| (t.clone(), *h)).collect();
            let now_coins: std::collections::HashMap<String, u64> = sc.coins.iter().map(|c| (format!("{}:{}", c.txid, c.vout), c.value)).collect();
            if seen.primed {
                for (txid, h) in &now_txs {
                    let gained: u64 = now_coins.iter().filter(|(k, _)| k.starts_with(txid.as_str()) && !seen.coins.contains_key(*k)).map(|(_, v)| v).sum();
                    match seen.txs.get(txid) {
                        None => {
                            let lost: u64 = seen.coins.iter().filter(|(k, _)| !now_coins.contains_key(*k)).map(|(_, v)| v).sum();
                            let net = gained as i64 - lost as i64;
                            let short = format!("{}…{}", &txid[..8], &txid[txid.len() - 6..]);
                            if net >= 0 {
                                out.push(Event { vault: v.name.clone(), title: format!("{}: {} sats arrived", v.name, fmt(net as u64)), body: format!("{short} · {}", if *h > 0 { "confirmed" } else { "waiting for a confirmation" }) });
                            } else {
                                out.push(Event { vault: v.name.clone(), title: format!("{}: {} sats sent", v.name, fmt((-net) as u64)), body: format!("{short} · {}", if *h > 0 { "confirmed" } else { "in the mempool" }) });
                            }
                        }
                        Some(prev) if *prev <= 0 && *h > 0 => {
                            out.push(Event { vault: v.name.clone(), title: format!("{}: confirmed", v.name), body: format!("{}…{} is in block {}", &txid[..8], &txid[txid.len() - 6..], fmt(*h as u64)) });
                        }
                        _ => {}
                    }
                }
            }
            seen.txs = now_txs;
            seen.coins = now_coins;
            seen.primed = true;
            if let Some(vm) = s.vaults.iter_mut().find(|x| x.name == v.name) { vm.next = sc.next; vm.scan_max = sc.scan_max; }
        }
        let _ = wallet::save(&s);
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
                    let _ = app.notification().builder().title(&e.title).body(&e.body).show();
                    let _ = app.emit("vault-changed", &e.vault);
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
        .invoke_handler(tauri::generate_handler![
            cards_list, card_generate, store_get, vault_create, descriptor_parse, vault_restore, vault_recovery, vault_forget,
            settings_set, vault_sync, fees_get, servers_test, spend_build, spend_sign, spend_status, spend_broadcast, spend_export, spend_discard
        ])
        .run(tauri::generate_context!())
        .expect("error while running Any Two Keys");
}
