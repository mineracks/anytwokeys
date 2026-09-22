//! A wallet descriptor carried as UR — how Sparrow and others back one up on a QR code or a metal plate:
//! `ur:crypto-output` (BCR-2020-010, version 1, what Sparrow writes) and `ur:output-descriptor` (BCR-2023-010,
//! version 3). Either becomes the descriptor TEXT, which then goes through exactly the same checks as a pasted one.
//! One part or several (a plate per part, or an animated code); parts in any order.
//!
//! Tested on every official vector of both specifications (tests/fixtures/ur-output-vectors.json, fetched from
//! BlockchainCommons/Research), and against a second encoder, `urtypes` (Krux's), on a throwaway 2-of-3.
use ciborium::value::Value as C;

pub fn is_ur(text: &str) -> bool {
    let t = text.trim_start().to_ascii_lowercase();
    t.starts_with("ur:crypto-output/") || t.starts_with("ur:output-descriptor/")
}

pub struct Decoded { pub descriptor: String, pub name: Option<String> }

pub fn decode(text: &str) -> Result<Decoded, String> {
    let parts: Vec<String> = text.split_whitespace().map(|p| p.to_ascii_lowercase()).collect();
    let kind = parts.first().and_then(|p| p.strip_prefix("ur:")).and_then(|p| p.split('/').next()).unwrap_or("").to_string();
    if parts.iter().any(|p| !p.starts_with(&format!("ur:{kind}/"))) { return Err("those codes are not all parts of the same descriptor".into()); }
    let cbor = if parts.len() == 1 && parts[0].matches('/').count() == 1 { ur::ur::decode(&parts[0]).map_err(|e| format!("that code could not be read: {e}"))?.1 } else {
        let mut d = ur::Decoder::default();
        for p in &parts { d.receive(p).map_err(|e| format!("that code could not be read: {e}"))?; }
        d.message().map_err(|e| e.to_string())?.ok_or_else(|| format!("some QR parts are missing: {} of {} pieces recovered so far; keep the code in the frame until it completes", d.resolved_fragment_count().unwrap_or(0), d.fragment_count()))?
    };
    let v: C = ciborium::from_reader(cbor.as_slice()).map_err(|e| format!("the code's contents are not readable: {e}"))?;
    match kind.as_str() {
        "crypto-output" => Ok(Decoded { descriptor: script(untag(&v, &[308]))?, name: None }),
        "output-descriptor" => v3(untag(&v, &[40308])),
        _ => Err(format!("a ur:{kind} code is not a wallet descriptor")),
    }
}

fn untag<'a>(v: &'a C, tags: &[u64]) -> &'a C { match v { C::Tag(t, inner) if tags.contains(t) => inner, _ => v } }
fn get<'a>(m: &'a C, k: u64) -> Option<&'a C> { match m { C::Map(e) => e.iter().find(|(key, _)| matches!(key, C::Integer(i) if u64::try_from(*i).ok() == Some(k))).map(|(_, v)| v), _ => None } }
fn uint(v: &C) -> Option<u64> { match v { C::Integer(i) => u64::try_from(*i).ok(), _ => None } }
fn bytes(v: &C) -> Option<&[u8]> { match v { C::Bytes(b) => Some(b), _ => None } }
fn boolean(v: Option<&C>) -> bool { matches!(v, Some(C::Bool(true))) }

/// Version 1: a tree of tagged script expressions.
fn script(v: &C) -> Result<String, String> {
    let C::Tag(tag, inner) = v else { return Err("the descriptor's script is not tagged".into()) };
    let one = |f: &str| -> Result<String, String> { Ok(format!("{f}({})", script(inner)?)) };
    let key1 = |f: &str| -> Result<String, String> { Ok(format!("{f}({})", key(inner)?)) };
    match tag {
        400 => one("sh"), 401 => one("wsh"), 409 => one("tr"),
        402 => key1("pk"), 403 => key1("pkh"), 404 => key1("wpkh"), 405 => key1("combo"), 410 => key1("cosigner"),
        406 | 407 => {
            let k = get(inner, 1).and_then(uint).ok_or("a multisig without its threshold")?;
            let C::Array(keys) = get(inner, 2).ok_or("a multisig without its keys")? else { return Err("a multisig's keys are not a list".into()) };
            let keys = keys.iter().map(key).collect::<Result<Vec<_>, _>>()?;
            Ok(format!("{}({k},{})", if *tag == 406 { "multi" } else { "sortedmulti" }, keys.join(",")))
        }
        307 => Err("that descriptor is a single address; this app restores wallets".into()),
        408 => Err("that descriptor is a raw script; this app restores wallets".into()),
        t => Err(format!("that descriptor uses a script this app does not read (tag {t})")),
    }
}

