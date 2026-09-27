//! An exchange account, read from the exports the exchange gives. Coins are bought and held at an exchange before
//! self-custody; few exchanges offer any API, but every one offers a statement. Read here, an account gives three
//! things the chain cannot: a real cost basis (a buy's fiat paid, fee included), the balance still held there, and
//! the withdrawals that became deposits in the vaults — linked by amount and time like any other move.
//!
//! Two formats are known by their header: Amber (one CSV, an id on every row, local time, no balance) and Bitaroo
//! (a zip per financial year holding an AUD statement and a BTC statement, UTC, a running balance, no ids). Every
//! figure is read from the file's own decimal — never a float — and a running balance, where there is one, is
//! checked row by row and the result shown, never assumed.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;

type R<T> = Result<T, String>;

/// One row of the account, as kept. `kind`: buy · sell · fiat_in · fiat_out · btc_out · btc_in · other.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExRow {
    pub id: String,
    pub time: u32,
    pub kind: String,
    /// signed sats: + into the account, − out; the fee is separate
    pub sats: i64,
    /// signed cents of the account's currency: + into the account, − out; the fee is separate
    pub fiat: i64,
    #[serde(default)] pub fee_sats: u64,
    #[serde(default)] pub fee_fiat: u64,
    /// cents per bitcoin, as the exchange stated it, when it did
    #[serde(default)] pub rate: Option<u64>,
    #[serde(default)] pub memo: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Import { pub at: u64, pub files: Vec<String>, pub added: usize }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Exchange {
    /// "Amber" | "Bitaroo"
    pub provider: String,
    pub ccy: String,
    pub rows: Vec<ExRow>,
    pub imports: Vec<Import>,
    /// what is held there now: the newest statement's stated balance, or the rows' sum when nothing is stated
    pub btc_now: i64,
    pub fiat_now: i64,
    /// the balances are the exchange's own statement (Bitaroo), not our sum
    #[serde(default)] pub stated: bool,
    /// a stated running balance chained row by row through every file: Some(true) when it did
    #[serde(default)] pub agrees: Option<bool>,
    /// for another exchange's CSV: which column is what, kept so the next export needs no mapping again
    #[serde(default)] pub mapping: Option<Mapping>,
}

/// Which column of a CSV is what, for an exchange this app does not know by header. Column indexes into the header;
/// a role left out is simply not read.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Mapping {
    pub provider: String,
    pub date: usize,
    /// hours east of UTC the file's times are written in
    #[serde(default)] pub tz_hours: i32,
    #[serde(default)] pub kind: Option<usize>,
    #[serde(default)] pub sats: Option<usize>,
    /// "btc" (a decimal of bitcoin) or "sats"
    #[serde(default)] pub sats_unit: String,
    #[serde(default)] pub fiat: Option<usize>,
    #[serde(default)] pub fee_fiat: Option<usize>,
    #[serde(default)] pub fee_sats: Option<usize>,
    #[serde(default)] pub id: Option<usize>,
    #[serde(default)] pub ccy: String,
}

/// The head of a CSV, for the person to say which column is what.
#[derive(Serialize, Debug, Clone)]
pub struct Raw { pub files: Vec<String>, pub header: Vec<String>, pub sample: Vec<Vec<String>>, pub rows: usize }

/// What the person is shown before anything is kept.
#[derive(Serialize, Debug, Clone)]
pub struct Preview {
    pub provider: String, pub ccy: String, pub files: Vec<String>, pub rows: usize,
    pub first: u32, pub last: u32,
    pub buys: usize, pub bought_sats: u64, pub spent_fiat: u64, pub buy_fees_fiat: u64,
    pub sells: usize, pub sold_sats: u64, pub got_fiat: u64,
    pub withdrawals: usize, pub withdrawn_sats: u64, pub wd_fee_sats: u64,
    pub deposits: usize, pub deposited_fiat: u64, pub fiat_out: u64, pub rewards_sats: u64,
    pub btc_now: i64, pub fiat_now: i64, pub stated: bool, pub agrees: Option<bool>,
    pub unknown: Vec<String>,
    /// sats held after each row, oldest first, for the sketch
    pub series: Vec<(u32, i64)>,
    /// against an account already here: how many of these rows it does not have
    pub new: Option<usize>,
}

// ---------- reading numbers and dates exactly ----------

/// "$1830.00" / "-199.62" / "1,234.5" → signed cents, exactly.
pub fn cents(s: &str) -> R<i64> {
    let t: String = s.trim().chars().filter(|c| !matches!(c, '$' | ',' | ' ' | 'A' | 'U' | 'D')).collect();
    if t.is_empty() { return Ok(0); }
    let (neg, t) = match t.strip_prefix('-') { Some(r) => (true, r), None => (false, t.as_str()) };
    let (w, f) = t.split_once('.').unwrap_or((t, ""));
    if !w.chars().all(|c| c.is_ascii_digit()) || !f.chars().all(|c| c.is_ascii_digit()) || f.len() > 2 { return Err(format!("'{}' is not an amount of money", s.trim())); }
    let v = w.parse::<i64>().map_err(|_| format!("'{}' is not an amount of money", s.trim()))? * 100 + format!("{f:0<2}").parse::<i64>().unwrap();
    Ok(if neg { -v } else { v })
}
/// "0.00437102" / "-0.0043" → signed sats, exactly.
pub fn sats_signed(s: &str) -> R<i64> {
    let t = s.trim();
    if t.is_empty() { return Ok(0); }
    let (neg, t) = match t.strip_prefix('-') { Some(r) => (true, r), None => (false, t) };
    let v = crate::statement::sats(t)? as i64;
    Ok(if neg { -v } else { v })
}
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) as i64 + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
fn epoch(y: i64, m: u32, d: u32, h: u32, mi: u32, s: u32) -> u32 { (days_from_civil(y, m, d) * 86400 + h as i64 * 3600 + mi as i64 * 60 + s as i64) as u32 }
/// "2022-02-15 16:22:33" at `offset` hours east of UTC.
fn when_iso(s: &str, offset: i32) -> R<u32> {
    let t = s.trim();
    let bad = || format!("'{t}' is not a date and time");
    let (d, c) = t.split_once(' ').ok_or_else(bad)?;
    let dp: Vec<u32> = d.split('-').map(|x| x.parse().map_err(|_| bad())).collect::<R<_>>()?;
    let cp: Vec<u32> = c.split(':').map(|x| x.parse().map_err(|_| bad())).collect::<R<_>>()?;
    if dp.len() != 3 || cp.len() != 3 { return Err(bad()); }
    Ok((epoch(dp[0] as i64, dp[1], dp[2], cp[0], cp[1], cp[2]) as i64 - offset as i64 * 3600) as u32)
}
/// "June 30, 2023 9:10:02 PM" (UTC).
fn when_long(s: &str) -> R<u32> {
    let t = s.trim();
    let bad = || format!("'{t}' is not a date and time");
    let p: Vec<&str> = t.split_whitespace().collect();
    if p.len() != 5 { return Err(bad()); }
    let m = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"].iter().position(|x| x.starts_with(&p[0].to_lowercase())).ok_or_else(bad)? as u32 + 1;
    let d: u32 = p[1].trim_end_matches(',').parse().map_err(|_| bad())?;
    let y: i64 = p[2].parse().map_err(|_| bad())?;
    let cp: Vec<u32> = p[3].split(':').map(|x| x.parse().map_err(|_| bad())).collect::<R<_>>()?;
    if cp.len() != 3 { return Err(bad()); }
    let h = match (cp[0] % 12, p[4].to_uppercase().as_str()) { (h, "AM") => h, (h, "PM") => h + 12, _ => return Err(bad()) };
    Ok(epoch(y, m, d, h, cp[1], cp[2]))
}

