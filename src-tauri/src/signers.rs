//! A signing device's key, in whatever form the device hands it over.
//!
//! Every form ends in the same three things a vault needs and a hardware signer checks: the MASTER fingerprint, the
//! full path from master, and the extended public key at that path. Each form is read from the source that defines
//! it, not from memory — see the tests, whose expected values come from outside this code.
use bitcoin::base58;
use bitcoin::bip32::Xpub;
use serde::Serialize;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeviceKey {
    pub fingerprint: String,   // master fingerprint, 8 hex, lowercase
    pub path: String,          // m/48h/0h/0h/2h
    pub xpub: String,          // standard xpub / tpub, never SLIP-132
    pub script: String,        // "wsh-multi" | "sh-wsh-multi" | "sh-multi" | "wpkh" | "sh-wpkh" | "pkh" | "tr" | "unknown"
    pub source: String,        // "text" | "coldcard-multisig" | "coldcard-generic" | "ur-account" | "ur-hdkey"
}
impl DeviceKey {
    /// the key as a descriptor key expression, origin and all — what a signer recognises as its own
    pub fn expression(&self) -> String { format!("[{}/{}]{}", self.fingerprint, self.path.trim_start_matches("m/"), self.xpub) }
    pub fn mainnet(&self) -> bool { self.xpub.starts_with("xpub") }
}

/// SLIP-132 (fetched 2026-09-21): the version bytes some wallets put on an extended key to say its script type.
/// Descriptors carry the script type themselves, so every one is written back as a plain xpub / tpub.
pub fn standard_xpub(s: &str) -> Result<String, String> {
    let mut raw = base58::decode_check(s.trim()).map_err(|_| format!("{} is not an extended public key", short(s)))?;
    if raw.len() != 78 { return Err(format!("{} is not an extended public key", short(s))); }
    let v = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
    let std: u32 = match v {
        0x0488b21e | 0x0295b43f | 0x02aa7ed3 | 0x04b24746 | 0x049d7cb2 => 0x0488b21e,   // xpub Ypub Zpub zpub ypub
        0x043587cf | 0x024289ef | 0x02575483 | 0x045f1cf6 | 0x044a5262 => 0x043587cf,   // tpub Upub Vpub vpub upub
        _ => return Err(format!("{} has version bytes {v:08x}, which is not a public extended key this app knows", short(s))),
    };
    raw[..4].copy_from_slice(&std.to_be_bytes());
    let out = base58::encode_check(&raw);
    Xpub::from_str(&out).map_err(|e| format!("{}: {e}", short(s)))?;
    Ok(out)
}
fn short(s: &str) -> String { let s = s.trim(); if s.len() > 14 { format!("{}…", &s[..12]) } else { s.to_string() } }

fn norm_path(p: &str) -> Result<String, String> {
    let p = p.trim().trim_start_matches("m/").trim_start_matches('m');
    let mut out = vec!["m".to_string()];
    for part in p.split('/').filter(|x| !x.is_empty()) {
        let hard = part.ends_with('h') || part.ends_with('H') || part.ends_with('\'');
        let n: u32 = part.trim_end_matches(['h', 'H', '\'']).parse().map_err(|_| format!("{part:?} is not a step of a derivation path"))?;
        out.push(format!("{n}{}", if hard { "h" } else { "" }));
    }
    Ok(out.join("/"))
}
fn script_of_path(path: &str) -> String {
    let p: Vec<&str> = path.split('/').collect();
    match (p.get(1).copied(), p.get(4).copied()) {
        (Some("48h"), Some("2h")) => "wsh-multi", (Some("48h"), Some("1h")) => "sh-wsh-multi", (Some("45h"), _) => "sh-multi",
        (Some("84h"), _) => "wpkh", (Some("49h"), _) => "sh-wpkh", (Some("44h"), _) => "pkh", (Some("86h"), _) => "tr", _ => "unknown",
    }.into()
}

