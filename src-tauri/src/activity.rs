//! What the Home screen says about each transaction beyond its id: when, how much moved, and what that was worth.
//! Times come from block headers, amounts from the transactions themselves, fiat from the price service
//! (hourly candle × the ECB cross rate, falling back to the daily price). Block times and past prices never change,
//! so they are kept in `cache.json`. A lookup that FAILS is left empty and is not cached: no answer is never a zero.
use crate::{chain::Servers, wallet};
use bitcoin::{Transaction, Txid};
use electrum_client::ElectrumApi;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

pub const PRICES: &str = "https://prices.mineracks.com";

#[derive(Default, Serialize, Deserialize)]
struct Cache { #[serde(default)] times: HashMap<String, u32>, #[serde(default)] prices: HashMap<String, Quote> }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quote { pub price: f64, pub basis: String }   // basis: "hourly 2026-09-18 08:00 UTC" | "daily 2026-09-19"

#[derive(Debug, Serialize)]
pub struct Row { pub txid: String, pub time: Option<u32>, pub net: Option<i64>, pub fee: Option<u64>, pub quote: Option<Quote>,
    /// the lineage of the vault's own coins through this transaction: which of its coins went IN ("txid:vout", sats),
    /// and which outputs came back to it (vout, sats). Enough to follow each coin's cost from the day it arrived to the
    /// day it left — so a gain is measured against the coins actually spent, which the chain itself records.
    #[serde(default)] pub ins: Vec<(String, u64)>, #[serde(default)] pub outs: Vec<(u32, u64)> }
#[derive(Debug, Serialize)]
pub struct Activity { pub ccy: String, pub rows: Vec<Row>, pub now: Option<Quote>, pub note: Option<String> }

fn cache_path() -> std::path::PathBuf { wallet::store_path().with_file_name("cache.json") }
// a cache that cannot be read is an empty cache: everything in it can be asked for again
fn load_cache() -> Cache {
    let bytes = if crate::lockbox::sealed_mode() { crate::lockbox::read_sealed(&cache_path().with_extension("enc"), "cache").ok().flatten() } else { std::fs::read(cache_path()).ok() };
    bytes.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}
fn save_cache(c: &Cache) {
    let Ok(b) = serde_json::to_vec(c) else { return };
    if crate::lockbox::sealed_mode() { let _ = crate::lockbox::write_sealed(&cache_path().with_extension("enc"), "cache", &b); } else { let _ = std::fs::write(cache_path(), b); }
}

/// (day, hour) in UTC for a Unix time.
fn day_hour(t: u32) -> (String, u32) { (crate::export::ymd(t as u64), (t % 86_400) / 3_600) }

/// Parse the price service's answer for one currency. `None` for anything that is not a positive finite price.
pub fn read_quote(body: &serde_json::Value, ccy: &str, day: &str) -> Option<Quote> {
    let price = body.get("prices").and_then(|p| p.get(ccy)).and_then(|v| v.as_f64())
        .or_else(|| if ccy == "USD" { body.get("btc_usd").and_then(|v| v.as_f64()) } else { None })
        .filter(|p| p.is_finite() && *p > 0.0)?;
    let basis = match body.get("candle").and_then(|c| c.get("ts")).and_then(|t| t.as_str()) {
        Some(ts) if ts.len() >= 16 => format!("hourly {} {} UTC", &ts[..10], &ts[11..16]),
        _ => format!("daily {}", body.get("day").and_then(|d| d.as_str()).unwrap_or(day)),
    };
    Some(Quote { price, basis })
}

fn fetch(http: &reqwest::blocking::Client, base: &str, ccy: &str, day: &str, hour: Option<u32>) -> Option<Quote> {
    let ask = |with_time: bool| -> Option<Quote> {
        let mut url = format!("{}/v1/price?day={day}&ccy={ccy}", base.trim_end_matches('/'));
        if let (true, Some(h)) = (with_time, hour) { url.push_str(&format!("&time={h:02}:00")); }   // the hour, never the minute: the server need not learn more
        crate::net::out("prices").ok()?;
        let r = http.get(&url).send().ok()?;
        if !r.status().is_success() { return None; }
        read_quote(&r.json::<serde_json::Value>().ok()?, ccy, day)
    };
    if hour.is_some() { if let Some(q) = ask(true) { return Some(q); } }
    ask(false)
}

pub fn build(v: &wallet::Vault, servers: &Servers, history: &[(String, i32)], ccy: &str, prices_url: &str) -> Result<Activity, String> {
    let mut cache = load_cache();
    let mut dirty = false;
    // every transaction in the history, in one batched request; a spent coin's parent is always in the history too
    let ids = history.iter().map(|(t, _)| Txid::from_str(t).map_err(|e| e.to_string())).collect::<Result<Vec<_>, _>>()?;
    // confirmed ones come from the chain cache after the first time; only what is new or unconfirmed goes out
    let confirmed: std::collections::HashSet<Txid> = ids.iter().zip(history).filter(|(_, (_, h))| *h > 0).map(|(i, _)| *i).collect();
    let txs: HashMap<String, Transaction> = history.iter().map(|(t, _)| t.clone()).zip(crate::chain::transactions_kept(servers, &v.network, &ids, &|i| confirmed.contains(i))?).collect();
    // block times not yet known, in one batched request too (a header never changes, so each is asked once, ever)
    let want: Vec<u32> = { let mut w: Vec<u32> = history.iter().filter(|(_, h)| *h > 0 && !cache.times.contains_key(&format!("{}:{h}", v.network))).map(|(_, h)| *h as u32).collect(); w.sort(); w.dedup(); w };
    if !want.is_empty() { if let Ok(times) = crate::chain::header_times(servers, &want) { for (h, t) in want.iter().zip(times) { cache.times.insert(format!("{}:{h}", v.network), t); dirty = true; } } }
    let table = wallet::ours_table(v)?;
    let ours = |spk: &bitcoin::ScriptBuf| table(spk).is_some();
    let fiat = v.network == "mainnet" && !ccy.is_empty() && !prices_url.is_empty();
    let http = reqwest::blocking::Client::builder().user_agent("anytwokeys/0.1").timeout(std::time::Duration::from_secs(12)).build().map_err(|e| e.to_string())?;
    let mut price_failed = false;
    let mut rows = Vec::new();
    for (txid, height) in history {
        let tx = &txs[txid];
        let received: u64 = tx.output.iter().filter(|o| ours(&o.script_pubkey)).map(|o| o.value.to_sat()).sum();
        let mut spent = 0u64; let mut all_ours = !tx.input.is_empty(); let mut ins = Vec::new();
        for i in &tx.input {
            match txs.get(&i.previous_output.txid.to_string()).and_then(|p| p.output.get(i.previous_output.vout as usize)).filter(|o| ours(&o.script_pubkey)) {
                Some(o) => { spent += o.value.to_sat(); ins.push((i.previous_output.to_string(), o.value.to_sat())); } None => all_ours = false }
        }
        let outs: Vec<(u32, u64)> = tx.output.iter().enumerate().filter(|(_, o)| ours(&o.script_pubkey)).map(|(n, o)| (n as u32, o.value.to_sat())).collect();
        let fee = if all_ours { Some(spent.saturating_sub(tx.output.iter().map(|o| o.value.to_sat()).sum())) } else { None };
        let time = if *height > 0 {
            let key = format!("{}:{height}", v.network);
            cache.times.get(&key).copied()
        } else { None };
        let quote = match (fiat, time) {
            (true, Some(t)) => { let (day, hour) = day_hour(t); let key = format!("{ccy}:{day}T{hour:02}");
                match cache.prices.get(&key) { Some(q) => Some(q.clone()), None => { let q = fetch(&http, prices_url, ccy, &day, Some(hour)); match &q { Some(q) => { cache.prices.insert(key, q.clone()); dirty = true; } None => price_failed = true } q } } }
            _ => None,
        };
        rows.push(Row { txid: txid.clone(), time, net: Some(received as i64 - spent as i64), fee, quote, ins, outs });
    }
    let now = if fiat { let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0); let (day, hour) = day_hour(t);
        let q = fetch(&http, prices_url, ccy, &day, Some(hour)); if q.is_none() { price_failed = true; } q } else { None };
    if dirty { save_cache(&cache); }
    let note = if v.network != "mainnet" { Some("Test coins have no price.".into()) } else if !fiat { None } else if price_failed { Some("Some prices could not be fetched; they are left blank rather than guessed.".into()) } else { None };
    Ok(Activity { ccy: ccy.to_string(), rows, now, note })
}

/// The price at one moment, from the cache when it is there. Used by a sheet that prints one transaction long after
/// the screen worked the same number out; `None` when there is no price to be had, never a guess.
pub fn quote_at(prices_url: &str, ccy: &str, t: Option<u32>) -> Option<Quote> {
    if ccy.is_empty() || prices_url.is_empty() { return None; }
    let t = t.or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs() as u32))?;
    let (day, hour) = day_hour(t);
    let key = format!("{ccy}:{day}T{hour:02}");
    let mut cache = load_cache();
    if let Some(q) = cache.prices.get(&key) { return Some(q.clone()); }
    let http = reqwest::blocking::Client::builder().user_agent("anytwokeys/0.1").timeout(std::time::Duration::from_secs(12)).build().ok()?;
    let q = fetch(&http, prices_url, ccy, &day, Some(hour))?;
    cache.prices.insert(key, q.clone());
    save_cache(&cache);
    Some(q)
}