// ---------- files: CSVs, and zips of CSVs ----------

fn csv_rows(text: &str) -> Vec<Vec<String>> { crate::statement::csv_rows_pub(text) }

/// The CSV texts in what was given: a CSV as itself, a zip opened for the CSVs inside it (stored or deflated;
/// nothing else is read, and nothing is written anywhere).
pub fn texts(files: &[(String, Vec<u8>)]) -> R<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (name, bytes) in files {
        if bytes.starts_with(b"PK\x03\x04") {
            for (inner, data) in unzip(bytes)? {
                if inner.to_lowercase().ends_with(".csv") && !inner.contains("__MACOSX") && !inner.rsplit('/').next().unwrap_or("").starts_with("._") {
                    out.push((format!("{name} › {inner}"), String::from_utf8_lossy(&data).into_owned()));
                }
            }
        } else {
            out.push((name.clone(), String::from_utf8_lossy(bytes).into_owned()));
        }
    }
    if out.is_empty() { return Err("no CSV was found in what was chosen".into()); }
    Ok(out)
}
/// A small reader for the zips exchanges hand out: local headers walked in order; method 0 (stored) or 8 (deflate).
/// Sizes are taken from the central directory when the local header defers them (bit 3), so streamed zips read too.
fn unzip(bytes: &[u8]) -> R<Vec<(String, Vec<u8>)>> {
    let u16 = |i: usize| -> R<u16> { bytes.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or("the zip is cut short".into()) };
    let u32 = |i: usize| -> R<u32> { bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or("the zip is cut short".into()) };
    // the central directory: name → (method, compressed size, uncompressed size, local header offset)
    let eocd = (0..bytes.len().saturating_sub(21)).rev().find(|&i| &bytes[i..i + 4] == b"PK\x05\x06").ok_or("the zip has no directory")?;
    let (n, cd) = (u16(eocd + 10)? as usize, u32(eocd + 16)? as usize);
    let mut out = Vec::new();
    let mut p = cd;
    for _ in 0..n {
        if bytes.get(p..p + 4) != Some(b"PK\x01\x02") { return Err("the zip's directory is damaged".into()); }
        let (method, csize, usize_, nlen, elen, clen, off) = (u16(p + 10)?, u32(p + 20)? as usize, u32(p + 24)? as usize, u16(p + 28)? as usize, u16(p + 30)? as usize, u16(p + 32)? as usize, u32(p + 42)? as usize);
        let name = String::from_utf8_lossy(bytes.get(p + 46..p + 46 + nlen).ok_or("the zip is cut short")?).into_owned();
        p += 46 + nlen + elen + clen;
        if name.ends_with('/') { continue; }
        if bytes.get(off..off + 4) != Some(b"PK\x03\x04") { return Err("the zip's entries are damaged".into()); }
        let (lnlen, lelen) = (u16(off + 26)? as usize, u16(off + 28)? as usize);
        let start = off + 30 + lnlen + lelen;
        let data = bytes.get(start..start + csize).ok_or("the zip is cut short")?;
        if usize_ > 64 << 20 { return Err(format!("{name} is too large to be a statement")); }
        let raw = match method {
            0 => data.to_vec(),
            8 => { let mut v = Vec::with_capacity(usize_); flate2::read::DeflateDecoder::new(data).take(usize_ as u64 + 1).read_to_end(&mut v).map_err(|e| format!("{name}: {e}"))?; v }
            m => return Err(format!("{name} is compressed in a way this app does not read (method {m})")),
        };
        out.push((name, raw));
    }
    Ok(out)
}

// ---------- Amber ----------

const AMBER_HEAD: [&str; 5] = ["product", "status", "From Currency", "To Currency", "id"];

