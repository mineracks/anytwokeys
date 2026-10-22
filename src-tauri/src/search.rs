//! One search across everything a profile holds: every vault, its keys, labels and notes, the transactions and coins
//! the watcher has recorded for it, its addresses, the address book, the places keys are kept and the entities that own
//! vaults. Nothing is asked of the network: it reads the store, the chain cache and the price cache, so an answer comes
//! back in the time it takes to type. A query is read three ways at once — as words, as an amount and as a date — and
//! each reading finds what it can; the window shows them together, best first.
use crate::wallet::{self, Store, Vault};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize, Debug, Clone)]
pub struct Hit {
    /// vault · key · xkey · tx · coin · addr · contact · place · entity
    pub kind: &'static str,
    pub vault: Option<String>,
    pub id: String,
    pub title: String,
    pub sub: String,
    pub when: Option<u32>,
    pub sats: Option<i64>,
    pub fiat: Option<String>,
    pub score: u32,
}

/// How the query was read: the words looked for, the amount it may name, the span of days it may name.
#[derive(Serialize, Debug, Default, Clone, PartialEq)]
pub struct Reading {
    pub text: String,
    pub sats: Option<u64>,
    /// a sum of money in the profile's currency, when the query could be one
    pub fiat: Option<f64>,
    pub from: Option<u32>,
    pub to: Option<u32>,
    /// the span in words, for the window to say what it looked for
    pub span: Option<String>,
}

#[derive(Serialize, Debug, Default)]
pub struct Found {
    pub hits: Vec<Hit>,
    /// hits left out of each kind once its share of the list was full
    pub more: HashMap<&'static str, usize>,
    pub reading: Reading,
    pub vaults: usize,
    pub txs: usize,
    pub coins: usize,
    pub addresses: usize,
    pub ms: u128,
}

const PER_KIND: usize = 40;
const TOTAL: usize = 160;
const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

// ---------- reading the query ----------

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097); let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}
fn month_add(y: i64, m: u32, by: i64) -> (i64, u32) { let t = y * 12 + (m as i64 - 1) + by; (t.div_euclid(12), (t.rem_euclid(12) + 1) as u32) }
fn days_in(y: i64, m: u32) -> u32 { let (ny, nm) = month_add(y, m, 1); (days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)) as u32 }

#[derive(Debug, PartialEq, Clone, Copy)]
enum Span { Day(i64, u32, u32), Month(i64, u32), Year(i64) }

impl Span {
    fn words(&self) -> String {
        match *self {
            Span::Day(y, m, d) => format!("{d} {} {y}", title(MONTHS[m as usize - 1])),
            Span::Month(y, m) => format!("{} {y}", title(MONTHS[m as usize - 1])),
            Span::Year(y) => format!("{y}"),
        }
    }
    /// [from, to) in Unix seconds, for a window whose clock is `tz_min` minutes behind UTC (what JavaScript's
    /// getTimezoneOffset says: Brisbane is −600)
    fn range(&self, tz_min: i64) -> (u32, u32) {
        let (a, b) = match *self {
            Span::Day(y, m, d) => (days_from_civil(y, m, d), days_from_civil(y, m, d) + 1),
            Span::Month(y, m) => { let (ny, nm) = month_add(y, m, 1); (days_from_civil(y, m, 1), days_from_civil(ny, nm, 1)) }
            Span::Year(y) => (days_from_civil(y, 1, 1), days_from_civil(y + 1, 1, 1)),
        };
        let shift = tz_min * 60;
        ((a * 86_400 + shift).max(0) as u32, (b * 86_400 + shift).max(0) as u32)
    }
}
fn title(s: &str) -> String { let mut c = s.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() }

fn month_of(tok: &str) -> Option<u32> {
    if tok.len() < 3 { return None; }
    let full = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    (0..12).find(|&i| full[i].starts_with(tok) && tok.len() <= full[i].len()).map(|i| i as u32 + 1)
}

