//! Where the price sat on the long cycle when a coin was bought, spent, or is about to be.
//!
//! One number: the USD price that day divided by the 200-week moving average of weekly USD closes. A ratio, so the
//! person's own currency does not come into it; a long average, so it moves slowly and a week's noise does not
//! change the answer. Beside it, where that multiple falls among every week since the average first existed
//! (mid-2014), so "1.9×" can be read as "higher than most weeks" without knowing the history by heart.
//!
//! Everything here is a fact about the price on a day. It is never a view on what to do, and the window says so.
//!
//! The weekly closes are the price service's own daily closes on a fixed grid (every seventh day from 2010-08-19,
//! the day after its earliest price). A closed day's price never changes, so each is kept in the sealed price cache
//! like any other, and after the first build the whole curve comes from the cache with no request at all.
use crate::activity::{self, Quote};
use serde::Serialize;

const DAY0_T: u32 = 1_282_176_000;   // 2010-08-19 00:00 UTC
const WEEK: u32 = 604_800;
const WINDOW: usize = 200;
const ENOUGH: usize = 150;           // weeks with a price a window needs before its average is trusted
const CHUNK: usize = 390;            // the service refuses a series of more than 400 points

#[derive(Clone, Serialize)]
pub struct Week { pub day: String, pub t: u32, pub close: Option<f64>, pub wma: Option<f64>, pub mult: Option<f64>,
    /// where this week's multiple ranked among the weeks up to and including it: the share at or below
    pub pct: Option<f64> }

#[derive(Clone, Serialize)]
pub struct Curve {
    pub weeks: Vec<Week>,
    /// every week's multiple that exists, sorted: where a multiple ranks is its place in this list
    pub ranks: Vec<f64>,
    /// the first week that has an average
    pub since: Option<String>,
}

/// One moment: the price then, the average then, their ratio, and where that ratio ranked among the weeks known AT
/// THAT TIME — the share of weeks up to then at or below it. "What was the context when this happened" is a question
/// about that day, so a later bull or bear market does not re-rank an old purchase.
#[derive(Clone, Serialize)]
pub struct Mark { pub t: u32, pub price: f64, pub wma: f64, pub mult: f64, pub pct: f64 }

fn grid_day(i: usize) -> (String, u32) { let t = DAY0_T + i as u32 * WEEK; (crate::export::ymd(t as u64), t) }

fn now_t() -> u32 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0) }

/// The curve to the last closed grid day. Built once a day per process; the first build of all asks the service
/// for the weeks the cache does not hold (three requests for the whole history), later builds ask for nothing.
pub fn curve(prices_url: &str) -> Result<Curve, String> {
    static MEMO: std::sync::Mutex<Option<(String, Curve)>> = std::sync::Mutex::new(None);
    let today = crate::export::ymd(now_t() as u64);
    if let Ok(g) = MEMO.lock() { if let Some((d, c)) = g.as_ref() { if *d == today { return Ok(c.clone()); } } }
    let n = (0..).take_while(|&i| grid_day(i).0 < today).count();
    let mut closes: Vec<Option<f64>> = (0..n).map(|i| activity::cached_price(&format!("USD:{}", grid_day(i).0)).map(|q| q.price)).collect();
    // ask only for the chunks with a hole; the service answers the same grid when asked from a grid day every 7
    for s in (0..n).step_by(CHUNK) {
        let e = (s + CHUNK).min(n);
        if closes[s..e].iter().all(|c| c.is_some()) { continue; }
        let (from, _) = grid_day(s); let (to, _) = grid_day(e - 1);
        let points = activity::price_series(prices_url, "USD", &from, &to, 7)?;
        let by_day: std::collections::HashMap<String, Option<Quote>> = points.into_iter().collect();
        for i in s..e { if closes[i].is_none() { closes[i] = by_day.get(&grid_day(i).0).and_then(|q| q.as_ref()).map(|q| q.price); } }
    }
    let wmas = averages(&closes);
    let mut weeks: Vec<Week> = (0..n).map(|i| { let (day, t) = grid_day(i); let wma = wmas[i];
        Week { day, t, close: closes[i], wma, mult: match (closes[i], wma) { (Some(c), Some(w)) if w > 0.0 => Some(c / w), _ => None }, pct: None } }).collect();
    for i in 0..n { if let Some(m) = weeks[i].mult { weeks[i].pct = Some(rank_upto(&weeks, i, m)); } }
    let mut ranks: Vec<f64> = weeks.iter().filter_map(|w| w.mult).collect();
    ranks.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let since = weeks.iter().find(|w| w.wma.is_some()).map(|w| w.day.clone());
    let c = Curve { weeks, ranks, since };
    if let Ok(mut g) = MEMO.lock() { *g = Some((today, c.clone())); }
    Ok(c)
}