/// Version 3: the descriptor's own text with @0, @1… standing for the keys that follow it.
fn v3(m: &C) -> Result<Decoded, String> {
    let C::Text(source) = get(m, 1).ok_or("the descriptor has no text")? else { return Err("the descriptor's text is not text".into()) };
    let keys = match get(m, 2) { Some(C::Array(a)) => a.iter().map(key).collect::<Result<Vec<_>, _>>()?, _ => vec![] };
    let mut out = String::new(); let mut rest = source.as_str();
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        let digits: String = rest[at + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
        let n: usize = digits.parse().map_err(|_| "a placeholder without its number")?;
        out.push_str(keys.get(n).ok_or_else(|| format!("the text names key @{n}, which is not there"))?);
        rest = &rest[at + 1 + digits.len()..];
    }
    out.push_str(rest);
    let name = match get(m, 3) { Some(C::Text(t)) if !t.trim().is_empty() => Some(t.trim().to_string()), _ => None };
    Ok(Decoded { descriptor: out, name })
}

fn key(v: &C) -> Result<String, String> {
    match v {
        C::Tag(306 | 40306, m) => {
            if boolean(get(m, 2)) { return Err("that descriptor holds a PRIVATE key; this app only ever keeps public ones".into()); }
            Ok(hex::encode(get(m, 3).and_then(bytes).ok_or("a key without its data")?))
        }
        C::Tag(303 | 40303, m) => hdkey(m),
        C::Tag(307 | 40307, _) => Err("that descriptor names an address, not a key".into()),
        _ => Err("a key this app does not read".into()),
    }
}

/// A keypath's components: (index, hardened) pairs, `[]` for *, [low, high] for a range, [a, ha, b, hb] for <a;b>.
fn path(m: &C) -> Result<(Vec<String>, Option<u32>, Option<u8>), String> {
    let C::Array(c) = get(m, 1).ok_or("a key path without its steps")? else { return Err("a key path's steps are not a list".into()) };
    let h = |b: bool| if b { "h" } else { "" };
    let mut out = Vec::new(); let mut i = 0;
    while i < c.len() {
        match &c[i] {
            C::Integer(_) => { let n = uint(&c[i]).ok_or("a step out of range")?; let hard = boolean(c.get(i + 1)); out.push(format!("{n}{}", h(hard))); i += 2; }
            C::Array(a) if a.is_empty() => { out.push(format!("*{}", h(boolean(c.get(i + 1))))); i += 2; }
            C::Array(a) if a.len() == 4 => { let (x, y) = (uint(&a[0]).ok_or("a step out of range")?, uint(&a[2]).ok_or("a step out of range")?); out.push(format!("<{x}{};{y}{}>", h(boolean(a.get(1))), h(boolean(a.get(3))))); i += 1; }
            C::Array(_) => return Err("a range of steps in a key path; this app needs a wallet's own <0;1>/*".into()),
            _ => return Err("a key path step this app does not read".into()),
        }
    }
    let fp = get(m, 2).and_then(uint).map(|f| f as u32);
    let depth = get(m, 3).and_then(uint).map(|d| d as u8);
    Ok((out, fp, depth))
}

