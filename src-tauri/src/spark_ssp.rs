//! The Lightning side of a Spark wallet. Spark's operators record a Lightning payment as a bare transfer: id, parties,
//! amount, state, time — no text. The description a payer or the wallet wrote ("Mining reward for block …") lives
//! with the invoice, at the service provider that bridged the payment into Spark (Lightspark's, which Wallet of
//! Satoshi uses). That provider answers only the wallet's OWNER: a session opened by the identity key — challenge,
//! sign its SHA-256, verify, bearer token — exactly the operators' own dance. The reader key a card holds for the
//! operators is a stranger to it. So this read takes the wallet's words for the moment of the read, derives the
//! identity, accepts it only if it is the identity the vault already records, signs once, reads the invoices for the
//! transfer ids it is given, and forgets the key. Nothing is written to the provider.
use bitcoin::base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;
use crate::{spark_auth, wallet};

type R<T> = Result<T, String>;
const SSP: &str = "https://api.lightspark.com/graphql/spark/2025-03-19";

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct Memo { pub id: String, pub text: String, pub kind: String }

fn gql(token: Option<&str>, query: &str, variables: Value) -> R<Value> {
    crate::net::out("Spark")?;
    static H: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    let http = H.get_or_init(|| reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none()).user_agent("anytwokeys/0.1").build().expect("an HTTP client"));
    let mut req = http.post(SSP).timeout(std::time::Duration::from_secs(30)).json(&json!({ "query": query, "variables": variables }));
    if let Some(t) = token { req = req.bearer_auth(t); }
    let res = req.send().map_err(|_| "the Lightning service provider could not be reached")?;
    let status = res.status().as_u16();
    let body: Value = res.json().map_err(|_| format!("the Lightning service provider answered HTTP {status} without JSON"))?;
    if let Some(errs) = body.get("errors").and_then(|e| e.as_array()).filter(|a| !a.is_empty()) {
        let msg = errs[0].get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        return Err(format!("the Lightning service provider refused: {msg}"));
    }
    body.get("data").cloned().ok_or_else(|| format!("the Lightning service provider answered HTTP {status} without data"))
}

/// The words must make the identity the vault records, on one of the schemes the app knows; nothing else is tried.
fn owner(words: &str, sp: &wallet::Spark) -> R<spark_auth::Identity> {
    for scheme in ["direct", "blitz"] {
        for account in 0..8u32 {
            if let Ok(k) = spark_auth::derive_identity(words, account, scheme) {
                if hex::encode(k.public().serialize()).eq_ignore_ascii_case(&sp.identity) { return Ok(k); }
            }
        }
    }
    Err("these words do not make this wallet's identity, so nothing was signed".into())
}

fn session(key: &spark_auth::Identity) -> R<Zeroizing<String>> {
    let public = hex::encode(key.public().serialize());
    let got = gql(None, "mutation GetChallenge($public_key: PublicKey!) { get_challenge(input: { public_key: $public_key }) { protected_challenge } }", json!({ "public_key": public }))?;
    let protected = got.pointer("/get_challenge/protected_challenge").and_then(|v| v.as_str()).ok_or("the Lightning service provider sent no challenge")?.to_string();
    let bytes = B64.decode(&protected).map_err(|_| "the Lightning service provider sent an unreadable challenge")?;
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let signature = B64.encode(key.sign(&digest)?);
    let got = gql(None, "mutation VerifyChallenge($protected_challenge: String!, $signature: String!, $identity_public_key: PublicKey!) { verify_challenge(input: { protected_challenge: $protected_challenge, signature: $signature, identity_public_key: $identity_public_key }) { session_token valid_until } }",
        json!({ "protected_challenge": protected, "signature": signature, "identity_public_key": public }))?;
    let token = got.pointer("/verify_challenge/session_token").and_then(|v| v.as_str()).filter(|t| !t.is_empty()).ok_or("the Lightning service provider opened no session")?;
    Ok(Zeroizing::new(token.to_string()))
}

const TRANSFERS: &str = "query Transfers($ids: [UUID!]!) { transfers(transfer_spark_ids: $ids) { spark_id user_request { __typename ... on LightningReceiveRequest { invoice { encoded_invoice memo } } ... on LightningSendRequest { encoded_invoice } } } }";

