//! A vault's recovery line split into SSKR shares — "any k of n" — for metal plates or paper, the Blockchain Commons
//! way: a descriptor is far bigger than the 16–32 bytes SSKR itself can split, so it goes into a Gordian Envelope,
//! encrypted under a random key, and only that key is split. Each share is `ur:envelope`: the encrypted descriptor
//! plus one share of the key. Fewer than k shares say nothing about the vault; any k rebuild it exactly.
//!
//! Built on Blockchain Commons' own crates (bc-envelope, bc-components → sskr), not reimplemented. Checked in both
//! directions against their `envelope` tool (tests/fixtures/sskr-envelope.json).
use bc_components::{SSKRGroupSpec, SSKRShare, SSKRSpec, SymmetricKey};
use bc_envelope::prelude::*;

pub fn is_share(text: &str) -> bool { text.trim_start().to_ascii_lowercase().starts_with("ur:envelope/") }

/// The recovery line as `n` shares, any `k` of which rebuild it.
pub fn split(line: &str, k: usize, n: usize) -> Result<Vec<String>, String> {
    if !(1..=16).contains(&n) || k < 1 || k > n { return Err("shares: between 1 and 16, and needing between 1 and all of them".into()); }
    if k == 1 && n > 1 { return Err("with any ONE share able to rebuild it, every share is the whole recovery line: choose at least 2".into()); }
    bc_components::register_tags();
    let key = SymmetricKey::new();
    let sealed = Envelope::new(line.trim()).wrap().encrypt_subject(&key).map_err(|e| e.to_string())?;
    let spec = SSKRSpec::new(1, vec![SSKRGroupSpec::new(k, n).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
    Ok(sealed.sskr_split_flattened(&spec, &key).map_err(|e| e.to_string())?.iter().map(|e| e.ur_string()).collect())
}

/// What one share says about itself: which split it belongs to, which member it is, and how many are needed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct About { pub set: String, pub member: usize, pub need: usize, pub groups: usize }
fn about(e: &Envelope) -> Result<About, String> {
    let s: SSKRShare = e.object_for_predicate(known_values::SSKR_SHARE).map_err(|_| "that envelope is not an SSKR share".to_string())?
        .extract_subject().map_err(|_| "that share could not be read".to_string())?;
    Ok(About { set: s.identifier_hex(), member: s.member_index(), need: s.member_threshold(), groups: s.group_count() })
}

/// The set a share belongs to (its 4-hex-digit identifier), for printing on its sheet.
pub fn set_of(share: &str) -> Option<String> { bc_components::register_tags(); Envelope::from_ur_string(share.to_ascii_lowercase()).ok().and_then(|e| about(&e).ok()).map(|a| a.set) }

pub struct Joined { pub text: String }

/// Rebuild from the shares read so far (one per line or space; the same share twice counts once).
pub fn join(text: &str) -> Result<Joined, String> {
    bc_components::register_tags();
    let mut shares: Vec<(About, Envelope)> = Vec::new();
    for t in text.split_whitespace() {
        if t.matches('/').count() > 1 { return Err("that share is split across several codes; show it as one code".into()); }
        let e = Envelope::from_ur_string(t.to_ascii_lowercase()).map_err(|_| "that code is not a share this app can read".to_string())?;
        let a = about(&e)?;
        if let Some((first, _)) = shares.first() { if first.set != a.set { return Err(format!("those shares come from two different splits ({} and {}); use shares of one split only", first.set, a.set)); } }
        if a.groups != 1 { return Err("that split uses groups of shares; this app reads a single group of k of n".into()); }
        if !shares.iter().any(|(b, _)| b.member == a.member) { shares.push((a, e)); }
    }
    let (first, _) = shares.first().ok_or("nothing to read")?;
    let need = first.need;
    if shares.len() < need { return Err(format!("some shares are missing: {} of {need} shares read so far; show the next one", shares.len())); }
    let refs: Vec<&Envelope> = shares.iter().map(|(_, e)| e).collect();
    let inner = Envelope::sskr_join(&refs).map_err(|_| "those shares do not fit together; one may be damaged".to_string())?.try_unwrap().map_err(|e| e.to_string())?;
    // the recovery line as text, or a descriptor as UR CBOR (another tool's way of putting one in an envelope)
    if let Ok(s) = inner.extract_subject::<String>() { return Ok(Joined { text: s }); }
    Err("the shares rebuilt something that is not a recovery line".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    const LINE: &str = "wsh(sortedmulti(2,[c9ad277c/48h/0h/0h/2h]xpub6EAC4GduaPj3EpUtjzHbL7gLZ9jNyf4vaVtYSpNRj3KWy2xrg84nRg5ajnnaH3m7sx4VHHCVKThWWtBHUXjrpZCh6dh64X2iA2FaAqc7m6f/<0;1>/*,0331794a619d6632ccce693398df84fa6e1433896fcb98ea13ac74dccfd21f0a99,03c58267e856160ede38923c7582224b211b306de4b42265a546ce4dea4c38c7ce))";
    #[test]
    fn any_k_shares_in_any_order_rebuild_it_and_fewer_say_so() {
        let s = split(LINE, 2, 3).unwrap();
        assert_eq!(s.len(), 3);
        for (a, b) in [(0, 1), (2, 0), (1, 2)] { assert_eq!(join(&format!("{}\n{}", s[a], s[b])).unwrap().text, LINE); }
        let one = join(&s[1]).err().unwrap();
        assert!(one.contains("1 of 2 shares"), "{one}");
        assert!(join(&format!("{} {}", s[1], s[1])).err().unwrap().contains("1 of 2"), "the same share twice counts once");
        assert_eq!(join(&s.iter().map(|x| x.to_uppercase()).collect::<Vec<_>>().join("\n")).unwrap().text, LINE, "engraved in capitals, all three");
        // one share says nothing of the vault: no key, no fingerprint in it
        assert!(!s[0].contains("xpub") && !s[0].contains("c9ad277c"));
    }
    /// Shares made by Blockchain Commons' own `envelope` tool rebuild here, any two of the three.
    #[test]
    fn shares_made_by_the_envelope_tool_rebuild_here() {
        let f: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/sskr-envelope.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
        let s: Vec<&str> = f["cli_shares"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
        for (a, b) in [(0, 1), (1, 2), (2, 0)] { assert_eq!(join(&format!("{}\n{}", s[a], s[b])).unwrap().text, f["line"].as_str().unwrap(), "shares {a} and {b}"); }
    }
    /// Prints shares made HERE, for the envelope tool to join (the other direction; see the fixture).
    #[test] #[ignore]
    fn emit_shares_for_the_envelope_tool() { for s in split(LINE, 2, 3).unwrap() { println!("SHARE {s}"); } }
    #[test]
    fn shares_of_two_different_splits_are_refused_by_name() {
        let (a, b) = (split(LINE, 2, 3).unwrap(), split(LINE, 2, 3).unwrap());
        assert!(join(&format!("{}\n{}", a[0], b[1])).err().unwrap().contains("two different splits"));
    }
    #[test]
    fn a_split_any_one_share_could_open_is_refused() {
        assert!(split(LINE, 1, 3).err().unwrap().contains("at least 2"));
        assert!(split(LINE, 4, 3).is_err());
    }
}