fn amber(name: &str, text: &str) -> R<Vec<ExRow>> {
    let rows = csv_rows(text.trim_start_matches('\u{feff}'));
    let hi = rows.iter().position(|r| r.first().map(|c| c.trim().starts_with("createdAt")).unwrap_or(false)).ok_or_else(|| format!("{name}: no createdAt header"))?;
    let head: Vec<String> = rows[hi].iter().map(|h| h.trim().to_string()).collect();
    let col = |want: &str| head.iter().position(|h| h == want).ok_or_else(|| format!("{name}: no '{want}' column"));
    for w in AMBER_HEAD { col(w)?; }
    // the offset is in the header's own words: "createdAt (+10) (Date)"
    let offset: i32 = head[0].split('(').nth(1).and_then(|s| s.split(')').next()).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let (c_prod, c_status, c_from, c_to, c_id) = (col("product")?, col("status")?, col("From Currency")?, col("To Currency")?, col("id")?);
    let (c_in_fiat, c_in_btc, c_out_fiat, c_out_btc) = (col("Input Amount (AUD)").ok(), col("Input Amount (BTC)")?, col("Output Amount (AUD)").ok(), col("Output Amount (BTC)")?);
    let (c_txfee, c_exfee, c_rate) = (head.iter().position(|h| h.starts_with("Transaction Fees")), head.iter().position(|h| h.starts_with("Exchange Fee")), head.iter().position(|h| h == "Exchange Rate"));
    let mut out = Vec::new();
    for (n, r) in rows[hi + 1..].iter().enumerate() {
        if r.iter().all(|f| f.trim().is_empty()) { continue; }
        if r.len() < head.len() { return Err(format!("{name}: row {} has {} columns, not {}", n + 1, r.len(), head.len())); }
        let get = |i: Option<usize>| i.map(|i| r[i].trim()).unwrap_or("");
        if !r[c_status].trim().eq_ignore_ascii_case("SETTLED") { continue; }   // a request that never settled moved nothing
        let (prod, from, to) = (r[c_prod].trim().to_uppercase(), r[c_from].trim().to_uppercase(), r[c_to].trim().to_uppercase());
        let time = when_iso(&r[0], offset)?;
        let (in_fiat, in_btc, out_fiat, out_btc) = (cents(get(c_in_fiat))?, sats_signed(&r[c_in_btc])?, cents(get(c_out_fiat))?, sats_signed(&r[c_out_btc])?);
        let fee_fiat = (cents(get(c_txfee))?.unsigned_abs() + cents(get(c_exfee))?.unsigned_abs()) as u64;
        let rate = cents(get(c_rate)).ok().filter(|&c| c > 0).map(|c| c as u64);
        let id = r[c_id].trim().to_string();
        let id = if id.is_empty() { hex::encode(Sha256::digest(format!("amber|{}|{prod}|{in_fiat}|{in_btc}|{out_fiat}|{out_btc}", r[0].trim()).as_bytes())) } else { id };
        let row = match (prod.as_str(), from.as_str(), to.as_str()) {
            (p, "AUD", "BTC") if p.contains("PURCHASE") || p.contains("BUY") => ExRow { id, time, kind: "buy".into(), sats: out_btc, fiat: -in_fiat.abs(), fee_sats: 0, fee_fiat, rate, memo: prod.replace('_', " ").to_lowercase() },
            (p, "BTC", "AUD") if p.contains("SELL") => ExRow { id, time, kind: "sell".into(), sats: -in_btc.abs(), fiat: out_fiat.abs(), fee_sats: 0, fee_fiat, rate, memo: prod.replace('_', " ").to_lowercase() },
            ("EFT", ..) | ("DEPOSIT", ..) | ("PAYID", ..) | ("NPP", ..) => ExRow { id, time, kind: "fiat_in".into(), sats: 0, fiat: in_fiat.abs().max(out_fiat.abs()), fee_sats: 0, fee_fiat, rate: None, memo: prod.to_lowercase() },
            ("WITHDRAWAL_REQUEST", "AUD", _) | ("WITHDRAWAL", "AUD", _) => ExRow { id, time, kind: "fiat_out".into(), sats: 0, fiat: -in_fiat.abs().max(out_fiat.abs()), fee_sats: 0, fee_fiat, rate: None, memo: "withdrawal".into() },
            ("SEND", "BTC", _) | ("WITHDRAWAL", "BTC", _) | ("WITHDRAWAL_REQUEST", "BTC", _) => ExRow { id, time, kind: "btc_out".into(), sats: -in_btc.abs().max(out_btc.abs()), fiat: 0, fee_sats: 0, fee_fiat: 0, rate: None, memo: "sent to a wallet".into() },
            ("SATS_BACK", ..) | ("RECEIVE", ..) | ("REWARD", ..) | ("REFERRAL", ..) => ExRow { id, time, kind: "btc_in".into(), sats: out_btc.abs().max(in_btc.abs()), fiat: 0, fee_sats: 0, fee_fiat: 0, rate: None, memo: if prod == "SATS_BACK" { "sats back".into() } else { prod.replace('_', " ").to_lowercase() } },
            _ => ExRow { id, time, kind: "other".into(), sats: out_btc - in_btc.abs().min(0).abs() + (if from == "BTC" { -in_btc.abs() } else { 0 }), fiat: (if to == "AUD" { out_fiat.abs() } else { 0 }) - (if from == "AUD" { in_fiat.abs() } else { 0 }), fee_sats: 0, fee_fiat, rate, memo: format!("{} {from}→{to}", prod.replace('_', " ").to_lowercase()) },
        };
        out.push(row);
    }
    Ok(out)
}

// ---------- Bitaroo ----------

struct BRow { time: u32, kind: String, amount: i64, balance: i64, id: String }

fn bitaroo_file(name: &str, text: &str) -> R<(String, Vec<BRow>, bool)> {
    let rows = csv_rows(text.trim_start_matches('\u{feff}'));
    let head: Vec<String> = rows.first().ok_or_else(|| format!("{name}: empty"))?.iter().map(|h| h.trim().to_string()).collect();
    if head.len() != 4 || head[0] != "Date & Time (UTC)" || head[1] != "Transaction Type" { return Err(format!("{name}: not a Bitaroo statement (columns {})", head.join(", "))); }
    let ccy = head[2].strip_prefix("Amount ").ok_or_else(|| format!("{name}: no amount column"))?.to_string();
    let money = ccy != "BTC";
    let mut out = Vec::new();
    for (n, r) in rows[1..].iter().enumerate() {
        if r.iter().all(|f| f.trim().is_empty()) { continue; }
        if r.len() != 4 { return Err(format!("{name}: row {} has {} columns, not 4", n + 2, r.len())); }
        let (amount, balance) = if money { (cents(&r[2])?, cents(&r[3])?) } else { (sats_signed(&r[2])?, sats_signed(&r[3])?) };
        let id = hex::encode(Sha256::digest(format!("bitaroo|{ccy}|{}|{}|{}|{}", r[0].trim(), r[1].trim(), r[2].trim(), r[3].trim()).as_bytes()));
        out.push(BRow { time: when_long(&r[0])?, kind: r[1].trim().to_string(), amount, balance, id });
    }
    // newest first in the file; oldest first here, keeping the file's order within one second — then the running
    // balance must chain: each row's balance is the one before plus its amount
    out.reverse();
    // balance must chain: each row's balance is the one before plus its amount — to the sat for coins, and to within a
    // cent for money, because the exchange rounds each fee to the cent on the statement while its own ledger runs finer
    let slack = if money { 1 } else { 0 };
    let agrees = out.windows(2).all(|w| (w[1].balance - (w[0].balance + w[1].amount)).abs() <= slack) && out.first().map(|f| f.balance - f.amount >= -slack).unwrap_or(true);
    Ok((ccy, out, agrees))
}

