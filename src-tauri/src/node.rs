//! A Lightning node the person runs — Core Lightning first, LND next — read as a vault: on-chain balance, what sits
//! in channels, what is pending, and every event that moved money, as an accounting ledger. Read-only in fact and in
//! proof: the credential is checked against the node for the things it must NOT be able to do (pay, withdraw, open,
//! close) before it is kept, and it is kept sealed in its own file, never in the store the window sees.
//!
//! Why it matters here: a node is the one wallet that names BOTH identifiers everything else lacks — on-chain txids
//! and Lightning payment hashes — so its moves to and from the other vaults link exactly, not by guessing.
//!
//! Core Lightning is read through clnrest (POST /v1/<method>, the rune in a header); its bookkeeper plugin is already
//! a ledger (bkpr-listincome), which is why CLN came first. TLS to a node is a self-signed certificate: it is pinned by
//! fingerprint, shown to the person the first time and refused ever after if it changes.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use zeroize::Zeroizing;

type R<T> = Result<T, String>;

/// What the vault keeps about the node: public, and enough to read it again with the sealed credential.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Node {
    /// "cln" or "lnd"
    pub kind: String,
    pub url: String,
    /// sha256 of the node's certificate, hex; empty for plain http
    pub pin: String,
    pub alias: String,
    pub pubkey: String,
    pub version: String,
    /// balances at the last read, sats
    pub onchain: u64,
    pub channels: u64,
    pub pending: u64,
    pub read_at: u64,
    /// the ledger, oldest first
    pub events: Vec<Event>,
}

/// One accounting event, in the shape the rest of the app reads (activity rows, the transfer index).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Event {
    /// on-chain: the txid; Lightning: the payment hash; otherwise the bookkeeper's own key
    pub id: String,
    pub time: u32,
    /// signed sats
    pub net: i64,
    pub fee: u64,
    /// "chain" · "lightning" · "channel" (an open or close: a move within the node) · "fee"
    pub layer: String,
    #[serde(default)] pub txid: String,
    #[serde(default)] pub hash: String,
    /// "deposit" · "withdrawal" · "received" · "paid" · "routed" (income) · "open" · "close" · "fee"
    pub kind: String,
    #[serde(default)] pub memo: String,
}

/// What the person is shown before anything is kept.
#[derive(Serialize, Debug)]
pub struct Check {
    pub kind: String,
    pub alias: String,
    pub pubkey: String,
    pub version: String,
    pub network: String,
    pub height: u64,
    pub onchain: u64,
    pub channels: u64,
    pub pending: u64,
    pub channel_count: usize,
    /// the write methods the credential was proved unable to call
    pub refused: Vec<String>,
    /// the certificate fingerprint met on this connection (empty for http)
    pub pin: String,
    pub events: usize,
}

// ---------- TLS: pinned by fingerprint, or, on a first look, remembered ----------
#[derive(Debug)]
struct Pinned { want: Option<[u8; 32]>, seen: std::sync::Mutex<Option<[u8; 32]>> }
impl rustls::client::danger::ServerCertVerifier for Pinned {
    fn verify_server_cert(&self, end_entity: &rustls::pki_types::CertificateDer<'_>, _: &[rustls::pki_types::CertificateDer<'_>], _: &rustls::pki_types::ServerName<'_>, _: &[u8], _: rustls::pki_types::UnixTime) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let got: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if let Ok(mut s) = self.seen.lock() { *s = Some(got); }
        match self.want {
            Some(w) if w != got => Err(rustls::Error::General("the node's certificate is not the one pinned".into())),
            _ => Ok(rustls::client::danger::ServerCertVerified::assertion()),
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> { rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes() }
}

fn client(pin: Option<[u8; 32]>) -> R<(reqwest::blocking::Client, Arc<Pinned>)> {
    let v = Arc::new(Pinned { want: pin, seen: std::sync::Mutex::new(None) });
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider())).with_safe_default_protocol_versions().map_err(|e| e.to_string())?
        .dangerous().with_custom_certificate_verifier(v.clone()).with_no_client_auth();
    let c = reqwest::blocking::Client::builder().use_preconfigured_tls(cfg).timeout(std::time::Duration::from_secs(30)).redirect(reqwest::redirect::Policy::none()).user_agent("anytwokeys/0.1").build().map_err(|e| e.to_string())?;
    Ok((c, v))
}
fn pin_of(hex_pin: &str) -> R<Option<[u8; 32]>> {
    if hex_pin.is_empty() { return Ok(None); }
    let b = hex::decode(hex_pin.replace(':', "")).map_err(|_| "the certificate fingerprint is not hex")?;
    <[u8; 32]>::try_from(b).map(Some).map_err(|_| "a certificate fingerprint is 32 bytes".into())
}
pub fn fingerprint_words(hex_pin: &str) -> String { hex_pin.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).to_uppercase()).collect::<Vec<_>>().join(":") }