/// What the provider holds for each transfer id: the invoice's memo, or the description inside the invoice itself.
fn memos_for(token: &str, ids: &[String]) -> R<Vec<Memo>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(50) {
        let got = gql(Some(token), TRANSFERS, json!({ "ids": chunk }))?;
        for t in got.get("transfers").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            let (Some(id), Some(ur)) = (t.get("spark_id").and_then(|v| v.as_str()), t.get("user_request")) else { continue };
            let kind = ur.get("__typename").and_then(|v| v.as_str()).unwrap_or("");
            let (text, kind) = match kind {
                "LightningReceiveRequest" => {
                    let memo = ur.pointer("/invoice/memo").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
                    let from_invoice = || ur.pointer("/invoice/encoded_invoice").and_then(|v| v.as_str()).and_then(bolt11_description);
                    (memo.or_else(from_invoice), "Lightning receive")
                }
                "LightningSendRequest" => (ur.get("encoded_invoice").and_then(|v| v.as_str()).and_then(bolt11_description), "Lightning send"),
                _ => (None, kind),
            };
            if let Some(text) = text { out.push(Memo { id: id.to_string(), text, kind: kind.to_string() }); }
        }
    }
    Ok(out)
}

pub fn read(words: Zeroizing<String>, sp: &wallet::Spark, ids: &[String]) -> R<Vec<Memo>> {
    let key = owner(&words, sp)?;
    drop(words);
    let token = session(&key)?;
    drop(key);
    memos_for(&token, ids)
}

/// The `d` field of a BOLT 11 invoice, which is where a payer's or a wallet's words about the payment are written.
/// Just enough of the format: bech32 without the 90-character limit, 35 bits of timestamp, then tagged fields of
/// 5-bit type, 10-bit length, data; the signature is the last 104 groups and is not checked here — the provider's
/// own session already vouches for whose invoice this is.
pub fn bolt11_description(invoice: &str) -> Option<String> {
    let s = invoice.trim();
    let lower = s.to_ascii_lowercase();
    if !lower.starts_with("ln") { return None; }
    let sep = lower.rfind('1')?;
    let data = &lower[sep + 1..];
    if data.len() < 6 + 104 + 7 { return None; }
    const CS: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let mut groups = Vec::with_capacity(data.len());
    for c in data.bytes() { groups.push(CS.iter().position(|x| *x == c)? as u8); }
    let body = &groups[..groups.len() - 6 - 104];   // drop the checksum and the signature
    let mut i = 7;   // the timestamp
    while i + 3 <= body.len() {
        let t = body[i]; let len = ((body[i + 1] as usize) << 5) | body[i + 2] as usize; i += 3;
        if i + len > body.len() { return None; }
        if t == 13 {
            let field = &body[i..i + len];
            let mut bytes = Vec::new(); let (mut acc, mut bits) = (0u32, 0u32);
            for g in field { acc = (acc << 5) | *g as u32; bits += 5; if bits >= 8 { bits -= 8; bytes.push((acc >> bits) as u8); acc &= (1 << bits) - 1; } }
            let text = String::from_utf8(bytes).ok()?.trim().to_string();
            return if text.is_empty() { None } else { Some(text) };
        }
        i += len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The BOLT 11 specification's own example: "Please consider supporting this project".
    #[test]
    fn reads_the_description_of_the_spec_example() {
        let inv = "lnbc1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdpl2pkx2ctnv5sxxmmwwd5kgetjypeh2ursdae8g6twvus8g6rfwvs8qun0dfjkxaq9qrsgq357wnc5r2ueh7ck6q93dj32dlqnls087fxdwk8qakdyafkq3yap9us6v52vjjsrvywa6rt52cm9r9zqt8r2t7mlcwspyetp5h2tztugp9lfyql";
        assert_eq!(bolt11_description(inv).as_deref(), Some("Please consider supporting this project"));
    }
    #[test]
    fn an_invoice_with_only_a_description_hash_has_no_words() {
        let inv = "lnbc20m1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqhp58yjmdan79s6qqdhdzgynm4zwqd5d7xmw5fk98klysy043l2ahrqs9qrsgq7ea976txfraylvgzuxs8kgcw23ezlrszfnh8r6qtfpg6ldk52q6f46gpk4vznjk5q0zh2m5kudg9tctdxgl2p2u5qpenqff8d3q5n2nkcgpm4ckjg";
        assert_eq!(bolt11_description(inv), None);
    }
    #[test]
    fn rubbish_is_not_an_invoice() {
        assert_eq!(bolt11_description("hello"), None);
        assert_eq!(bolt11_description("lnbc1qqq"), None);
    }
}
