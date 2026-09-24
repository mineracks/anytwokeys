//! Reading a Spark wallet — Wallet of Satoshi's self-custody mode and anything else built on Spark.
//!
//! Spark holds coins off-chain, in leaves under a 2-of-2 with an operator, so there is nothing on the chain to watch
//! and our own node cannot check a single figure here. **Everything this module returns is Spark's word.** It is
//! useful all the same: dated transfers are what a cost basis needs, and a wallet nobody can reconcile is worse.
//!
//! What makes it possible: the operators' `query_balance` and `query_all_transfers` are declared `AuthAnonymous` in
//! their own policy table, and the handler's `HasReadAccessToWallet` returns TRUE unless that wallet turned on
//! private mode. So a Spark address alone reads a wallet — which is also a privacy fact about Spark worth saying out
//! loud. If private mode is ever switched on, the answer comes back EMPTY with `offset = -1` rather than as an
//! error. And a refusal cannot be told from an empty wallet by the offset alone: a stranger's key also comes back
//! with an empty list and `offset = -1` (checked against a live operator, 2026-09-24). What DOES tell them apart is
//! the balance: sats held with no transfers to account for them means the history is being withheld, not absent.
//!
//! Read from github.com/buildonspark/spark (2026-09-24), not guessed.

/// mainnet operators, in the order the SDK lists them
pub const OPERATORS: [&str; 3] = ["https://0.spark.lightspark.com", "https://spark-operator.breez.technology", "https://2.spark.flashnet.xyz"];

// ---------- just enough protobuf ----------
fn varint(mut n: u64, out: &mut Vec<u8>) { loop { let b = (n & 0x7f) as u8; n >>= 7; out.push(if n > 0 { b | 0x80 } else { b }); if n == 0 { break; } } }
fn tag(field: u64, wire: u8, out: &mut Vec<u8>) { varint((field << 3) | wire as u64, out); }
fn bytes_field(field: u64, v: &[u8], out: &mut Vec<u8>) { tag(field, 2, out); varint(v.len() as u64, out); out.extend_from_slice(v); }
fn int_field(field: u64, v: u64, out: &mut Vec<u8>) { tag(field, 0, out); varint(v, out); }

struct Reader<'a> { b: &'a [u8], i: usize }
impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self { Self { b, i: 0 } }
    fn done(&self) -> bool { self.i >= self.b.len() }
    fn varint(&mut self) -> Option<u64> {
        let (mut n, mut shift) = (0u64, 0);
        loop {
            let byte = *self.b.get(self.i)?; self.i += 1;
            n |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 { return Some(n); }
            shift += 7; if shift > 63 { return None; }
        }
    }
    /// the next field as (number, payload) — a length-delimited one gives its bytes, a varint gives its value
    fn field(&mut self) -> Option<(u64, Vec<u8>, u64)> {
        let key = self.varint()?;
        let (num, wire) = (key >> 3, (key & 7) as u8);
        match wire {
            0 => { let v = self.varint()?; Some((num, Vec::new(), v)) }
            2 => { let len = self.varint()? as usize; let end = self.i.checked_add(len)?; let v = self.b.get(self.i..end)?.to_vec(); self.i = end; Some((num, v, 0)) }
            5 => { let end = self.i + 4; let v = self.b.get(self.i..end)?.to_vec(); self.i = end; Some((num, v, 0)) }
            1 => { let end = self.i + 8; let v = self.b.get(self.i..end)?.to_vec(); self.i = end; Some((num, v, 0)) }
            _ => None,
        }
    }
}