/// The span of days a query names, if it does: `2026-10-19`, `19/10/2026`, `19 Oct 2026`, `Oct 19`, `October`,
/// `Oct 2026`, `2026-10`, `2026`, `today`, `yesterday`, `this month`, `last month`, `this year`, `last year`.
fn span_of(q: &str, now: u32, tz_min: i64) -> Option<Span> {
    let local_days = (now as i64 - tz_min * 60).div_euclid(86_400);
    let (ty, tm, td) = civil_from_days(local_days);
    let q = q.trim().to_lowercase();
    match q.as_str() {
        "today" => return Some(Span::Day(ty, tm, td)),
        "yesterday" => { let (y, m, d) = civil_from_days(local_days - 1); return Some(Span::Day(y, m, d)); }
        "this month" => return Some(Span::Month(ty, tm)),
        "last month" => { let (y, m) = month_add(ty, tm, -1); return Some(Span::Month(y, m)); }
        "this year" => return Some(Span::Year(ty)),
        "last year" => return Some(Span::Year(ty - 1)),
        _ => {}
    }
    let toks: Vec<&str> = q.split(|c: char| c.is_whitespace() || matches!(c, '-' | '/' | '.' | ',')).filter(|t| !t.is_empty()).collect();
    if toks.is_empty() || toks.len() > 3 { return None; }
    let nums: Vec<Option<u32>> = toks.iter().map(|t| t.parse::<u32>().ok()).collect();
    let valid = |y: i64, m: u32, d: u32| (1..=12).contains(&m) && d >= 1 && d <= days_in(y, m) && (2009..=2100).contains(&y);
    if nums.iter().all(|n| n.is_some()) {
        let n: Vec<u32> = nums.iter().map(|n| n.unwrap()).collect();
        let year = |x: u32| (2009..=2100).contains(&x);
        return match n.as_slice() {
            [y, m, d] if year(*y) && valid(*y as i64, *m, *d) => Some(Span::Day(*y as i64, *m, *d)),
            [d, m, y] if year(*y) && valid(*y as i64, *m, *d) => Some(Span::Day(*y as i64, *m, *d)),
            [y, m] if year(*y) && (1..=12).contains(m) => Some(Span::Month(*y as i64, *m)),
            [m, y] if year(*y) && (1..=12).contains(m) => Some(Span::Month(*y as i64, *m)),
            [y] if year(*y) && toks[0].len() == 4 => Some(Span::Year(*y as i64)),
            _ => None,
        };
    }
    // a month by name, with a day and a year wherever they stand
    let mut month = None; let mut day = None; let mut year = None;
    for (t, n) in toks.iter().zip(&nums) {
        match n {
            Some(x) if t.len() == 4 && (2009..=2100).contains(x) => { if year.is_some() { return None; } year = Some(*x as i64); }
            Some(x) if (1..=31).contains(x) => { if day.is_some() { return None; } day = Some(*x); }
            Some(_) => return None,
            None => match month_of(t) { Some(m) if month.is_none() => month = Some(m), _ => return None },
        }
    }
    let m = month?;
    let y = year.unwrap_or(ty);
    match day { Some(d) if valid(y, m, d) => Some(Span::Day(y, m, d)), Some(_) => None, None => Some(Span::Month(y, m)) }
}

/// The amount a query may name. `0.25` or `0.25 btc` is bitcoin; `50,000` or `50000 sats` is sats; `$1,200`, `1200 aud`
/// is money; a bare whole number is sats and money both, and a bare decimal is bitcoin and money both — each reading
/// finds what it finds.
fn amount_of(q: &str, ccy: &str) -> (Option<u64>, Option<f64>) {
    let mut t = q.trim().to_lowercase();
    let mut unit = "";
    for sym in ["a$", "us$", "nz$", "ca$", "$", "€", "£", "¥"] { if let Some(r) = t.strip_prefix(sym) { t = r.trim().to_string(); unit = "fiat"; break; } }
    for (suf, u) in [(" btc", "btc"), ("btc", "btc"), (" sats", "sats"), (" sat", "sats"), ("sats", "sats"), ("sat", "sats")] { if let Some(r) = t.strip_suffix(suf) { t = r.trim().to_string(); unit = u; break; } }
    if unit.is_empty() { let c = ccy.to_lowercase(); if !c.is_empty() { if let Some(r) = t.strip_suffix(c.as_str()) { t = r.trim().to_string(); unit = "fiat"; } } }
    let digits: String = t.chars().filter(|c| !matches!(c, ',' | ' ' | '_' | '\'')).collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '.') || digits.matches('.').count() > 1 { return (None, None); }
    let v: f64 = match digits.parse::<f64>() { Ok(v) if v > 0.0 && v.is_finite() => v, _ => return (None, None) };
    let whole = !digits.contains('.');
    match unit {
        "btc" => (Some((v * 1e8).round() as u64), None),
        "sats" => (if whole { Some(v as u64) } else { None }, None),
        "fiat" => (None, Some(v)),
        _ => if whole { (Some(v as u64), Some(v)) } else { (if v <= 21_000_000.0 { Some((v * 1e8).round() as u64) } else { None }, Some(v)) },
    }
}

pub fn read(q: &str, ccy: &str, now: u32, tz_min: i64) -> Reading {
    let text = q.trim().to_lowercase();
    let (sats, fiat) = amount_of(&text, ccy);
    let span = span_of(&text, now, tz_min);
    let (from, to) = span.map(|s| s.range(tz_min)).map(|(a, b)| (Some(a), Some(b))).unwrap_or((None, None));
    Reading { text, sats, fiat, from, to, span: span.map(|s| s.words()) }
}

// ---------- matching ----------

/// How well a field answers the words typed: the whole of it, its start, a word in it, or somewhere inside it.
fn grade(hay: &str, needle: &str) -> Option<u32> {
    if needle.is_empty() { return None; }
    let h = hay.to_lowercase();
    if h == needle { Some(100) }
    else if h.starts_with(needle) { Some(85) }
    else if let Some(i) = h.find(needle) { Some(if i == 0 || !h.as_bytes()[i - 1].is_ascii_alphanumeric() { 75 } else { 60 }) }
    else { None }
}
/// an id (txid, outpoint, address, public key) is matched on at least four characters, anywhere in it
fn grade_id(id: &str, needle: &str) -> Option<u32> { if needle.len() < 4 { None } else { grade(id, needle).map(|g| g.min(80)) } }
fn short(id: &str) -> String { if id.len() > 20 { format!("{}…{}", &id[..10], &id[id.len() - 6..]) } else { id.to_string() } }
fn fmt(n: u64) -> String { let s = n.to_string(); let mut out = String::new(); for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { out.push(','); } out.push(c); } out }
fn day_words(t: u32) -> String { let (y, m, d) = civil_from_days((t as i64).div_euclid(86_400)); format!("{d} {} {y}", title(MONTHS[m as usize - 1])) }
fn near(a: f64, b: f64) -> bool { (a - b).abs() <= (0.005 * b.abs()).max(0.5) }