fn bitaroo(files: &[(String, String)]) -> R<(Vec<ExRow>, String, i64, i64, bool)> {
    let (mut fiat, mut btc, mut ccy, mut agrees) = (Vec::<BRow>::new(), Vec::<BRow>::new(), String::new(), true);
    let (mut fiat_now, mut btc_now, mut fiat_t, mut btc_t) = (0i64, 0i64, 0u32, 0u32);
    for (name, text) in files {
        let (c, rows, ok) = bitaroo_file(name, text)?;
        agrees &= ok;
        if let Some(last) = rows.last() { if c == "BTC" { if last.time >= btc_t { btc_t = last.time; btc_now = last.balance; } } else if last.time >= fiat_t { fiat_t = last.time; fiat_now = last.balance; } }
        if c == "BTC" { btc.extend(rows); } else { if !ccy.is_empty() && ccy != c { return Err(format!("statements in two currencies, {ccy} and {c}")); } ccy = c; fiat.extend(rows); }
    }
    if ccy.is_empty() { ccy = "AUD".into(); }
    // fold: a fee row belongs to the row it was charged on, at the same second; a buy is one row from two statements
    let mut seen = std::collections::HashSet::new();
    let (mut fiat_by_t, mut btc_by_t): (std::collections::BTreeMap<u32, Vec<BRow>>, std::collections::BTreeMap<u32, Vec<BRow>>) = (Default::default(), Default::default());
    for r in fiat { if seen.insert(r.id.clone()) { fiat_by_t.entry(r.time).or_default().push(r); } }
    for r in btc { if seen.insert(r.id.clone()) { btc_by_t.entry(r.time).or_default().push(r); } }
    let mut out = Vec::new();
    let is_buy = |k: &str| k == "Buy Bitcoin" || k == "Express Buy" || k == "Buy";
    let is_sell = |k: &str| k == "Sell Bitcoin" || k == "Express Sell" || k == "Sell";
    let times: std::collections::BTreeSet<u32> = fiat_by_t.keys().chain(btc_by_t.keys()).copied().collect();
    for t in times {
        let mut f = fiat_by_t.remove(&t).unwrap_or_default();
        let mut b = btc_by_t.remove(&t).unwrap_or_default();
        let take_fee = |v: &mut Vec<BRow>, k: &str| -> u64 { let mut fee = 0u64; v.retain(|r| if r.kind == k { fee += r.amount.unsigned_abs(); false } else { true }); fee };
        let (buy_fee, dep_fee, fwd_fee, wd_fee) = (take_fee(&mut f, "Buy Fee"), take_fee(&mut f, "Deposit Fee"), take_fee(&mut f, "Withdrawal Fee"), take_fee(&mut b, "Withdrawal Fee"));
        let sell_fee = take_fee(&mut f, "Sell Fee");
        // buys and sells: a BTC row and a fiat row of the same kind at the same second
        let mut i = 0;
        while i < b.len() {
            if is_buy(&b[i].kind) || is_sell(&b[i].kind) {
                let buy = is_buy(&b[i].kind);
                let j = f.iter().position(|r| if buy { is_buy(&r.kind) } else { is_sell(&r.kind) });
                let br = b.remove(i);
                let fr = j.map(|j| f.remove(j));
                let fiat_amt = fr.as_ref().map(|r| r.amount).unwrap_or(0);
                let rate = if br.amount != 0 && fiat_amt != 0 { Some(((fiat_amt.unsigned_abs() as u128 * 100_000_000) / br.amount.unsigned_abs() as u128) as u64) } else { None };
                out.push(ExRow { id: br.id, time: t, kind: if buy { "buy" } else { "sell" }.into(), sats: br.amount, fiat: fiat_amt, fee_sats: 0, fee_fiat: if buy { buy_fee } else { sell_fee }, rate, memo: br.kind.to_lowercase() });
                continue;
            }
            i += 1;
        }
        for br in b {
            let kind = match br.kind.as_str() { "Withdrawal" => "btc_out", "Deposit" | "Receive" => "btc_in", k if k.contains("Reward") || k.contains("Referral") || k.contains("Bonus") => "btc_in", _ => "other" };
            out.push(ExRow { id: br.id, time: t, kind: kind.into(), sats: br.amount, fiat: 0, fee_sats: if kind == "btc_out" { wd_fee } else { 0 }, fee_fiat: 0, rate: None, memo: br.kind.to_lowercase() });
        }
        for fr in f {
            let kind = match fr.kind.as_str() { "Deposit" => "fiat_in", "Withdrawal" => "fiat_out", k if is_buy(k) => "buy", k if is_sell(k) => "sell", _ => "other" };
            out.push(ExRow { id: fr.id, time: t, kind: kind.into(), sats: 0, fiat: fr.amount, fee_sats: 0, fee_fiat: match kind { "fiat_in" => dep_fee, "fiat_out" => fwd_fee, _ => 0 }, rate: None, memo: fr.kind.to_lowercase() });
        }
    }
    Ok((out, ccy, btc_now, fiat_now, agrees))
}

// ---------- another exchange's CSV, read by a mapping ----------