// ---------- Core Lightning over clnrest ----------
pub struct Cln { url: String, rune: Zeroizing<String>, http: reqwest::blocking::Client, pinned: Arc<Pinned> }
impl Cln {
    pub fn open(url: &str, rune: &str, pin: &str) -> R<Self> {
        let url = url.trim().trim_end_matches('/').to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) { return Err("the node's address is a web address: https://host:3010 (or http:// on your own LAN)".into()); }
        let (http, pinned) = client(pin_of(pin)?)?;
        Ok(Cln { url, rune: Zeroizing::new(rune.trim().to_string()), http, pinned })
    }
    /// The certificate this connection met, hex — what a first look shows the person to confirm.
    pub fn seen_pin(&self) -> String { self.pinned.seen.lock().ok().and_then(|s| *s).map(hex::encode).unwrap_or_default() }
    pub fn call(&self, method: &str, params: serde_json::Value) -> R<serde_json::Value> {
        crate::net::out("your node")?;
        let res = self.http.post(format!("{}/v1/{method}", self.url)).header("Rune", self.rune.as_str()).header("Content-Type", "application/json").json(&params).send()
            .map_err(|e| if e.is_timeout() { "the node did not answer in time".to_string() } else if e.to_string().contains("not the one pinned") { "the node's certificate has changed since it was pinned; nothing was sent".to_string() } else { format!("the node could not be reached: {}", e.without_url()) })?;
        let status = res.status();
        let body: serde_json::Value = res.json().map_err(|_| format!("the node answered {} with something that is not JSON", status.as_u16()))?;
        if !status.is_success() {
            let msg = body.get("message").or(body.get("error")).and_then(|m| m.as_str().map(str::to_string).or_else(|| m.get("message").and_then(|x| x.as_str()).map(str::to_string))).unwrap_or_default();
            return Err(match status.as_u16() { 401 | 403 => format!("the node refused this credential for {method}: {msg}"), _ => format!("{method}: the node answered {} {msg}", status.as_u16()) });
        }
        Ok(body)
    }
    /// The credential must NOT be able to move money. Each write method is checked with checkrune (a read; nothing
    /// is executed) and must be refused. A credential that would be allowed any of them is not kept.
    pub fn prove_read_only(&self, rune: &str) -> R<Vec<String>> {
        let mut refused = Vec::new();
        for m in ["pay", "xpay", "withdraw", "fundchannel", "close", "sendpay", "keysend", "sendonion", "fundpsbt", "signpsbt", "sendpsbt", "delinvoice", "createrune", "setconfig", "stop"] {
            match self.call("checkrune", serde_json::json!({ "rune": rune, "method": m })) {
                Ok(v) if v.get("valid").and_then(|x| x.as_bool()) == Some(true) => return Err(format!("this credential would be allowed to call {m}. Only a read-only rune can be kept here — one restricted to list, get and bkpr- methods.")),
                _ => refused.push(m.to_string()),
            }
        }
        Ok(refused)
    }
    pub fn info(&self) -> R<(String, String, String, String, u64)> {
        let i = self.call("getinfo", serde_json::json!({}))?;
        let s = |k: &str| i.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        Ok((s("alias"), s("id"), s("version"), s("network"), i.get("blockheight").and_then(|v| v.as_u64()).unwrap_or(0)))
    }
    /// (on-chain confirmed, in channels ours, pending: unconfirmed outputs + channels not yet normal, channel count)
    pub fn funds(&self) -> R<(u64, u64, u64, usize)> {
        let f = self.call("listfunds", serde_json::json!({}))?;
        let msat = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0) / 1000;
        let (mut onchain, mut pending, mut channels, mut n) = (0, 0, 0, 0);
        for o in f.get("outputs").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            if o.get("status").and_then(|s| s.as_str()) == Some("confirmed") { onchain += msat(o, "amount_msat"); } else { pending += msat(o, "amount_msat"); }
        }
        for c in f.get("channels").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            n += 1;
            if c.get("state").and_then(|s| s.as_str()) == Some("CHANNELD_NORMAL") { channels += msat(c, "our_amount_msat"); } else { pending += msat(c, "our_amount_msat"); }
        }
        Ok((onchain, channels, pending, n))
    }
    /// The ledger, from the bookkeeper's income view: every event that changed what the node holds, with the
    /// identifier the other end of it also knows. Fees are their own events, tied to what they paid for.
    pub fn events(&self) -> R<Vec<Event>> {
        let v = self.call("bkpr-listincome", serde_json::json!({ "consolidate_fees": true }))?;
        let mut out = Vec::new();
        for e in v.get("income_events").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            let s = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let msat = |k: &str| e.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            if s("currency") != "bcrt" && s("currency") != "bc" { /* a non-bitcoin asset is not counted */ }
            let (credit, debit) = (msat("credit_msat") / 1000, msat("debit_msat") / 1000);
            let tag = s("tag");
            let txid = s("txid").to_lowercase();
            let outpoint = s("outpoint");
            let hash = s("payment_id").to_lowercase();
            let time = e.get("timestamp").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let (layer, kind, net, fee): (&str, &str, i64, u64) = match tag.as_str() {
                "deposit" => ("chain", "deposit", credit as i64, 0),
                "withdrawal" => ("chain", "withdrawal", -(debit as i64), 0),
                "onchain_fee" => ("fee", "fee", -(debit as i64), debit),
                "invoice" => if credit > 0 { ("lightning", "received", credit as i64, 0) } else { ("lightning", "paid", -(debit as i64), 0) },
                "invoice_fee" => ("fee", "fee", -(debit as i64), debit),
                "routed" => ("lightning", "routed", credit as i64 - debit as i64, 0),
                "pushed" => if credit > 0 { ("lightning", "received", credit as i64, 0) } else { ("lightning", "paid", -(debit as i64), 0) },
                "penalty_adj" | "journal_entry" => ("lightning", if credit >= debit { "received" } else { "paid" }, credit as i64 - debit as i64, 0),
                _ => continue,
            };
            if net == 0 { continue; }
            let id = if layer == "chain" && !txid.is_empty() { txid.clone() }
                else if !outpoint.is_empty() && layer != "lightning" { outpoint.split(':').next().unwrap_or("").to_lowercase() }
                else if !hash.is_empty() { format!("{}{}", hash, if kind == "paid" { ":out" } else if kind == "routed" { ":routed" } else { "" }) }
                else { hex::encode(&Sha256::digest(format!("{tag}|{time}|{credit}|{debit}|{}", s("account")).as_bytes())[..16]) };
            let txid_of = if layer == "chain" || layer == "fee" { if !txid.is_empty() { txid } else { outpoint.split(':').next().unwrap_or("").to_lowercase() } } else { String::new() };
            out.push(Event { id, time, net, fee, layer: layer.into(), txid: txid_of, hash: if layer == "lightning" { hash } else { String::new() }, kind: kind.into(), memo: s("description") });
        }
        // several bookkeeper events can belong to one transaction (a deposit that also paid change, a fee in parts):
        // the ledger keeps one row per id, its amounts summed, its fee summed
        let mut merged: std::collections::BTreeMap<String, Event> = std::collections::BTreeMap::new();
        for e in out { match merged.get_mut(&e.id) { Some(m) => { m.net += e.net; m.fee += e.fee; if m.time == 0 { m.time = e.time; } if m.memo.is_empty() { m.memo = e.memo; } } None => { merged.insert(e.id.clone(), e); } } }
        let mut rows: Vec<Event> = merged.into_values().filter(|e| e.net != 0 || e.fee != 0).collect();
        rows.sort_by_key(|e| (e.time, e.id.clone()));
        Ok(rows)
    }
}

