//! A fictional person for a fiction profile: four entities, two places, two signet vaults, two exchange accounts,
//! all from one seed — so the same persona can be made again on any machine and grown when a new asset class lands.
//!
//! Two sources, mixed on purpose (PROFILES-SPEC §4):
//!   - RECORDS are fixtures written in the formats the real readers take. The exchange history is an Amber CSV,
//!     parsed by `exchange::parse`, so the demo exercises the importer and not a side door.
//!   - VAULTS are real signet descriptors derived from the seed (BIP 84, watch-only here), so addresses resolve and
//!     confirmations happen. The final exchange withdrawals name the exact amounts paid to those addresses, dated
//!     at FUNDED_AT, so the transfers matcher links them live once the coins are there.
//!
//! The default seed's vaults were funded from the walletplayground dispenser on 2026-10-01 (see `docs/PROFILES.md`).
//! Any other seed makes an unfunded persona: the same story, with "fund these" for its addresses.
//!
//! Prices are a baked yearly curve in AUD with seeded noise, rounded on the fiat side (a buy is $200.00, not
//! $197.31). They are not the market's own history; the rows carry their rate, so the app values them from the
//! rows, as it does a real Amber export. A persona is never a real person: names come from short lists.

use crate::wallet::{self, Entity, Place, Vault};
use bitcoin::bip32::{DerivationPath, Xpriv, Xpub};
use bitcoin::secp256k1::Secp256k1;
use serde::Serialize;

pub const DEFAULT_SEED: &str = "7f3a9c21d4e85b60";
/// 2026-10-01 02:00:00 UTC — when the default persona's vaults were paid, and the time its last withdrawals carry
pub const FUNDED_AT: u32 = 1_790_820_000;
/// what the default persona's cold storage received, by (vault, receive index) — the dispenser paid exactly these
pub const FUNDING: [(&str, u32, u64); 3] = [("self", 0, 150_000), ("self", 1, 120_000), ("smsf", 0, 200_000)];

struct Rng(u64);
impl Rng {
    fn from(seed: &str) -> Rng { use sha2::Digest; let h = sha2::Sha256::digest(format!("a2k-persona|{seed}").as_bytes()); Rng(u64::from_le_bytes(h[..8].try_into().unwrap()) | 1) }
    fn next(&mut self) -> u64 { let mut x = self.0; x ^= x << 13; x ^= x >> 7; x ^= x << 17; self.0 = x; x }
    fn below(&mut self, n: u64) -> u64 { self.next() % n.max(1) }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T { &xs[self.below(xs.len() as u64) as usize] }
    /// a fraction in [-1, 1)
    fn jitter(&mut self) -> f64 { (self.below(20_000) as f64 - 10_000.0) / 10_000.0 }
}

const FIRST: [&str; 12] = ["Maya", "Elin", "Tomas", "Priya", "Jonah", "Sofia", "Arun", "Greta", "Felix", "Noor", "Hugo", "Ines"];
const LAST: [&str; 12] = ["Lindqvist", "Okafor", "Marchetti", "Haugen", "Delacroix", "Nakamura", "Petrov", "Abernathy", "Soto", "Varga", "Keane", "Brandt"];
const PARTNER: [&str; 8] = ["Sam", "Alex", "Robin", "Kai", "Jules", "Morgan", "Rowan", "Casey"];
const STREETS: [&str; 6] = ["Boundary Street", "Latrobe Terrace", "Vulture Street", "Given Terrace", "Oxford Street", "Riding Road"];
const SUBURBS: [(&str, &str, f64, f64); 6] = [("West End", "4101", -27.481, 153.010), ("Paddington", "4064", -27.460, 152.998), ("Bulimba", "4171", -27.452, 153.059), ("New Farm", "4005", -27.467, 153.045), ("Red Hill", "4059", -27.453, 153.005), ("Woolloongabba", "4102", -27.489, 153.035)];

