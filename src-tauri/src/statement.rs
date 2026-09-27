//! A closed custodial wallet, imported from the statement its custodian gave. Wallet of Satoshi wound its custodial
//! wallet down when it moved to self-custody and let each person take their history out as a CSV; that file is the
//! only record of the wallet there will ever be. Its rows are the facts: what came in, what went out, when, at which
//! address or by which invoice — and the two identifiers in it (a bitcoin address, a Lightning payment hash) are
//! what let the moves out of it into today's vaults be linked exactly rather than guessed.
//!
//! Nothing here can be spent. The reading is proved by the closing balance: a closed wallet ends at zero, and only one
//! reading of the fee column gets there (a DEBIT's fee was charged on top of its amount; a CREDIT's fee is what the
//! sender paid, and was not deducted). The reconciliation is shown, never assumed.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type R<T> = Result<T, String>;

/// One row of the statement, as kept in the vault. Everything is derived from the file's own columns, nothing added.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Row {
    /// sha256 of the row's own columns: the same row in a later export is the same id
    pub id: String,
    pub time: u32,
    /// signed sats: +amount for a credit, −(amount + fee) for a debit
    pub net: i64,
    pub fee: u64,
    pub credit: bool,
    /// "lightning" or "chain"
    pub layer: String,
    /// chain rows: the address paid to or received at, lower-cased
    #[serde(default)] pub address: String,
    /// lightning rows: the invoice's payment hash, hex — the identifier the other end of the payment also knows
    #[serde(default)] pub hash: String,
    #[serde(default)] pub memo: String,
    /// received through the custodian's point of sale: a sale, i.e. income
    #[serde(default)] pub pos: bool,
    /// "income" (a sale, a reward) · "received" · "paid"
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Statement {
    pub source: String,
    pub rows: Vec<Row>,
    pub credits: u64,
    pub debits: u64,
    /// fees charged on debits — the ones that left the balance
    pub fees: u64,
    /// what the rows add up to: a closed wallet's is zero or dust
    pub closes_at: i64,
    pub imported: u64,
}

/// What the person is shown before anything is saved.
#[derive(Serialize, Debug)]
pub struct Preview {
    pub source: String,
    pub rows: usize,
    pub lightning: usize,
    pub chain: usize,
    pub pos: usize,
    pub rewards: usize,
    pub first: u32,
    pub last: u32,
    pub credits: u64,
    pub debits: u64,
    pub fees: u64,
    pub closes_at: i64,
    /// balance after each row, oldest first, for the sketch
    pub series: Vec<(u32, i64)>,
}

// ---------- the CSV ----------
/// RFC 4180: fields split on commas, quoted fields may hold commas, quotes ("") and newlines.
pub fn csv_rows_pub(text: &str) -> Vec<Vec<String>> { csv_rows(text) }
fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut chars) = (false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') => { if chars.peek() == Some(&'"') { field.push('"'); chars.next(); } else { quoted = false; } }
            (true, c) => field.push(c),
            (false, '"') => quoted = true,
            (false, ',') => { row.push(std::mem::take(&mut field)); }
            (false, '\r') => {}
            (false, '\n') => { row.push(std::mem::take(&mut field)); rows.push(std::mem::take(&mut row)); }
            (false, c) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() { row.push(field); rows.push(row); }
    rows
}

/// "0.00000003" → 3, exactly: the string is read digit by digit, never through a float.
pub fn sats(s: &str) -> R<u64> {
    let s = s.trim();
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if whole.is_empty() && frac.is_empty() { return Ok(0); }
    if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) || frac.len() > 8 { return Err(format!("'{s}' is not an amount in BTC")); }
    let w: u64 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| format!("'{s}' is too large"))? };
    let f: u64 = if frac.is_empty() { 0 } else { format!("{frac:0<8}").parse().map_err(|_| format!("'{s}' is not an amount"))? };
    w.checked_mul(100_000_000).and_then(|x| x.checked_add(f)).ok_or_else(|| format!("'{s}' is too large"))
}