/// The step between days for a span, so a chart never needs more than about a year of points: every day up to 365
/// days, then every other day, and so on. The price service refuses a request of more than 400 points outright.
pub fn series_every(from: &str, to: &str) -> u32 {
    let day = |d: &str| -> Option<i64> { let (y, m, dd) = (d.get(0..4)?.parse::<i64>().ok()?, d.get(5..7)?.parse::<i64>().ok()?, d.get(8..10)?.parse::<i64>().ok()?);
        // days from civil (Howard Hinnant), enough to count the days between two dates
        let y = if m <= 2 { y - 1 } else { y }; let era = y.div_euclid(400); let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + dd - 1; let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Some(era * 146097 + doe) };
    let span = match (day(from), day(to)) { (Some(a), Some(b)) if b > a => (b - a) as u32, _ => 0 };
    (span / 365 + 1).clamp(1, 31)
}

/// One run of daily closes from the price service's `series` question: ONE request where the chart used to make up
/// to 36. Each day is exactly what the service says for that day asked alone. Past days go into the sealed cache
/// for good — a closed day's price never changes — so a range already seen costs no request at all; today's is not
/// kept, the day has not closed. A day with no price is a hole, `None`, never filled from its neighbour.
pub fn price_series(prices_url: &str, ccy: &str, from: &str, to: &str, every: u32) -> Result<Vec<(String, Option<Quote>)>, String> {
    let today = crate::export::ymd(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
    let mut cache = load_cache();
    // the cache answers when it holds every closed day the service would return
    let cached = |c: &Cache, d: &str| c.prices.get(&format!("{ccy}:{d}")).cloned();
    let url = format!("{}/v1/series?from={from}&to={to}&every={every}&ccy={ccy}", prices_url.trim_end_matches('/'));
    let http = reqwest::blocking::Client::builder().user_agent("anytwokeys/0.1").timeout(std::time::Duration::from_secs(60)).build().map_err(|e| e.to_string())?;
    let mut body = None;
    for attempt in 0..2 {   // one request a second at the front: a second chart drawn a moment later waits, then asks
        crate::net::out("prices")?;
        let r = http.get(&url).send().map_err(|_| "the price service could not be reached".to_string())?;
        if r.status().as_u16() == 429 && attempt == 0 { std::thread::sleep(std::time::Duration::from_millis(1100)); continue; }
        if !r.status().is_success() { return Err(format!("the price service said {}", r.status())); }
        body = Some(r.json::<serde_json::Value>().map_err(|_| "the price service's answer was not readable".to_string())?);
        break;
    }
    let body = body.ok_or("the price service is busy; try again in a moment")?;
    let points = read_series(&body, ccy);
    let mut dirty = false;
    for (d, q) in &points { if let (Some(q), true) = (q, d.as_str() < today.as_str()) { if cached(&cache, d).is_none() { cache.prices.insert(format!("{ccy}:{d}"), q.clone()); dirty = true; } } }
    if dirty { save_cache(&cache); }
    Ok(points)
}

/// The days out of a `series` answer. A point whose `prices` is null is a hole the service could not fill, and stays one.
pub fn read_series(body: &serde_json::Value, ccy: &str) -> Vec<(String, Option<Quote>)> {
    body.get("points").and_then(|p| p.as_array()).map(|pts| pts.iter().filter_map(|p| {
        let day = p.get("day")?.as_str()?.to_string();
        let q = if p.get("prices").map(|v| v.is_null()).unwrap_or(true) { None } else { read_quote(p, ccy, &day) };
        Some((day, q))
    }).collect()).unwrap_or_default()
}

// ---------- one transaction, in full ----------
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct TxInput { pub outpoint: String, pub address: Option<String>, pub sats: Option<u64>, pub ours: Option<u32>, pub sequence: u32, pub witness: usize, pub signers: Vec<String> }
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct TxOutput { pub n: usize, pub address: Option<String>, pub sats: u64, pub kind: String, pub ours: Option<u32> }
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct Detail { pub txid: String, pub wtxid: String, pub version: i32, pub locktime: u32, pub size: usize, pub vsize: usize, pub weight: u64, pub rbf: bool,
    pub net: i64, pub fee: Option<u64>, pub fee_rate: Option<f64>, pub height: i32, pub tip: u32, pub block: Option<String>, pub time: Option<u32>,
    /// "proved" — the server's merkle path hashes up to the root in the block header; "unconfirmed"; or why it could not be shown
    pub proof: String, pub position: Option<usize>, pub inputs: Vec<TxInput>, pub outputs: Vec<TxOutput>, pub hex: String }

fn kind_of(spk: &bitcoin::ScriptBuf) -> &'static str {
    if spk.is_p2wpkh() { "P2WPKH" } else if spk.is_p2wsh() { "P2WSH" } else if spk.is_p2tr() { "P2TR" } else if spk.is_p2pkh() { "P2PKH" } else if spk.is_p2sh() { "P2SH" } else if spk.is_op_return() { "OP_RETURN" } else { "other" }
}