/// AUD per BTC at a year's midpoint: a shape, not the market's record.
fn price_aud(year: i32, month: u32, rng: &mut Rng) -> f64 {
    let anchors: [(i32, f64); 11] = [(2016, 900.0), (2017, 5_500.0), (2018, 9_000.0), (2019, 11_000.0), (2020, 15_000.0), (2021, 62_000.0), (2022, 42_000.0), (2023, 46_000.0), (2024, 98_000.0), (2025, 160_000.0), (2026, 185_000.0)];
    let t = year as f64 + (month as f64 - 0.5) / 12.0;
    let (mut a, mut b) = (anchors[0], anchors[anchors.len() - 1]);
    for w in anchors.windows(2) { if t >= w[0].0 as f64 + 0.5 && t < w[1].0 as f64 + 0.5 { a = w[0]; b = w[1]; } }
    let f = ((t - (a.0 as f64 + 0.5)) / ((b.0 - a.0) as f64).max(1.0)).clamp(0.0, 1.0);
    let base = a.1 * (b.1 / a.1).powf(f);   // geometric between anchors
    (base * (1.0 + 0.08 * rng.jitter())).max(500.0)
}

fn epoch(y: i32, m: u32, d: u32, h: u32, mi: u32) -> u32 {
    // days from civil, Howard Hinnant's algorithm
    let (y2, m2) = if m <= 2 { (y - 1, m as i64 + 9) } else { (y, m as i64 - 3) };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = (y2 - era * 400) as i64;
    let doy = (153 * m2 + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * 146_097 + doe - 719_468;
    (days * 86_400 + h as i64 * 3600 + mi as i64 * 60) as u32
}
fn ymd(t: u32) -> (i32, u32, u32, u32, u32, u32) {
    let z = t as i64 / 86_400 + 719_468; let era = z.div_euclid(146_097); let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; let y = yoe + era * 400; let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153; let d = (doy - (153 * mp + 2) / 5 + 1) as u32; let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let s = t as i64 % 86_400;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d, (s / 3600) as u32, (s % 3600 / 60) as u32, (s % 60) as u32)
}
/// Amber writes local time with the offset in the header: "+10"
fn amber_time(t: u32) -> String { let (y, m, d, h, mi, s) = ymd(t + 10 * 3600); format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}") }

const AMBER_HEAD: &str = "createdAt (+10) (Date),Customer Email,product,status,From Currency,Input Amount (AUD),Input Amount (BTC),To Currency,Output Amount (AUD),Output Amount (BTC),\"Transaction Fees (DD, CC etc)\",Exchange Fee ($),Exchange Rate,id";

struct Row { t: u32, product: &'static str, from: &'static str, in_aud: i64, in_btc: i64, to: &'static str, out_aud: i64, out_btc: i64, fee: i64, rate: i64 }
fn csv(email: &str, rows: &[Row]) -> String {
    let money = |c: i64| if c == 0 { String::new() } else { format!("{}.{:02}", c / 100, (c % 100).abs()) };
    let btc = |s: i64| if s == 0 { String::new() } else { format!("{}.{:08}", s / 100_000_000, (s % 100_000_000).abs()) };
    let mut out = format!("\nCustomer Email: {email}\n\n\n{AMBER_HEAD}\n");
    for (i, r) in rows.iter().enumerate() {
        out.push_str(&format!("{},{email},{},SETTLED,{},{},{},{},{},{},,{},{},{}\n", amber_time(r.t), r.product, r.from, money(r.in_aud), btc(r.in_btc), r.to, money(r.out_aud), btc(r.out_btc), money(r.fee), money(r.rate), format!("p-{i:04}")));
    }
    out
}

/// One account's story: `monthly` dollars a month from `from_year`, a few sells, and `sends` (sats, at FUNDED_AT) out.
fn story(rng: &mut Rng, from_year: i32, monthly: i64, sells: usize, sends: &[u64], end: u32) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut held: i64 = 0;
    let (end_y, end_m, ..) = ymd(end);
    let mut y = from_year; let mut m = 1 + rng.below(12) as u32;
    let mut n = 0;
    while (y, m) < (end_y, end_m) {
        let day = 1 + rng.below(27) as u32;
        let t = epoch(y, m, day, 8 + rng.below(10) as u32, rng.below(60) as u32);
        if n % 6 == 0 { rows.push(Row { t: t - 3600, product: "EFT", from: "AUD", in_aud: monthly * 600, in_btc: 0, to: "AUD", out_aud: 0, out_btc: 0, fee: 0, rate: 0 }); }
        let price = price_aud(y, m, rng);
        let aud = monthly * 100;
        let fee = aud / 100;   // 1 %
        let sats = (((aud - fee) as f64 / 100.0) / price * 1e8).round() as i64;
        rows.push(Row { t, product: "RECURRING_BUY", from: "AUD", in_aud: aud, in_btc: 0, to: "BTC", out_aud: 0, out_btc: sats, fee, rate: (price * 100.0).round() as i64 });
        held += sats; n += 1;
        m += 1; if m > 12 { m = 1; y += 1; }
    }
    // the sells: a slice of what is held, at a later month each
    let total_months = rows.len().max(1);
    for i in 0..sells {
        let at = rows[(total_months * (i + 2)) / (sells + 2)].t + 86_400 * 9;
        let (yy, mm, ..) = ymd(at); let price = price_aud(yy, mm, rng);
        let sats = held / 7;
        let aud = (sats as f64 / 1e8 * price * 100.0).round() as i64; let fee = aud / 100;
        rows.push(Row { t: at, product: "SELL", from: "BTC", in_aud: 0, in_btc: sats, to: "AUD", out_aud: aud - fee, out_btc: 0, fee, rate: (price * 100.0).round() as i64 });
        held -= sats;
        rows.push(Row { t: at + 7200, product: "WITHDRAWAL_REQUEST", from: "AUD", in_aud: aud - fee, in_btc: 0, to: "AUD", out_aud: 0, out_btc: 0, fee: 0, rate: 0 });
    }
    for (i, s) in sends.iter().enumerate() { rows.push(Row { t: FUNDED_AT - 1800 + i as u32 * 300, product: "SEND", from: "BTC", in_aud: 0, in_btc: *s as i64, to: "BTC", out_aud: 0, out_btc: 0, fee: 0, rate: 0 }); }
    rows.sort_by_key(|r| r.t);
    rows
}

