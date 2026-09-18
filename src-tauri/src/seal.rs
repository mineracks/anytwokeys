//! The seal: a vault's recovery record, split k-of-n and kept on the cards themselves.
//! Any k cards rebuild the vault; fewer learn nothing beyond their own public key.
//! The format is public and frozen by the tests below — see docs/SEAL.md and tools/seal-recover.py.
use bitcoin::hashes::{sha256, Hash};
use serde::{Deserialize, Serialize};

pub const MAGIC: &[u8; 4] = b"A2K\x01";

// ---------- GF(2^8), the AES field (x^8 + x^4 + x^3 + x + 1) ----------
fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    while b != 0 {
        if b & 1 == 1 { p ^= a; }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 { a ^= 0x1B; }
        b >>= 1;
    }
    p
}
fn inv(a: u8) -> u8 {
    // a^254; a is never 0 here (x coordinates are 1..=255 and distinct)
    let mut r = 1u8;
    let mut base = a;
    let mut e = 254u8;
    while e != 0 { if e & 1 == 1 { r = mul(r, base); } base = mul(base, base); e >>= 1; }
    r
}

/// The polynomial through `points` (x, share), evaluated at `at`, byte by byte. `at` = 0 gives the secret.
pub fn interpolate(points: &[(u8, Vec<u8>)], at: u8) -> Result<Vec<u8>, String> {
    let len = points.first().ok_or("no shares")?.1.len();
    if points.iter().any(|(x, s)| *x == 0 || s.len() != len) { return Err("shares of different vaults or lengths".into()); }
    for (i, a) in points.iter().enumerate() { if points[..i].iter().any(|b| b.0 == a.0) { return Err("the same share twice".into()); } }
    let mut out = vec![0u8; len];
    for (i, (xi, si)) in points.iter().enumerate() {
        let mut w = 1u8; // Lagrange weight of point i at `at`
        for (j, (xj, _)) in points.iter().enumerate() { if i != j { w = mul(w, mul(at ^ xj, inv(xi ^ xj))); } }
        for (o, s) in out.iter_mut().zip(si) { *o ^= mul(w, *s); }
    }
    Ok(out)
}

/// Split `secret` into `n` shares (x = 1..=n), any `k` of which rebuild it. `random` supplies (k-1)·len bytes.
pub fn split(secret: &[u8], k: usize, n: usize, random: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    if k < 1 || k > n || n > 255 { return Err("threshold must be between 1 and the number of shares".into()); }
    if random.len() != (k - 1) * secret.len() { return Err("wrong amount of randomness".into()); }
    Ok((1..=n as u8).map(|x| secret.iter().enumerate().map(|(j, s)| {
        // Horner, highest coefficient first; coefficient c (1..k) of byte j is random[(c-1)*len + j]
        let mut y = 0u8;
        for c in (1..k).rev() { y = mul(y ^ random[(c - 1) * secret.len() + j], x); }
        y ^ *s
    }).collect()).collect())
}

// ---------- the record ----------
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record { pub network: String, pub threshold: usize, pub pubkeys: Vec<String>, pub chain_code: Option<String>, pub name: String, pub key_names: Vec<String> }

fn clip(s: &str, max: usize) -> &[u8] { let mut e = s.len().min(max); while !s.is_char_boundary(e) { e -= 1; } &s.as_bytes()[..e] }

pub fn encode(r: &Record) -> Result<Vec<u8>, String> {
    let n = r.pubkeys.len();
    if n == 0 || n > 15 || r.threshold < 1 || r.threshold > n || r.key_names.len() != n { return Err("not a vault this record can hold".into()); }
    let mut b = vec![1u8, match r.network.as_str() { "mainnet" => 0, "signet" => 1, _ => return Err("unknown network".into()) }, r.threshold as u8, n as u8, r.chain_code.is_some() as u8];
    for p in &r.pubkeys { let k = hex::decode(p).map_err(|e| e.to_string())?; if k.len() != 33 { return Err("a public key is not 33 bytes".into()); } b.extend(k); }
    if let Some(cc) = &r.chain_code { let c = hex::decode(cc).map_err(|e| e.to_string())?; if c.len() != 32 { return Err("the chain code is not 32 bytes".into()); } b.extend(c); }
    for (s, max) in std::iter::once((&r.name, 40)).chain(r.key_names.iter().map(|k| (k, 24))) { let t = clip(s, max); b.push(t.len() as u8); b.extend(t); }
    let sum = sha256::Hash::hash(&b);
    b.extend(&sum[..4]);
    Ok(b)
}