/// Which of the vault's keys made the signatures on input `i`, read off the witness: each signature is checked against each
/// key's child at that address until one verifies. Nothing is taken on trust from a label.
fn signers_of(v: &wallet::Vault, tx: &Transaction, i: usize, index: u32, value: bitcoin::Amount) -> Vec<String> {
    use bitcoin::sighash::{EcdsaSighashType, SighashCache};
    let secp = bitcoin::secp256k1::Secp256k1::verification_only();
    let wit: Vec<&[u8]> = tx.input[i].witness.iter().collect();
    let Some(script) = wit.last().filter(|_| v.keys.len() > 1).map(|b| bitcoin::ScriptBuf::from_bytes(b.to_vec())) else { return vec![] };
    let children: Vec<(String, bitcoin::secp256k1::PublicKey)> = v.keys.iter().filter_map(|k| wallet::child_key(v, &k.pubkey, index).ok().map(|c| (k.name.clone(), c))).collect();
    let mut cache = SighashCache::new(tx);
    let mut out = Vec::new();
    for sig in &wit[..wit.len() - 1] {
        let Some((ty, der)) = sig.split_last() else { continue };   // the empty item multisig needs is skipped here
        let (Ok(ty), Ok(parsed)) = (EcdsaSighashType::from_standard(*ty as u32), bitcoin::secp256k1::ecdsa::Signature::from_der(der)) else { continue };
        let Ok(h) = cache.p2wsh_signature_hash(i, &script, value, ty) else { continue };
        let msg = bitcoin::secp256k1::Message::from_digest(bitcoin::hashes::Hash::to_byte_array(h));
        if let Some((name, _)) = children.iter().find(|(_, pk)| secp.verify_ecdsa(&msg, &parsed, pk).is_ok()) { out.push(name.clone()); }
    }
    out
}