/// Every key the text names. A device may hand over one key or a whole account of them; the caller picks.
pub fn parse(text: &str) -> Result<Vec<DeviceKey>, String> {
    let t = text.trim();
    if t.is_empty() { return Err("nothing to read".into()); }
    if t.len() > 3 && t[..3].eq_ignore_ascii_case("ur:") { return parse_ur(t); }
    if t.starts_with('{') { return parse_json(t); }
    parse_text(t).map(|k| vec![k])
}

/// `[fingerprint/48h/0h/0h/2h]xpub…`, as SeedSigner, Specter, Krux and Sparrow show it; a trailing /<0;1>/* or /0/* is
/// allowed and ignored. A bare xpub is refused: without its origin no signer can tell it is one of its own keys.
fn parse_text(t: &str) -> Result<DeviceKey, String> {
    let t = t.split('#').next().unwrap_or(t).trim();
    let rest = t.strip_prefix('[').ok_or("a key needs its origin, like [dc567276/48h/0h/0h/2h]xpub…: without the fingerprint and path its device cannot recognise it")?;
    let (origin, key) = rest.split_once(']').ok_or("the origin in […] is not closed")?;
    let (fp, path) = origin.split_once('/').unwrap_or((origin, ""));
    if fp.len() != 8 || !fp.chars().all(|c| c.is_ascii_hexdigit()) { return Err(format!("{fp:?} is not a fingerprint (8 hex digits)")); }
    let key = key.split('/').next().unwrap_or(key);
    let path = norm_path(path)?;
    Ok(DeviceKey { fingerprint: fp.to_lowercase(), script: script_of_path(&path), path, xpub: standard_xpub(key)?, source: "text".into() })
}

/// Coldcard's two JSON exports, read as its firmware writes them (shared/multisig.py, shared/export.py, 2026-09-21).
/// The MASTER fingerprint is the top-level `xfp`. Inside a generic export each section carries an `xfp` too — that is
/// the section key's OWN fingerprint, not the master's, and using it builds an origin the device refuses.
fn parse_json(t: &str) -> Result<Vec<DeviceKey>, String> {
    let v: serde_json::Value = serde_json::from_str(t).map_err(|_| "that is not JSON a signing device writes".to_string())?;
    let master = v.get("xfp").and_then(|x| x.as_str()).ok_or("the file names no master fingerprint (xfp)")?.to_lowercase();
    if master.len() != 8 || !master.chars().all(|c| c.is_ascii_hexdigit()) { return Err(format!("{master:?} is not a fingerprint")); }
    let mut out = Vec::new();
    for (sect, source) in [("bip48_2", "coldcard-generic"), ("bip48_1", "coldcard-generic"), ("bip84", "coldcard-generic"), ("bip49", "coldcard-generic"), ("bip44", "coldcard-generic"), ("bip86", "coldcard-generic")] {
        if let Some(s) = v.get(sect) {
            let (Some(deriv), Some(xp)) = (s.get("deriv").and_then(|x| x.as_str()), s.get("xpub").or_else(|| s.get("_pub")).and_then(|x| x.as_str())) else { continue };
            let path = norm_path(deriv)?;
            out.push(DeviceKey { fingerprint: master.clone(), script: script_of_path(&path), path, xpub: standard_xpub(xp)?, source: source.into() });
        }
    }
    for name in ["p2wsh", "p2sh_p2wsh", "p2sh"] {   // the multisig export: <name>, <name>_deriv
        if let (Some(xp), Some(deriv)) = (v.get(name).and_then(|x| x.as_str()), v.get(format!("{name}_deriv")).and_then(|x| x.as_str())) {
            let path = norm_path(deriv)?;
            if out.iter().any(|k| k.path == path) { continue; }
            out.push(DeviceKey { fingerprint: master.clone(), script: script_of_path(&path), path, xpub: standard_xpub(xp)?, source: "coldcard-multisig".into() });
        }
    }
    if out.is_empty() { return Err("the file names a fingerprint but no key this app can use".into()); }
    Ok(out)
}