/// The header and a few rows of the CSVs, so the person can point at the columns.
pub fn raw(files: &[(String, Vec<u8>)]) -> R<Raw> {
    let texts = texts(files)?;
    let mut header: Option<Vec<String>> = None; let mut sample = Vec::new(); let mut rows = 0;
    for (name, text) in &texts {
        let rs: Vec<Vec<String>> = csv_rows(text.trim_start_matches('\u{feff}')).into_iter().filter(|r| r.iter().any(|f| !f.trim().is_empty())).collect();
        // the header is the first row with two or more columns; anything above it (a title, an email) is skipped
        let hi = rs.iter().position(|r| r.len() >= 2).ok_or_else(|| format!("{name}: no table in it"))?;
        let h: Vec<String> = rs[hi].iter().map(|c| c.trim().to_string()).collect();
        match &header { None => header = Some(h), Some(have) if have != &h => return Err(format!("{name} has different columns from the first file: {} against {}", h.join(", "), have.join(", "))), _ => {} }
        for r in &rs[hi + 1..] { if sample.len() < 6 { sample.push(r.clone()); } rows += 1; }
    }
    Ok(Raw { files: files.iter().map(|(n, _)| n.clone()).collect(), header: header.unwrap_or_default(), sample, rows })
}
/// A time in any of the ways exports write it: ISO "2022-02-15 16:22:33" (or with a T), "15/02/2022 16:22",
/// "June 30, 2023 9:10:02 PM", or seconds since 1970. Hours east of UTC are taken off.
fn when_any(s: &str, tz: i32) -> R<u32> {
    let t = s.trim().trim_end_matches('Z');
    if let Ok(n) = t.parse::<u64>() { return Ok(if n > 20_000_000_000 { n / 1000 } else { n } as u32); }
    if let Ok(v) = when_long(t) { return Ok((v as i64 - tz as i64 * 3600) as u32); }
    let iso = t.replacen('T', " ", 1);
    let (d, c) = iso.split_once(' ').unwrap_or((iso.as_str(), "00:00:00"));
    let c = c.split(['+', '.']).next().unwrap_or("00:00:00");
    let cp: Vec<u32> = c.split(':').map(|x| x.trim().parse().unwrap_or(0)).collect();
    let (h, mi, se) = (cp.first().copied().unwrap_or(0), cp.get(1).copied().unwrap_or(0), cp.get(2).copied().unwrap_or(0));
    let dp: Vec<&str> = d.split(['-', '/', '.']).collect();
    if dp.len() != 3 { return Err(format!("'{}' is not a date and time", s.trim())); }
    let n: Vec<i64> = dp.iter().map(|x| x.trim().parse().map_err(|_| format!("'{}' is not a date and time", s.trim()))).collect::<R<_>>()?;
    let (y, m, dd) = if n[0] > 31 { (n[0], n[1] as u32, n[2] as u32) } else if n[2] > 31 { (n[2], n[1] as u32, n[0] as u32) } else { return Err(format!("'{}': which part is the year?", s.trim())); };
    if !(1..=12).contains(&m) || !(1..=31).contains(&dd) { return Err(format!("'{}' is not a date and time", s.trim())); }
    Ok((epoch(y, m, dd, h, mi, se) as i64 - tz as i64 * 3600) as u32)
}
/// What kind of row this is, from its kind column's words when there is one, else from the signs of its amounts.
fn kind_of(word: &str, sats: i64, fiat: i64) -> &'static str {
    let w = word.to_lowercase();
    if w.contains("buy") || w.contains("purchase") || w.contains("bought") { return "buy"; }
    if w.contains("sell") || w.contains("sold") { return "sell"; }
    if w.contains("withdraw") || w.contains("send") || w.contains("sent") { return if sats != 0 { "btc_out" } else { "fiat_out" }; }
    if w.contains("deposit") || w.contains("eft") || w.contains("payid") || w.contains("transfer in") { return if sats != 0 { "btc_in" } else { "fiat_in" }; }
    if w.contains("receiv") || w.contains("reward") || w.contains("bonus") || w.contains("referral") || w.contains("airdrop") { return "btc_in"; }
    match (sats.signum(), fiat.signum()) { (1, -1) => "buy", (-1, 1) => "sell", (1, _) => "btc_in", (-1, _) => "btc_out", (0, 1) => "fiat_in", (0, -1) => "fiat_out", _ => "other" }
}
pub fn mapped(files: &[(String, Vec<u8>)], m: &Mapping) -> R<Exchange> {
    if m.provider.trim().is_empty() { return Err("name the exchange".into()); }
    let texts = texts(files)?;
    let mut out = Vec::new();
    for (name, text) in &texts {
        let rs: Vec<Vec<String>> = csv_rows(text.trim_start_matches('\u{feff}')).into_iter().filter(|r| r.iter().any(|f| !f.trim().is_empty())).collect();
        let hi = rs.iter().position(|r| r.len() >= 2).ok_or_else(|| format!("{name}: no table in it"))?;
        let head = &rs[hi];
        let need = [Some(m.date), m.kind, m.sats, m.fiat, m.fee_fiat, m.fee_sats, m.id].into_iter().flatten().max().unwrap_or(0);
        if need >= head.len() { return Err(format!("{name} has {} columns; the mapping points past them", head.len())); }
        for (n, r) in rs[hi + 1..].iter().enumerate() {
            let get = |i: Option<usize>| i.and_then(|i| r.get(i)).map(|s| s.trim()).unwrap_or("");
            let time = when_any(get(Some(m.date)), m.tz_hours).map_err(|e| format!("{name}, row {}: {e}", n + 2))?;
            let sats = match m.sats_unit.as_str() { "sats" => get(m.sats).replace([',', ' '], "").parse::<i64>().map_err(|_| format!("{name}, row {}: '{}' is not a number of sats", n + 2, get(m.sats)))?, _ => sats_signed(&get(m.sats).replace([',', ' '], "")).map_err(|e| format!("{name}, row {}: {e}", n + 2))? };
            let fiat = cents(get(m.fiat)).map_err(|e| format!("{name}, row {}: {e}", n + 2))?;
            let fee_fiat = cents(get(m.fee_fiat)).map_err(|e| format!("{name}, row {}: {e}", n + 2))?.unsigned_abs();
            let fee_sats = match m.sats_unit.as_str() { "sats" => get(m.fee_sats).replace([',', ' '], "").parse::<i64>().unwrap_or(0).unsigned_abs(), _ => sats_signed(&get(m.fee_sats).replace([',', ' '], "")).unwrap_or(0).unsigned_abs() };
            let word = get(m.kind);
            let kind = kind_of(word, sats, fiat);
            // amounts take the sign their kind implies when the file wrote them unsigned
            let (sats, fiat) = match kind { "buy" => (sats.abs(), -fiat.abs()), "sell" => (-sats.abs(), fiat.abs()), "btc_out" => (-sats.abs(), fiat), "btc_in" => (sats.abs(), fiat), "fiat_in" => (sats, fiat.abs()), "fiat_out" => (sats, -fiat.abs()), _ => (sats, fiat) };
            let id = { let given = get(m.id); if given.is_empty() { hex::encode(Sha256::digest(format!("{}|{}|{}|{}|{}|{}", m.provider.to_lowercase(), get(Some(m.date)), word, get(m.sats), get(m.fiat), n).as_bytes())) } else { format!("{}:{given}", m.provider.to_lowercase()) } };
            out.push(ExRow { id, time, kind: kind.into(), sats, fiat, fee_sats, fee_fiat, rate: None, memo: word.to_lowercase() });
        }
    }
    if out.is_empty() { return Err("the file has no rows under its header".into()); }
    let mut seen = std::collections::HashSet::new(); out.retain(|r| seen.insert(r.id.clone()));
    out.sort_by_key(|r| (r.time, r.id.clone()));
    let btc: i64 = out.iter().map(|r| r.sats - r.fee_sats as i64).sum();
    let fiat: i64 = out.iter().map(|r| r.fiat - r.fee_fiat as i64).sum();
    Ok(Exchange { provider: m.provider.trim().to_string(), ccy: if m.ccy.trim().is_empty() { "AUD".into() } else { m.ccy.trim().to_uppercase() }, rows: out, imports: vec![Import { at: 0, files: files.iter().map(|(n, _)| n.clone()).collect(), added: 0 }], btc_now: btc, fiat_now: fiat, stated: false, agrees: None, mapping: Some(m.clone()) })
}

// ---------- the account ----------

pub fn parse(files: &[(String, Vec<u8>)]) -> R<Exchange> {
    let texts = texts(files)?;
    let amber_files: Vec<_> = texts.iter().filter(|(_, t)| t.lines().take(12).any(|l| l.trim_start().starts_with("createdAt"))).collect();
    let bitaroo_files: Vec<_> = texts.iter().filter(|(_, t)| t.lines().next().map(|l| l.contains("Date & Time (UTC)")).unwrap_or(false)).cloned().collect();
    let (provider, mut rows, ccy, btc_now, fiat_now, stated, agrees) = if !amber_files.is_empty() && bitaroo_files.is_empty() {
        let mut rows = Vec::new();
        for (n, t) in &amber_files { rows.extend(amber(n, t)?); }
        let btc: i64 = rows.iter().map(|r| r.sats - r.fee_sats as i64).sum();
        let fiat: i64 = rows.iter().map(|r| r.fiat - r.fee_fiat as i64).sum();
        ("Amber".to_string(), rows, "AUD".to_string(), btc, fiat, false, None)
    } else if !bitaroo_files.is_empty() && amber_files.is_empty() {
        let (rows, ccy, b, f, ok) = bitaroo(&bitaroo_files)?;
        ("Bitaroo".to_string(), rows, ccy, b, f, true, Some(ok))
    } else if amber_files.is_empty() && bitaroo_files.is_empty() {
        return Err("this is not an export this app knows: Amber (a header starting createdAt) or Bitaroo (Date & Time (UTC), Transaction Type, Amount, Balance)".into());
    } else { return Err("these files are from two different exchanges; import one exchange at a time".into()); };
    if rows.is_empty() { return Err("the export has no settled rows".into()); }
    let mut seen = std::collections::HashSet::new(); rows.retain(|r| seen.insert(r.id.clone()));
    rows.sort_by_key(|r| (r.time, r.id.clone()));
    Ok(Exchange { provider, ccy, rows, imports: vec![Import { at: 0, files: files.iter().map(|(n, _)| n.clone()).collect(), added: 0 }], btc_now, fiat_now, stated, agrees, mapping: None })
}