/// The 200-week average at each week: the mean of the closes the window holds, once it holds enough of them. A
/// missing close is left out, never filled from a neighbour; a window with too few is no average at all.
fn averages(closes: &[Option<f64>]) -> Vec<Option<f64>> {
    let (mut sum, mut count) = (0.0, 0usize);
    closes.iter().enumerate().map(|(i, c)| {
        if let Some(c) = c { sum += c; count += 1; }
        if i >= WINDOW { if let Some(c) = closes[i - WINDOW] { sum -= c; count -= 1; } }
        if i + 1 >= WINDOW && count >= ENOUGH { Some(sum / count as f64) } else { None }
    }).collect()
}

/// The average in force at a moment: the last grid week at or before it that has one (the newest week's average
/// serves for days after the last closed grid day, including today).
fn wma_at(curve: &Curve, t: u32) -> Option<f64> {
    if t < DAY0_T || curve.weeks.is_empty() { return None; }
    let i = (((t - DAY0_T) / WEEK) as usize).min(curve.weeks.len() - 1);
    curve.weeks[..=i].iter().rev().take(4).find_map(|w| w.wma)
}

/// The share of the weeks up to and including week `i` whose multiple is at or below `mult`.
fn rank_upto(weeks: &[Week], i: usize, mult: f64) -> f64 {
    let (mut at_or_below, mut known) = (0usize, 0usize);
    for w in &weeks[..=i] { if let Some(m) = w.mult { known += 1; if m <= mult { at_or_below += 1; } } }
    if known == 0 { 0.0 } else { at_or_below as f64 / known as f64 }
}

pub fn mark(curve: &Curve, t: u32, price: f64) -> Option<Mark> {
    let wma = wma_at(curve, t)?;
    if !(price > 0.0 && wma > 0.0) { return None; }
    let mult = price / wma;
    let i = (((t.max(DAY0_T) - DAY0_T) / WEEK) as usize).min(curve.weeks.len() - 1);
    let pct = rank_upto(&curve.weeks, i, mult);
    Some(Mark { t, price, wma, mult, pct })
}

/// Days that are not kept yet, gathered into spans the series question can answer in one go (at most 395 days
/// each, so a vault with a buy in 2017 and one in 2026 asks for two short runs, not nine years).
fn spans(mut days: Vec<i64>) -> Vec<(i64, i64)> {
    days.sort_unstable(); days.dedup();
    let mut out: Vec<(i64, i64)> = Vec::new();
    for d in days {
        match out.last_mut() { Some((from, to)) if d - *from <= 395 => *to = d, _ => out.push((d, d)) }
    }
    out
}
fn day_number(t: u32) -> i64 { (t / 86_400) as i64 }
fn day_of(n: i64) -> String { crate::export::ymd(n as u64 * 86_400) }

/// Marks for a list of moments, from the daily USD close of each day. Days already kept cost nothing; the rest come
/// in a few runs from the series question (one request a year of history), kept for good. Today, which has no close,
/// uses the current quote.
pub fn marks(prices_url: &str, times: &[u32]) -> Result<Vec<Option<Mark>>, String> {
    let curve = curve(prices_url)?;
    let today = crate::export::ymd(now_t() as u64);
    let missing: Vec<i64> = times.iter().map(|&t| day_number(t)).filter(|&n| day_of(n) < today && activity::cached_price(&format!("USD:{}", day_of(n))).is_none()).collect();
    for (from, to) in spans(missing) { let _ = activity::price_series(prices_url, "USD", &day_of(from), &day_of(to), 1); }   // a run that fails leaves holes, which stay holes
    let now = if times.iter().any(|&t| crate::export::ymd(t as u64) == today) { activity::now_quote(prices_url, "USD") } else { None };
    Ok(times.iter().map(|&t| {
        let day = crate::export::ymd(t as u64);
        let q = if day == today { now.clone() } else { activity::cached_price(&format!("USD:{day}")) };
        q.and_then(|q| mark(&curve, t, q.price))
    }).collect())
}