// ---------- UR: crypto-account (BCR-2020-015) and crypto-hdkey (BCR-2020-007) ----------
use ciborium::value::Value as C;
fn tagged(v: &C) -> (Vec<u64>, &C) { let mut tags = vec![]; let mut x = v; while let C::Tag(t, inner) = x { tags.push(*t); x = inner; } (tags, x) }
fn map_get<'a>(m: &'a C, k: u64) -> Option<&'a C> { if let C::Map(e) = m { e.iter().find(|(key, _)| matches!(key, C::Integer(i) if u64::try_from(*i).ok() == Some(k))).map(|(_, v)| v) } else { None } }
fn as_u64(v: &C) -> Option<u64> { if let C::Integer(i) = v { u64::try_from(*i).ok() } else { None } }
fn as_bytes(v: &C) -> Option<&[u8]> { if let C::Bytes(b) = v { Some(b) } else { None } }

/// An hdkey (tag 303) as an xpub: version, depth, parent fingerprint, child number, chain code, key — built from the
/// map's own fields. `master` stands in when the key's origin names no source fingerprint.
fn hdkey(m: &C, master: Option<u32>, testnet_hint: bool) -> Result<(String, String, String), String> {
    let key = map_get(m, 3).and_then(as_bytes).ok_or("the key has no key data")?;
    let chain = map_get(m, 4).and_then(as_bytes).ok_or("the key has no chain code — it is not an extended key")?;
    let origin = map_get(m, 6).map(|o| tagged(o).1).ok_or("the key has no origin: its device could not recognise it")?;
    let comps = match map_get(origin, 1) { Some(C::Array(a)) => a.clone(), _ => return Err("the key's origin has no path".into()) };
    let mut path = vec!["m".to_string()]; let mut last = 0u32;
    for pair in comps.chunks(2) {
        let n = as_u64(&pair[0]).ok_or("a path step is not a number")? as u32;
        let h = matches!(pair.get(1), Some(C::Bool(true)));
        path.push(format!("{n}{}", if h { "h" } else { "" })); last = n | if h { 0x8000_0000 } else { 0 };
    }
    let fp = map_get(origin, 2).and_then(as_u64).map(|x| x as u32).or(master).ok_or("the key's origin names no master fingerprint")?;
    let parent = map_get(m, 8).and_then(as_u64).unwrap_or(0) as u32;
    let testnet = matches!(map_get(m, 5).map(|u| tagged(u).1).and_then(|u| map_get(u, 2)).and_then(as_u64), Some(1)) || testnet_hint;
    let mut raw = Vec::with_capacity(78);
    raw.extend_from_slice(&(if testnet { 0x043587cfu32 } else { 0x0488b21eu32 }).to_be_bytes());
    raw.push((path.len() - 1) as u8); raw.extend_from_slice(&parent.to_be_bytes()); raw.extend_from_slice(&last.to_be_bytes());
    raw.extend_from_slice(chain); raw.extend_from_slice(key);
    let xpub = base58::encode_check(&raw);
    Xpub::from_str(&xpub).map_err(|e| format!("the key does not make an extended key: {e}"))?;
    Ok((format!("{fp:08x}"), path.join("/"), xpub))
}
fn parse_ur(t: &str) -> Result<Vec<DeviceKey>, String> {
    let parts: Vec<String> = t.split_whitespace().map(|p| p.to_lowercase()).collect();
    let kind = parts[0].trim_start_matches("ur:").split('/').next().unwrap_or("").to_string();
    let cbor = if parts.len() == 1 && parts[0].matches('/').count() == 1 { ur::ur::decode(&parts[0]).map_err(|e| e.to_string())?.1 } else {
        let mut d = ur::Decoder::default();
        for p in &parts { d.receive(p).map_err(|e| e.to_string())?; }
        d.message().map_err(|e| e.to_string())?.ok_or_else(|| format!("some QR parts are missing: {} of {} pieces recovered so far; keep the code in the frame until it completes", d.resolved_fragment_count().unwrap_or(0), d.fragment_count()))?
    };
    let v: C = ciborium::from_reader(cbor.as_slice()).map_err(|e| format!("the code's contents are not readable: {e}"))?;
    match kind.as_str() {
        "crypto-account" => {
            let master = map_get(&v, 1).and_then(as_u64).map(|x| x as u32).ok_or("the account names no master fingerprint")?;
            let list = match map_get(&v, 2) { Some(C::Array(a)) => a, _ => return Err("the account holds no keys".into()) };
            let mut out = Vec::new();
            for entry in list {
                let (tags, inner) = tagged(entry);
                // wrappers (BCR-2020-010, as the official vector uses them): 400 sh, 401 wsh, 403 pkh, 404 wpkh, 409 tr, 410 cosigner; 303 = hdkey
                let script = match tags.iter().filter(|t| **t != 308 && **t != 303).copied().collect::<Vec<_>>().as_slice() {
                    [401, 410] => "wsh-multi", [400, 401, 410] => "sh-wsh-multi", [400, 410] => "sh-multi", [404] => "wpkh", [400, 404] => "sh-wpkh", [403] => "pkh", [409] => "tr", _ => "unknown",
                };
                let (fp, path, xpub) = hdkey(inner, Some(master), false)?;
                out.push(DeviceKey { fingerprint: fp, path, xpub, script: script.into(), source: "ur-account".into() });
            }
            Ok(out)
        }
        "crypto-hdkey" => { let (fp, path, xpub) = hdkey(tagged(&v).1, None, false)?; Ok(vec![DeviceKey { script: script_of_path(&path), fingerprint: fp, path, xpub, source: "ur-hdkey".into() }]) }
        other => Err(format!("a ur:{other} code is not a key; a device shows its key as ur:crypto-account or ur:crypto-hdkey")),
    }
}