pub fn detail(v: &wallet::Vault, servers: &Servers, txid: &str, height: i32) -> Result<Detail, String> {
    use bitcoin::consensus::encode::serialize_hex;
    let client = crate::chain::client(servers)?;
    let id = Txid::from_str(txid).map_err(|_| "that is not a transaction id".to_string())?;
    let tx = crate::chain::transactions_kept(servers, &v.network, &[id], &|_| height > 0).map_err(|e| format!("transaction {}: {e}", &txid[..8.min(txid.len())]))?.remove(0);
    let net_kind = wallet::network_of(&v.network);
    let addr = |spk: &bitcoin::ScriptBuf| bitcoin::Address::from_script(spk, net_kind).ok().map(|a| a.to_string());
    let table = wallet::ours_table(v)?;
    let ours = |spk: &bitcoin::ScriptBuf| table(spk);
    // what each input spent: its parent has to be fetched. Past 40 inputs the amounts are left out rather than hammering the server.
    let mut parents: HashMap<Txid, Transaction> = HashMap::new();
    let (mut total_in, mut complete) = (0u64, tx.input.len() <= 40 && !tx.is_coinbase());
    if complete {
        // every parent in one batched request
        let mut want: Vec<Txid> = tx.input.iter().map(|i| i.previous_output.txid).collect(); want.sort(); want.dedup();
        // a confirmed transaction's parents are confirmed too, so they are kept alike
        if let Ok(got) = crate::chain::transactions_kept(servers, &v.network, &want, &|_| height > 0) { parents.extend(want.into_iter().zip(got)); }
    }
    let mut inputs = Vec::new();
    for (i, inp) in tx.input.iter().enumerate() {
        let prev = if complete { if !parents.contains_key(&inp.previous_output.txid) { match client.transaction_get(&inp.previous_output.txid) { Ok(p) => { parents.insert(inp.previous_output.txid, p); } Err(_) => complete = false } }
            parents.get(&inp.previous_output.txid).and_then(|p| p.output.get(inp.previous_output.vout as usize)).cloned() } else { None };
        if prev.is_none() { complete = false; }
        let mine = prev.as_ref().and_then(|o| ours(&o.script_pubkey));
        if let Some(o) = &prev { total_in += o.value.to_sat(); }
        let signers = match (&prev, mine) { (Some(o), Some(index)) => signers_of(v, &tx, i, index, o.value), _ => vec![] };
        inputs.push(TxInput { outpoint: inp.previous_output.to_string(), address: prev.as_ref().and_then(|o| addr(&o.script_pubkey)), sats: prev.as_ref().map(|o| o.value.to_sat()),
            ours: mine, sequence: inp.sequence.0, witness: inp.witness.len(), signers });
    }
    let outputs: Vec<TxOutput> = tx.output.iter().enumerate().map(|(n, o)| TxOutput { n, address: addr(&o.script_pubkey), sats: o.value.to_sat(), kind: kind_of(&o.script_pubkey).into(), ours: ours(&o.script_pubkey) }).collect();
    let total_out: u64 = outputs.iter().map(|o| o.sats).sum();
    let net = outputs.iter().filter(|o| o.ours.is_some()).map(|o| o.sats as i64).sum::<i64>() - inputs.iter().filter(|i| i.ours.is_some()).map(|i| i.sats.unwrap_or(0) as i64).sum::<i64>();
    let fee = if complete { Some(total_in.saturating_sub(total_out)) } else { None };
    let (weight, vsize) = (tx.weight().to_wu(), tx.vsize());
    let tip = crate::chain::height(servers).unwrap_or(0);
    // the server SAYS the transaction is in that block; the merkle path, hashed up to the header's root, shows it
    let (mut block, mut time, mut position, mut proof) = (None, None, None, "unconfirmed".to_string());
    if height > 0 {
        proof = match (client.block_header(height as usize), client.transaction_get_merkle(&id, height as usize)) {
            (Ok(h), Ok(m)) => { block = Some(h.block_hash().to_string()); time = Some(h.time); position = Some(m.pos);
                if electrum_client::utils::validate_merkle_proof(&id, &h.merkle_root, &m) { "proved".into() } else { "the server's proof does NOT lead to that block's root".into() } }
            (Ok(h), Err(e)) => { block = Some(h.block_hash().to_string()); time = Some(h.time); format!("the server gave no proof: {e}") }
            (Err(e), _) => format!("the block header could not be fetched: {e}"),
        };
    }
    Ok(Detail { txid: id.to_string(), wtxid: tx.compute_wtxid().to_string(), version: tx.version.0, locktime: tx.lock_time.to_consensus_u32(), size: tx.total_size(), vsize, weight,
        rbf: tx.input.iter().any(|i| i.sequence.is_rbf()), net, fee, fee_rate: fee.map(|f| f as f64 / vsize.max(1) as f64), height, tip, block, time, proof, position, inputs, outputs, hex: serialize_hex(&tx) })
}