pub fn preview(ex: &Exchange, have: Option<&Exchange>) -> Preview {
    let r = &ex.rows;
    let by = |k: &'static str| r.iter().filter(move |x| x.kind == k);
    let mut bal = 0i64;
    let series = r.iter().map(|x| { bal += x.sats - x.fee_sats as i64; (x.time, bal) }).collect();
    let mut unknown: Vec<String> = by("other").map(|x| x.memo.clone()).collect(); unknown.sort(); unknown.dedup();
    Preview {
        provider: ex.provider.clone(), ccy: ex.ccy.clone(), files: ex.imports.last().map(|i| i.files.clone()).unwrap_or_default(), rows: r.len(),
        first: r.first().map(|x| x.time).unwrap_or(0), last: r.last().map(|x| x.time).unwrap_or(0),
        buys: by("buy").count(), bought_sats: by("buy").map(|x| x.sats.unsigned_abs()).sum(), spent_fiat: by("buy").map(|x| x.fiat.unsigned_abs() + x.fee_fiat).sum(), buy_fees_fiat: by("buy").map(|x| x.fee_fiat).sum(),
        sells: by("sell").count(), sold_sats: by("sell").map(|x| x.sats.unsigned_abs()).sum(), got_fiat: by("sell").map(|x| x.fiat.unsigned_abs()).sum(),
        withdrawals: by("btc_out").count(), withdrawn_sats: by("btc_out").map(|x| x.sats.unsigned_abs()).sum(), wd_fee_sats: by("btc_out").map(|x| x.fee_sats).sum(),
        deposits: by("fiat_in").count(), deposited_fiat: by("fiat_in").map(|x| x.fiat.unsigned_abs()).sum(), fiat_out: by("fiat_out").map(|x| x.fiat.unsigned_abs() + x.fee_fiat).sum(), rewards_sats: by("btc_in").map(|x| x.sats.unsigned_abs()).sum(),
        btc_now: ex.btc_now, fiat_now: ex.fiat_now, stated: ex.stated, agrees: ex.agrees, unknown, series,
        new: have.map(|h| { let ids: std::collections::HashSet<&str> = h.rows.iter().map(|x| x.id.as_str()).collect(); r.iter().filter(|x| !ids.contains(x.id.as_str())).count() }),
    }
}

/// A later export merged in: rows already here are left alone; new ones added; the balances and the check are the
/// newer export's when it reaches later than what was here. Returns how many rows were new.
pub fn merge(into: &mut Exchange, from: Exchange, at: u64) -> usize {
    let have: std::collections::HashSet<String> = into.rows.iter().map(|r| r.id.clone()).collect();
    let new: Vec<ExRow> = from.rows.into_iter().filter(|r| !have.contains(&r.id)).collect();
    let n = new.len();
    let newer = new.iter().map(|r| r.time).max().unwrap_or(0) >= into.rows.last().map(|r| r.time).unwrap_or(0);
    into.rows.extend(new);
    into.rows.sort_by_key(|r| (r.time, r.id.clone()));
    if from.stated && newer { into.btc_now = from.btc_now; into.fiat_now = from.fiat_now; into.agrees = match (into.agrees, from.agrees) { (Some(a), Some(b)) => Some(a && b), (a, b) => b.or(a) }; }
    if !into.stated { into.btc_now = into.rows.iter().map(|r| r.sats - r.fee_sats as i64).sum(); into.fiat_now = into.rows.iter().map(|r| r.fiat - r.fee_fiat as i64).sum(); }
    let mut imp = from.imports.into_iter().last().unwrap_or_default(); imp.at = at; imp.added = n; into.imports.push(imp);
    n
}

/// The account's rows as activity rows: sats in and out, the fee on a withdrawal part of what left, like a
/// statement's; a fiat-only row is a row of zero sats, kept so the person sees the money move.
pub fn activity_rows(ex: &Exchange) -> Vec<crate::activity::Row> {
    ex.rows.iter().map(|r| crate::activity::Row { txid: r.id.clone(), time: Some(r.time), net: Some(r.sats - r.fee_sats as i64), fee: if r.fee_sats > 0 { Some(r.fee_sats) } else { None }, quote: None, ins: Vec::new(), outs: Vec::new() }).collect()
}
/// The price a buy or a sell was really made at, fee included: cents per bitcoin from the row's own figures.
pub fn real_price(r: &ExRow) -> Option<f64> {
    if r.sats == 0 { return None; }
    let paid = match r.kind.as_str() { "buy" => r.fiat.unsigned_abs() + r.fee_fiat, "sell" => r.fiat.unsigned_abs().saturating_sub(r.fee_fiat), _ => return None };
    Some(paid as f64 / 100.0 / (r.sats.unsigned_abs() as f64 / 1e8))
}

