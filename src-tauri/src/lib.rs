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

#[tauri::command]
fn vault_create(name: String, threshold: usize, keys: Vec<NewKey>) -> R<Vault> {
    let mut s = wallet::load();
    if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
    let pubs: Vec<String> = keys.iter().map(|k| k.pubkey.to_lowercase()).collect();
    let descriptor = wallet::descriptor_for(threshold, &pubs)?;
    let network = wallet::network_of(&s.settings.network);
    let address = wallet::address_of(&descriptor, network)?.to_string();
    let v = Vault { name, network: s.settings.network.clone(), threshold,
        keys: keys.into_iter().map(|k| wallet::KeyRef { name: k.name, serial: k.serial, pubkey: k.pubkey.to_lowercase() }).collect(),
        descriptor, address, created: now() };
    s.vaults.push(v.clone());
    wallet::save(&s)?;
    Ok(v)
}

#[derive(Serialize)]
struct Parsed { threshold: usize, keys: Vec<String>, descriptor: String, address: String }

#[tauri::command]
fn descriptor_parse(text: String) -> R<Parsed> {
    let s = wallet::load();
    let (threshold, keys) = wallet::keys_of(&text)?;
    let descriptor = wallet::descriptor_for(threshold, &keys)?;
    let address = wallet::address_of(&descriptor, wallet::network_of(&s.settings.network))?.to_string();
    Ok(Parsed { threshold, keys, descriptor, address })
}

#[tauri::command]
fn vault_restore(name: String, text: String, names: Vec<String>) -> R<Vault> {
    let mut s = wallet::load();
    if s.vaults.iter().any(|v| v.name == name) { return Err(format!("a vault called '{name}' already exists")); }
    let (threshold, keys) = wallet::keys_of(&text)?;
    let descriptor = wallet::descriptor_for(threshold, &keys)?;
    let network = wallet::network_of(&s.settings.network);
    let address = wallet::address_of(&descriptor, network)?.to_string();
    let v = Vault { name, network: s.settings.network.clone(), threshold,
        keys: keys.iter().enumerate().map(|(i, k)| wallet::KeyRef { name: names.get(i).cloned().unwrap_or_else(|| format!("key {}", i + 1)), serial: 0, pubkey: k.clone() }).collect(),
        descriptor, address, created: now() };
    s.vaults.push(v.clone());
    wallet::save(&s)?;
    Ok(v)
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
struct Snapshot { coins: Vec<chain::Coin>, balance: u64, height: u32, history: Vec<chain::HistoryItem>, servers: chain::Servers }

#[tauri::command]
async fn vault_sync(name: String) -> R<Snapshot> {
    block(move || {
        let s = wallet::load();
        let v = s.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?.clone();
        let servers = wallet::servers(&s);
        let spk = wallet::script_of(&v.descriptor)?;
        let coins = chain::coins(&servers, &spk)?;
        let balance = coins.iter().map(|c| c.value).sum();
        let height = chain::height(&servers).unwrap_or(0);
        let history = chain::history(&servers, &spk).unwrap_or_default();
        Ok(Snapshot { coins, balance, height, history, servers })
    }).await
}

#[tauri::command]
async fn fees_get() -> R<chain::Fees> {
    block(|| { let s = wallet::load(); chain::fees(&wallet::servers(&s)) }).await
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
        let servers = wallet::servers(&s);
        let spk = wallet::script_of(&v.descriptor)?;
        let coins = chain::coins(&servers, &spk)?;
        let b = wallet::build(&v, &coins, &to, amount, rate, send_all)?;
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
        let todo = wallet::sighashes_for(&psbt, &pub_hex)?;
        if todo.is_empty() { return Err("nothing for this card to sign (already signed, or not its key)".into()); }
        let mut done = Vec::new();
        for (i, digest) in todo {
            let (r, sg) = cards::sign_digest(&reader, &pin, &digest).map_err(|e| e.to_string())?;
            psbt = wallet::add_signature(&psbt, &pub_hex, i, &r, &sg, &digest)?;
            done.push(i);
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
        let servers = wallet::servers(&s);
        let txid = chain::broadcast(&servers, &tx)?;
        s.last_psbt = None;
        wallet::save(&s)?;
        Ok(Sent { txid: txid.to_string(), link: chain::tx_link(&servers, &txid.to_string()) })
    }).await
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            cards_list, card_generate, store_get, vault_create, descriptor_parse, vault_restore, vault_forget,
            settings_set, vault_sync, fees_get, servers_test, spend_build, spend_sign, spend_status, spend_broadcast, spend_discard
        ])
        .run(tauri::generate_context!())
        .expect("error while running Any Two Keys");
}