#[derive(Serialize, Clone, Debug)]
pub struct Made { pub name: String, pub seed: String, pub entities: usize, pub vaults: Vec<String>, pub exchange_rows: usize, pub funded: bool, pub fund: Vec<(String, String, u64)> }

fn account(master: &Xpriv, idx: u32) -> Result<(String, String), String> {
    let secp = Secp256k1::new();
    let path: DerivationPath = format!("m/84h/1h/{idx}h").parse().map_err(|e| format!("{e}"))?;
    let acct = master.derive_priv(&secp, &path).map_err(|e| e.to_string())?;
    let xpub = Xpub::from_priv(&secp, &acct);
    let fp = master.fingerprint(&secp);
    Ok((format!("wpkh([{fp}/84h/1h/{idx}h]{xpub}/<0;1>/*)"), format!("[{fp}/84h/1h/{idx}h]{xpub}")))
}

fn watch_vault(name: &str, desc: &str, key_name: &str, created: u64, entity: &str) -> Result<Vault, String> {
    let info = wallet::ext_parse(desc)?.ok_or("the persona's descriptor named no extended key")?;
    let mut v = crate::ext_vault(name.into(), &info, &[key_name.to_string()]);
    v.network = "signet".into(); v.created = created; v.entity = Some(entity.into());
    v.address = wallet::address_at(&v, 0)?;
    Ok(v)
}

