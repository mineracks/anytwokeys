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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
