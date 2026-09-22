//! The chain, through an Electrum server (coins, history, broadcast) and an esplora-style
//! explorer (fee levels, links). Both default to ours; both are the user's to override.
use bitcoin::{Network, ScriptBuf, Transaction, Txid};
use electrum_client::{Client, ConfigBuilder, ElectrumApi};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Servers {
    pub electrum: String,   // e.g. ssl://electrum.mineracks.com:50002
    pub explorer: String,   // e.g. https://mempool.mineracks.com
}

pub fn defaults(network: Network) -> Servers {
    match network {
        Network::Bitcoin => Servers { electrum: "ssl://electrum.mineracks.com:50002".into(), explorer: "https://mempool.mineracks.com".into() },
        _ => Servers { electrum: "tcp://100.94.4.10:50001".into(), explorer: "https://signet.mineracks.com".into() },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Coin {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub height: u32,   // 0 = unconfirmed
}

#[derive(Debug, Clone, Serialize)]
pub struct Fees { pub slow: f64, pub medium: f64, pub fast: f64 }

// One connection per server, kept open. Opening one is a TCP and TLS handshake (~80 ms to ours); a scan used to open
// two per address — a 3.4 s scan of an empty 2-of-3 that one kept connection answers in ~30 ms. A connection idle for a
// while is pinged before use, and any failure drops it so the next request opens a fresh one.
struct Held { c: Arc<Client>, used: Instant }
static POOL: Mutex<Option<HashMap<String, Held>>> = Mutex::new(None);
const IDLE_CHECK: Duration = Duration::from_secs(30);

fn open(servers: &Servers) -> Result<Client, String> {
    let cfg = ConfigBuilder::new().timeout(Some(20)).validate_domain(true).build();
    Client::from_config(&servers.electrum, cfg).map_err(|e| format!("electrum {}: {e}", servers.electrum))
}
/// The kept connection to this server (opened if there is none, or if the one kept no longer answers).
pub fn client(servers: &Servers) -> Result<Arc<Client>, String> {
    crate::net::out("the node")?;
    let held = POOL.lock().map_err(|_| "connection pool poisoned")?.get_or_insert_with(HashMap::new).get(&servers.electrum).map(|h| (h.c.clone(), h.used.elapsed()));
    if let Some((c, idle)) = held {
        if idle < IDLE_CHECK || c.ping().is_ok() { touch(servers, &c); return Ok(c); }
    }
    let c = Arc::new(open(servers)?);
    touch(servers, &c);
    Ok(c)
}
fn touch(servers: &Servers, c: &Arc<Client>) {
    if let Ok(mut g) = POOL.lock() { g.get_or_insert_with(HashMap::new).insert(servers.electrum.clone(), Held { c: c.clone(), used: Instant::now() }); }
}
/// Drop the kept connection after a failure: the next request opens a fresh one rather than reusing a broken one.
pub fn forget(servers: &Servers) { if let Ok(mut g) = POOL.lock() { if let Some(m) = g.as_mut() { m.remove(&servers.electrum); } } }
/// Run a read over the kept connection; on failure, once more over a fresh one (a server may have closed it).
fn read<T>(servers: &Servers, f: impl Fn(&Client) -> Result<T, electrum_client::Error>) -> Result<T, String> {
    match f(&*client(servers)?) {
        Ok(v) => Ok(v),
        Err(_) => { forget(servers); f(&*client(servers)?).map_err(|e| { forget(servers); e.to_string() }) }
    }
}
const BATCH: usize = 100;

#[cfg(test)]
pub fn coins(servers: &Servers, spk: &ScriptBuf) -> Result<Vec<Coin>, String> { Ok(unspents(servers, std::slice::from_ref(spk))?.pop().unwrap_or_default()) }

/// Unspent coins at many scripts, in one request per hundred.
pub fn unspents(servers: &Servers, spks: &[ScriptBuf]) -> Result<Vec<Vec<Coin>>, String> {
    let mut out = Vec::new();
    for chunk in spks.chunks(BATCH) {
        let lists = read(servers, |c| c.batch_script_list_unspent(chunk.iter().map(|s| s.as_script()))).map_err(|e| format!("listunspent: {e}"))?;
        out.extend(lists.into_iter().map(|l| l.into_iter().map(|u| Coin { txid: u.tx_hash.to_string(), vout: u.tx_pos as u32, value: u.value, height: u.height as u32 }).collect()));
    }
    Ok(out)
}

pub fn height(servers: &Servers) -> Result<u32, String> {
    read(servers, |c| c.block_headers_subscribe()).map(|h| h.height as u32).map_err(|e| format!("headers: {e}"))
}
/// The tip over a NEW connection: for the light that says whether the server answers right now, which a kept
/// connection's answer must not stand in for.
pub fn height_fresh(servers: &Servers) -> Result<u32, String> {
    crate::net::out("the node")?;
    open(servers)?.block_headers_subscribe().map(|h| h.height as u32).map_err(|e| format!("headers: {e}"))
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryItem { pub txid: String, pub height: i32 }

#[cfg(test)]
pub fn history(servers: &Servers, spk: &ScriptBuf) -> Result<Vec<HistoryItem>, String> { Ok(histories(servers, std::slice::from_ref(spk))?.pop().unwrap_or_default()) }

/// The history of many scripts, in one request per hundred, in the order asked.
pub fn histories(servers: &Servers, spks: &[ScriptBuf]) -> Result<Vec<Vec<HistoryItem>>, String> {
    let mut out = Vec::new();
    for chunk in spks.chunks(BATCH) {
        let lists = read(servers, |c| c.batch_script_get_history(chunk.iter().map(|s| s.as_script()))).map_err(|e| format!("history: {e}"))?;
        out.extend(lists.into_iter().map(|l| l.into_iter().map(|i| HistoryItem { txid: i.tx_hash.to_string(), height: i.height }).collect()));
    }
    Ok(out)
}

/// Whole transactions, in one request per hundred, in the order asked.
pub fn transactions(servers: &Servers, ids: &[Txid]) -> Result<Vec<Transaction>, String> {
    let mut out = Vec::new();
    for chunk in ids.chunks(BATCH) { out.extend(read(servers, |c| c.batch_transaction_get(chunk.iter())).map_err(|e| format!("transactions: {e}"))?); }
    Ok(out)
}

/// Transactions from the chain cache where kept, the rest in one batched request, in the order asked. Each fetched one
/// must hash to the id asked for; those `confirmed` says are in a block are kept, and never fetched again.
pub fn transactions_kept(servers: &Servers, net: &str, ids: &[Txid], confirmed: &dyn Fn(&Txid) -> bool) -> Result<Vec<Transaction>, String> {
    let kept = crate::db::txs(net, ids);
    let missing: Vec<Txid> = ids.iter().zip(&kept).filter(|(_, k)| k.is_none()).map(|(i, _)| *i).collect();
    let fetched = if missing.is_empty() { Vec::new() } else { transactions(servers, &missing)? };
    if let Some((id, _)) = missing.iter().zip(&fetched).find(|(id, t)| t.compute_txid() != **id) { return Err(format!("the server answered {} with a different transaction", &id.to_string()[..8])); }
    let keep: Vec<&Transaction> = missing.iter().zip(&fetched).filter(|(id, _)| confirmed(id)).map(|(_, t)| t).collect();
    if !keep.is_empty() { crate::db::keep_txs(net, &keep); }
    let mut fresh = fetched.into_iter();
    kept.into_iter().map(|k| k.or_else(|| fresh.next()).ok_or_else(|| "a transaction went missing".to_string())).collect()
}

/// Block times at heights, in one request per hundred.
pub fn header_times(servers: &Servers, heights: &[u32]) -> Result<Vec<u32>, String> {
    let mut out = Vec::new();
    for chunk in heights.chunks(BATCH) { out.extend(read(servers, |c| c.batch_block_header(chunk.iter().copied())).map_err(|e| format!("headers: {e}"))?.into_iter().map(|h| h.time)); }
    Ok(out)
}

pub fn broadcast(servers: &Servers, tx: &Transaction) -> Result<Txid, String> {
    crate::net::out("the node")?;
    let c = open(servers)?;
    c.transaction_broadcast(tx).map_err(|e| format!("broadcast refused: {e}"))
}

pub fn fees(servers: &Servers) -> Result<Fees, String> {
    crate::net::out("fee rates")?;
    let url = format!("{}/api/v1/fees/recommended", servers.explorer.trim_end_matches('/'));
    let http = reqwest::blocking::Client::builder().user_agent("anytwokeys/0.1").timeout(std::time::Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let v: serde_json::Value = http.get(&url).send().map_err(|e| format!("fees: {e}"))?.json().map_err(|e| format!("fees: {e}"))?;
    let g = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(1.0);
    Ok(Fees { slow: g("hourFee").max(1.0), medium: g("halfHourFee").max(1.0), fast: g("fastestFee").max(1.0) })
}

pub fn tx_link(servers: &Servers, txid: &str) -> String {
    format!("{}/tx/{txid}", servers.explorer.trim_end_matches('/'))
}
