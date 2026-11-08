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
        _ => Servers { electrum: "ssl://electrum.blockstream.info:60002".into(), explorer: "https://signet.mineracks.com".into() },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Coin {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub height: u32,   // 0 = unconfirmed
}

#[derive(Debug, Clone, Serialize)]
pub struct Fees { pub slow: f64, pub medium: f64, pub fast: f64 }

// Connections to a server, kept open and LEASED one caller at a time. Opening one is a TCP and TLS handshake (~80 ms
// to ours), so they are kept; but the Electrum client's way of sharing one connection between threads (one thread
// reads for all and hands over) left callers parked for a reply nobody was reading — three syncs of the overview, for
// a minute and more, on 8 Nov 2026. So no sharing: a caller takes a connection out of the pool, has it to itself for
// the request, and puts it back. Up to LANES per server are open; a caller beyond that waits for one to come back.
// A connection idle for a while is pinged before use, and one that failed is dropped, never returned.
struct Held { c: Arc<Client>, used: Instant }
#[derive(Default)]
struct Lanes { idle: Vec<Held>, open: usize }
static POOL: std::sync::LazyLock<Mutex<HashMap<String, Lanes>>> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static FREE: std::sync::Condvar = std::sync::Condvar::new();
const IDLE_CHECK: Duration = Duration::from_secs(30);
const LANES: usize = 6;
const WAIT: Duration = Duration::from_secs(90);

/// One connection, this caller's alone until it is dropped. Dropping it returns it to the pool; `broken()` first
/// and it is closed instead.
pub struct Lease { c: Option<Arc<Client>>, server: String, broken: bool }
impl std::ops::Deref for Lease { type Target = Client; fn deref(&self) -> &Client { self.c.as_ref().expect("a leased connection") } }
impl Lease {
    /// The connection misbehaved: close it rather than hand it to the next caller.
    pub fn broken(&mut self) { self.broken = true; }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let c = self.c.take();
        if let Ok(mut g) = POOL.lock() {
            let l = g.entry(self.server.clone()).or_default();
            if self.broken { l.open = l.open.saturating_sub(1); } else if let Some(c) = c { l.idle.push(Held { c, used: Instant::now() }); }
        }
        FREE.notify_one();
    }
}