pub fn decode(b: &[u8]) -> Result<Record, String> {
    let bad = || "the shares did not rebuild a record (wrong cards together, or too few)".to_string();
    if b.len() < 9 { return Err(bad()); }
    let (body, sum) = b.split_at(b.len() - 4);
    if &sha256::Hash::hash(body)[..4] != sum || body[0] != 1 { return Err(bad()); }
    let (threshold, n, rotating) = (body[2] as usize, body[3] as usize, body[4] & 1 == 1);
    let mut i = 5;
    let mut take = |len: usize| -> Result<&[u8], String> { let s = body.get(i..i + len).ok_or_else(bad)?; i += len; Ok(s) };
    let pubkeys = (0..n).map(|_| take(33).map(hex::encode)).collect::<Result<Vec<_>, _>>()?;
    let chain_code = if rotating { Some(hex::encode(take(32)?)) } else { None };
    let mut text = || -> Result<String, String> { let l = take(1)?[0] as usize; Ok(String::from_utf8_lossy(take(l)?).into_owned()) };
    let name = text()?;
    let key_names = (0..n).map(|_| text()).collect::<Result<Vec<_>, _>>()?;
    Ok(Record { network: match body[1] { 0 => "mainnet", 1 => "signet", _ => return Err(bad()) }.into(), threshold, pubkeys, chain_code, name, key_names })
}

/// What names a vault on the cards: nothing that a rename changes, nothing one card can compute alone.
pub fn vault_id(network: &str, threshold: usize, pubkeys: &[String], chain_code: Option<&str>) -> String {
    let mut keys: Vec<String> = pubkeys.iter().map(|k| k.to_lowercase()).collect();
    keys.sort();
    let text = format!("a2k-seal|{network}|{threshold}|{}|{}", keys.join(","), chain_code.unwrap_or(""));
    hex::encode(&sha256::Hash::hash(text.as_bytes())[..4])
}

// ---------- what sits on a card ----------
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry { pub id: String, pub k: u8, pub x: u8, pub share: String }

/// A card's container: MAGIC, then entries of id(4) k(1) x(1) len(2, big-endian) share. Empty = no entries.
pub fn parse_container(b: &[u8]) -> Result<Vec<Entry>, String> {
    if b.is_empty() { return Ok(vec![]); }
    if b.len() < 4 || &b[..4] != MAGIC { return Err("the card's certificate object holds something that is not ours".into()); }
    let (mut i, mut out) = (4, vec![]);
    while i < b.len() {
        let h = b.get(i..i + 8).ok_or("truncated entry")?;
        let len = u16::from_be_bytes([h[6], h[7]]) as usize;
        let share = b.get(i + 8..i + 8 + len).ok_or("truncated share")?;
        out.push(Entry { id: hex::encode(&h[..4]), k: h[4], x: h[5], share: hex::encode(share) });
        i += 8 + len;
    }
    Ok(out)
}