/// "2026-05-27T10:43:31.897Z" → unix seconds. Days-from-civil (Hinnant); the file is UTC.
fn when(s: &str) -> R<u32> {
    let s = s.trim();
    let bad = || format!("'{s}' is not a date");
    let (date, rest) = s.split_once('T').ok_or_else(bad)?;
    let d: Vec<i64> = date.split('-').map(|x| x.parse::<i64>().map_err(|_| bad())).collect::<R<_>>()?;
    let t: Vec<i64> = rest.trim_end_matches('Z').split(':').take(3).map(|x| x.split('.').next().unwrap_or("0").parse::<i64>().map_err(|_| bad())).collect::<R<_>>()?;
    if d.len() != 3 || t.len() != 3 { return Err(bad()); }
    let (y, m, day) = (if d[1] <= 2 { d[0] - 1 } else { d[0] }, d[1], d[2]);
    let era = y.div_euclid(400); let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok(((era * 146097 + doe - 719468) * 86400 + t[0] * 3600 + t[1] * 60 + t[2]) as u32)
}

// ---------- the invoice ----------
/// The payment hash inside a bolt11 invoice: tagged field `p`, 52 five-bit groups. Only the hash is read; the
/// invoice's amount and description are the row's own columns already.
pub fn payment_hash(invoice: &str) -> Option<String> {
    use bech32::primitives::decode::CheckedHrpstring;
    use bech32::Bech32;
    let inv = invoice.trim().to_lowercase();
    if !inv.starts_with("lnbc") { return None; }
    let parsed = CheckedHrpstring::new::<Bech32>(&inv).ok()?;
    let d: Vec<u8> = parsed.fe32_iter::<std::iter::Empty<u8>>().map(|f| f.to_u8()).collect();
    if d.len() < 7 { return None; }
    let mut i = 7;   // after the 35-bit timestamp
    while i + 3 <= d.len() {
        let (tag, len) = (d[i], (d[i + 1] as usize) * 32 + d[i + 2] as usize);
        let vals = d.get(i + 3..i + 3 + len)?;
        if tag == 1 && len == 52 {
            let mut bits = 0u128; let mut out = Vec::with_capacity(32); let mut n = 0;
            for &v in vals { bits = (bits << 5) | v as u128; n += 5; while n >= 8 { n -= 8; out.push(((bits >> n) & 0xff) as u8); } }
            out.truncate(32);
            return Some(hex::encode(out));
        }
        i += 3 + len;
    }
    None
}

// ---------- the reading ----------
const HEADER: [&str; 8] = ["utcDate", "type", "currency", "amount", "fees", "address", "description", "pointOfSale"];

pub fn parse(text: &str) -> R<Statement> {
    let text = text.trim_start_matches('\u{feff}');
    let mut rows = csv_rows(text).into_iter().filter(|r| r.iter().any(|f| !f.trim().is_empty()));
    let head: Vec<String> = rows.next().ok_or("the file is empty")?.iter().map(|h| h.trim().to_string()).collect();
    if head != HEADER {
        return Err(format!("this is not a Wallet of Satoshi statement: expected the columns {}, found {}", HEADER.join(", "), head.join(", ")));
    }
    let mut out = Vec::new();
    for (n, r) in rows.enumerate() {
        if r.len() != 8 { return Err(format!("row {} has {} columns, not 8", n + 2, r.len())); }
        let (date, kind, ccy, amount, fees, address, memo, pos) = (&r[0], r[1].trim(), r[2].trim(), &r[3], &r[4], r[5].trim(), r[6].trim(), r[7].trim());
        let credit = match kind { "CREDIT" => true, "DEBIT" => false, other => return Err(format!("row {}: '{other}' is neither CREDIT nor DEBIT", n + 2)) };
        let layer = match ccy { "LIGHTNING" => "lightning", "BTC" => "chain", other => return Err(format!("row {}: '{other}' is neither LIGHTNING nor BTC", n + 2)) };
        let (amt, fee) = (sats(amount)?, sats(fees)?);
        let time = when(date)?;
        let pos = pos.eq_ignore_ascii_case("true");
        let reward = memo.to_lowercase().contains("referral reward");
        let id = hex::encode(Sha256::digest(format!("{}|{}|{}|{}|{}|{}|{}", date.trim(), kind, ccy, amount.trim(), fees.trim(), address, memo).as_bytes()));
        out.push(Row {
            id, time, credit, layer: layer.into(),
            net: if credit { amt as i64 } else { -((amt + fee) as i64) },
            fee: if credit { 0 } else { fee },
            address: if layer == "chain" { address.to_lowercase() } else { String::new() },
            hash: if layer == "lightning" { payment_hash(address).unwrap_or_default() } else { String::new() },
            memo: memo.to_string(), pos,
            kind: if credit && (pos || reward) { "income" } else if credit { "received" } else { "paid" }.into(),
        });
    }
    if out.is_empty() { return Err("the statement has no rows".into()); }
    out.sort_by_key(|r| (r.time, r.id.clone()));
    let credits = out.iter().filter(|r| r.credit).map(|r| r.net as u64).sum();
    let debits = out.iter().filter(|r| !r.credit).map(|r| (-r.net) as u64 - r.fee).sum();
    let fees = out.iter().map(|r| r.fee).sum();
    let closes_at = out.iter().map(|r| r.net).sum();
    Ok(Statement { source: "Wallet of Satoshi".into(), rows: out, credits, debits, fees, closes_at, imported: 0 })
}