fn hdkey(m: &C) -> Result<String, String> {
    if boolean(get(m, 2)) { return Err("that descriptor holds a PRIVATE key; this app only ever keeps public ones".into()); }
    let data = get(m, 3).and_then(bytes).ok_or("an extended key without its key")?;
    let chain = get(m, 4).and_then(bytes).ok_or("an extended key without its chain code")?;
    // network: 0 mainnet (the default), 1 testnet
    let test = get(m, 5).map(|u| untag(u, &[305, 40305])).and_then(|u| get(u, 2)).and_then(uint) == Some(1);
    let master = boolean(get(m, 1));
    let (origin, fp, depth) = match get(m, 6).map(|o| untag(o, &[304, 40304])) { Some(o) => path(o)?, None => (vec![], None, None) };
    // a key one step below the master, with no parent fingerprint of its own: its parent IS the master
    let parent = get(m, 8).and_then(uint).map(|p| p as u32).or_else(|| if origin.len() == 1 { fp } else { None }).unwrap_or(0);
    let child = if master { 0 } else { origin.last().map(|s| { let hard = s.ends_with('h'); s.trim_end_matches('h').parse::<u32>().map(|n| if hard { n | 0x8000_0000 } else { n }).unwrap_or(0) }).unwrap_or(0) };
    let depth = if master { 0 } else { depth.unwrap_or(origin.len() as u8) };
    let mut raw = Vec::with_capacity(78);
    raw.extend_from_slice(&if test { [0x04, 0x35, 0x87, 0xcf] } else { [0x04, 0x88, 0xb2, 0x1e] });
    raw.push(depth); raw.extend_from_slice(&if master { 0 } else { parent }.to_be_bytes()); raw.extend_from_slice(&child.to_be_bytes());
    raw.extend_from_slice(chain); raw.extend_from_slice(data);
    if raw.len() != 78 { return Err("an extended key of the wrong length".into()); }
    let xpub = bitcoin::base58::encode_check(&raw);
    let origin_text = match (fp, origin.is_empty()) { (Some(f), false) => format!("[{f:08x}/{}]", origin.join("/")), (Some(f), true) => format!("[{f:08x}]"), (None, _) => String::new() };
    let children = match get(m, 7).map(|c| untag(c, &[304, 40304])) {
        Some(c) => { let (steps, _, _) = path(c)?; if steps.is_empty() { String::new() } else { format!("/{}", steps.join("/")) } }
        None => String::new(),
    };
    Ok(format!("{origin_text}{xpub}{children}"))
}