pub fn build_container(entries: &[Entry]) -> Result<Vec<u8>, String> {
    let mut b = MAGIC.to_vec();
    for e in entries {
        let (id, share) = (hex::decode(&e.id).map_err(|e| e.to_string())?, hex::decode(&e.share).map_err(|e| e.to_string())?);
        if id.len() != 4 || share.len() > 0xFFFF { return Err("malformed entry".into()); }
        b.extend(id); b.push(e.k); b.push(e.x); b.extend((share.len() as u16).to_be_bytes()); b.extend(share);
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn slow_mul(a: u8, b: u8) -> u8 { // carry-less multiply, then reduce by 0x11B: a second route to the same product
        let mut p: u16 = 0;
        for i in 0..8 { if b >> i & 1 == 1 { p ^= (a as u16) << i; } }
        for i in (8..16).rev() { if p >> i & 1 == 1 { p ^= 0x11B << (i - 8); } }
        p as u8
    }
    #[test]
    fn the_field_is_a_field() {
        for a in 0..=255u8 { for b in 0..=255u8 { assert_eq!(mul(a, b), slow_mul(a, b)); } if a != 0 { assert_eq!(mul(a, inv(a)), 1); } }
    }
    fn record(rotating: bool, n: usize, k: usize) -> Record {
        let keys = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe", "038410f240613b33fe73d968f331ca82eb1a2b43ddb2ad6c4a27b2f6d0c1e2a3b4"];
        Record { network: "signet".into(), threshold: k, pubkeys: (0..n).map(|i| keys[i % 3].to_string()).collect(), chain_code: rotating.then(|| "ab".repeat(32)),
            name: "fämily vault".into(), key_names: (0..n).map(|i| format!("key {i}")).collect() }
    }
    fn stream(len: usize) -> Vec<u8> { (0..len).map(|i| sha256::Hash::hash(&(i as u32).to_be_bytes())[0]).collect() }

    #[test]
    fn every_k_subset_rebuilds_and_fewer_do_not() {
        for (k, n) in [(2usize, 3usize), (3, 5), (1, 1), (15, 15), (2, 15)] {
            let r = record(true, n, k);
            let secret = encode(&r).unwrap();
            let shares = split(&secret, k, n, &stream((k - 1) * secret.len())).unwrap();
            // every window of k consecutive (wrapping) shares, which covers first/last/mixed positions
            for start in 0..n {
                let pts: Vec<(u8, Vec<u8>)> = (0..k).map(|j| { let i = (start + j) % n; (i as u8 + 1, shares[i].clone()) }).collect();
                assert_eq!(decode(&interpolate(&pts, 0).unwrap()).unwrap(), r);
                if k > 1 { assert!(decode(&interpolate(&pts[..k - 1], 0).unwrap()).is_err(), "k-1 shares must not decode"); }
                // a missing card's share can be re-made from any k others
                let other = (start + k) % n;
                assert_eq!(interpolate(&pts, other as u8 + 1).unwrap(), shares[other]);
            }
        }
    }
    #[test]
    fn the_format_is_frozen() {
        // If this fails, cards sealed by an earlier build can no longer be read. Do not "fix" the vector.
        let r = record(true, 3, 2);
        let secret = encode(&r).unwrap();
        assert_eq!(secret.len(), 5 + 99 + 32 + 1 + 13 + 3 * 6 + 4);
        let shares = split(&secret, 2, 3, &stream(secret.len())).unwrap();
        let digest = hex::encode(sha256::Hash::hash(&shares.concat()));
        assert_eq!(digest, include_str!("../../tools/seal-vector.sha256").trim());
        assert_eq!(vault_id(&r.network, 2, &r.pubkeys, r.chain_code.as_deref()), vault_id(&r.network, 2, &r.pubkeys.iter().rev().cloned().collect::<Vec<_>>(), r.chain_code.as_deref()));
    }
    #[test]
    fn containers_round_trip_and_refuse_strangers() {
        let es = vec![Entry { id: "01020304".into(), k: 2, x: 1, share: "aabbcc".into() }, Entry { id: "0a0b0c0d".into(), k: 3, x: 5, share: "00".repeat(300) }];
        assert_eq!(parse_container(&build_container(&es).unwrap()).unwrap(), es);
        assert_eq!(parse_container(&[]).unwrap(), vec![]);
        assert!(parse_container(b"\x30\x82\x01\x0a certificate").is_err());
        let mut cut = build_container(&es).unwrap(); cut.truncate(20);
        assert!(parse_container(&cut).is_err());
    }
}