/// What the operating system says about where this person is: the Region setting, which does not move when they
/// travel and does not depend on the language they read. macOS: AppleLocale ("en_AU", "en_US@rg=auzzzz",
/// "en_AU@currency=USD"); elsewhere the POSIX locale ("en_AU.UTF-8").
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Device { pub region: Option<String>, pub currency: Option<String> }

pub fn read_locale(id: &str) -> Device {
    let (base, ext) = id.trim().split_once('@').unwrap_or((id.trim(), ""));
    let key = |k: &str| ext.split(';').find_map(|kv| kv.strip_prefix(k).and_then(|v| v.strip_prefix('='))).map(|v| v.to_string());
    let upper2 = |r: &str| (r.len() >= 2 && r[..2].chars().all(|c| c.is_ascii_alphabetic())).then(|| r[..2].to_uppercase());
    let base = base.split('.').next().unwrap_or(base);
    let region = key("rg").and_then(|r| upper2(&r)).or_else(|| base.rsplit(['_', '-']).next().filter(|r| r.len() == 2 && *r != base).and_then(upper2));
    let currency = key("currency").filter(|c| c.len() == 3 && c.chars().all(|c| c.is_ascii_alphabetic())).map(|c| c.to_uppercase());
    Device { region, currency }
}

pub fn device() -> Device {
    #[cfg(target_os = "macos")]
    if let Ok(o) = std::process::Command::new("/usr/bin/defaults").args(["read", "-g", "AppleLocale"]).output() {
        if o.status.success() { return read_locale(&String::from_utf8_lossy(&o.stdout)); }
    }
    ["LC_ALL", "LC_MONETARY", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty() && v != "C" && v != "POSIX")).map(|v| read_locale(&v)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_price_answer_is_read_strictly() {
        // the two shapes prices.mineracks.com really returned on 2026-09-19 (hourly candle; daily fallback)
        let hourly: serde_json::Value = serde_json::from_str(r#"{"prices": {"AUD": 109811.15706806284, "USD": 78188.0}, "btc_usd": 78188.0, "when": "2026-09-18T08:26:00+00:00", "intraday": true, "candle": {"step": "hourly", "ts": "2026-09-18T08:00:00+00:00", "close": 78188.0}}"#).unwrap();
        let daily: serde_json::Value = serde_json::from_str(r#"{"prices": {"AUD": 114209.60035340313}, "btc_usd": 81319.79, "day": "2026-09-19", "intraday": false}"#).unwrap();
        assert_eq!(read_quote(&hourly, "AUD", "2026-09-18"), Some(Quote { price: 109811.15706806284, basis: "hourly 2026-09-18 08:00 UTC".into() }));
        assert_eq!(read_quote(&daily, "AUD", "2026-09-19").unwrap().basis, "daily 2026-09-19");
        // never a silent other currency, a zero, or an error body passed off as a price
        assert_eq!(read_quote(&hourly, "EUR", "2026-09-18"), None);
        assert_eq!(read_quote(&serde_json::json!({"error": "no intraday candle"}), "AUD", "x"), None);
        assert_eq!(read_quote(&serde_json::json!({"prices": {"AUD": 0.0}}), "AUD", "x"), None);
        assert_eq!(read_quote(&serde_json::json!({"prices": {"AUD": "109811"}}), "AUD", "x"), None);
        assert_eq!(day_hour(1_789_719_960), ("2026-09-18".to_string(), 8));
    }
    #[test]
    fn the_operating_systems_region_is_read_in_every_shape_it_comes_in() {
        let d = |r: Option<&str>, c: Option<&str>| Device { region: r.map(String::from), currency: c.map(String::from) };
        assert_eq!(read_locale("en_AU\n"), d(Some("AU"), None));
        assert_eq!(read_locale("en_US@rg=auzzzz"), d(Some("AU"), None));            // English (US) on a Mac whose Region is Australia
        assert_eq!(read_locale("en_AU@currency=USD"), d(Some("AU"), Some("USD")));  // the person overrode the currency in System Settings
        assert_eq!(read_locale("zh-Hans_CN"), d(Some("CN"), None));
        assert_eq!(read_locale("en_AU.UTF-8"), d(Some("AU"), None));
        assert_eq!(read_locale("en"), d(None, None));
        assert_eq!(read_locale(""), d(None, None));
    }
}

#[cfg(test)]
mod live {
    /// Reads the real store and talks to the real servers: `cargo test live_activity -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_detail() {
        let s = crate::wallet::load().unwrap();
        for v in s.vaults.iter() {
            let servers = crate::wallet::servers_for(&s, &v.network);
            let sc = crate::wallet::scan(v, &servers, s.settings.gap).unwrap();
            let mut hist: Vec<(String, i32)> = sc.history.iter().map(|(t, h, _)| (t.clone(), *h)).collect(); hist.sort(); hist.dedup();
            for (txid, h) in hist {
                let d = super::detail(v, &servers, &txid, h).unwrap();
                println!("== {} {} net={} fee={:?} rate={:?} vsize={} rbf={} proof={} pos={:?} block={:?}", v.name, &d.txid[..12], d.net, d.fee, d.fee_rate.map(|r| (r * 100.0).round() / 100.0), d.vsize, d.rbf, d.proof, d.position, d.block.as_deref().map(|b| &b[..16]));
                for i in &d.inputs { println!("   in  {} ours={:?} sats={:?} signers={:?}", &i.outpoint[..12], i.ours, i.sats, i.signers); }
                for o in &d.outputs { println!("   out #{} {} {} ours={:?}", o.n, o.kind, o.sats, o.ours); }
            }
        }
    }
    #[test]
    #[ignore]
    fn live_activity() {
        let s = crate::wallet::load().unwrap();
        for v in s.vaults.iter().filter(|v| v.network == "mainnet") {
            let servers = crate::wallet::servers_for(&s, &v.network);
            let sc = crate::wallet::scan(v, &servers, s.settings.gap).unwrap();
            let mut hist: Vec<(String, i32)> = sc.history.iter().map(|(t, h, _)| (t.clone(), *h)).collect(); hist.sort(); hist.dedup();
            let a = super::build(v, &servers, &hist, "AUD", super::PRICES).unwrap();
            println!("== {} · now {:?} · note {:?}", v.name, a.now, a.note);
            for r in &a.rows { println!("{} time={:?} ({}) net={:?} fee={:?} quote={:?}", &r.txid[..12], r.time, r.time.map(|t| crate::export::ymd(t as u64)).unwrap_or_default(), r.net, r.fee, r.quote); }
        }
    }
}

#[cfg(test)]
mod series_tests {
    use super::*;
    fn fx(name: &str) -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap() }

    #[test]
    fn a_series_reads_every_day_including_the_mt_gox_crash() {
        let p = read_series(&fx("series-aud-2013-04.json"), "AUD");
        assert_eq!(p.len(), 5);
        // fetched from prices.mineracks.com on 2026-09-21: A$219.45 on the 10th, A$151.25 on the 11th
        let q = |d: &str| p.iter().find(|x| x.0 == d).unwrap().1.clone().unwrap().price;
        assert!((q("2013-04-10") - 219.44982806052266).abs() < 1e-9 && (q("2013-04-11") - 151.2516716213126).abs() < 1e-9);
        assert_eq!(p[0].1.as_ref().unwrap().basis, "daily 2013-04-08");
    }
    #[test]
    fn a_day_with_no_price_is_a_hole_never_a_zero() {
        let p = read_series(&fx("series-holes-2009.json"), "AUD");
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|x| x.1.is_none()), "{p:?}");
    }
    #[test]
    fn usd_is_read_from_prices_and_from_btc_usd_for_an_older_service() {
        let p = read_series(&fx("series-usd-2026-09.json"), "USD");
        assert_eq!(p.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["2026-09-01", "2026-09-03", "2026-09-05"]);
        assert!((p[0].1.as_ref().unwrap().price - 78549.65).abs() < 1e-9);
        // the shape the service answered with before the fix: prices empty, the number only in btc_usd
        let old = serde_json::json!({ "day": "2026-09-01", "prices": {}, "btc_usd": 78549.65 });
        assert!((read_quote(&old, "USD", "2026-09-01").unwrap().price - 78549.65).abs() < 1e-9);
        assert!(read_quote(&old, "AUD", "2026-09-01").is_none(), "only USD may fall back to btc_usd");
    }
    #[test]
    fn a_long_span_steps_so_it_never_asks_for_more_than_the_service_allows() {
        assert_eq!(series_every("2026-01-01", "2026-09-21"), 1);
        assert_eq!(series_every("2025-01-01", "2026-09-21"), 2);
        assert_eq!(series_every("2011-08-18", "2026-09-21"), 16);
        for (a, b) in [("2011-08-18", "2026-09-21"), ("2020-02-29", "2024-02-29"), ("2026-09-21", "2026-09-21")] {
            let (e, span) = (series_every(a, b), { let d = |s: &str| chrono_days(s); d(b) - d(a) });
            assert!(span / e as i64 + 2 <= 400, "{a}..{b} every {e} would be {} points", span / e as i64 + 2);
        }
    }
    /// The real code path against the real service — URL, user agent, the series question, parsing — and each day
    /// checked against the single-day question the service already answered. Not in the normal run: it needs the
    /// network. `cargo test live_series -- --ignored`
    #[test]
    #[ignore]
    fn live_series_agrees_with_the_single_day_question() {
        let run = price_series(PRICES, "AUD", "2026-09-01", "2026-09-10", 3).expect("the series question answered");
        assert_eq!(run.last().unwrap().0, "2026-09-10", "the last point is always `to`");
        let http = reqwest::blocking::Client::builder().user_agent("anytwokeys/0.1").build().unwrap();
        for (day, q) in &run {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let one = fetch(&http, PRICES, "AUD", day, None).expect("the single-day question answered");
            assert!((one.price - q.as_ref().unwrap().price).abs() < 1e-9, "{day}: series {:?} vs single {}", q, one.price);
        }
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let usd = price_series(PRICES, "USD", "2026-09-01", "2026-09-03", 1).unwrap();
        assert!(usd.iter().all(|(_, q)| q.is_some()), "USD comes back priced now the service fills prices.USD: {usd:?}");
    }
    fn chrono_days(d: &str) -> i64 { let t = |s: &str, a, b| s[a..b].parse::<i64>().unwrap(); let (y, m, dd) = (t(d, 0, 4), t(d, 5, 7), t(d, 8, 10));
        let y = if m <= 2 { y - 1 } else { y }; let era = y.div_euclid(400); let yoe = y - era * 400; let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + dd - 1; era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy }
}

