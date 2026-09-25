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

// ---------- LND over REST ----------
/// What a credential for LND can arrive as: a macaroon in hex or base64, or an `lndconnect://host:port?cert=…&macaroon=…`
/// link, which every node UI makes. The link's cert becomes the pin, its macaroon the credential.
pub struct LndCred { pub macaroon_hex: String, pub url: Option<String>, pub pin: Option<String> }
pub fn lnd_cred(text: &str) -> R<LndCred> {
    use bitcoin::base64::{engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD, URL_SAFE}, Engine};
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("lndconnect://") {
        let (hostport, query) = rest.split_once('?').ok_or("an lndconnect link carries ?cert=…&macaroon=…")?;
        let mut cert = None; let mut mac = None;
        for kv in query.split('&') { if let Some((k, v)) = kv.split_once('=') { match k { "cert" => cert = Some(v.to_string()), "macaroon" => mac = Some(v.to_string()), _ => {} } } }
        let mac = mac.ok_or("the lndconnect link has no macaroon")?;
        let mac_b = URL_SAFE_NO_PAD.decode(&mac).or_else(|_| URL_SAFE.decode(&mac)).or_else(|_| STANDARD.decode(&mac)).map_err(|_| "the link's macaroon is not base64")?;
        let pin = match cert { Some(c) => { let der = URL_SAFE_NO_PAD.decode(&c).or_else(|_| URL_SAFE.decode(&c)).or_else(|_| STANDARD.decode(&c)).map_err(|_| "the link's certificate is not base64")?; Some(hex::encode(Sha256::digest(&der))) } None => None };
        return Ok(LndCred { macaroon_hex: hex::encode(mac_b), url: Some(format!("https://{hostport}")), pin });
    }
    if !t.is_empty() && t.len() % 2 == 0 && t.chars().all(|c| c.is_ascii_hexdigit()) { return Ok(LndCred { macaroon_hex: t.to_lowercase(), url: None, pin: None }); }
    if let Ok(b) = STANDARD.decode(t).or_else(|_| URL_SAFE_NO_PAD.decode(t)) { return Ok(LndCred { macaroon_hex: hex::encode(b), url: None, pin: None }); }
    Err("the macaroon is hex (what lncli bakemacaroon prints), base64, or an lndconnect:// link".into())
}