pub fn preview(st: &Statement) -> Preview {
    let mut bal = 0i64;
    let series = st.rows.iter().map(|r| { bal += r.net; (r.time, bal) }).collect();
    Preview {
        source: st.source.clone(), rows: st.rows.len(),
        lightning: st.rows.iter().filter(|r| r.layer == "lightning").count(), chain: st.rows.iter().filter(|r| r.layer == "chain").count(),
        pos: st.rows.iter().filter(|r| r.pos).count(), rewards: st.rows.iter().filter(|r| r.kind == "income" && !r.pos).count(),
        first: st.rows.first().map(|r| r.time).unwrap_or(0), last: st.rows.last().map(|r| r.time).unwrap_or(0),
        credits: st.credits, debits: st.debits, fees: st.fees, closes_at: st.closes_at, series,
    }
}

/// A later export of the same wallet: rows already here are left alone, new ones added. Returns how many were new.
pub fn merge(into: &mut Statement, from: Statement) -> usize {
    let have: std::collections::HashSet<String> = into.rows.iter().map(|r| r.id.clone()).collect();
    let new: Vec<Row> = from.rows.into_iter().filter(|r| !have.contains(&r.id)).collect();
    let n = new.len();
    into.rows.extend(new);
    into.rows.sort_by_key(|r| (r.time, r.id.clone()));
    into.credits = into.rows.iter().filter(|r| r.credit).map(|r| r.net as u64).sum();
    into.debits = into.rows.iter().filter(|r| !r.credit).map(|r| (-r.net) as u64 - r.fee).sum();
    into.fees = into.rows.iter().map(|r| r.fee).sum();
    into.closes_at = into.rows.iter().map(|r| r.net).sum();
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("../../test/fixtures/statement-sample.csv");

    #[test]
    fn amounts_are_read_exactly_never_through_a_float() {
        assert_eq!(sats("0.00000003").unwrap(), 3);
        assert_eq!(sats("0.0000007").unwrap(), 70);
        assert_eq!(sats("0.00021").unwrap(), 21_000);
        assert_eq!(sats("1").unwrap(), 100_000_000);
        assert_eq!(sats("0").unwrap(), 0);
        assert_eq!(sats("0.29999999").unwrap(), 29_999_999, "the float reading of this is 29999998.99…");
        assert!(sats("0.123456789").is_err() && sats("abc").is_err());
    }

    #[test]
    fn the_date_is_utc_to_the_second() {
        assert_eq!(when("2026-05-27T10:43:31.897Z").unwrap(), 1_779_878_611);
        assert_eq!(when("2023-01-28T09:48:59.944Z").unwrap(), 1_674_899_339);
    }

    #[test]
    fn a_bolt11_gives_its_payment_hash_and_anything_else_gives_none() {
        // BOLT 11's own first test vector (fetched): payment hash 0001020304050607080900010203040506070809000102030405060708090102
        let inv = "lnbc1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdpl2pkx2ctnv5sxxmmwwd5kgetjypeh2ursdae8g6twvus8g6rfwvs8qun0dfjkxaq9qrsgq357wnc5r2ueh7ck6q93dj32dlqnls087fxdwk8qakdyafkq3yap9us6v52vjjsrvywa6rt52cm9r9zqt8r2t7mlcwspyetp5h2tztugp9lfyql";
        assert_eq!(payment_hash(inv).as_deref(), Some("0001020304050607080900010203040506070809000102030405060708090102"));
        assert_eq!(payment_hash(""), None);
        assert_eq!(payment_hash("bc1qexample"), None);
        assert_eq!(payment_hash("lnbc1notaninvoice"), None);
    }

    #[test]
    fn the_sample_reads_reconciles_and_classifies() {
        let st = parse(FIXTURE).unwrap();
        assert_eq!(st.rows.len(), 9);
        assert_eq!(st.rows.iter().filter(|r| r.layer == "chain").count(), 3);
        // a DEBIT's fee is on top of its amount; a CREDIT's fee is the sender's and not deducted: closes at zero
        assert_eq!(st.closes_at, 0, "{:?}", st.rows.iter().map(|r| r.net).collect::<Vec<_>>());
        let kinds: Vec<&str> = st.rows.iter().map(|r| r.kind.as_str()).collect();
        assert!(kinds.contains(&"income") && kinds.contains(&"received") && kinds.contains(&"paid"));
        let pos = st.rows.iter().find(|r| r.pos).unwrap(); assert_eq!(pos.kind, "income");
        let reward = st.rows.iter().find(|r| r.memo.contains("Referral")).unwrap(); assert_eq!(reward.kind, "income"); assert!(reward.hash.is_empty());
        let chain = st.rows.iter().find(|r| r.layer == "chain" && !r.credit).unwrap();
        assert_eq!(chain.address, chain.address.to_lowercase(), "addresses are normalised"); assert!(chain.address.starts_with("bc1q"));
        assert!(st.rows.iter().filter(|r| r.layer == "lightning" && !r.memo.contains("Referral")).all(|r| r.hash.len() == 64), "every invoice row has its hash");
        assert!(st.rows.windows(2).all(|w| w[0].time <= w[1].time), "oldest first");
        let memo_with_comma = st.rows.iter().find(|r| r.memo.contains(',')).expect("a quoted memo with a comma survives");
        assert!(memo_with_comma.memo.contains("sent: coffee, thanks"));
    }

    #[test]
    fn the_same_file_twice_changes_nothing_and_a_later_export_adds_only_what_is_new() {
        let mut st = parse(FIXTURE).unwrap();
        let again = parse(FIXTURE).unwrap();
        assert_eq!(merge(&mut st, again), 0);
        let mut later = FIXTURE.to_string();
        later.push_str("2026-06-01T00:00:00.000Z,CREDIT,LIGHTNING,0.00000100,0,,late row,false\n");
        assert_eq!(merge(&mut st, parse(&later).unwrap()), 1);
        assert_eq!(st.rows.len(), 10); assert_eq!(st.closes_at, 100);
    }

    /// The real export, named by A2K_STATEMENT: it must close at dust and every invoice row must yield its hash.
    #[test] #[ignore]
    fn the_real_statement_reads_and_closes() {
        let path = match std::env::var("A2K_STATEMENT") { Ok(p) => p, Err(_) => return };
        let st = parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(st.closes_at.abs() <= 100, "closes at {}", st.closes_at);
        let inv = st.rows.iter().filter(|r| r.layer == "lightning" && r.kind != "income").count();
        let hashed = st.rows.iter().filter(|r| r.layer == "lightning" && r.hash.len() == 64).count();
        assert!(hashed >= inv, "{hashed} hashes for {inv} invoice rows");
        eprintln!("rows {} · chain {} · lightning {} · hashes {} · closes at {} · credits {} debits {} fees {}", st.rows.len(), st.rows.iter().filter(|r| r.layer == "chain").count(), st.rows.len() - st.rows.iter().filter(|r| r.layer == "chain").count(), hashed, st.closes_at, st.credits, st.debits, st.fees);
    }

    #[test]
    fn a_file_of_another_shape_is_refused_by_name() {
        let e = parse("date,amount\n2026-01-01,1\n").unwrap_err();
        assert!(e.contains("not a Wallet of Satoshi statement") && e.contains("utcDate"), "{e}");
        assert!(parse("").is_err());
    }
}