#[cfg(test)]
mod tests {
    use super::*;
    const AMBER: &str = "\nCustomer Email: someone@example.com\n\n\ncreatedAt (+10) (Date),Customer Email,product,status,From Currency,Input Amount (AUD),Input Amount (BTC),To Currency,Output Amount (AUD),Output Amount (BTC),\"Transaction Fees (DD, CC etc)\",Exchange Fee ($),Exchange Rate,id\n2022-02-15 16:22:33,someone@example.com,SEND,SETTLED,BTC,,0.00985362,BTC,,0.00985362,,,,149de0aa-1\n2022-02-15 16:21:19,someone@example.com,WITHDRAWAL_REQUEST,SETTLED,AUD,$1830.00,,AUD,$1830.00,,,,,af003b76-2\n2022-02-10 10:00:00,someone@example.com,EFT,SETTLED,AUD,$370.00,,AUD,$370.00,,,,,05b3463e-3\n2022-02-08 10:07:18,someone@example.com,RECURRING_PURCHASE,SETTLED,AUD,$50.00,,BTC,,0.00081325,,$0.00,$61481.40,31eeb3fd-4\n2022-02-07 10:06:51,someone@example.com,RECURRING_PURCHASE,PENDING,AUD,$50.00,,BTC,,0.00083781,,$0.00,$59679.26,c3b9a353-5\n2021-11-06 10:06:23,someone@example.com,SATS_BACK,SETTLED,BTC,,,BTC,,0.00000500,,,,5fa1ccca-6\n";
    const B_AUD: &str = "\"Date & Time (UTC)\",\"Transaction Type\",\"Amount AUD\",\"Balance AUD\"\n\"June 30, 2023 9:10:02 PM\",\"Buy Fee\",\"-0.38\",\"166.10\"\n\"June 30, 2023 9:10:02 PM\",\"Buy Bitcoin\",\"-199.62\",\"166.48\"\n\"June 29, 2023 9:10:01 PM\",\"Buy Fee\",\"-0.38\",\"366.10\"\n\"June 29, 2023 9:10:01 PM\",\"Buy Bitcoin\",\"-199.62\",\"366.48\"\n\"June 29, 2023 3:00:00 AM\",\"Deposit Fee\",\"-0.10\",\"566.10\"\n\"June 29, 2023 3:00:00 AM\",\"Deposit\",\"400.00\",\"566.20\"\n";
    const B_BTC: &str = "\"Date & Time (UTC)\",\"Transaction Type\",\"Amount BTC\",\"Balance BTC\"\n\"June 30, 2023 9:28:44 PM\",\"Withdrawal Fee\",\"-0.00000817\",\"0.00006813\"\n\"June 30, 2023 9:28:44 PM\",\"Withdrawal\",\"-0.00430000\",\"0.00007630\"\n\"June 30, 2023 9:10:02 PM\",\"Buy Bitcoin\",\"0.00437102\",\"0.00437630\"\n\"June 29, 2023 9:10:01 PM\",\"Buy Bitcoin\",\"0.00000528\",\"0.00000528\"\n";
    fn f(name: &str, t: &str) -> (String, Vec<u8>) { (name.into(), t.as_bytes().to_vec()) }
    #[test] fn money_and_sats_are_read_exactly() {
        assert_eq!(cents("$1830.00").unwrap(), 183000); assert_eq!(cents("-199.62").unwrap(), -19962); assert_eq!(cents("1,234.5").unwrap(), 123450); assert_eq!(cents("").unwrap(), 0);
        assert!(cents("1.234").is_err()); assert_eq!(sats_signed("-0.00430000").unwrap(), -430_000); assert_eq!(sats_signed("0.00000817").unwrap(), 817);
        assert_eq!(when_long("June 30, 2023 9:10:02 PM").unwrap(), 1688159402); assert_eq!(when_long("July 1, 2022 11:02:02 AM").unwrap(), 1656673322);
        assert_eq!(when_iso("2022-02-15 16:22:33", 10).unwrap(), 1644906153);
    }
    #[test] fn amber_reads_buys_sends_deposits_and_skips_what_never_settled() {
        let ex = parse(&[f("amber.csv", AMBER)]).unwrap();
        assert_eq!(ex.provider, "Amber"); assert!(!ex.stated); assert_eq!(ex.agrees, None);
        let kinds: Vec<&str> = ex.rows.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(kinds, vec!["btc_in", "buy", "fiat_in", "fiat_out", "btc_out"], "oldest first; the pending purchase is not a row");
        let buy = ex.rows.iter().find(|r| r.kind == "buy").unwrap();
        assert_eq!((buy.sats, buy.fiat, buy.fee_fiat, buy.rate, buy.id.as_str()), (81_325, -5000, 0, Some(6_148_140), "31eeb3fd-4"));
        assert_eq!(real_price(buy).map(|p| p.round()), Some(61482.0), "the real price, from the row's own figures: 50.00 / 0.00081325");
        let send = ex.rows.iter().find(|r| r.kind == "btc_out").unwrap(); assert_eq!(send.sats, -985_362);
        assert_eq!(ex.rows[0].time, when_iso("2021-11-06 10:06:23", 10).unwrap(), "the +10 in the header is honoured");
        assert_eq!(ex.btc_now, 500 + 81_325 - 985_362); assert_eq!(ex.fiat_now, 37000 - 5000 - 183000);
        let p = preview(&ex, None); assert_eq!((p.buys, p.bought_sats, p.spent_fiat, p.withdrawals, p.deposits, p.rewards_sats), (1, 81_325, 5000, 1, 1, 500));
    }
    #[test] fn bitaroo_folds_fees_into_their_rows_pairs_the_two_statements_and_checks_the_running_balance() {
        let ex = parse(&[f("Bitaroo Statement AUD FY23.csv", B_AUD), f("Bitaroo Statement BTC FY23.csv", B_BTC)]).unwrap();
        assert_eq!(ex.provider, "Bitaroo"); assert_eq!(ex.ccy, "AUD"); assert!(ex.stated); assert_eq!(ex.agrees, Some(true));
        let kinds: Vec<&str> = ex.rows.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(kinds, vec!["fiat_in", "buy", "buy", "btc_out"]);
        let dep = &ex.rows[0]; assert_eq!((dep.fiat, dep.fee_fiat), (40000, 10));
        let buy = &ex.rows[2]; assert_eq!((buy.sats, buy.fiat, buy.fee_fiat), (437_102, -19962, 38)); assert_eq!(real_price(buy).map(|p| p.round()), Some(45756.0));
        let wd = &ex.rows[3]; assert_eq!((wd.sats, wd.fee_sats), (-430_000, 817));
        assert_eq!((ex.btc_now, ex.fiat_now), (6813, 16610), "the newest stated balances");
        let p = preview(&ex, None); assert_eq!((p.buys, p.bought_sats, p.spent_fiat, p.buy_fees_fiat, p.withdrawals, p.withdrawn_sats, p.wd_fee_sats), (2, 437_630, 40000, 76, 1, 430_000, 817));
        // a broken chain is reported, not hidden
        let bad = B_BTC.replace("\"0.00437630\"", "\"0.00437631\"");
        assert_eq!(parse(&[f("a.csv", B_AUD), f("b.csv", &bad)]).unwrap().agrees, Some(false));
    }
    #[test] fn a_later_export_adds_only_what_is_new_and_a_zip_is_read_like_its_files() {
        let mut ex = parse(&[f("Bitaroo Statement AUD FY23.csv", B_AUD), f("Bitaroo Statement BTC FY23.csv", B_BTC)]).unwrap();
        let more = B_BTC.replacen("\"Date & Time (UTC)\",\"Transaction Type\",\"Amount BTC\",\"Balance BTC\"\n", "\"Date & Time (UTC)\",\"Transaction Type\",\"Amount BTC\",\"Balance BTC\"\n\"July 2, 2023 1:00:00 AM\",\"Buy Bitcoin\",\"0.00100000\",\"0.00106813\"\n", 1);
        let later = parse(&[f("BTC FY24.csv", &more)]).unwrap();
        let p = preview(&later, Some(&ex)); assert_eq!(p.new, Some(1));
        assert_eq!(merge(&mut ex, later, 5), 1); assert_eq!(ex.rows.len(), 5); assert_eq!(ex.btc_now, 106_813); assert_eq!(ex.imports.len(), 2);
        // the same files zipped
        let zip = zip_of(&[("Bitaroo Statement AUD FY23.csv", B_AUD), ("Bitaroo Statement BTC FY23.csv", B_BTC)]);
        let ez = parse(&[("Bitaroo Statements FY23.zip".into(), zip)]).unwrap();
        assert_eq!(ez.rows.len(), 4); assert_eq!(ez.imports[0].files, vec!["Bitaroo Statements FY23.zip"]);
        assert!(parse(&[f("amber.csv", AMBER), f("b.csv", B_BTC)]).unwrap_err().contains("two different exchanges"));
        assert!(parse(&[f("x.csv", "a,b,c\n1,2,3\n")]).unwrap_err().contains("not an export this app knows"));
    }
    #[test] fn another_exchanges_csv_reads_by_a_mapping_in_any_date_style_with_kinds_from_words_or_signs() {
        let csv = "Exchange statement\nWhen,Type,BTC,AUD,Fee,Ref\n15/02/2022 16:22,Buy,0.0010,-50.00,0.50,x1\n2022-03-01T09:00:00,Withdrawal,0.0005,,,x2\n1646300000,,,200,,x3\n\"March 5, 2022 10:00:00 AM\",Sold,0.0002,9.99,,\n";
        let m = Mapping { provider: "Swyftx".into(), date: 0, tz_hours: 10, kind: Some(1), sats: Some(2), sats_unit: "btc".into(), fiat: Some(3), fee_fiat: Some(4), fee_sats: None, id: Some(5), ccy: "aud".into() };
        let raw = raw(&[f("s.csv", csv)]).unwrap(); assert_eq!(raw.header, vec!["When", "Type", "BTC", "AUD", "Fee", "Ref"]); assert_eq!(raw.rows, 4); assert_eq!(raw.sample.len(), 4);
        let ex = mapped(&[f("s.csv", csv)], &m).unwrap();
        assert_eq!(ex.provider, "Swyftx"); assert_eq!(ex.ccy, "AUD"); assert!(ex.mapping.is_some());
        let kinds: Vec<(&str, i64, i64)> = ex.rows.iter().map(|r| (r.kind.as_str(), r.sats, r.fiat)).collect();
        assert_eq!(kinds, vec![("buy", 100_000, -5000), ("btc_out", -50_000, 0), ("fiat_in", 0, 20000), ("sell", -20_000, 999)], "{kinds:?}");
        assert_eq!(ex.rows[0].fee_fiat, 50); assert_eq!(ex.rows[0].id, "swyftx:x1"); assert!(ex.rows[3].id.len() == 64, "a row with no id gets one from its columns");
        assert_eq!(ex.rows[0].time, when_iso("2022-02-15 16:22:00", 10).unwrap()); assert_eq!(ex.rows[2].time, 1646300000, "seconds since 1970 pass through");
        assert!(mapped(&[f("s.csv", csv)], &Mapping { date: 9, ..m.clone() }).unwrap_err().contains("points past"));
        assert_eq!(kind_of("", 5, 0), "btc_in"); assert_eq!(kind_of("", -5, 7), "sell"); assert_eq!(kind_of("Instant purchase", 1, -1), "buy"); assert_eq!(kind_of("Send", 0, -1), "fiat_out");
    }
    // a stored zip, written by hand: enough to prove the reader walks the directory
    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new(); let mut cd = Vec::new(); let mut offs = Vec::new();
        for (name, text) in files {
            offs.push(out.len() as u32);
            let crc = crc32(text.as_bytes());
            out.extend_from_slice(b"PK\x03\x04"); out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(text.len() as u32).to_le_bytes()); out.extend_from_slice(&(text.len() as u32).to_le_bytes()); out.extend_from_slice(&(name.len() as u16).to_le_bytes()); out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes()); out.extend_from_slice(text.as_bytes());
        }
        let cd_off = out.len() as u32;
        for (i, (name, text)) in files.iter().enumerate() {
            cd.extend_from_slice(b"PK\x01\x02"); cd.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); cd.extend_from_slice(&crc32(text.as_bytes()).to_le_bytes());
            cd.extend_from_slice(&(text.len() as u32).to_le_bytes()); cd.extend_from_slice(&(text.len() as u32).to_le_bytes()); cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
            cd.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); cd.extend_from_slice(&offs[i].to_le_bytes()); cd.extend_from_slice(name.as_bytes());
        }
        out.extend_from_slice(&cd); out.extend_from_slice(b"PK\x05\x06"); out.extend_from_slice(&[0, 0, 0, 0]); out.extend_from_slice(&(files.len() as u16).to_le_bytes()); out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(cd.len() as u32).to_le_bytes()); out.extend_from_slice(&cd_off.to_le_bytes()); out.extend_from_slice(&[0, 0]);
        out
    }
    /// The person's own exports, when A2K_EX_FILES names them (colon-separated); prints counts only, never a row.
    #[test] #[ignore] fn the_real_exports_read() {
        let Ok(list) = std::env::var("A2K_EX_FILES") else { return };
        let files: Vec<(String, Vec<u8>)> = list.split(':').filter(|p| !p.is_empty()).map(|p| (std::path::Path::new(p).file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(p).unwrap())).collect();
        let ex = parse(&files).unwrap();
        let p = preview(&ex, None);
        let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default(); for r in &ex.rows { *kinds.entry(r.kind.as_str()).or_default() += 1; }
        if std::env::var("A2K_EX_BREAKS").is_ok() { for (name, text) in texts(&files).unwrap() { if let Ok((ccy, rows, ok)) = bitaroo_file(&name, &text) { if !ok { let i = rows.windows(2).position(|w| (w[1].balance - (w[0].balance + w[1].amount)).abs() > if ccy == "BTC" { 0 } else { 1 }); match i { Some(i) => println!("BREAK {name} [{ccy}] at row {i}: ({}, {}, {}, {}) then ({}, {}, {}, {})", rows[i].time, rows[i].kind, rows[i].amount, rows[i].balance, rows[i+1].time, rows[i+1].kind, rows[i+1].amount, rows[i+1].balance), None => println!("BREAK {name} [{ccy}]: opening {} − {} < 0", rows[0].balance, rows[0].amount) } } } } }
        println!("REAL {} · {} rows · kinds {:?} · buys {} for {} c · withdrawals {} of {} sats · held {} sats / {} c · stated {} · agrees {:?} · unknown {:?}", ex.provider, ex.rows.len(), kinds, p.buys, p.spent_fiat, p.withdrawals, p.withdrawn_sats, ex.btc_now, ex.fiat_now, ex.stated, ex.agrees, p.unknown);
    }
    fn crc32(b: &[u8]) -> u32 { let mut c = !0u32; for &x in b { c ^= x as u32; for _ in 0..8 { c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 }; } } !c }
}