/// What the chain cache and the price cache know about one transaction of a vault.
struct Fact { net: Option<i64>, time: Option<u32>, fiat: Option<f64> }

/// The kind words of a vault, as its own screens say them.
fn kind_words(v: &Vault) -> String {
    if let Some(n) = &v.node { return format!("Node · {} · {}", if n.kind == "cln" { "Core Lightning" } else { "LND" }, n.alias); }
    if let Some(e) = &v.exchange { return format!("Exchange · {}", e.provider); }
    if v.statement.is_some() { return "Closed wallet · from its statement".into(); }
    if v.spark.is_some() { return "Spark wallet · watched".into(); }
    if v.muun.is_some() { return "Muun wallet · watched".into(); }
    if v.watch.is_some() { return "Watched from an extended key".into(); }
    if v.ext.is_some() { return format!("{} of {} signers", v.threshold, v.keys.len().max(v.threshold)); }
    format!("{} of {} keys", v.threshold, v.keys.len())
}

/// A card key's fingerprint, as a signer would show it: hash160 of the key, first four bytes.
fn card_fingerprint(pubkey_hex: &str) -> Option<String> {
    let pk = bitcoin::PublicKey::from_str(pubkey_hex).ok()?;
    Some(hex::encode(&pk.pubkey_hash()[..4]))
}
use std::str::FromStr;