/// Everything into the open store, which must be a fiction profile's: what was there goes first.
pub fn generate(seed: &str, now: u64) -> Result<Made, String> {
    let seed = seed.trim(); if seed.is_empty() || seed.len() > 64 { return Err("a persona seed is 1 to 64 characters".into()); }
    let mut rng = Rng::from(seed);
    let first = *rng.pick(&FIRST); let last = *rng.pick(&LAST); let partner = *rng.pick(&PARTNER);
    let name = format!("{first} {last}");
    let (street_no, street, (suburb, post, lat, lon)) = (12 + rng.below(180), *rng.pick(&STREETS), *rng.pick(&SUBURBS));
    let mut id = |tag: &str| format!("{tag}-{}", &hex::encode(rng.next().to_le_bytes())[..6]);
    let (e_self, e_partner, e_smsf, e_co) = (id("p"), id("p"), id("f"), id("c"));
    let (p_home, p_bank) = (id("l"), id("l"));
    // the keys: BIP 32 from the seed, signet
    let mut seed_bytes = [0u8; 32]; { use sha2::Digest; seed_bytes.copy_from_slice(&sha2::Sha256::digest(format!("a2k-persona-keys|{seed}").as_bytes())); }
    let master = Xpriv::new_master(bitcoin::Network::Signet, &seed_bytes).map_err(|e| e.to_string())?;
    let (d_self, k_self) = account(&master, 0)?; let (d_smsf, k_smsf) = account(&master, 1)?;
    let funded = seed == DEFAULT_SEED;
    let created = epoch(2021, 3, 14, 2, 0) as u64;
    let v_self = watch_vault("Cold storage", &d_self, &format!("{first}'s signer"), created, &e_self)?;
    let v_smsf = watch_vault(&format!("{last} Super Fund · cold storage"), &d_smsf, "the fund's signer", epoch(2023, 7, 2, 1, 0) as u64, &e_smsf)?;
    let sends_self: Vec<u64> = FUNDING.iter().filter(|f| f.0 == "self").map(|f| f.2).collect();
    let sends_smsf: Vec<u64> = FUNDING.iter().filter(|f| f.0 == "smsf").map(|f| f.2).collect();
    let email = format!("{}.{}@example.com", first.to_lowercase(), last.to_lowercase());
    let csv_self = csv(&email, &story(&mut rng, 2018, 200, 2, &sends_self, FUNDED_AT));
    let csv_smsf = csv(&format!("trustee@{}super.example.com", last.to_lowercase()), &story(&mut rng, 2023, 1500, 1, &sends_smsf, FUNDED_AT));
    let ex_self = crate::exchange::parse(&[(format!("amber-{}.csv", first.to_lowercase()), csv_self.into_bytes())])?;
    let ex_smsf = crate::exchange::parse(&[("amber-smsf.csv".to_string(), csv_smsf.into_bytes())])?;
    let rows = ex_self.rows.len() + ex_smsf.rows.len();
    let ex_vault = |name: String, ex: crate::exchange::Exchange, entity: &str, t: u64| Vault { watch: None, spark: None, muun: None, statement: None, node: None, exchange: Some(ex),
        name, network: "mainnet".into(), threshold: 1, keys: Vec::new(), descriptor: String::new(), address: String::new(), created: t, chain_code: None, next: 0, scan_max: 0, next_change: 0, scan_max_change: 0,
        labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default(), ext: None, cost_method: Some("fifo".into()), entity: Some(entity.into()) };
    let mut fund = vec![];
    for (who, i, sats) in FUNDING { let v = if who == "self" { &v_self } else { &v_smsf }; fund.push((v.name.clone(), wallet::address_at(v, i)?, sats)); }
    let names = vec![v_self.name.clone(), v_smsf.name.clone(), format!("Amber · {first}"), format!("Amber · {last} Super Fund")];
    let home = format!("{street_no} {street}, {suburb}, Brisbane, Queensland {post}, Australia");
    let made = Made { name: name.clone(), seed: seed.into(), entities: 4, vaults: names.clone(), exchange_rows: rows, funded, fund };
    wallet::update(|s| {
        *s = wallet::Store::default();
        s.settings.network = "signet".into();
        s.entities = vec![
            Entity { id: e_self.clone(), name: name.clone(), kind: "person".into(), note: format!("Born 1986. Buys a little every month since 2018; moved to cold storage in 2026. Lives at {home}."), added: created },
            Entity { id: e_partner.clone(), name: format!("{partner} {last}"), kind: "person".into(), note: "Partner. No holdings of their own yet; the home is in both names.".into(), added: created },
            Entity { id: e_smsf.clone(), name: format!("{last} Super Fund"), kind: "smsf".into(), note: format!("Self-managed super fund, two members. Corporate trustee: {last} Holdings Pty Ltd. Buys quarterly since 2023."), added: epoch(2023, 7, 1, 0, 0) as u64 },
            Entity { id: e_co.clone(), name: format!("{last} Holdings Pty Ltd"), kind: "company".into(), note: "Corporate trustee of the fund. Holds nothing of its own.".into(), added: epoch(2023, 6, 20, 0, 0) as u64 },
        ];
        s.places = vec![
            Place { id: p_home.clone(), name: "Home safe".into(), note: "Fire safe, study cupboard.".into(), place: home.clone(), lat: Some(lat + 0.002 * rng.jitter()), lon: Some(lon + 0.002 * rng.jitter()), added: created },
            Place { id: p_bank.clone(), name: "Bank deposit box".into(), note: "Box 2140. Both trustees' signatures to open.".into(), place: "259 Queen Street, Brisbane City, Queensland 4000, Australia".into(), lat: Some(-27.4679), lon: Some(153.0281), added: epoch(2023, 8, 10, 0, 0) as u64 },
        ];
        s.key_place.insert(k_self.clone(), p_home.clone()); s.key_place.insert(k_smsf.clone(), p_bank.clone());
        s.icons.insert(format!("entity:{e_self}"), "glyph:entity".into()); s.icons.insert(format!("entity:{e_smsf}"), "glyph:shield".into()); s.icons.insert(format!("entity:{e_co}"), "glyph:building".into());
        s.icons.insert(format!("place:{p_home}"), "glyph:home".into()); s.icons.insert(format!("place:{p_bank}"), "glyph:bank".into());
        let mut v1 = v_self.clone(); v1.notes.insert("vault:about".into(), "Single-signature for now; a 2-of-3 with the fund's signer is the plan for next year.".into());
        s.vaults = vec![v1, v_smsf.clone(), ex_vault(names[2].clone(), ex_self.clone(), &e_self, epoch(2018, 2, 3, 0, 0) as u64), ex_vault(names[3].clone(), ex_smsf.clone(), &e_smsf, epoch(2023, 7, 20, 0, 0) as u64)];
    })?;
    crate::transfers::clear();
    let _ = now;
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_persona_is_the_same_every_time_and_its_withdrawals_name_the_funding() {
        let d = std::env::temp_dir().join(format!("a2k-persona-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap();
        crate::profiles::test_root(Some(d.clone()));
        crate::profiles::migrate_legacy(1).unwrap();
        let f = crate::profiles::create("Fiction", crate::profiles::Class::Fiction, None, 1).unwrap();
        crate::profiles::open(&f.id, 2).unwrap();
        let a = generate(DEFAULT_SEED, 3).unwrap();
        let b = generate(DEFAULT_SEED, 4).unwrap();
        assert_eq!(a.name, b.name); assert_eq!(a.fund, b.fund); assert!(a.funded);
        let s = wallet::load().unwrap();
        assert_eq!(s.entities.len(), 4); assert_eq!(s.places.len(), 2); assert_eq!(s.vaults.len(), 4);
        let cold = s.vaults.iter().find(|v| v.name == "Cold storage").unwrap();
        assert_eq!(cold.network, "signet"); assert!(cold.address.starts_with("tb1q"), "{}", cold.address);
        assert_eq!(a.fund[0].1, cold.address, "funding index 0 is the vault's first address");
        assert_ne!(a.fund[0].1, a.fund[1].1);
        let ex = s.vaults.iter().find(|v| v.name.starts_with("Amber · ") && !v.name.contains("Super")).unwrap().exchange.as_ref().unwrap();
        assert_eq!(ex.provider, "Amber");
        let sends: Vec<i64> = ex.rows.iter().filter(|r| r.kind == "btc_out").map(|r| r.sats).collect();
        assert_eq!(sends, vec![-150_000, -120_000], "the withdrawals are the funding, in order");
        assert!(ex.rows.iter().filter(|r| r.kind == "buy").count() > 90, "monthly buys since 2018");
        assert!(ex.rows.iter().any(|r| r.kind == "sell") && ex.rows.iter().any(|r| r.kind == "fiat_in"));
        let last_send = ex.rows.iter().filter(|r| r.kind == "btc_out").map(|r| r.time).max().unwrap();
        assert!(last_send < FUNDED_AT && last_send > FUNDED_AT - 3600, "sent just before the coins landed");
        assert!(s.key_place.values().any(|p| p.starts_with("l-")));
        // another seed: another person, unfunded, still whole
        let c = generate("someone-else", 5).unwrap();
        assert_ne!(c.name, a.name); assert!(!c.funded); assert_eq!(c.fund.len(), 3);
        println!("{} · {} · {:?}", a.name, a.exchange_rows, a.fund);
        crate::profiles::close(); crate::profiles::test_root(None);
    }
}