/// Now — or, when the service has no price for this hour yet, the newest closed week, carrying its own date so the
/// window can say "as of".
pub fn now(prices_url: &str) -> Result<Option<Mark>, String> {
    let curve = curve(prices_url)?;
    if let Some(m) = activity::now_quote(prices_url, "USD").and_then(|q| mark(&curve, now_t(), q.price)) { return Ok(Some(m)); }
    Ok(curve.weeks.iter().rev().find(|w| w.mult.is_some()).and_then(|w| mark(&curve, w.t, w.close?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(mults: &[f64]) -> Curve {
        // 260 weeks of a flat 100 close, so the average is 100 and the multiple is the close over 100
        let mut weeks: Vec<Week> = (0..260).map(|i| { let (day, t) = grid_day(i); Week { day, t, close: Some(100.0), wma: if i + 1 >= WINDOW { Some(100.0) } else { None }, mult: if i + 1 >= WINDOW { Some(1.0) } else { None }, pct: None } }).collect();
        for (k, m) in mults.iter().enumerate() { let w = &mut weeks[200 + k]; w.close = Some(100.0 * m); w.mult = Some(*m); }
        let mut ranks: Vec<f64> = weeks.iter().filter_map(|w| w.mult).collect(); ranks.sort_by(|a, b| a.partial_cmp(b).unwrap());
        Curve { weeks, ranks, since: Some(grid_day(199).0) }
    }

    #[test]
    fn the_grid_starts_the_day_after_the_first_price_and_steps_a_week() {
        assert_eq!(grid_day(0).0, "2010-08-19");
        assert_eq!(grid_day(1).0, "2010-08-26");
        assert_eq!(grid_day(52).0, "2011-08-18");
    }

    #[test]
    fn a_mark_is_the_price_over_the_average_in_force_that_week() {
        let c = fake(&[]);
        let m = mark(&c, grid_day(230).1 + 3 * 86_400, 140.0).unwrap();
        assert!((m.mult - 1.4).abs() < 1e-9 && (m.wma - 100.0).abs() < 1e-9);
        // before the average exists there is no mark, not a guess
        assert!(mark(&c, grid_day(10).1, 140.0).is_none());
        // after the last closed week the newest average serves
        assert!(mark(&c, grid_day(259).1 + 20 * 86_400, 90.0).map(|m| m.mult).unwrap() < 1.0);
    }

    #[test]
    fn the_percentile_is_the_share_of_weeks_known_at_the_time_at_or_below() {
        let c = fake(&[0.5, 0.7, 2.0, 3.0, 4.0]);   // weeks 199..=259 have a multiple; 200..=204 carry these, the rest 1.0
        let m = mark(&c, grid_day(240).1, 250.0).unwrap();   // 2.5× in week 240: 42 weeks known, all but 3.0 and 4.0 at or below
        assert!((m.pct - 40.0 / 42.0).abs() < 1e-9, "{}", m.pct);
        let low = mark(&c, grid_day(240).1, 60.0).unwrap();   // 0.6×: only 0.5 sits at or below
        assert!((low.pct - 1.0 / 42.0).abs() < 1e-9, "{}", low.pct);
        // the same price in week 259 ranks against 61 weeks: later history does not re-rank an earlier mark
        let later = mark(&c, grid_day(259).1, 250.0).unwrap();
        assert!((later.pct - 59.0 / 61.0).abs() < 1e-9, "{}", later.pct);
        let early = mark(&c, grid_day(200).1, 250.0).unwrap();   // week 200: two weeks known (1.0 and 0.5), both at or below
        assert!((early.pct - 1.0).abs() < 1e-9, "{}", early.pct);
    }

    #[test]
    fn a_window_with_too_few_prices_has_no_average() {
        let full: Vec<Option<f64>> = (0..260).map(|i| Some(if i < 200 { 100.0 } else { 200.0 })).collect();
        let a = averages(&full);
        assert!(a[..199].iter().all(|x| x.is_none()) && (a[199].unwrap() - 100.0).abs() < 1e-9, "no average before 200 weeks, then the mean of the window");
        assert!((a[259].unwrap() - (140.0 * 100.0 + 60.0 * 200.0) / 200.0).abs() < 1e-6, "the window slides: 140 old weeks and 60 new");
        let holes: Vec<Option<f64>> = (0..260).map(|i| if i % 3 == 0 { Some(100.0) } else { None }).collect();
        assert!(averages(&holes).iter().all(|x| x.is_none()), "a third of the weeks is not enough to average");
        let some: Vec<Option<f64>> = (0..260).map(|i| if i % 5 == 0 { None } else { Some(100.0) }).collect();
        assert!((averages(&some)[259].unwrap() - 100.0).abs() < 1e-9, "a few holes are left out, not filled");
    }

    #[test]
    fn missing_days_are_asked_for_in_short_runs_not_one_long_one() {
        assert_eq!(spans(vec![]), vec![]);
        assert_eq!(spans(vec![100, 100, 101, 150]), vec![(100, 150)]);
        // a buy in year one and a buy nine years later: two runs, each a day long
        assert_eq!(spans(vec![100, 100 + 9 * 365]), vec![(100, 100), (3385, 3385)]);
        // a weekly buyer over three years: three runs of at most 395 days, in order
        let days: Vec<i64> = (0..160).map(|i| 1000 + i * 7).collect();
        let r = spans(days.clone());
        assert!(r.iter().all(|(a, b)| b - a <= 395) && r.first().unwrap().0 == 1000 && r.last().unwrap().1 == *days.last().unwrap(), "{r:?}");
        assert_eq!(r.len(), 3, "{r:?}");
    }

    /// The real curve from the real service: prints the newest week and today's mark. `cargo test cycle::tests::live -- --ignored --nocapture`
    #[test] #[ignore]
    fn live_curve_from_the_price_service() {
        let t = std::time::Instant::now();
        let c = curve(activity::PRICES).unwrap();
        let w = c.weeks.iter().rev().find(|w| w.mult.is_some()).unwrap();
        eprintln!("{} weeks, average since {:?}, newest {} close {:?} wma {:?} mult {:?}, {} ranked, built in {:?}", c.weeks.len(), c.since, w.day, w.close, w.wma, w.mult, c.ranks.len(), t.elapsed());
        let holes = c.weeks.iter().filter(|w| w.close.is_none()).count(); eprintln!("weeks without a close: {holes}");
        let t2 = std::time::Instant::now(); let c2 = curve(activity::PRICES).unwrap(); eprintln!("second build {:?} ({} weeks)", t2.elapsed(), c2.weeks.len());
        if let Ok(Some(m)) = now(activity::PRICES) { eprintln!("now: price {} wma {:.0} mult {:.2} pct {:.0}%", m.price, m.wma, m.mult, m.pct * 100.0); }
        for d in ["2017-12-14", "2018-12-13", "2021-11-11", "2022-11-17", "2024-03-14"] { if let Some(w) = c.weeks.iter().find(|w| w.day == d) { eprintln!("{d}: mult {:.2} pct-as-of {:.0}%", w.mult.unwrap_or(0.0), w.pct.unwrap_or(0.0) * 100.0); } }
        for q in [0.1, 0.25, 0.5, 0.75, 0.9] { eprintln!("quantile {q}: {:.2}", c.ranks[((c.ranks.len() - 1) as f64 * q) as usize]); }
        // marks from daily closes, in spans: a 2021 top, a 2022 bottom, a 2024 day, and now
        let t = std::time::Instant::now();
        let ms = marks(activity::PRICES, &[1_636_632_000, 1_668_686_400, 1_710_417_600, now_t()]).unwrap();
        for m in &ms { eprintln!("mark: {}", m.as_ref().map(|m| format!("{} price {:.0} mult {:.2} pct {:.0}%", crate::export::ymd(m.t as u64), m.price, m.mult, m.pct * 100.0)).unwrap_or("none".into())); }
        eprintln!("marks in {:?}", t.elapsed());
        assert!(ms.iter().all(|m| m.is_some()));
        assert!(c.weeks.len() > 800 && c.ranks.len() > 600);
    }
}