/// The one key a native-segwit multisig vault wants from what a device handed over: its BIP48 /2h cosigner key.
pub fn multisig_key(keys: &[DeviceKey]) -> Option<&DeviceKey> { keys.iter().find(|k| k.script == "wsh-multi") }

#[cfg(test)]
mod tests {
    use super::*;
    fn fx() -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/device-keys.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap() }

    /// UR is a fountain code: a camera that starts late sees only mixtures, numbered above the total ("7-4"). The
    /// account must still assemble from those alone, as it would from a SeedSigner already animating.
    #[test]
    fn an_animated_account_assembles_from_fountain_mixtures_alone_with_the_first_lap_missed() {
        let single = fx()["bcr015_ur"].as_str().unwrap().to_string();
        let want = parse(&single).unwrap();
        let (_, cbor) = ur::ur::decode(&single.to_lowercase()).unwrap();
        let mut enc = ur::Encoder::new(&cbor, 60, "crypto-account").unwrap();
        let n = enc.fragment_count(); assert!(n >= 3, "enough parts to be animated: {n}");
        let all: Vec<String> = (0..n * 4).map(|_| enc.next_part().unwrap()).collect();
        let late: Vec<&String> = all.iter().skip(n).collect();   // the first lap — every plain part — never seen
        let mut got = None;
        for k in 1..=late.len() {
            let text = late[..k].iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
            match parse(&text) { Ok(keys) => { got = Some((k, keys)); break; } Err(e) => assert!(e.contains("missing"), "part {k}: {e}") }
        }
        let (k, keys) = got.expect("it never assembled from mixtures");
        assert_eq!(keys.len(), want.len()); assert_eq!(keys[0].xpub, want[0].xpub);
        eprintln!("assembled from {k} mixtures of a {n}-part account");
    }
    #[test]
    fn the_official_crypto_account_vector_reads_to_the_keys_decoded_independently() {
        let f = fx(); let keys = parse(f["bcr015_ur"].as_str().unwrap()).unwrap();
        let want = f["bcr015_expected"]["entries"].as_array().unwrap();
        assert_eq!(keys.len(), want.len());
        for (k, w) in keys.iter().zip(want) {
            assert_eq!(k.fingerprint, "37b5eed4");
            assert_eq!(k.xpub, w["xpub"].as_str().unwrap(), "{} — built by cbor2 + embit outside the app", k.path);
            assert_eq!(k.path.replace('h', "'"), w["path"].as_str().unwrap());
        }
        let ms = multisig_key(&keys).expect("a native-segwit multisig cosigner key");
        assert_eq!((ms.path.as_str(), ms.script.as_str()), ("m/48h/0h/0h/2h", "wsh-multi"));
        assert!(ms.xpub.starts_with("xpub6EC9f7mLFJQoRQ6q"));
    }
    #[test]
    fn a_zpub_becomes_the_same_xpub_it_was_made_from() {
        let s = &fx()["slip132"];
        assert_eq!(standard_xpub(s["zpub"].as_str().unwrap()).unwrap(), s["xpub"].as_str().unwrap());
    }
    #[test]
    fn coldcard_multisig_export_reads_as_its_firmware_writes_it() {
        let s = &fx()["slip132"];
        let json = format!(r#"{{"p2sh_deriv":"m/45h","p2wsh_deriv":"m/48h/0h/0h/2h","p2wsh":"{}","account":"0","xfp":"{}"}}"#, s["zpub"].as_str().unwrap(), s["fingerprint"].as_str().unwrap().to_uppercase());
        let keys = parse(&json).unwrap();
        let k = multisig_key(&keys).unwrap();
        assert_eq!((k.fingerprint.as_str(), k.xpub.as_str(), k.path.as_str()), (s["fingerprint"].as_str().unwrap(), s["xpub"].as_str().unwrap(), "m/48h/0h/0h/2h"));
    }
    #[test]
    fn coldcard_generic_export_takes_the_MASTER_fingerprint_not_the_sections_own() {
        let s = &fx()["slip132"];
        // in the generic export each section's xfp is that key's own fingerprint (shared/export.py: swab32(node.my_fp()))
        let json = format!(r#"{{"xfp":"{}","bip48_2":{{"name":"p2wsh","xfp":"DEADBEEF","deriv":"m/48h/0h/0h/2h","xpub":"{}","_pub":"{}"}}}}"#,
            s["fingerprint"].as_str().unwrap().to_uppercase(), s["xpub"].as_str().unwrap(), s["zpub"].as_str().unwrap());
        let k = parse(&json).unwrap().into_iter().find(|k| k.script == "wsh-multi").unwrap();
        assert_eq!(k.fingerprint, s["fingerprint"].as_str().unwrap(), "the origin a device checks is its MASTER fingerprint");
    }
    #[test]
    fn text_keys_read_with_or_without_their_wallet_suffix_and_a_bare_xpub_is_refused() {
        let s = &fx()["slip132"];
        let (fp, xp) = (s["fingerprint"].as_str().unwrap(), s["xpub"].as_str().unwrap());
        for t in [format!("[{fp}/48h/0h/0h/2h]{xp}"), format!("[{fp}/48'/0'/0'/2']{xp}/<0;1>/*"), format!("  [{}/48H/0H/0H/2H]{xp}/0/*  ", fp.to_uppercase())] {
            let k = &parse(&t).unwrap()[0];
            assert_eq!((k.fingerprint.as_str(), k.path.as_str(), k.xpub.as_str(), k.script.as_str()), (fp, "m/48h/0h/0h/2h", xp, "wsh-multi"), "{t}");
        }
        assert!(parse(xp).unwrap_err().contains("origin"));
        assert!(parse(&format!("[{fp}/48h/0h/0h/2h]{}", s["zpub"].as_str().unwrap())).unwrap()[0].xpub == xp, "a Zpub in brackets too");
    }
}