pub struct Lnd { url: String, macaroon: Zeroizing<String>, http: reqwest::blocking::Client, pinned: Arc<Pinned> }
impl Lnd {
    pub fn open(url: &str, macaroon_hex: &str, pin: &str) -> R<Self> {
        let url = url.trim().trim_end_matches('/').to_string();
        if !url.starts_with("https://") { return Err("LND's REST address is https://host:8080".into()); }
        let (http, pinned) = client(pin_of(pin)?)?;
        Ok(Lnd { url, macaroon: Zeroizing::new(macaroon_hex.trim().to_lowercase()), http, pinned })
    }
    pub fn seen_pin(&self) -> String { self.pinned.seen.lock().ok().and_then(|s| *s).map(hex::encode).unwrap_or_default() }
    fn send(&self, req: reqwest::blocking::RequestBuilder, what: &str) -> R<serde_json::Value> {
        crate::net::out("your node")?;
        let res = req.header("Grpc-Metadata-macaroon", self.macaroon.as_str()).send()
            .map_err(|e| if e.is_timeout() { "the node did not answer in time".to_string() } else if e.to_string().contains("not the one pinned") { "the node's certificate has changed since it was pinned; nothing was sent".to_string() } else { format!("the node could not be reached: {}", e.without_url()) })?;
        let status = res.status();
        let body: serde_json::Value = res.json().map_err(|_| format!("the node answered {} with something that is not JSON", status.as_u16()))?;
        if !status.is_success() {
            let msg = body.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
            return Err(match status.as_u16() { 401 | 403 => format!("the node refused this credential for {what}: {msg}"), _ => format!("{what}: the node answered {} {msg}", status.as_u16()) });
        }
        Ok(body)
    }
    pub fn get(&self, path: &str) -> R<serde_json::Value> { self.send(self.http.get(format!("{}{path}", self.url)), path) }
    pub fn post(&self, path: &str, body: serde_json::Value) -> R<serde_json::Value> { self.send(self.http.post(format!("{}{path}", self.url)).json(&body), path) }
    /// The macaroon must NOT carry a write permission. Each is checked with CheckMacaroonPermissions (a read) and
    /// must come back invalid. A macaroon without macaroon:read cannot be checked, and so cannot be kept.
    /// LND answers a permission the macaroon lacks with a "permission denied" ERROR, and one it has with valid: true.
    /// So the proof runs the other way round from CLN's: first a permission this macaroon must have (info:read) has
    /// to pass — otherwise the macaroon cannot use the check at all and nothing can be proved — then each write
    /// permission must be denied.
    pub fn prove_read_only(&self) -> R<Vec<String>> {
        use bitcoin::base64::{engine::general_purpose::STANDARD, Engine};
        let mac = STANDARD.encode(hex::decode(self.macaroon.as_str()).map_err(|_| "the macaroon is not hex")?);
        let ask = |entity: &str, action: &str| self.post("/v1/macaroon/checkpermissions", serde_json::json!({ "macaroon": mac, "permissions": [{ "entity": entity, "action": action }] }));
        let hint = "bake it with info:read onchain:read offchain:read invoices:read macaroon:read";
        match ask("info", "read") {
            Ok(v) if v.get("valid").and_then(|x| x.as_bool()) == Some(true) => {}
            Ok(_) => return Err(format!("the macaroon cannot read the node's info, so nothing can be read with it — {hint}")),
            Err(e) => return Err(format!("{e}. The macaroon needs macaroon:read so the app can prove what it cannot do — {hint}")),
        }
        let mut refused = Vec::new();
        for (entity, action) in [("offchain", "write"), ("onchain", "write"), ("invoices", "write"), ("address", "write"), ("peers", "write"), ("message", "write"), ("macaroon", "write"), ("macaroon", "generate"), ("signer", "generate"), ("uri", "write")] {
            match ask(entity, action) {
                Ok(v) if v.get("valid").and_then(|x| x.as_bool()) == Some(true) => return Err(format!("this macaroon carries {entity}:{action}. Only a read-only macaroon can be kept here — {hint}.")),
                Ok(_) => refused.push(format!("{entity}:{action}")),
                Err(e) if e.contains("permission denied") || e.contains("permission") => refused.push(format!("{entity}:{action}")),
                Err(e) => return Err(e),
            }
        }
        Ok(refused)
    }
    pub fn info(&self) -> R<(String, String, String, String, u64)> {
        let i = self.get("/v1/getinfo")?;
        let s = |k: &str| i.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let network = i.get("chains").and_then(|c| c.as_array()).and_then(|a| a.first()).and_then(|c| c.get("network")).and_then(|n| n.as_str()).unwrap_or("").to_string();
        Ok((s("alias"), s("identity_pubkey"), s("version"), network, i.get("block_height").and_then(|v| v.as_u64()).unwrap_or(0)))
    }
    pub fn funds(&self) -> R<(u64, u64, u64, usize)> {
        let n = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| x.as_u64())).unwrap_or(0);
        let b = self.get("/v1/balance/blockchain")?;
        let c = self.get("/v1/balance/channels")?;
        let local = c.get("local_balance").map(|l| n(l, "sat")).unwrap_or_else(|| n(&c, "balance"));
        let pending_local = c.get("pending_open_local_balance").map(|l| n(l, "sat")).unwrap_or_else(|| n(&c, "pending_open_balance"));
        let chans = self.get("/v1/channels")?;
        let count = chans.get("channels").and_then(|a| a.as_array()).map(|a| a.len()).unwrap_or(0);
        Ok((n(&b, "confirmed_balance"), local, n(&b, "unconfirmed_balance") + pending_local, count))
    }
    /// The ledger, assembled: on-chain transactions (channel opens and closes recognised by their outpoints and
    /// counted as moves within the node, fee only), settled invoices, succeeded payments, and routing fees earned.
    pub fn events(&self) -> R<Vec<Event>> {
        use bitcoin::base64::{engine::general_purpose::STANDARD, Engine};
        let n = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| x.as_i64())).unwrap_or(0);
        let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let arr = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
        // which transactions are channels being opened or closed
        let mut opens = std::collections::HashSet::new(); let mut closes = std::collections::HashSet::new();
        for c in arr(&self.get("/v1/channels")?, "channels") { if let Some(t) = s(&c, "channel_point").split(':').next() { opens.insert(t.to_lowercase()); } }
        for c in arr(&self.get("/v1/channels/closed")?, "channels") { if let Some(t) = s(&c, "channel_point").split(':').next() { opens.insert(t.to_lowercase()); } let ct = s(&c, "closing_tx_hash").to_lowercase(); if !ct.is_empty() { closes.insert(ct); } }
        for c in arr(&self.get("/v1/channels/pending")?, "pending_open_channels") { if let Some(t) = c.get("channel").and_then(|x| x.get("channel_point")).and_then(|x| x.as_str()).and_then(|p| p.split(':').next()) { opens.insert(t.to_lowercase()); } }
        let mut out = Vec::new();
        for t in arr(&self.get("/v1/transactions")?, "transactions") {
            let txid = s(&t, "tx_hash").to_lowercase(); let amount = n(&t, "amount"); let fee = n(&t, "total_fees").max(0) as u64; let time = n(&t, "time_stamp") as u32;
            if txid.is_empty() { continue; }
            let (layer, kind, net) = if opens.contains(&txid) { ("channel", "open", -(fee as i64)) } else if closes.contains(&txid) { ("channel", "close", 0) } else if amount >= 0 { ("chain", "deposit", amount) } else { ("chain", "withdrawal", amount) };
            if net == 0 && fee == 0 && layer != "channel" { continue; }
            out.push(Event { id: txid.clone(), time, net, fee: if amount < 0 || layer == "channel" { fee } else { 0 }, layer: layer.into(), txid, hash: String::new(), kind: kind.into(), memo: s(&t, "label") });
        }
        for i in arr(&self.get("/v1/invoices?num_max_invoices=100000")?, "invoices") {
            if s(&i, "state") != "SETTLED" && i.get("settled").and_then(|x| x.as_bool()) != Some(true) { continue; }
            let hash = STANDARD.decode(s(&i, "r_hash")).map(hex::encode).unwrap_or_default();
            let amt = n(&i, "amt_paid_sat"); if amt <= 0 || hash.is_empty() { continue; }
            out.push(Event { id: hash.clone(), time: n(&i, "settle_date") as u32, net: amt, fee: 0, layer: "lightning".into(), txid: String::new(), hash, kind: "received".into(), memo: s(&i, "memo") });
        }
        for p in arr(&self.get("/v1/payments?include_incomplete=false&max_payments=100000")?, "payments") {
            if s(&p, "status") != "SUCCEEDED" { continue; }
            let hash = s(&p, "payment_hash").to_lowercase(); let amt = n(&p, "value_sat"); let fee = n(&p, "fee_sat").max(0) as u64;
            let time = { let ns = n(&p, "creation_time_ns"); if ns > 0 { (ns / 1_000_000_000) as u32 } else { n(&p, "creation_date") as u32 } };
            if hash.is_empty() { continue; }
            out.push(Event { id: format!("{hash}:out"), time, net: -(amt + fee as i64), fee, layer: "lightning".into(), txid: String::new(), hash, kind: "paid".into(), memo: String::new() });
        }
        let fw = self.post("/v1/switch", serde_json::json!({ "start_time": "0", "num_max_events": 50000 }))?;
        for (k, f) in arr(&fw, "forwarding_events").iter().enumerate() {
            let fee = n(f, "fee"); if fee <= 0 { continue; }
            let time = { let ns = n(f, "timestamp_ns"); if ns > 0 { (ns / 1_000_000_000) as u32 } else { n(f, "timestamp") as u32 } };
            out.push(Event { id: format!("routed:{time}:{k}"), time, net: fee, fee: 0, layer: "lightning".into(), txid: String::new(), hash: String::new(), kind: "routed".into(), memo: String::new() });
        }
        out.sort_by_key(|e| (e.time, e.id.clone()));
        Ok(out)
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
    /// The real LND, named by A2K_LND_URL, a macaroon hex file at A2K_LND_MAC and its pin at A2K_LND_PIN.
    #[test] #[ignore]
    fn the_real_lnd_reads_and_the_macaroon_cannot_spend() {
        let (Ok(url), Ok(path)) = (std::env::var("A2K_LND_URL"), std::env::var("A2K_LND_MAC")) else { return };
        let mac = std::fs::read_to_string(path).unwrap();
        let c = Lnd::open(&url, mac.trim(), &std::env::var("A2K_LND_PIN").unwrap_or_default()).unwrap();
        let refused = c.prove_read_only().unwrap();
        assert!(refused.len() == 10, "{refused:?}");
        let (alias, id, version, network, height) = c.info().unwrap();
        assert!(!alias.is_empty() && id.len() == 66 && network == "mainnet" && height > 900_000, "{alias} {id} {version} {network} {height}");
        let (onchain, channels, pending, n) = c.funds().unwrap();
        let ev = c.events().unwrap();
        assert!(!ev.is_empty()); assert!(ev.windows(2).all(|w| w[0].time <= w[1].time));
        let kinds: std::collections::BTreeMap<&str, usize> = ev.iter().fold(Default::default(), |mut m, e| { *m.entry(e.kind.as_str()).or_default() += 1; m });
        eprintln!("{alias} {version} height {height} · on-chain {onchain} · channels {channels} ({n}) · pending {pending} · events {} {:?} · pin {} · refused {}", ev.len(), kinds, c.seen_pin(), refused.len());
    }
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
    fn an_lnd_credential_is_hex_base64_or_an_lndconnect_link() {
        assert_eq!(lnd_cred("0201036c6e64").unwrap().macaroon_hex, "0201036c6e64");
        assert_eq!(lnd_cred("AgEDbG5k").unwrap().macaroon_hex, "0201036c6e64", "base64 of the same bytes");
        let link = lnd_cred("lndconnect://10.0.0.5:8080?cert=MIIB&macaroon=AgEDbG5k").unwrap();
        assert_eq!((link.url.as_deref(), link.macaroon_hex.as_str()), (Some("https://10.0.0.5:8080"), "0201036c6e64"));
        assert_eq!(link.pin.as_ref().map(|p| p.len()), Some(64), "the link's certificate becomes the pin");
        assert!(lnd_cred("lndconnect://host:8080").is_err() && lnd_cred("not a credential!").is_err());
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