#[cfg(test)]
mod lineage_tests {
    /// Writes a real wallet's rows, with prices, to ROWS_OUT — for checking the cost engine on real history.
    /// `EXT_DESC='…' ROWS_OUT=/tmp/x.json cargo test rows_dump -- --ignored`. Never committed: it is someone's wallet.
    #[test]
    #[ignore]
    fn rows_dump() {
        let info = crate::wallet::ext_parse(&std::env::var("EXT_DESC").unwrap()).unwrap().unwrap();
        let v = crate::wallet::Vault { name: "real".into(), network: info.network.clone(), threshold: info.threshold, keys: vec![], descriptor: info.descriptor.clone(), address: String::new(), created: 0,
            chain_code: None, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(),
            files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default(), ext: Some(info.descriptor), cost_method: None };
        let servers = crate::chain::Servers { electrum: "ssl://electrum.mineracks.com:50002".into(), explorer: String::new() };
        let sc = crate::wallet::scan(&v, &servers, 20).unwrap();
        let mut hist: Vec<(String, i32)> = sc.history.iter().map(|h| (h.0.clone(), h.1)).collect(); hist.sort(); hist.dedup();
        let a = super::build(&v, &servers, &hist, "AUD", super::PRICES).unwrap();
        std::fs::write(std::env::var("ROWS_OUT").unwrap(), serde_json::to_string(&serde_json::json!({ "rows": a.rows, "now": a.now, "balance": sc.coins.iter().map(|c| c.value).sum::<u64>() })).unwrap()).unwrap();
    }
    /// A real wallet's coin lineage, checked for the two things a cost engine rests on: each transaction's coins in and
    /// out add up to its net, and every coin spent was made by an earlier transaction of the same wallet — so nothing
    /// arrives from nowhere. `EXT_DESC='…' cargo test lineage_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lineage_live_closes_on_a_real_wallet() {
        let info = crate::wallet::ext_parse(&std::env::var("EXT_DESC").unwrap()).unwrap().unwrap();
        let v = crate::wallet::Vault { name: "x".into(), network: info.network.clone(), threshold: info.threshold, keys: vec![], descriptor: info.descriptor.clone(), address: String::new(), created: 0,
            chain_code: None, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(),
            files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default(), ext: Some(info.descriptor), cost_method: None };
        let servers = crate::chain::Servers { electrum: "ssl://electrum.mineracks.com:50002".into(), explorer: String::new() };
        let sc = crate::wallet::scan(&v, &servers, 20).unwrap();
        let mut hist: Vec<(String, i32)> = sc.history.iter().map(|h| (h.0.clone(), h.1)).collect(); hist.sort(); hist.dedup();
        let a = super::build(&v, &servers, &hist, "", "").unwrap();
        let made: std::collections::HashSet<String> = a.rows.iter().flat_map(|r| r.outs.iter().map(move |(n, _)| format!("{}:{n}", r.txid))).collect();
        let (mut ok, mut orphan) = (0, 0);
        for r in &a.rows {
            let (i, o): (u64, u64) = (r.ins.iter().map(|x| x.1).sum(), r.outs.iter().map(|x| x.1).sum());
            assert_eq!(o as i64 - i as i64, r.net.unwrap(), "{}: coins in and out do not add to the net", &r.txid[..8]);
            for (op, _) in &r.ins { if made.contains(op) { ok += 1 } else { orphan += 1; println!("ORPHAN {op}") } }
        }
        println!("LINEAGE rows={} coins spent={} all made by an earlier row of this wallet: {}", a.rows.len(), ok + orphan, orphan == 0);
        assert_eq!(orphan, 0);
    }
}

