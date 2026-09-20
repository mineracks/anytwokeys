//! Search-as-you-type for a place. What is typed is a street address, a few letters at a time, so it goes ONLY to a
//! server of ours (Photon on OpenStreetMap data, https://geocode.mineracks.com — no request logs, nothing through
//! anyone else), from here and never from the window, and only once the person has stopped typing (the window waits;
//! this refuses anything under four characters). A position to rank by is sent rounded to ONE decimal (about 11 km).
//! Nothing typed is kept: only the place that is finally chosen goes into the transaction's sealed record.

use serde::Serialize;

#[derive(Debug, Serialize, PartialEq)]
pub struct Hit { pub label: String, pub detail: String, pub lat: f64, pub lon: f64 }

/// One Photon feature → the line a person would write on an envelope, and a quieter second line.
fn hit(f: &serde_json::Value) -> Option<Hit> {
    let p = f.get("properties")?;
    let s = |k: &str| p.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());
    let c = f.get("geometry")?.get("coordinates")?.as_array()?;
    let (lon, lat) = (c.first()?.as_f64()?, c.get(1)?.as_f64()?);
    let street = match (s("housenumber"), s("street")) { (Some(n), Some(st)) => Some(format!("{n} {st}")), (None, Some(st)) => Some(st.to_string()), _ => None };
    let town = [s("district").or(s("locality")), s("city").filter(|c| s("district") != Some(c))].into_iter().flatten().collect::<Vec<_>>();
    let region = [s("state"), s("postcode")].into_iter().flatten().collect::<Vec<_>>().join(" ");
    let mut parts: Vec<String> = Vec::new();
    if let Some(n) = s("name") { parts.push(n.to_string()); }
    if let Some(st) = &street { if s("name").map(|n| !st.contains(n)).unwrap_or(true) { parts.push(st.clone()); } }
    if s("name").is_none() && street.is_none() { return None; }
    let label = parts.join(", ");
    let detail = [town.join(", "), region, s("country").unwrap_or("").to_string()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
    Some(Hit { label, detail, lat, lon })
}

pub fn parse(body: &[u8]) -> Result<Vec<Hit>, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "the place search answered with something unreadable".to_string())?;
    let mut out: Vec<Hit> = Vec::new();
    for h in v.get("features").and_then(|f| f.as_array()).ok_or("the place search answered with something unreadable")?.iter().filter_map(hit) {
        if !out.iter().any(|o| o.label == h.label && o.detail == h.detail) { out.push(h); }
    }
    Ok(out)
}

pub fn search(host: &str, q: &str, near: Option<(f64, f64)>) -> Result<Vec<Hit>, String> {
    let q = q.trim();
    if q.chars().count() < 4 { return Ok(vec![]); }
    if host.is_empty() { return Ok(vec![]); }
    let mut url = reqwest::Url::parse(&format!("{}/api", host.trim_end_matches('/'))).map_err(|_| "the place-search host is not a web address".to_string())?;
    url.query_pairs_mut().append_pair("q", &q.chars().take(120).collect::<String>()).append_pair("limit", "6").append_pair("lang", "en");
    if let Some((lat, lon)) = near { url.query_pairs_mut().append_pair("lat", &format!("{:.1}", lat)).append_pair("lon", &format!("{:.1}", lon)); }
    let c = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(8)).user_agent("AnyTwoKeys").build().map_err(|e| e.to_string())?;
    let r = c.get(url).send().map_err(|_| "the place search could not be reached".to_string())?;
    if !r.status().is_success() { return Err(format!("the place search answered {}", r.status().as_u16())); }
    parse(&r.bytes().map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    // real answers, fetched 2026-09-20 for two public landmarks: test/fixtures/photon-*.json
    #[test]
    fn a_real_answer_reads_as_an_address_a_person_would_write() {
        let hits = parse(include_bytes!("../../test/fixtures/photon-brisbane-city-hall.json")).unwrap();
        assert_eq!(hits[0].label, "Brisbane City Hall, 64 Adelaide Street");
        assert_eq!(hits[0].detail, "Brisbane City, Brisbane · Queensland 4000 · Australia");
        assert!((hits[0].lat + 27.4689738).abs() < 1e-6 && (hits[0].lon - 153.0234885).abs() < 1e-6, "Photon gives lon, lat — in that order");
        let hits = parse(include_bytes!("../../test/fixtures/photon-adelaide-street.json")).unwrap();
        assert_eq!(hits[1].label, "64 Adelaide Street"); assert!(hits[1].detail.starts_with("Clayfield, Brisbane"));
        assert!(parse(b"<html>502</html>").is_err());
    }
    #[test]
    fn a_few_letters_are_never_sent() {
        // no host is contacted for these: the answer is empty before any request is built
        assert_eq!(search("https://127.0.0.1:1", "445", None).unwrap(), vec![]);
        assert_eq!(search("", "445 Upper Edward", None).unwrap(), vec![]);
    }
}