// ---------- a Spark address ----------
/// `spark1…` (bech32m) wraps a protobuf whose first field is the identity public key. That key is all a read needs.
pub fn identity_of(address: &str) -> Result<(Vec<u8>, String), String> {
    use bech32::primitives::decode::CheckedHrpstring;
    let s = find_address(address).ok_or("that is not a Spark address — it should read like spark1…".to_string())?;
    let s = s.as_str();
    let parsed = CheckedHrpstring::new::<bech32::Bech32m>(s).map_err(|_| "that is not a Spark address — it should read like spark1…".to_string())?;
    let network = match parsed.hrp().to_lowercase().as_str() {
        "spark" => "mainnet", "sparkt" => "testnet", "sparks" => "signet", "sparkrt" => "regtest",
        other => return Err(format!("that address is for an unknown Spark network ({other})")),
    }.to_string();
    let body: Vec<u8> = parsed.byte_iter().collect();
    let mut r = Reader::new(&body);
    while !r.done() {
        match r.field() {
            Some((1, v, _)) if v.len() == 33 => return Ok((v, network)),
            Some(_) => continue,
            None => break,
        }
    }
    Err("that Spark address carries no identity key".into())
}

/// The `spark…1…` inside whatever was pasted or scanned: a bare address, an invoice, a `spark:` URI, a BIP 21 line
/// with a `spark=` parameter, or a QR's upper-case rendering of any of those. A camera held up to a wallet's Receive
/// screen should just work, whichever of these that wallet chose to show.
pub fn find_address(text: &str) -> Option<String> {
    let lower = text.trim().to_lowercase();
    // "spark:" as a scheme is followed by the address itself, so every "spark" is tried, not just the first
    lower.match_indices("spark").map(|(start, _)| lower[start..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>())
        .find(|cand| cand.len() >= 20 && cand.contains('1'))
}

// ---------- from the seed words, when the app shows no Spark address ----------
//
// Wallet of Satoshi shows Lightning and on-chain on its Receive screen and no Spark code, but it does show the
// seed words. Spark's published scheme (docs.spark.money/wallets/identity-key-derivation, fetched 2026-09-25):
// a standard BIP 39 seed with no passphrase, and the identity key at `m/8797555'/account'/0'` — 8797555 being the
// last three bytes of SHA256("spark"). **Mainnet defaults to account 1**, for wallets made before accounts existed.
//
// The words unlock everything the wallet can spend. They are used here for one derivation and then dropped: what
// the vault keeps is the identity PUBLIC key, which is what a Spark address carries anyway. The same shape as a Muun
// recovery code, and the same rule: nothing that can spend is written down.
pub const PURPOSE: u32 = 8797555;

/// The identity public key of one account of a wallet made from these words.
pub fn identity_from_words(words: &str, account: u32) -> Result<Vec<u8>, String> {
    use bitcoin::bip32::{ChildNumber, Xpriv};
    use zeroize::Zeroize;
    // Every secret made on the way — the normalised words, the seed, the root, the derived private key — is wiped
    // when it has done its one job, not left for the allocator to hand out later. The error never quotes the words.
    let mut norm = words.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let parsed = bip39::Mnemonic::parse_normalized(&norm);
    norm.zeroize();
    let m = parsed.map_err(|_| "those are not a valid set of seed words (a word is not in the list, or the checksum fails — check each one)".to_string())?;
    let mut seed = m.to_seed("");
    drop(m);
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let root = Xpriv::new_master(bitcoin::Network::Bitcoin, &seed);
    seed.zeroize();
    let mut root = root.map_err(|e| e.to_string())?;
    let path = [ChildNumber::from_hardened_idx(PURPOSE), ChildNumber::from_hardened_idx(account), ChildNumber::from_hardened_idx(0)]
        .into_iter().collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    let derived = root.derive_priv(&secp, &path);
    root.private_key.non_secure_erase();
    let mut k = derived.map_err(|e| e.to_string())?;
    let public = k.private_key.public_key(&secp).serialize().to_vec();
    k.private_key.non_secure_erase();
    Ok(public)
}

/// Does this set of words check out — right count, every word in the list, checksum true? Nothing is derived and
/// nothing is kept; the screen asks this the moment the last box is filled, so a slip is caught before the words go
/// anywhere near a key.
pub fn words_check(words: &str) -> Result<usize, String> {
    use zeroize::Zeroize;
    let mut norm = words.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let n = norm.split(' ').filter(|w| !w.is_empty()).count();
    let r = bip39::Mnemonic::parse_normalized(&norm).map(|_| n).map_err(|e| match e {
        bip39::Error::BadWordCount(c) => format!("{c} words — a set is 12 or 24"),
        bip39::Error::UnknownWord(i) => format!("word {} is not in the list", i + 1),
        bip39::Error::InvalidChecksum => "the checksum does not match: one of the words is wrong, or two are swapped".into(),
        other => other.to_string(),
    });
    norm.zeroize();
    r
}

/// The 2048 English words, for a screen that completes them as they are typed — from the same list the parser
/// checks against, so the two cannot disagree.
pub fn wordlist() -> Vec<&'static str> { bip39::Language::English.word_list().to_vec() }

/// The `spark1…` for an identity key: the same envelope `identity_of` reads, written the other way.
pub fn address_of(identity: &[u8], network: &str) -> Result<String, String> {
    let mut body = Vec::new();
    bytes_field(1, identity, &mut body);
    let hrp = bech32::Hrp::parse(match network { "mainnet" => "spark", "regtest" => "sparkrt", "testnet" => "sparkt", _ => "sparks" }).map_err(|e| e.to_string())?;
    bech32::encode::<bech32::Bech32m>(hrp, &body).map_err(|e| e.to_string())
}

// ---------- gRPC-Web, which is what their own browser client speaks ----------
fn call(base: &str, method: &str, body: Vec<u8>) -> Result<Vec<u8>, String> {
    let mut framed = vec![0u8];
    framed.extend_from_slice(&(body.len() as u32).to_be_bytes());
    framed.extend_from_slice(&body);
    let url = format!("{}/spark.SparkService/{method}", base.trim_end_matches('/'));
    static H: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    let http = H.get_or_init(|| reqwest::blocking::Client::builder().user_agent("anytwokeys/0.1").timeout(std::time::Duration::from_secs(12)).build().expect("an HTTP client"));
    let res = http.post(&url)
        .header("content-type", "application/grpc-web+proto")
        .header("x-grpc-web", "1")
        .header("accept", "application/grpc-web+proto")
        .body(framed).send().map_err(|e| format!("{base}: {e}"))?;
    if !res.status().is_success() { return Err(format!("{base} answered {}", res.status())); }
    let raw = res.bytes().map_err(|e| e.to_string())?.to_vec();
    // frames: [flag][4-byte length][payload]; flag 0x80 marks the trailers, which carry grpc-status
    let (mut i, mut msg, mut status) = (0usize, Vec::new(), None::<String>);
    while i + 5 <= raw.len() {
        let flag = raw[i];
        let len = u32::from_be_bytes([raw[i + 1], raw[i + 2], raw[i + 3], raw[i + 4]]) as usize;
        let (start, end) = (i + 5, i + 5 + len);
        if end > raw.len() { break; }
        if flag & 0x80 != 0 {
            let t = String::from_utf8_lossy(&raw[start..end]).to_string();
            for line in t.split("\r\n") { if let Some(v) = line.strip_prefix("grpc-status:") { status = Some(v.trim().to_string()); } }
            if let Some(m) = t.split("grpc-message:").nth(1) { if status.as_deref() != Some("0") { return Err(format!("Spark refused: {}", m.split("\r\n").next().unwrap_or("").trim())); } }
        } else { msg = raw[start..end].to_vec(); }
        i = end;
    }
    match status.as_deref() { None | Some("0") => Ok(msg), Some(s) => Err(format!("Spark answered with status {s}")) }
}

fn net_id(network: &str) -> u64 { match network { "mainnet" => 1, "regtest" => 2, "testnet" => 3, _ => 4 } }

/// What Spark says this wallet holds, in sats. Its word, not the chain's.
pub fn balance(identity: &[u8], network: &str) -> Result<u64, String> {
    let mut req = Vec::new();
    bytes_field(1, identity, &mut req);
    int_field(2, net_id(network), &mut req);
    let out = try_operators("query_balance", req)?;
    let mut r = Reader::new(&out);
    while !r.done() { if let Some((1, _, v)) = r.field() { return Ok(v); } }
    Ok(0)
}

#[derive(serde::Serialize, Clone, Debug, Default)]
pub struct Transfer {
    pub id: String,
    pub sats: u64,
    /// "in" when this wallet received it, "out" when it sent it
    pub direction: String,
    pub status: u64,
    pub kind: u64,
    /// unix seconds, from the transfer's created time
    pub at: i64,
}

/// Every transfer this wallet was part of, newest page first. An empty page means either an empty wallet or a
/// wallet withholding its history — see the note at the top of this file; the balance is what tells them apart.
pub fn transfers(identity: &[u8], network: &str, limit: u64, offset: u64) -> Result<(Vec<Transfer>, i64), String> {
    let mut req = Vec::new();
    bytes_field(60, identity, &mut req);   // sender_or_receiver_identity_public_key
    int_field(4, net_id(network), &mut req);
    int_field(40, limit, &mut req);
    int_field(50, offset, &mut req);
    let out = try_operators("query_all_transfers", req)?;
    let (mut list, mut next) = (Vec::new(), 0i64);
    let mut r = Reader::new(&out);
    while !r.done() {
        match r.field() {
            Some((1, v, _)) => list.push(one_transfer(&v, identity)),
            Some((2, _, v)) => next = v as i64,
            Some(_) => continue,
            None => break,
        }
    }
    Ok((list, next))
}

fn one_transfer(b: &[u8], me: &[u8]) -> Transfer {
    let mut t = Transfer::default();
    let mut r = Reader::new(b);
    while !r.done() {
        match r.field() {
            Some((1, v, _)) => t.id = String::from_utf8_lossy(&v).to_string(),
            Some((2, v, _)) => t.direction = if v == me { "out".into() } else { t.direction.clone() },
            Some((3, v, _)) => { if v == me && t.direction.is_empty() { t.direction = "in".into(); } }
            Some((4, _, v)) => t.status = v,
            Some((5, _, v)) => t.sats = v,
            Some((8, v, _)) => t.at = timestamp(&v),
            Some((10, _, v)) => t.kind = v,
            Some(_) => continue,
            None => break,
        }
    }
    t
}

/// google.protobuf.Timestamp: seconds in field 1
fn timestamp(b: &[u8]) -> i64 {
    let mut r = Reader::new(b);
    while !r.done() { if let Some((1, _, v)) = r.field() { return v as i64; } }
    0
}

/// The operators answer the same questions, and they do not answer equally well: on 2026-09-24 Lightspark's own
/// operator timed out on `query_all_transfers` while Breez's and Flashnet's replied in ten to fourteen seconds. So
/// each is given a short turn and a slow one is passed over rather than waited on.
fn try_operators(method: &str, req: Vec<u8>) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for base in OPERATORS {
        match call(base, method, req.clone()) { Ok(v) => return Ok(v), Err(e) => last = e }
    }
    Err(if last.is_empty() { "no Spark operator answered".into() } else { last })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Real Spark addresses from their own SDK tests, with the networks their tests expect. A wrong offset into the
    /// payload would still yield 33 bytes, so the key is also required to be a point on the curve.
    #[test]
    fn a_spark_address_gives_up_its_key_and_says_which_network() {
        for (addr, want) in [
            ("spark1pgss9qg3vdslzmt2name9v550skuvlu6lj5xt9sly90k7p0gxughlqv023jqmc", "mainnet"),
            ("sparkrt1pgssx5us3wkqjza8g80xz3a9gznx25msq6g3ty8exfym9q3ahcv86vsnxxdy83", "regtest"),
        ] {
            let (id, net) = identity_of(addr).unwrap();
            assert_eq!(net, want, "{addr}");
            assert_eq!(id.len(), 33, "an identity key is a compressed public key");
            bitcoin::secp256k1::PublicKey::from_slice(&id).unwrap_or_else(|e| panic!("{addr}: not a public key: {e}"));
        }
        // An INVOICE is the same envelope with more in it (`spark_invoice_fields` is field 2), so pasting one works
        // just as well — which the restore screen now tells people, and this is why that is true.
        let key = hex::decode("02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5").unwrap();
        let mut body = Vec::new();
        bytes_field(1, &key, &mut body);
        bytes_field(2, &[0x08, 0x01, 0x12, 0x04, 1, 2, 3, 4], &mut body);   // some invoice fields riding along
        let hrp = bech32::Hrp::parse("spark").unwrap();
        let invoice = bech32::encode::<bech32::Bech32m>(hrp, &body).unwrap();
        let (got, net) = identity_of(&invoice).expect("an invoice carries the same key");
        assert_eq!(got, key);
        assert_eq!(net, "mainnet");
        // what a camera sees: the same thing upper-cased, behind a scheme, or inside a BIP 21 line
        let (k2, _) = identity_of(&invoice.to_uppercase()).expect("QR codes render bech32 in upper case");
        assert_eq!(k2, key);
        assert_eq!(identity_of(&format!("spark:{invoice}")).unwrap().0, key);
        assert_eq!(identity_of(&format!("bitcoin:bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4?amount=0.001&spark={invoice}")).unwrap().0, key);
        assert!(identity_of("spark1notanaddress").is_err());
        assert!(identity_of("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").unwrap_err().contains("Spark address"));
    }
    /// Spark publishes the scheme but no vector, so the check is two implementations that never met: this one, and
    /// embit in Python, on the standard test words. Account 1 is what mainnet uses by default.
    #[test]
    fn the_identity_key_from_seed_words_matches_an_independent_implementation() {
        let words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        assert_eq!(hex::encode(identity_from_words(words, 0).unwrap()), "02698b27ac308b275671b3ca25436346469d04a5bba578ae39feba1d65897a6abc");
        assert_eq!(hex::encode(identity_from_words(words, 1).unwrap()), "0281363910b0dc0015a4a25e758da30f0e28388ea5252c0e3713936f2d4ef7d3d5");
        // and the address written for it reads back as the same key, on the right network
        let id = identity_from_words(words, 1).unwrap();
        let addr = address_of(&id, "mainnet").unwrap();
        assert!(addr.starts_with("spark1"), "{addr}");
        assert_eq!(identity_of(&addr).unwrap(), (id, "mainnet".to_string()));
        assert!(identity_from_words("abandon abandon not words", 1).is_err());
    }
    /// A key nobody uses: the answer must be a clean zero, which proves the wire format rather than the wallet.
    #[test] #[ignore]
    fn probe_each_operator() {
        let id = hex::decode("02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5").unwrap();
        for base in OPERATORS {
            let mut req = Vec::new();
            bytes_field(1, &id, &mut req); int_field(2, 1, &mut req);
            println!("{base} query_balance -> {:?}", call(base, "query_balance", req).map(|v| v.len()));
            let mut t = Vec::new();
            bytes_field(60, &id, &mut t); int_field(4, 1, &mut t); int_field(40, 5, &mut t); int_field(50, 0, &mut t);
            let started = std::time::Instant::now();
            println!("{base} query_all_transfers -> {:?} after {:?}", call(base, "query_all_transfers", t).map(|v| v.len()), started.elapsed());
        }
    }
    #[test] #[ignore]
    fn a_live_operator_answers_in_the_shape_we_expect() {
        let id = hex::decode("02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5").unwrap();
        let b = balance(&id, "mainnet").expect("an operator should answer");
        println!("balance for a stranger's key: {b}");
        assert_eq!(b, 0);
        let (list, next) = transfers(&id, "mainnet", 5, 0).expect("an operator should answer");
        println!("transfers: {} · next offset {next}", list.len());
        assert!(list.is_empty());
    }
}
