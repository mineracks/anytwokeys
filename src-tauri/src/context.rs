//! Where a transaction was made, and on what: written ONCE into the transaction's synced record (`ctx`, labels.rs), so
//! it is sealed on this computer and encrypted on the label host like the label itself. For a spend it is the moment
//! of broadcast. For a deposit it is only "first seen from here" — the money did not happen where the laptop was — and
//! the record says which.
//!
//! The place comes from the system's Location Services when the person has turned that on (location.rs), kept to
//! three decimals (about 110 m) unless they ask for exact. Its NAME comes from a list inside the app
//! (data/cities.tsv, GeoNames, CC BY 4.0): asking anyone's geocoder would tell them where the payment was made.

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy)]
pub struct Fix { pub lat: f64, pub lon: f64, pub acc: f64, pub t: u64 }

fn sh(cmd: &str, args: &[&str]) -> Option<String> {
    let o = std::process::Command::new(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// This computer, in the words its owner would use.
pub fn machine() -> Value {
    static M: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    M.get_or_init(|| {
        let (name, model, os) = if cfg!(target_os = "macos") {
            (sh("/usr/sbin/scutil", &["--get", "ComputerName"]), sh("/usr/sbin/sysctl", &["-n", "hw.model"]), sh("/usr/bin/sw_vers", &["-productVersion"]).map(|v| format!("macOS {v}")))
        } else { (sh("hostname", &[]), None, Some(std::env::consts::OS.to_string())) };
        json!({ "name": name.unwrap_or_else(|| "this computer".into()), "model": model, "os": os, "app": env!("CARGO_PKG_VERSION") })
    }).clone()
}

fn km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, la2, dla, dlo) = (a.0.to_radians(), b.0.to_radians(), (b.0 - a.0).to_radians(), (b.1 - a.1).to_radians());
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    6371.0 * 2.0 * h.sqrt().asin()
}

/// "Brisbane, AU" within a city's reach, "near Brisbane, AU" further out, nothing beyond 150 km of anywhere listed.
pub fn place(lat: f64, lon: f64) -> Option<String> {
    let mut best: Option<(f64, &str, &str)> = None;
    for line in include_str!("../data/cities.tsv").lines() {
        let mut f = line.split('\t');
        let (Some(n), Some(c), Some(la), Some(lo)) = (f.next(), f.next(), f.next().and_then(|v| v.parse::<f64>().ok()), f.next().and_then(|v| v.parse::<f64>().ok())) else { continue };
        if (la - lat).abs() > 2.0 { continue; }
        let d = km((lat, lon), (la, lo));
        if best.map(|b| d < b.0).unwrap_or(true) { best = Some((d, n, c)); }
    }
    best.filter(|b| b.0 <= 150.0).map(|(d, n, c)| if d <= 12.0 { format!("{n}, {c}") } else { format!("near {n}, {c}") })
}

/// `how`: "made" (broadcast from here) or "seen" (first noticed from here).
pub fn build(how: &str, fix: Option<Fix>, exact: bool, now: u64) -> Value {
    let mut ctx = json!({ "t": now, "how": how, "device": machine() });
    if let Some(f) = fix {
        let r = |v: f64| if exact { (v * 1e6).round() / 1e6 } else { (v * 1e3).round() / 1e3 };
        ctx["geo"] = json!({ "lat": r(f.lat), "lon": r(f.lon), "acc": f.acc.round().max(if exact { 0.0 } else { 110.0 }) });
        if let Some(p) = place(f.lat, f.lon) { ctx["place"] = p.into(); }
    }
    ctx
}

/// Write it into the vault's record for this transaction unless one is already there. Written once: a record that
/// has a `ctx` — including a removed one — is never given another.
pub fn stamp(vault: &mut crate::wallet::Vault, txid: &str, ctx: Value, now: u64) -> bool {
    let key = format!("tx:{txid}");
    let rec = vault.more.entry(key.clone()).or_default();
    if rec.contains_key("ctx") { return false; }
    rec.insert("ctx".into(), ctx);
    vault.label_meta.entry(key).or_insert(now);   // a record that did not exist begins now; one that did keeps its time
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_place_is_named_from_the_list_inside_the_app() {
        assert_eq!(place(-27.4689, 153.0235).as_deref(), Some("Brisbane, AU"));
        assert_eq!(place(46.2044, 6.1432).as_deref(), Some("Geneva, CH"));
        assert!(place(-27.20, 152.20).unwrap().starts_with("near "), "{:?}", place(-27.20, 152.20));
        assert_eq!(place(-48.0, -123.0), None, "the middle of the Pacific is near nowhere");
    }
    #[test]
    fn where_is_kept_to_about_110_m_unless_exact_is_asked_for_and_is_written_once() {
        let fix = Fix { lat: -27.468912, lon: 153.023456, acc: 8.0, t: 1 };
        let c = build("made", Some(fix), false, 100);
        assert_eq!((c["geo"]["lat"].as_f64(), c["geo"]["lon"].as_f64(), c["geo"]["acc"].as_f64()), (Some(-27.469), Some(153.023), Some(110.0)));
        assert_eq!(build("made", Some(fix), true, 100)["geo"]["lat"].as_f64(), Some(-27.468912));
        assert_eq!(c["place"], "Brisbane, AU"); assert_eq!(c["how"], "made"); assert!(c["device"]["app"].is_string());
        assert!(build("seen", None, false, 100).get("geo").is_none(), "no fix, no place: never a guess");
        let mut v: crate::wallet::Vault = serde_json::from_value(serde_json::json!({ "name": "t", "network": "signet", "threshold": 2, "keys": [], "descriptor": "", "address": "", "created": 0 })).unwrap();
        assert!(stamp(&mut v, "ab", c.clone(), 100));
        assert!(!stamp(&mut v, "ab", build("seen", None, false, 200), 200), "a second stamp must not replace the first");
        assert_eq!(v.more["tx:ab"]["ctx"]["how"], "made"); assert_eq!(v.label_meta["tx:ab"], 100);
        v.more.get_mut("tx:ab").unwrap().insert("ctx".into(), serde_json::json!({ "removed": true }));
        assert!(!stamp(&mut v, "ab", c, 300), "a removed place stays removed");
    }
}