fn open(servers: &Servers) -> Result<Client, String> {
    let cfg = ConfigBuilder::new().timeout(Some(20)).validate_domain(true).build();
    Client::from_config(&servers.electrum, cfg).map_err(|e| format!("electrum {}: {e}", servers.electrum))
}
/// A connection to this server for the caller alone: a kept one, a fresh one if there is room, or a wait for one.
pub fn client(servers: &Servers) -> Result<Lease, String> {
    crate::net::out("the node")?;
    let deadline = Instant::now() + WAIT;
    let mut g = POOL.lock().map_err(|_| "connection pool poisoned")?;
    loop {
        let l = g.entry(servers.electrum.clone()).or_default();
        if let Some(h) = l.idle.pop() {
            let idle = h.used.elapsed();
            drop(g);
            if idle < IDLE_CHECK || h.c.ping().is_ok() { return Ok(Lease { c: Some(h.c), server: servers.electrum.clone(), broken: false }); }
            // a kept connection that no longer answers: closed, and its place taken by a fresh one
            g = POOL.lock().map_err(|_| "connection pool poisoned")?;
            g.entry(servers.electrum.clone()).or_default().open = g.entry(servers.electrum.clone()).or_default().open.saturating_sub(1);
            continue;
        }
        if l.open < LANES {
            l.open += 1;
            drop(g);
            match open(servers) {
                Ok(c) => return Ok(Lease { c: Some(Arc::new(c)), server: servers.electrum.clone(), broken: false }),
                Err(e) => { if let Ok(mut g) = POOL.lock() { g.entry(servers.electrum.clone()).or_default().open -= 1; } FREE.notify_one(); return Err(e); }
            }
        }
        let now = Instant::now();
        if now >= deadline { return Err(format!("electrum {}: every connection is busy", servers.electrum)); }
        let (g2, _) = FREE.wait_timeout(g, deadline - now).map_err(|_| "connection pool poisoned")?;
        g = g2;
    }
}
/// Drop the kept connections to a server after a failure: the next request opens a fresh one rather than reusing a
/// broken one. (A leased connection that failed is closed by its own lease; this is for a server-wide failure.)
pub fn forget(servers: &Servers) { if let Ok(mut g) = POOL.lock() { if let Some(l) = g.get_mut(&servers.electrum) { l.open = l.open.saturating_sub(l.idle.len()); l.idle.clear(); } } FREE.notify_all(); }
/// Run a read over a leased connection; on failure, once more over a fresh one (a server may have closed it).
fn read<T>(servers: &Servers, f: impl Fn(&Client) -> Result<T, electrum_client::Error>) -> Result<T, String> {
    let mut lease = client(servers)?;
    match f(&lease) {
        Ok(v) => Ok(v),
        Err(_) => { lease.broken(); drop(lease); let mut again = client(servers)?; f(&again).map_err(|e| { again.broken(); e.to_string() }) }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

/// Each script's STATUS: Electrum's hash of its whole history, mempool included, or None for a script never seen.
/// One small answer per script, in one request per hundred — the cheap first question, before any history is asked.
pub fn statuses(servers: &Servers, spks: &[ScriptBuf]) -> Result<Vec<Option<String>>, String> {
    let mut out = Vec::new();
    for chunk in spks.chunks(BATCH) {
        let list = read(servers, |c| c.batch_script_subscribe(chunk.iter().map(|s| s.as_script()))).map_err(|e| format!("status: {e}"))?;
        out.extend(list.into_iter().map(|s| s.map(|h| hex::encode(*h))));
    }
    Ok(out)
}

/// Histories, asked for only where the status has changed since they were last kept (db.rs): a refresh in which
/// nothing moved asks each script one small question and fetches nothing. A script never seen has no history and
/// is not asked at all. Without the cache (locked, or none) every history is fetched, as before.
pub fn histories_known(servers: &Servers, net: &str, spks: &[ScriptBuf]) -> Result<Vec<Vec<HistoryItem>>, String> {
    let status = statuses(servers, spks)?;
    let known = crate::db::spk_get(net, spks);
    let (mut out, ask) = plan(&status, &known);
    if !ask.is_empty() {
        let fetched = histories(servers, &ask.iter().map(|&i| spks[i].clone()).collect::<Vec<_>>())?;
        let mut keep = Vec::new();
        for (&i, h) in ask.iter().zip(fetched) {
            if let Some(s) = &status[i] { keep.push(crate::db::Known { status: s.clone(), history: h.clone(), coins: None }); }
            out[i] = Some(h);
        }
        crate::db::spk_put(net, &ask.iter().map(|&i| spks[i].clone()).collect::<Vec<_>>(), &keep);
    }
    Ok(out.into_iter().map(|h| h.unwrap_or_default()).collect())
}

/// Which scripts need their history fetched: a script never seen has none; one whose status is what it was when its
/// history was kept has that history; every other is asked.
fn plan(status: &[Option<String>], known: &[Option<crate::db::Known>]) -> (Vec<Option<Vec<HistoryItem>>>, Vec<usize>) {
    let mut out: Vec<Option<Vec<HistoryItem>>> = vec![None; status.len()];
    let mut ask = Vec::new();
    for (i, st) in status.iter().enumerate() {
        match (st, known.get(i).and_then(|k| k.as_ref())) {
            (None, _) => out[i] = Some(Vec::new()),
            (Some(s), Some(k)) if &k.status == s => out[i] = Some(k.history.clone()),
            _ => ask.push(i),
        }
    }
    (out, ask)
}

/// Unspent coins, from the cache where it holds them for the script's current status, fetched otherwise.
pub fn unspents_known(servers: &Servers, net: &str, spks: &[ScriptBuf]) -> Result<Vec<Vec<Coin>>, String> {
    let known = crate::db::spk_get(net, spks);
    let mut out: Vec<Option<Vec<Coin>>> = known.iter().map(|k| k.as_ref().and_then(|k| k.coins.clone())).collect();
    let ask: Vec<usize> = (0..spks.len()).filter(|&i| out[i].is_none()).collect();
    if !ask.is_empty() {
        let fetched = unspents(servers, &ask.iter().map(|&i| spks[i].clone()).collect::<Vec<_>>())?;
        let (mut put_spks, mut put) = (Vec::new(), Vec::new());
        for (&i, c) in ask.iter().zip(fetched) {
            if let Some(k) = &known[i] { put_spks.push(spks[i].clone()); put.push(crate::db::Known { status: k.status.clone(), history: k.history.clone(), coins: Some(c.clone()) }); }
            out[i] = Some(c);
        }
        crate::db::spk_put(net, &put_spks, &put);
    }
    Ok(out.into_iter().map(|c| c.unwrap_or_default()).collect())
}

/// Whole transactions, in one request per hundred, in the order asked.
pub fn transactions(servers: &Servers, ids: &[Txid]) -> Result<Vec<Transaction>, String> {
    let mut out = Vec::new();
    for chunk in ids.chunks(BATCH) { out.extend(read(servers, |c| c.batch_transaction_get(chunk.iter())).map_err(|e| format!("transactions: {e}"))?); }
    Ok(out)
}

/// Each transaction on its own: Ok(Some) when the chain has it, Ok(None) when NO server has a transaction with that
/// id (the answer that proves a phantom), Err when the server could not be asked. One request each, after the batch
/// has failed: the batch fails as a whole on one unknown id, and this tells which.
pub fn transactions_each(servers: &Servers, ids: &[Txid]) -> Vec<Result<Option<Transaction>, String>> {
    ids.iter().map(|id| match read(servers, |c| c.transaction_get(id)) {
        Ok(t) => Ok(Some(t)),
        Err(e) => { let m = e.to_lowercase(); if m.contains("no such") || m.contains("not found") || m.contains("missing transaction") || m.contains("unknown") { Ok(None) } else { Err(e) } }
    }).collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Known;
    fn h(t: &str) -> Vec<HistoryItem> { vec![HistoryItem { txid: t.into(), height: 1 }] }
    #[test]
    fn only_a_changed_status_asks_for_history() {
        let status = vec![None, Some("s1".to_string()), Some("s2".to_string()), Some("s3".to_string())];
        let known = vec![None, Some(Known { status: "s1".into(), history: h("kept"), coins: None }), Some(Known { status: "old".into(), history: h("stale"), coins: None }), None];
        let (out, ask) = plan(&status, &known);
        assert_eq!(out[0], Some(vec![]), "never seen: no history, nothing asked");
        assert_eq!(out[1], Some(h("kept")), "same status: the kept history");
        assert_eq!(out[2], None); assert_eq!(out[3], None);
        assert_eq!(ask, vec![2, 3], "a changed status and an unknown script are asked, nothing else");
    }
    #[test]
    fn a_status_is_never_taken_for_history_when_the_cache_is_empty() {
        let (out, ask) = plan(&[Some("s".into()), Some("t".into())], &[]);
        assert_eq!(out, vec![None, None]); assert_eq!(ask, vec![0, 1]);
    }
    /// Our own server, two mainnet scripts: one with history (the genesis coinbase address), one that has never existed.
    #[test] #[ignore]
    fn statuses_live() {
        use std::str::FromStr;
        let s = defaults(Network::Bitcoin);
        let used = bitcoin::Address::from_str("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa").unwrap().assume_checked().script_pubkey();
        let fresh = ScriptBuf::from(vec![0x00u8, 0x14, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]);
        let st = statuses(&s, &[used.clone(), fresh.clone()]).unwrap();
        assert!(st[0].as_ref().map(|x| x.len() == 64).unwrap_or(false), "{st:?}"); assert!(st[1].is_none());
        let hs = histories_known(&s, "mainnet", &[used, fresh]).unwrap();
        assert!(hs[0].len() > 100 && hs[1].is_empty());
    }
}