pub fn run(s: &Store, q: &str, ccy: &str, now: u32, tz_min: i64) -> Found {
    let t0 = std::time::Instant::now();
    let r = read(q, ccy, now, tz_min);
    let mut out = Found { reading: r.clone(), vaults: s.vaults.len(), ..Default::default() };
    if r.text.is_empty() { return out; }
    let needle = r.text.as_str();
    let mut hits: Vec<Hit> = Vec::new();
    let entity_name = |id: &Option<String>| id.as_ref().and_then(|i| s.entities.iter().find(|e| &e.id == i)).map(|e| e.name.clone());

    // vaults
    for v in &s.vaults {
        let ent = entity_name(&v.entity);
        let fields = [Some(v.name.clone()), Some(kind_words(v)), Some(v.network.clone()), ent.clone()];
        let best = fields.iter().flatten().enumerate().filter_map(|(i, f)| grade(f, needle).map(|g| if i == 0 { g } else { g.min(70) })).max();
        if let Some(g) = best {
            let sub = format!("Vault · {} · {}{}", kind_words(v), v.network, ent.map(|e| format!(" · {e}")).unwrap_or_default());
            hits.push(Hit { kind: "vault", vault: Some(v.name.clone()), id: v.name.clone(), title: v.name.clone(), sub, when: None, sats: None, fiat: None, score: g });
        }
    }

    // keys on cards, once each, with every vault they sign for
    let mut seen_keys: HashMap<String, usize> = HashMap::new();
    for v in &s.vaults {
        // a vault of extended keys lists its signers below, by fingerprint; its key rows hold origins, not card keys
        for k in v.keys.iter().filter(|_| v.ext.is_none()) {
            if k.pubkey.is_empty() { continue; }
            if let Some(&i) = seen_keys.get(&k.pubkey) { if !hits[i].sub.contains(&v.name) { hits[i].sub.push_str(&format!(", {}", v.name)); } continue; }
            let fp = card_fingerprint(&k.pubkey).unwrap_or_default();
            let label = v.labels.get(&format!("key:{}", k.pubkey)).cloned().unwrap_or_default();
            let note = v.notes.get(&format!("key:{}", k.pubkey)).cloned().unwrap_or_default();
            let g = [grade(&k.name, needle), grade(&label, needle), grade(&note, needle).map(|g| g.min(70)), grade_id(&k.pubkey, needle), grade(&fp, needle).filter(|_| needle.len() >= 4),
                if k.serial > 0 { grade(&k.serial.to_string(), needle).filter(|_| needle.len() >= 4) } else { None }, k.device.as_deref().and_then(|d| grade(d, needle)).map(|g| g.min(65))].into_iter().flatten().max();
            if let Some(g) = g {
                let sub = format!("Key{}{} · {} · signs {}", if k.serial > 0 { format!(" · serial {}", k.serial) } else { String::new() }, if fp.is_empty() { String::new() } else { format!(" · fingerprint {fp}") }, short(&k.pubkey), v.name);
                seen_keys.insert(k.pubkey.clone(), hits.len());
                hits.push(Hit { kind: "key", vault: Some(v.name.clone()), id: k.pubkey.clone(), title: if label.is_empty() { k.name.clone() } else { format!("{} · {}", k.name, label) }, sub, when: None, sats: None, fiat: None, score: g });
            }
        }
        // (extended keys follow below)
        if let Some(ext) = &v.ext {
            if let Ok(Some(info)) = wallet::ext_parse(ext) {
                for (i, k) in info.keys.iter().enumerate() {
                    let name = v.keys.get(i).map(|k| k.name.clone()).unwrap_or_else(|| format!("key {}", i + 1));
                    let fp = if k.fixed { card_fingerprint(&k.xpub).unwrap_or_default() } else { k.fingerprint.clone() };
                    let g = [grade(&name, needle), grade(&fp, needle).filter(|_| needle.len() >= 4), grade_id(&k.xpub, needle), grade(&k.path, needle).filter(|_| needle.len() >= 4)].into_iter().flatten().max();
                    if let Some(g) = g {
                        let sub = if k.fixed { format!("Key on a card · fingerprint {fp} · {} · signs {}", short(&k.xpub), v.name) } else { format!("Extended key · fingerprint {fp} · {} · {} · signs {}", k.path, short(&k.xpub), v.name) };
                        hits.push(Hit { kind: "xkey", vault: Some(v.name.clone()), id: format!("{}:{}", v.name, fp), title: name, sub, when: None, sats: None, fiat: None, score: g });
                    }
                }
            }
        }
    }

    // keys set up on this computer that are not in a vault yet
    for (pubkey, kr) in &s.key_ready {
        if seen_keys.contains_key(pubkey) { continue; }
        let fp = card_fingerprint(pubkey).unwrap_or_default();
        let g = [grade(&kr.name, needle), grade_id(pubkey, needle), grade(&fp, needle).filter(|_| needle.len() >= 4), if kr.serial > 0 { grade(&kr.serial.to_string(), needle).filter(|_| needle.len() >= 4) } else { None }].into_iter().flatten().max();
        if let Some(g) = g {
            seen_keys.insert(pubkey.clone(), hits.len());
            hits.push(Hit { kind: "key", vault: None, id: pubkey.clone(), title: kr.name.clone(), sub: format!("Key · set up here, in no vault yet{}{} · {}", if kr.serial > 0 { format!(" · serial {}", kr.serial) } else { String::new() }, if fp.is_empty() { String::new() } else { format!(" · fingerprint {fp}") }, short(pubkey)), when: None, sats: None, fiat: None, score: g });
        }
    }

    // what the chain cache knows about a vault's transactions: made once per vault, only when something asks
    let cache_times = crate::activity::cached_times();
    let mut facts: HashMap<String, HashMap<String, Fact>> = HashMap::new();
    let want_facts = r.sats.is_some() || r.fiat.is_some() || r.from.is_some();
    let fiat_on = !ccy.is_empty();
    let mut facts_of = |v: &Vault| -> HashMap<String, Fact> {
        if let Some(f) = facts.get(&v.name) { return f.iter().map(|(k, f)| (k.clone(), Fact { net: f.net, time: f.time, fiat: f.fiat })).collect(); }
        let mut m: HashMap<String, Fact> = HashMap::new();
        let price = |t: Option<u32>| -> Option<f64> { if !fiat_on || v.network != "mainnet" { return None; } let t = t?; let (day, hour) = crate::activity::day_hour(t); crate::activity::cached_price(&format!("{ccy}:{day}T{hour:02}")).or_else(|| crate::activity::cached_price(&format!("{ccy}:{day}"))).map(|q| q.price) };
        if let Some(n) = &v.node { for e in &n.events { let p = price(Some(e.time)); m.insert(e.id.clone(), Fact { net: Some(e.net), time: Some(e.time), fiat: p.map(|p| p * e.net.abs() as f64 / 1e8) }); } }
        else if let Some(e) = &v.exchange { for r in &e.rows { m.insert(r.id.clone(), Fact { net: Some(r.sats), time: Some(r.time), fiat: Some(r.fiat.abs() as f64 / 100.0) }); } }
        else if let Some(st) = &v.statement { for r in &st.rows { let p = price(Some(r.time)); m.insert(r.id.clone(), Fact { net: Some(r.net), time: Some(r.time), fiat: p.map(|p| p * r.net.abs() as f64 / 1e8) }); } }
        else if let Some(seen) = s.seen.get(&v.name) {
            let ids: Vec<(String, i32)> = seen.txs.iter().map(|(t, h)| (t.clone(), *h)).collect();
            let txids: Vec<bitcoin::Txid> = ids.iter().filter_map(|(t, _)| bitcoin::Txid::from_str(t).ok()).collect();
            let kept = crate::db::txs(&v.network, &txids);
            let by_id: HashMap<bitcoin::Txid, &bitcoin::Transaction> = txids.iter().zip(&kept).filter_map(|(i, t)| t.as_ref().map(|t| (*i, t))).collect();
            let ours = wallet::ours_table(v).ok();
            for (txid, h) in &ids {
                let time = if *h > 0 { cache_times.get(&format!("{}:{h}", v.network)).copied() } else { None };
                let net = match (bitcoin::Txid::from_str(txid).ok().and_then(|i| by_id.get(&i)), &ours) {
                    (Some(tx), Some(ours)) => {
                        let received: u64 = tx.output.iter().filter(|o| ours(&o.script_pubkey).is_some()).map(|o| o.value.to_sat()).sum();
                        let spent: u64 = tx.input.iter().filter_map(|i| by_id.get(&i.previous_output.txid).and_then(|p| p.output.get(i.previous_output.vout as usize)).filter(|o| ours(&o.script_pubkey).is_some())).map(|o| o.value.to_sat()).sum();
                        Some(received as i64 - spent as i64)
                    }
                    _ => None,
                };
                let p = price(time);
                m.insert(txid.clone(), Fact { fiat: p.and_then(|p| net.map(|n| p * n.unsigned_abs() as f64 / 1e8)), net, time });
            }
        }
        let copy: HashMap<String, Fact> = m.iter().map(|(k, f)| (k.clone(), Fact { net: f.net, time: f.time, fiat: f.fiat })).collect();
        facts.insert(v.name.clone(), m);
        copy
    };
    let money = |x: f64| -> String { let whole = x.abs() >= 100.0; if whole { format!("{} {}", fmt(x.round() as u64), ccy) } else { format!("{x:.2} {ccy}") } };

    // labels and notes: a match on either opens the thing it is about
    let kinds = |key: &str| -> Option<(&'static str, String)> {
        let (p, id) = key.split_once(':')?;
        Some((match p { "addr" => "addr", "tx" => "tx", "out" | "output" => "coin", "key" => "key", "place" => "place", _ => return None }, id.to_string()))
    };
    let mut rec_hits: Vec<(Option<String>, &'static str, String, String, u32)> = Vec::new();
    for v in &s.vaults {
        for (key, label) in &v.labels {
            if let Some((k, id)) = kinds(key) { if k != "key" { if let Some(g) = grade(label, needle) { rec_hits.push((Some(v.name.clone()), k, id, label.clone(), g)); } } }
        }
        for (key, note) in &v.notes {
            if let Some((k, id)) = kinds(key) { if k != "key" { if let Some(g) = grade(note, needle) { rec_hits.push((Some(v.name.clone()), k, id, v.labels.get(key).cloned().unwrap_or_default(), g.min(70))); } } }
        }
    }
    for (key, label) in &s.records.labels { if let Some((k, id)) = kinds(key) { if let Some(g) = grade(label, needle) { rec_hits.push((None, k, id, label.clone(), g)); } } }
    for (key, note) in &s.records.notes { if let Some((k, id)) = kinds(key) { if let Some(g) = grade(note, needle) { rec_hits.push((None, k, id, s.records.labels.get(key).cloned().unwrap_or_default(), g.min(70))); } } }

    // transactions and coins of every vault: by id, by amount, by day — and the record hits above, filled in
    let addr_like = needle.len() >= 4 && needle.chars().all(|c| c.is_ascii_alphanumeric());
    let hex_like = needle.len() >= 4 && needle.chars().all(|c| c.is_ascii_hexdigit());
    for v in &s.vaults {
        let label_of = |key: String| v.labels.get(&key).cloned().unwrap_or_default();
        let mut rows: Vec<(String, u32)> = Vec::new();   // (txid, score) from ids and records
        let ids: Vec<String> = if let Some(n) = &v.node { n.events.iter().map(|e| e.id.clone()).collect() } else if let Some(e) = &v.exchange { e.rows.iter().map(|r| r.id.clone()).collect() } else if let Some(st) = &v.statement { st.rows.iter().map(|r| r.id.clone()).collect() } else { s.seen.get(&v.name).map(|x| x.txs.keys().cloned().collect()).unwrap_or_default() };
        out.txs += ids.len();
        if hex_like { for id in &ids { if let Some(g) = grade_id(id, needle) { rows.push((id.clone(), g)); } } }
        for (vn, k, id, _, g) in &rec_hits { if *k == "tx" && vn.as_deref() == Some(&v.name) { rows.push((id.clone(), *g)); } }
        // a memo on a node's or an exchange's row counts as its label
        if let Some(n) = &v.node { for e in &n.events { if let Some(g) = grade(&e.memo, needle) { rows.push((e.id.clone(), g.min(70))); } } }
        if let Some(e) = &v.exchange { for r in &e.rows { if let Some(g) = grade(&r.memo, needle) { rows.push((r.id.clone(), g.min(70))); } } }
        let need = want_facts || !rows.is_empty();
        let f = if need { facts_of(v) } else { HashMap::new() };
        if want_facts {
            for (txid, fact) in &f {
                let by_amount = fact.net.map(|n| r.sats.map(|s| n.unsigned_abs() == s).unwrap_or(false)).unwrap_or(false)
                    || fact.fiat.zip(r.fiat).map(|(a, b)| near(a, b)).unwrap_or(false);
                let by_day = fact.time.zip(r.from.zip(r.to)).map(|(t, (a, b))| t >= a && t < b).unwrap_or(false);
                if by_amount { rows.push((txid.clone(), 90)); } else if by_day { rows.push((txid.clone(), 70)); }
            }
        }
        rows.sort_by(|a, b| b.1.cmp(&a.1)); rows.dedup_by(|a, b| a.0 == b.0);
        for (txid, g) in rows {
            let fact = f.get(&txid);
            let label = label_of(format!("tx:{txid}"));
            // the amount and its value go in the hit's own column; the words say where and when
            let mut sub = vec![v.name.clone()];
            if let Some(t) = fact.and_then(|f| f.time) { sub.push(day_words(t)); } else if v.node.is_none() && v.exchange.is_none() && v.statement.is_none() { sub.push(if s.seen.get(&v.name).and_then(|x| x.txs.get(&txid)).map(|h| *h > 0).unwrap_or(false) { "confirmed".into() } else { "pending".into() }); }
            if !label_of(format!("tx:{txid}")).is_empty() { sub.push(short(&txid)); }
            if fact.and_then(|f| f.net).is_none() && want_facts { sub.push("amount not cached yet".into()); }
            hits.push(Hit { kind: "tx", vault: Some(v.name.clone()), id: txid.clone(), title: if label.is_empty() { short(&txid) } else { label }, sub: sub.join(" · "), when: fact.and_then(|f| f.time), sats: fact.and_then(|f| f.net), fiat: fact.and_then(|f| f.fiat).map(money), score: g });
        }
        // coins
        if let Some(seen) = s.seen.get(&v.name) {
            out.coins += seen.coins.len();
            let mut cs: Vec<(String, u32)> = Vec::new();
            for (op, val) in &seen.coins {
                let g = [if hex_like { grade_id(op, needle) } else { None }, r.sats.filter(|s| s == val).map(|_| 90), r.fiat.and_then(|want| { let txid = op.split(':').next()?; let p = f.get(txid).and_then(|x| x.fiat.zip(x.net)).map(|(fi, n)| fi / (n.unsigned_abs() as f64 / 1e8))?; near(p * *val as f64 / 1e8, want).then_some(85) })].into_iter().flatten().max();
                if let Some(g) = g { cs.push((op.clone(), g)); }
            }
            for (vn, k, id, _, g) in &rec_hits { if *k == "coin" && vn.as_deref() == Some(&v.name) { cs.push((id.clone(), *g)); } }
            cs.sort_by(|a, b| b.1.cmp(&a.1)); cs.dedup_by(|a, b| a.0 == b.0);
            for (op, g) in cs {
                let label = v.labels.get(&format!("out:{op}")).or_else(|| v.labels.get(&format!("output:{op}"))).cloned().unwrap_or_default();
                let val = seen.coins.get(&op).copied();
                let txid = op.split(':').next().unwrap_or("");
                let t = f.get(txid).and_then(|x| x.time);
                let mut sub = vec![v.name.clone(), "coin".into()];
                if let Some(t) = t { sub.push(format!("since {}", day_words(t))); }
                if !label.is_empty() { sub.push(short(&op)); }
                hits.push(Hit { kind: "coin", vault: Some(v.name.clone()), id: op.clone(), title: if label.is_empty() { short(&op) } else { label }, sub: sub.join(" · "), when: t, sats: val.map(|x| x as i64), fiat: None, score: g });
            }
        }
        // addresses: the vault's own, derived (cached after the first search) — looked at only when the words could be one
        if addr_like || rec_hits.iter().any(|(vn, k, ..)| *k == "addr" && vn.as_deref() == Some(&v.name)) {
            let addrs = wallet::all_addresses(v);
            out.addresses += addrs.len();
            let mut found: Vec<(u32, String, u32)> = Vec::new();
            if addr_like { for (i, a) in &addrs { if let Some(g) = grade_id(a, needle) { found.push((*i, a.clone(), g)); } } }
            for (vn, k, id, _, g) in &rec_hits { if *k == "addr" && vn.as_deref() == Some(&v.name) { if let Some((i, a)) = addrs.iter().find(|(_, a)| a == id) { found.push((*i, a.clone(), *g)); } else { found.push((u32::MAX, id.clone(), *g)); } } }
            found.sort_by(|a, b| b.2.cmp(&a.2)); found.dedup_by(|a, b| a.1 == b.1);
            for (i, a, g) in found {
                let label = v.labels.get(&format!("addr:{a}")).cloned().unwrap_or_default();
                let place = if i == u32::MAX { String::new() } else if !wallet::rotates(v) { "the vault's address".into() } else if wallet::is_change(i) { format!("change #{}", wallet::position(i)) } else { format!("receive #{}", wallet::position(i) & 0x0fff_ffff) };
                let sub = [v.name.clone(), place, if label.is_empty() { a.clone() } else { String::new() }].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
                hits.push(Hit { kind: "addr", vault: Some(v.name.clone()), id: a.clone(), title: if label.is_empty() { a.clone() } else { label }, sub, when: None, sats: None, fiat: None, score: g });
            }
        }
    }

    // the address book, places and entities
    for c in &s.contacts {
        if let Some(g) = [grade(&c.name, needle), grade_id(&c.address, needle)].into_iter().flatten().max() {
            hits.push(Hit { kind: "contact", vault: None, id: c.address.clone(), title: c.name.clone(), sub: format!("Address book · {} · {}{}", c.network, c.address, if c.used > 0 { format!(" · paid {} time{}", c.used, if c.used == 1 { "" } else { "s" }) } else { String::new() }), when: None, sats: None, fiat: None, score: g });
        }
    }
    for p in &s.places {
        let key = format!("place:{}", p.id);
        let g = [grade(&p.name, needle), grade(&p.place, needle).map(|g| g.min(75)), grade(&p.note, needle).map(|g| g.min(70)), s.records.labels.get(&key).and_then(|l| grade(l, needle)), s.records.notes.get(&key).and_then(|n| grade(n, needle)).map(|g| g.min(70))].into_iter().flatten().max();
        if let Some(g) = g {
            let keys: usize = s.vaults.iter().flat_map(|v| v.keys.iter()).filter(|k| s.key_place.get(&k.pubkey).map(|x| x == &p.id).unwrap_or(false)).map(|k| k.pubkey.clone()).collect::<std::collections::HashSet<_>>().len();
            hits.push(Hit { kind: "place", vault: None, id: p.id.clone(), title: p.name.clone(), sub: format!("Location{}{}", if p.place.is_empty() { String::new() } else { format!(" · {}", p.place) }, if keys > 0 { format!(" · {keys} key{}", if keys == 1 { "" } else { "s" }) } else { String::new() }), when: None, sats: None, fiat: None, score: g });
        }
    }
    for e in &s.entities {
        if let Some(g) = [grade(&e.name, needle), grade(&e.kind, needle).map(|g| g.min(65)), grade(&e.note, needle).map(|g| g.min(70))].into_iter().flatten().max() {
            let n = s.vaults.iter().filter(|v| v.entity.as_deref() == Some(&e.id)).count();
            hits.push(Hit { kind: "entity", vault: None, id: e.id.clone(), title: e.name.clone(), sub: format!("Entity · {}{}", if e.kind.is_empty() { "owner".to_string() } else { e.kind.clone() }, if n > 0 { format!(" · {n} vault{}", if n == 1 { "" } else { "s" }) } else { String::new() }), when: None, sats: None, fiat: None, score: g });
        }
    }

    // best first; the same thing found twice keeps its best score; each kind gets its share of the list
    let rank = |k: &str| match k { "vault" => 0, "entity" => 1, "place" => 2, "key" => 3, "xkey" => 4, "contact" => 5, "tx" => 6, "coin" => 7, _ => 8 };
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(rank(a.kind).cmp(&rank(b.kind))).then(b.when.cmp(&a.when)).then(a.title.cmp(&b.title)));
    let mut seen_ids = std::collections::HashSet::new();
    let mut per: HashMap<&'static str, usize> = HashMap::new();
    for h in hits {
        if !seen_ids.insert((h.kind, h.vault.clone(), h.id.clone())) { continue; }
        let n = per.entry(h.kind).or_default();
        if *n >= PER_KIND || out.hits.len() >= TOTAL { *out.more.entry(h.kind).or_default() += 1; continue; }
        *n += 1; out.hits.push(h);
    }
    out.ms = t0.elapsed().as_millis();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    const NOW: u32 = 1_792_560_000;   // 2026-10-21 04:53 UTC, 14:53 in Brisbane
    #[test]
    fn dates_are_read_every_way_people_write_them() {
        let tz = -600;   // Brisbane
        assert_eq!(span_of("2026-10-19", NOW, tz), Some(Span::Day(2026, 10, 19)));
        assert_eq!(span_of("19/10/2026", NOW, tz), Some(Span::Day(2026, 10, 19)));
        assert_eq!(span_of("19 Oct 2026", NOW, tz), Some(Span::Day(2026, 10, 19)));
        assert_eq!(span_of("Oct 19, 2026", NOW, tz), Some(Span::Day(2026, 10, 19)));
        assert_eq!(span_of("october 2026", NOW, tz), Some(Span::Month(2026, 10)));
        assert_eq!(span_of("2026-10", NOW, tz), Some(Span::Month(2026, 10)));
        assert_eq!(span_of("oct", NOW, tz), Some(Span::Month(2026, 10)));
        assert_eq!(span_of("19 oct", NOW, tz), Some(Span::Day(2026, 10, 19)));
        assert_eq!(span_of("2026", NOW, tz), Some(Span::Year(2026)));
        assert_eq!(span_of("today", NOW, tz), Some(Span::Day(2026, 10, 21)));
        assert_eq!(span_of("last month", NOW, tz), Some(Span::Month(2026, 9)));
        assert_eq!(span_of("31 feb 2026", NOW, tz), None);
        assert_eq!(span_of("rent", NOW, tz), None);
        assert_eq!(span_of("50000", NOW, tz), None);
        assert_eq!(span_of("bc1qxyz", NOW, tz), None);
        // a day in Brisbane starts ten hours before it does in UTC
        let (a, b) = Span::Day(2026, 10, 19).range(tz);
        assert_eq!(a, 1_792_368_000 - 36_000 + 86_400 * 0); assert_eq!(b - a, 86_400);
    }
    #[test]
    fn amounts_are_read_as_bitcoin_sats_or_money() {
        assert_eq!(amount_of("0.25", "AUD"), (Some(25_000_000), Some(0.25)));
        assert_eq!(amount_of("0.25 btc", "AUD"), (Some(25_000_000), None));
        assert_eq!(amount_of("50,000", "AUD"), (Some(50_000), Some(50_000.0)));
        assert_eq!(amount_of("50000 sats", "AUD"), (Some(50_000), None));
        assert_eq!(amount_of("$1,200", "AUD"), (None, Some(1_200.0)));
        assert_eq!(amount_of("1200 aud", "AUD"), (None, Some(1_200.0)));
        assert_eq!(amount_of("rent", "AUD"), (None, None));
        assert_eq!(amount_of("1.2.3", "AUD"), (None, None));
    }
    #[test]
    fn grades_prefer_the_whole_then_the_start_then_a_word() {
        assert_eq!(grade("Rent", "rent"), Some(100));
        assert_eq!(grade("Rent October", "rent"), Some(85));
        assert_eq!(grade("October rent", "rent"), Some(75));
        assert_eq!(grade("Parent", "rent"), Some(60));
        assert_eq!(grade("Parent", "x"), None);
        assert_eq!(grade_id("abcdef0123", "abc"), None);
        assert_eq!(grade_id("abcdef0123", "def0"), Some(60));
    }
    /// A whole profile, searched every way the window will ask: by a word in a label, a note, a name, an id fragment,
    /// an amount, a fingerprint — and each hit says which vault and which page it opens.
    #[test]
    fn one_search_reaches_every_kind_of_thing_in_the_profile() {
        let desc = "wpkh([d34db33f/84h/1h/0h]tpubDBf5wCeqg3KrLJiXaveDzD5JtFJ1ss9NVvFMx4RYS73SjwPEEawcAQ7V1B5DGM4gunWDeYNrnkc49sUaf7mS1wUKiJJQD6WEctExUQoLvrg/<0;1>/*)";
        let info = wallet::ext_parse(desc).unwrap().unwrap();
        let mut v = crate::ext_vault("Rainy day".into(), &info, &["Coldcard".into()]);
        v.network = "signet".into(); v.scan_max = 3; v.next = 2;
        let a0 = wallet::address_at(&v, 0).unwrap();
        let txid = "f1e2d3c4b5a69788f1e2d3c4b5a69788f1e2d3c4b5a69788f1e2d3c4b5a69788".to_string();
        v.labels.insert(format!("tx:{txid}"), "Rent October".into());
        v.notes.insert(format!("tx:{txid}"), "paid from the office".into());
        v.labels.insert(format!("addr:{a0}"), "Salary".into());
        v.labels.insert(format!("out:{txid}:0"), "Kept for the car".into());
        let mut s = Store { vaults: vec![v.clone()], ..Default::default() };
        s.seen.insert(v.name.clone(), wallet::Seen { txs: [(txid.clone(), 100)].into(), coins: [(format!("{txid}:0"), 50_000u64), (format!("{txid}:1"), 7_000u64)].into(), primed: true });
        s.contacts.push(wallet::Contact { name: "Alice Vendor".into(), address: "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx".into(), network: "signet".into(), added: 1, used: 2, last_used: 1 });
        s.places.push(wallet::Place { id: "p1".into(), name: "Office safe".into(), note: String::new(), place: "12 Example St".into(), lat: None, lon: None, added: 1 });
        s.entities.push(wallet::Entity { id: "e1".into(), name: "Mineracks Pty Ltd".into(), kind: "company".into(), note: String::new(), added: 1 });
        s.vaults[0].entity = Some("e1".into());
        let find = |q: &str| run(&s, q, "", NOW, -600);
        let kinds = |f: &Found| f.hits.iter().map(|h| (h.kind, h.title.clone())).collect::<Vec<_>>();
        // a label on a transaction, and the note under it
        let f = find("rent"); assert_eq!(kinds(&f), vec![("tx", "Rent October".into())]); assert_eq!(f.hits[0].vault.as_deref(), Some("Rainy day")); assert_eq!(f.hits[0].id, txid);
        assert_eq!(kinds(&find("office")), vec![("place", "Office safe".into()), ("tx", "Rent October".into())]);
        // the id itself, by a fragment of at least four characters
        assert_eq!(kinds(&find("a69788f1")), vec![("tx", "Rent October".into()), ("coin", "Kept for the car".into()), ("coin", short(&format!("{txid}:1")))]);
        assert!(find("a69").hits.is_empty());
        // a coin by its exact value, in sats
        let f = find("50,000"); assert_eq!(kinds(&f), vec![("coin", "Kept for the car".into())]); assert_eq!(f.hits[0].sats, Some(50_000)); assert_eq!(f.reading.sats, Some(50_000));
        assert!(find("50,001").hits.is_empty());
        // an address by its tail, and by its label; the hit says where on the chain it is
        let tail = &a0[a0.len() - 6..];
        let f = find(tail); assert_eq!(kinds(&f), vec![("addr", "Salary".into())]); assert_eq!(f.hits[0].id, a0); assert!(f.hits[0].sub.contains("receive #0"), "{}", f.hits[0].sub); assert!(f.addresses >= 4 + 20);
        assert_eq!(kinds(&find("salary")), vec![("addr", "Salary".into())]);
        // the vault, the entity that owns it, the signer by fingerprint and by name, the address book, the place
        assert_eq!(kinds(&find("rainy")), vec![("vault", "Rainy day".into())]);
        assert_eq!(kinds(&find("mineracks")), vec![("entity", "Mineracks Pty Ltd".into()), ("vault", "Rainy day".into())]);
        let f = find("d34db33f"); assert_eq!(kinds(&f), vec![("xkey", "Coldcard".into())]); assert!(f.hits[0].sub.contains("m/84"), "{}", f.hits[0].sub);
        assert_eq!(kinds(&find("coldcard")), vec![("xkey", "Coldcard".into())]);
        assert_eq!(kinds(&find("alice")), vec![("contact", "Alice Vendor".into())]);
        assert_eq!(kinds(&find("xw7kxpjzsx")), vec![("contact", "Alice Vendor".into())]);
        assert_eq!(kinds(&find("example st")), vec![("place", "Office safe".into())]);
        // nothing, and nothing asked: an empty query looks at nothing
        assert!(find("zebra").hits.is_empty()); assert!(find("   ").hits.is_empty());
        // the days of a month are read even though no transaction here has a time yet
        assert_eq!(find("oct 2026").reading.span.as_deref(), Some("Oct 2026"));
    }
    #[test]
    fn civil_days_round_trip() {
        for d in [0i64, 1, 59, 60, 365, 20_000, 20_746, 50_000] { let (y, m, dd) = civil_from_days(d); assert_eq!(days_from_civil(y, m, dd), d, "{d}"); }
        assert_eq!(civil_from_days(20_747), (2026, 10, 21));
    }
}
