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
pub struct Row { pub txid: String, pub time: Option<u32>, pub net: Option<i64>, pub fee: Option<u64>, pub quote: Option<Quote> }
#[derive(Debug, Serialize)]
pub struct Activity { pub ccy: String, pub rows: Vec<Row>, pub now: Option<Quote>, pub note: Option<String> }

fn cache_path() -> std::path::PathBuf { wallet::store_path().with_file_name("cache.json") }
fn load_cache() -> Cache { std::fs::read(cache_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() }
fn save_cache(c: &Cache) { if let Ok(b) = serde_json::to_vec(c) { let _ = std::fs::write(cache_path(), b); } }

/// (day, hour) in UTC for a Unix time.
fn day_hour(t: u32) -> (String, u32) { (crate::export::ymd(t as u64), (t % 86_400) / 3_600) }

/// Parse the price service's answer for one currency. `None` for anything that is not a positive finite price.
pub fn read_quote(body: &serde_json::Value, ccy: &str, day: &str) -> Option<Quote> {
    let price = body.get("prices")?.get(ccy)?.as_f64().filter(|p| p.is_finite() && *p > 0.0)?;
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
        let r = http.get(&url).send().ok()?;
        if !r.status().is_success() { return None; }
        read_quote(&r.json::<serde_json::Value>().ok()?, ccy, day)
    };
    if hour.is_some() { if let Some(q) = ask(true) { return Some(q); } }
    ask(false)
}

pub fn build(v: &wallet::Vault, servers: &Servers, history: &[(String, i32)], ccy: &str, prices_url: &str) -> Result<Activity, String> {
    let client = crate::chain::client(servers)?;
    let mut cache = load_cache();
    let mut dirty = false;
    // every transaction in the history, fetched once; a spent coin's parent is always in the history too
    let mut txs: HashMap<String, Transaction> = HashMap::new();
    for (txid, _) in history {
        let id = Txid::from_str(txid).map_err(|e| e.to_string())?;
        txs.insert(txid.clone(), client.transaction_get(&id).map_err(|e| format!("transaction {}: {e}", &txid[..8]))?);
    }
    let ours = |spk: &bitcoin::ScriptBuf| wallet::index_of_input(v, spk).ok().flatten().is_some();
    let fiat = v.network == "mainnet" && !ccy.is_empty() && !prices_url.is_empty();
    let http = reqwest::blocking::Client::builder().user_agent("anytwokeys/0.1").timeout(std::time::Duration::from_secs(12)).build().map_err(|e| e.to_string())?;
    let mut price_failed = false;
    let mut rows = Vec::new();
    for (txid, height) in history {
        let tx = &txs[txid];
        let received: u64 = tx.output.iter().filter(|o| ours(&o.script_pubkey)).map(|o| o.value.to_sat()).sum();
        let mut spent = 0u64; let mut all_ours = !tx.input.is_empty();
        for i in &tx.input {
            match txs.get(&i.previous_output.txid.to_string()).and_then(|p| p.output.get(i.previous_output.vout as usize)).filter(|o| ours(&o.script_pubkey)) {
                Some(o) => spent += o.value.to_sat(), None => all_ours = false }
        }
        let fee = if all_ours { Some(spent.saturating_sub(tx.output.iter().map(|o| o.value.to_sat()).sum())) } else { None };
        let time = if *height > 0 {
            let key = format!("{}:{height}", v.network);
            match cache.times.get(&key) { Some(t) => Some(*t), None => client.block_header(*height as usize).ok().map(|h| { cache.times.insert(key, h.time); dirty = true; h.time }) }
        } else { None };
        let quote = match (fiat, time) {
            (true, Some(t)) => { let (day, hour) = day_hour(t); let key = format!("{ccy}:{day}T{hour:02}");
                match cache.prices.get(&key) { Some(q) => Some(q.clone()), None => { let q = fetch(&http, prices_url, ccy, &day, Some(hour)); match &q { Some(q) => { cache.prices.insert(key, q.clone()); dirty = true; } None => price_failed = true } q } } }
            _ => None,
        };
        rows.push(Row { txid: txid.clone(), time, net: Some(received as i64 - spent as i64), fee, quote });
    }
    let now = if fiat { let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0); let (day, hour) = day_hour(t);
        let q = fetch(&http, prices_url, ccy, &day, Some(hour)); if q.is_none() { price_failed = true; } q } else { None };
    if dirty { save_cache(&cache); }
    let note = if v.network != "mainnet" { Some("Test coins have no price.".into()) } else if !fiat { None } else if price_failed { Some("Some prices could not be fetched; they are left blank rather than guessed.".into()) } else { None };
    Ok(Activity { ccy: ccy.to_string(), rows, now, note })
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
    fn live_activity() {
        let s = crate::wallet::load();
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