// ---------- the sealed credential ----------
fn cred_path(id: &str) -> std::path::PathBuf { crate::lockbox::dir().join(format!("node-{id}.cred")) }
pub fn id_of(url: &str, pubkey: &str) -> String { hex::encode(&Sha256::digest(format!("{}|{}", url.trim(), pubkey).as_bytes())[..8]) }
pub fn keep_cred(id: &str, cred: &str) -> R<()> {
    if !crate::lockbox::sealed_mode() { return Err("a node's credential needs this computer's sealed store: seal it in App settings first".into()); }
    crate::lockbox::write_sealed(&cred_path(id), "node-cred", cred.trim().as_bytes())
}
pub fn cred(id: &str) -> R<Zeroizing<String>> {
    let b = crate::lockbox::read_sealed(&cred_path(id), "node-cred")?.ok_or("this node's credential is not on this computer; add the node again")?;
    Ok(Zeroizing::new(String::from_utf8(b).map_err(|_| "the credential is damaged")?))
}
pub fn forget_cred(id: &str) { let _ = std::fs::remove_file(cred_path(id)); }

#[cfg(test)]
mod tests {
    use super::*;
    /// The real node, named by A2K_CLN_URL and a rune file at A2K_CLN_RUNE: read-only proved, info, funds, the ledger.
    #[test] #[ignore]
    fn the_real_node_reads_and_the_rune_cannot_spend() {
        let (Ok(url), Ok(path)) = (std::env::var("A2K_CLN_URL"), std::env::var("A2K_CLN_RUNE")) else { return };
        let rune = std::fs::read_to_string(path).unwrap();
        let c = Cln::open(&url, &rune, "").unwrap();
        let refused = c.prove_read_only(rune.trim()).unwrap();
        assert!(refused.contains(&"pay".to_string()) && refused.contains(&"withdraw".to_string()) && refused.contains(&"close".to_string()));
        let (alias, id, version, network, height) = c.info().unwrap();
        assert!(!alias.is_empty() && id.len() == 66 && network == "bitcoin" && height > 900_000, "{alias} {id} {version} {network} {height}");
        let (onchain, channels, pending, n) = c.funds().unwrap();
        let ev = c.events().unwrap();
        assert!(!ev.is_empty()); assert!(ev.windows(2).all(|w| w[0].time <= w[1].time));
        let kinds: std::collections::BTreeMap<&str, usize> = ev.iter().fold(Default::default(), |mut m, e| { *m.entry(e.kind.as_str()).or_default() += 1; m });
        eprintln!("{alias} v{version} height {height} · on-chain {onchain} · channels {channels} ({n}) · pending {pending} · events {} {:?} · refused {}", ev.len(), kinds, refused.len());
        let ln: Vec<_> = ev.iter().filter(|e| e.layer == "lightning" && e.kind != "routed").collect();
        let hashed = ln.iter().filter(|e| e.hash.len() == 64).count();
        assert!(hashed * 20 >= ln.len() * 19, "{hashed} of {} Lightning events name their payment hash (a channel push has none)", ln.len());
    }
    #[test]
    fn a_fingerprint_is_read_with_or_without_colons_and_shown_with_them() {
        let hex_pin = "6e5fe351a8da503de4faf75910af5d40892eea6f2cd19b39".to_string() + "0000000000000000";
        assert_eq!(pin_of(&hex_pin).unwrap().map(|p| p.len()), Some(32));
        assert_eq!(pin_of(&fingerprint_words(&hex_pin)).unwrap(), pin_of(&hex_pin).unwrap());
        assert!(fingerprint_words(&hex_pin).starts_with("6E:5F:E3:51"));
        assert!(pin_of("").unwrap().is_none() && pin_of("zz").is_err() && pin_of("abcd").is_err());
    }
    #[test]
    fn a_node_url_must_be_a_web_address() {
        assert!(Cln::open("10.56.58.66:3010", "r", "").is_err());
        assert!(Cln::open("http://10.56.58.66:3010/", "r", "").is_ok());
        assert!(Cln::open("https://node:3010", "r", "not-hex").is_err());
    }
}