/// A wallet backup that names no derivation for its keys (Sparrow's multisig export, for one) means the wallet's own
/// receive and change chains, <0;1>/* — the one reading every wallet gives it. A key that names a single chain, or
/// anything else, is left as written, and the ordinary checks say what is missing.
pub fn as_wallet(desc: &str) -> String {
    use miniscript::ForEachKey;
    use std::str::FromStr;
    let Ok(d) = miniscript::Descriptor::<miniscript::DescriptorPublicKey>::from_str(desc) else { return desc.to_string() };
    let mut bare = false;
    d.for_each_key(|k| { if let miniscript::DescriptorPublicKey::XPub(x) = k { if x.derivation_path.is_empty() && x.wildcard == miniscript::descriptor::Wildcard::None { bare = true; } } true });
    if !bare { return desc.to_string(); }
    // add /<0;1>/* after every extended key that has none
    let mut out = String::new(); let mut rest = desc;
    while let Some(at) = rest.find(|c: char| c == 'x' || c == 't').filter(|_| true) {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        if tail.starts_with("xpub") || tail.starts_with("tpub") {
            let end = tail.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(tail.len());
            out.push_str(&tail[..end]);
            if !tail[end..].starts_with('/') { out.push_str("/<0;1>/*"); }
            rest = &tail[end..];
        } else { out.push_str(&tail[..1]); rest = &tail[1..]; }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    fn fx() -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/ur-output.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap() }
    /// the same descriptor, whether written 48' or 48h
    fn canon(d: &str) -> String { miniscript::Descriptor::<miniscript::DescriptorPublicKey>::from_str(d).map(|d| d.to_string()).unwrap_or_else(|_| d.replace('\'', "h")) }
    /// The same WALLET: the same scripts at the same positions, and the same origins. (Compared this way because an
    /// xpub's child-number field is metadata that derives nothing — and BCR-2020-010's vector 4 prints fffffffe there,
    /// a value its own encoding does not carry, so no decoder could reproduce that text.)
    fn same_wallet(a: &str, b: &str) -> bool {
        use miniscript::{Descriptor, DescriptorPublicKey};
        let (Ok(x), Ok(y)) = (Descriptor::<DescriptorPublicKey>::from_str(a), Descriptor::<DescriptorPublicKey>::from_str(b)) else { return canon(a) == canon(b) };
        let origins = |d: &Descriptor<DescriptorPublicKey>| d.to_string().split('[').skip(1).map(|s| s.split(']').next().unwrap_or("").to_string()).collect::<Vec<_>>();
        // every origin the expected text prints must be ours too (a text may leave out an origin its encoding carries)
        let ours = origins(&x); if !origins(&y).iter().all(|o| ours.contains(o)) { return false; }
        if !x.has_wildcard() { return x.to_string() == y.to_string(); }
        (0..4).all(|i| x.at_derivation_index(i).map(|d| d.script_pubkey()).ok() == y.at_derivation_index(i).map(|d| d.script_pubkey()).ok())
    }

    #[test]
    fn every_official_version_1_vector_reads_to_its_descriptor() {
        for v in fx()["official"]["v1"].as_array().unwrap() {
            let got = decode(v["ur"].as_str().unwrap()).unwrap_or_else(|e| panic!("vector {}: {e}", v["n"]));
            assert!(same_wallet(&got.descriptor, v["descriptor"].as_str().unwrap()), "vector {}: {} vs {}", v["n"], got.descriptor, v["descriptor"]);
        }
    }
    #[test]
    fn the_official_version_3_vectors_read_or_are_refused_for_the_right_reason() {
        for v in fx()["official"]["v3"].as_array().unwrap() {
            let cbor = hex::decode(v["hex"].as_str().unwrap()).unwrap();
            let ur_text = ur::ur::encode(&cbor, &ur::Type::Custom("output-descriptor"));
            let got = decode(&ur_text);
            match v["n"].as_u64().unwrap() {
                2 => assert!(got.err().unwrap().contains("address"), "a single address is not a wallet to restore"),
                3 => assert!(got.err().unwrap().contains("PRIVATE"), "a WIF private key is refused, never kept"),
                n => { let got = got.unwrap_or_else(|e| panic!("vector {n}: {e}")); assert!(same_wallet(&got.descriptor, v["text"].as_str().unwrap()), "vector {n}: {}", got.descriptor);
                       if n == 4 { assert_eq!(got.name.as_deref(), Some("Satoshi's Stash")); } }
            }
        }
    }
    /// A Sparrow-style backup — keys with origins and no child path — made by ANOTHER encoder (urtypes, Krux's),
    /// split into parts as on metal plates, read in any order, and taken as the wallet's own <0;1>/*: it must derive
    /// exactly the addresses embit derives for the same wallet.
    #[test]
    fn a_sparrow_style_backup_from_another_encoder_restores_the_same_wallet_from_parts_in_any_order() {
        let f = fx(); let s = &f["sparrow_style"];
        let cbor = hex::decode(s["cbor"].as_str().unwrap()).unwrap();
        let whole = decode(&ur::ur::encode(&cbor, &ur::Type::Custom("crypto-output"))).unwrap();
        assert_eq!(canon(&whole.descriptor), canon(s["urtypes_descriptor"].as_str().unwrap().split('#').next().unwrap()), "the same text urtypes itself reads back");
        let mut enc = ur::Encoder::new(&cbor, 120, "crypto-output").unwrap();
        let n = enc.fragment_count(); assert!(n >= 2, "split like a plate backup: {n}");
        let mut parts: Vec<String> = (0..n).map(|_| enc.next_part().unwrap().to_uppercase()).collect();   // engraved in capitals
        parts.reverse();
        assert!(decode(&parts[..1].join("\n")).err().unwrap().contains("missing"), "one plate alone says what is missing");
        let restored = as_wallet(&decode(&parts.join("\n")).unwrap().descriptor);
        assert!(restored.contains("/<0;1>/*"), "{restored}");
        let ext: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/ext-descriptors.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
        let info = crate::wallet::ext_parse(&restored).unwrap().unwrap();
        let v = crate::ext_vault("r".into(), &info, &[]);
        for (i, want) in ext["sortedmulti"]["recv"].as_array().unwrap().iter().enumerate() { assert_eq!(crate::wallet::address_at(&v, i as u32).unwrap(), want.as_str().unwrap(), "receive {i}"); }
        for (i, want) in ext["sortedmulti"]["chg"].as_array().unwrap().iter().enumerate() { assert_eq!(crate::wallet::address_at(&v, crate::wallet::CHANGE | i as u32).unwrap(), want.as_str().unwrap(), "change {i}"); }
    }
    #[test]
    fn keys_that_already_name_their_chains_are_left_exactly_as_written() {
        let d = "wsh(multi(1,xpub661MyMwAqRbcFW31YEwpkMuc5THy2PSt5bDMsktWQcFF8syAmRUapSCGu8ED9W6oDMSgv6Zz8idoc4a6mr8BDzTJY47LJhkJ8UB7WEGuduB/1/0/*,xpub69H7F5d8KSRgmmdJg2KhpAK8SR3DjMwAdkxj3ZuxV27CprR9LgpeyGmXUbC6wb7ERfvrnKZjXoUmmDznezpbZb7ap6r1D3tgFxHmwMkQTPH/0/0/*))";
        assert_eq!(as_wallet(d), d);
    }
}
