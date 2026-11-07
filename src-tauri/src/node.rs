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
    /// where the last read got to, per list, so the next asks only for what is new (LND: heights and indexes)
    #[serde(default)] pub marks: std::collections::BTreeMap<String, u64>,
    /// A node that is gone — rebuilt under a new identity, wiped, retired — is not asked again: its ledger stays as the
    /// last read left it, read-only, and the vault keeps every link it ever made. `retired_why` is the person's reason.
    #[serde(default)] pub retired: bool,
    #[serde(default)] pub retired_why: String,
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
    /// "deposit" · "withdrawal" · "received" · "paid" · "routed" (income) · "open" · "close" · "fee" · "unaccounted"
    pub kind: String,
    #[serde(default)] pub memo: String,
    /// where the money went, for a row that left on the chain or closed a channel: (address, sats) per output of the
    /// transaction that is not this node's own — read from the transaction itself, so it is evidence, not the node's word
    #[serde(default)] pub to: Vec<(String, u64)>,
    /// the block, when the node gave a height but no time (a close it never saw): the chain fills the time from it
    #[serde(default)] pub height: u32,
    /// true when the transaction this row names is on no server's chain: the node's bookkeeper recorded something
    /// that never happened (a restored copy's stale view, a transaction never broadcast). Counted as zero.
    #[serde(default)] pub phantom: bool,
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
        Ok(cln_income_events(&v))
    }
}

/// The ledger out of a `bkpr-listincome` answer — from the node, or from a file of its output (an offline restore).
pub fn cln_income_events(v: &serde_json::Value) -> Vec<Event> {
    {
        let mut out = Vec::new();
        for e in v.get("income_events").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            let s = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let msat = |k: &str| e.get(k).and_then(|x| x.as_u64().or_else(|| x.as_str().and_then(|t| t.trim_end_matches("msat").parse::<u64>().ok()))).unwrap_or(0);
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
                // the bookkeeper calls everything that leaves a channel to no output of this node's an "onchain_fee"; a
                // fee of tens of thousands of sats on a channel account is not a fee, it is the channel's balance delivered
                // elsewhere — a close with the money sent to an address this node does not hold. Said so, and the
                // transaction kept so the chain can show where it went.
                "onchain_fee" if debit >= 50_000 && s("account") != "wallet" => ("channel", "close", -(debit as i64), 0),
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
            let txid_of = if layer == "chain" || layer == "fee" || layer == "channel" { if !txid.is_empty() { txid } else { outpoint.split(':').next().unwrap_or("").to_lowercase() } } else { String::new() };
            let memo = if layer == "channel" && kind == "close" { format!("channel {} closed with {} sats of this node's delivered to no address it holds — the bookkeeper booked it as an on-chain fee; the outputs are under To, and the date is when the bookkeeper saw it, not the block", s("account").chars().take(12).collect::<String>(), debit) } else { s("description") };
            out.push(Event { id, time, net, fee, layer: layer.into(), txid: txid_of, hash: if layer == "lightning" { hash } else { String::new() }, kind: kind.into(), memo, to: Vec::new(), height: 0, phantom: false });
        }
        // several bookkeeper events can belong to one transaction (a deposit that also paid change, a fee in parts):
        // the ledger keeps one row per id, its amounts summed, its fee summed
        let mut merged: std::collections::BTreeMap<String, Event> = std::collections::BTreeMap::new();
        for e in out { match merged.get_mut(&e.id) { Some(m) => { m.net += e.net; m.fee += e.fee; if m.time == 0 { m.time = e.time; } if m.memo.is_empty() { m.memo = e.memo; } } None => { merged.insert(e.id.clone(), e); } } }
        let rows: Vec<Event> = merged.into_values().filter(|e| e.net != 0 || e.fee != 0).collect();
        fold_routed(rows)
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
        let d = LndDump { channels: self.get("/v1/channels")?, closed: self.get("/v1/channels/closed")?, pending: self.get("/v1/channels/pending")?,
            transactions: self.get("/v1/transactions")?, invoices: self.get("/v1/invoices?num_max_invoices=100000")?,
            payments: self.get("/v1/payments?include_incomplete=false&max_payments=100000")?,
            forwards: self.post("/v1/switch", serde_json::json!({ "start_time": "0", "num_max_events": 50000 }))? };
        Ok(lnd_events_from(&d))
    }
}

/// LND's seven answers that make a ledger — read from the node, or from a file of `lncli` output (an offline restore).
pub struct LndDump { pub channels: serde_json::Value, pub closed: serde_json::Value, pub pending: serde_json::Value, pub transactions: serde_json::Value, pub invoices: serde_json::Value, pub payments: serde_json::Value, pub forwards: serde_json::Value }

/// A payment hash as REST gives it (base64) or as lncli prints it (hex): 32 bytes either way, or nothing.
fn hash32(text: &str) -> String {
    use bitcoin::base64::{engine::general_purpose::STANDARD, Engine};
    if text.len() == 64 { if let Ok(b) = hex::decode(text) { if b.len() == 32 { return hex::encode(b); } } }
    STANDARD.decode(text).ok().filter(|b| b.len() == 32).map(hex::encode).unwrap_or_default()
}

/// One wallet transaction as a ledger row. A funding is an open (−fee), a known closing transaction a close (0), else a
/// deposit or a withdrawal by LND's `amount`. One correction: when the outputs that are not this node's add up to MORE
/// than LND says left, the transaction spent inputs the wallet had never counted as its own (a channel's close output
/// swept straight out, as in a rescue), and the row takes the transaction's own figure — the outputs plus what the
/// wallet did book — with a memo saying so. Otherwise a sweep to safety looks like a 3,200 sat fee.
fn lnd_chain_row(t: &serde_json::Value, opens: &std::collections::HashSet<String>, closes: &std::collections::HashSet<String>) -> Option<Event> {
    let s = |k: &str| t.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let n = |k: &str| t.get(k).and_then(|x| x.as_str().and_then(|v| v.parse::<i64>().ok()).or_else(|| x.as_i64())).unwrap_or(0);
    let txid = s("tx_hash").to_lowercase(); let amount = n("amount"); let fee = n("total_fees").max(0) as u64; let time = n("time_stamp") as u32;
    if txid.is_empty() { return None; }
    let (layer, kind, mut net) = if opens.contains(&txid) { ("channel", "open", -(fee as i64)) } else if closes.contains(&txid) { ("channel", "close", 0) } else if amount >= 0 { ("chain", "deposit", amount) } else { ("chain", "withdrawal", amount) };
    let to = lnd_outs(t); let mut memo = s("label");
    if kind == "withdrawal" { let paid: i64 = to.iter().map(|(_, v)| *v as i64).sum(); if paid > -amount {
        net = -(paid + (-amount));
        memo = format!("{}{}the node's wallet booked only {} sats for this; {} sats of it came from outputs the wallet had not counted as its own (a channel's close being swept), so the amount is taken from the transaction's outputs", memo, if memo.is_empty() { "" } else { " — " }, -amount, paid);
    } }
    if net == 0 && fee == 0 && layer != "channel" { return None; }
    Some(Event { id: txid.clone(), time, net, fee: if amount < 0 || layer == "channel" { fee } else { 0 }, layer: layer.into(), txid, hash: String::new(), kind: kind.into(), memo, to, height: 0, phantom: false })
}

/// The outputs of a wallet transaction that are NOT this node's: where money went. LND lists them with the transaction.
fn lnd_outs(t: &serde_json::Value) -> Vec<(String, u64)> {
    t.get("output_details").and_then(|x| x.as_array()).map(|a| a.iter()
        .filter(|o| o.get("is_our_address").and_then(|x| x.as_bool()) != Some(true))
        .filter_map(|o| { let addr = o.get("address").and_then(|x| x.as_str())?.to_string(); let amt = o.get("amount").and_then(|x| x.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| x.as_u64()))?; if addr.is_empty() { None } else { Some((addr, amt)) } })
        .collect()).unwrap_or_default()
}

/// The ledger, assembled: on-chain transactions (channel opens and closes recognised by their outpoints and counted as
/// moves within the node, fee only), settled invoices, succeeded payments, and routing fees earned.
pub fn lnd_events_from(d: &LndDump) -> Vec<Event> {
    {
        let outs = |t: &serde_json::Value| if t.get("amount").and_then(|x| x.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| x.as_i64())).unwrap_or(0) < 0 { lnd_outs(t) } else { Vec::new() };
        let n = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| x.as_i64())).unwrap_or(0);
        let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let arr = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
        // which transactions are channels being opened or closed
        let mut opens = std::collections::HashSet::new(); let mut closes = std::collections::HashSet::new();
        for c in arr(&d.channels, "channels") { if let Some(t) = s(&c, "channel_point").split(':').next() { opens.insert(t.to_lowercase()); } }
        for c in arr(&d.closed, "channels") { if let Some(t) = s(&c, "channel_point").split(':').next() { opens.insert(t.to_lowercase()); } let ct = s(&c, "closing_tx_hash").to_lowercase(); if !ct.is_empty() { closes.insert(ct); } }
        for c in arr(&d.pending, "pending_open_channels") { if let Some(t) = c.get("channel").and_then(|x| x.get("channel_point")).and_then(|x| x.as_str()).and_then(|p| p.split(':').next()) { opens.insert(t.to_lowercase()); } }
        let mut out = Vec::new();
        // a close whose transaction the wallet never saw (no output of it is ours) still happened: it gets a row with
        // the channel's size, so what left at that close can be shown — and the addresses looked up from the chain
        let known: std::collections::HashSet<String> = arr(&d.transactions, "transactions").iter().map(|t| s(t, "tx_hash").to_lowercase()).collect();
        for c in arr(&d.closed, "channels") {
            let ct = s(&c, "closing_tx_hash").to_lowercase(); if ct.is_empty() || known.contains(&ct) { continue; }
            let cap = n(&c, "capacity"); let settled = n(&c, "settled_balance");
            let memo = format!("channel {} ({} sats capacity) closed: {}; the wallet that reported this recognised none of the closing transaction's outputs as its own (a wallet restored from a backup does not know addresses derived after it was taken, so an output may still have been this node's and swept later — the outputs are listed under To){}", s(&c, "channel_point").split(':').next().unwrap_or("").chars().take(12).collect::<String>(), cap, s(&c, "close_type").to_lowercase().replace('_', " "), if settled > 0 { format!("; the node expected {settled} sats back") } else { String::new() });
            out.push(Event { id: ct.clone(), time: 0, net: 0, fee: 0, layer: "channel".into(), txid: ct, hash: String::new(), kind: "close".into(), memo, to: Vec::new(), height: n(&c, "close_height").max(0) as u32, phantom: false });
        }
        for t in arr(&d.transactions, "transactions") {
            if let Some(e) = lnd_chain_row(&t, &opens, &closes) { out.push(e); }
        }
        for i in arr(&d.invoices, "invoices") {
            if s(&i, "state") != "SETTLED" && i.get("settled").and_then(|x| x.as_bool()) != Some(true) { continue; }
            let hash = hash32(&s(&i, "r_hash"));
            let amt = n(&i, "amt_paid_sat"); if amt <= 0 || hash.is_empty() { continue; }
            out.push(Event { id: hash.clone(), time: n(&i, "settle_date") as u32, net: amt, fee: 0, layer: "lightning".into(), txid: String::new(), hash, kind: "received".into(), memo: s(&i, "memo"), to: Vec::new(), height: 0, phantom: false });
        }
        for p in arr(&d.payments, "payments") {
            if s(&p, "status") != "SUCCEEDED" { continue; }
            let hash = s(&p, "payment_hash").to_lowercase(); let amt = n(&p, "value_sat"); let fee = n(&p, "fee_sat").max(0) as u64;
            let time = { let ns = n(&p, "creation_time_ns"); if ns > 0 { (ns / 1_000_000_000) as u32 } else { n(&p, "creation_date") as u32 } };
            if hash.is_empty() { continue; }
            out.push(Event { id: format!("{hash}:out"), time, net: -(amt + fee as i64), fee, layer: "lightning".into(), txid: String::new(), hash, kind: "paid".into(), memo: String::new(), to: Vec::new(), height: 0, phantom: false });
        }
        for (k, f) in arr(&d.forwards, "forwarding_events").iter().enumerate() {
            let fee = n(f, "fee"); if fee <= 0 { continue; }
            let time = { let ns = n(f, "timestamp_ns"); if ns > 0 { (ns / 1_000_000_000) as u32 } else { n(f, "timestamp") as u32 } };
            out.push(Event { id: format!("routed:{time}:{k}"), time, net: fee, fee: 0, layer: "lightning".into(), txid: String::new(), hash: String::new(), kind: "routed".into(), memo: String::new(), to: Vec::new(), height: 0, phantom: false });
        }
        fold_routed(out)
    }
}

impl Lnd {
    /// What is new since the marks: transactions from a little below the last height, invoices and payments past
    /// their indexes, forwards after the last one seen. The marks to keep are returned with the events.
    pub fn events_since(&self, marks: &std::collections::BTreeMap<String, u64>) -> R<(Vec<Event>, std::collections::BTreeMap<String, u64>)> {
        let m = |k: &str| marks.get(k).copied().unwrap_or(0);
        if marks.is_empty() { let ev = self.events()?; return Ok((ev.clone(), self.marks_after(&ev)?)); }
        let n = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| x.as_i64())).unwrap_or(0);
        let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let arr = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
        let mut opens = std::collections::HashSet::new(); let mut closes = std::collections::HashSet::new();
        for c in arr(&self.get("/v1/channels")?, "channels") { if let Some(t) = s(&c, "channel_point").split(':').next() { opens.insert(t.to_lowercase()); } }
        for c in arr(&self.get("/v1/channels/closed")?, "channels") { if let Some(t) = s(&c, "channel_point").split(':').next() { opens.insert(t.to_lowercase()); } let ct = s(&c, "closing_tx_hash").to_lowercase(); if !ct.is_empty() { closes.insert(ct); } }
        let outs = |t: &serde_json::Value| if n(t, "amount") < 0 { lnd_outs(t) } else { Vec::new() };
        let mut out = Vec::new();
        let from_height = m("height").saturating_sub(6);
        for t in arr(&self.get(&format!("/v1/transactions?start_height={from_height}"))?, "transactions") {
            if let Some(e) = lnd_chain_row(&t, &opens, &closes) { out.push(e); }
        }
        let inv_from = m("invoice").saturating_sub(200);   // an invoice settles after it is added: look a little way back
        for i in arr(&self.get(&format!("/v1/invoices?index_offset={inv_from}&num_max_invoices=100000"))?, "invoices") {
            if s(&i, "state") != "SETTLED" && i.get("settled").and_then(|x| x.as_bool()) != Some(true) { continue; }
            let hash = hash32(&s(&i, "r_hash"));
            let amt = n(&i, "amt_paid_sat"); if amt <= 0 || hash.is_empty() { continue; }
            out.push(Event { id: hash.clone(), time: n(&i, "settle_date") as u32, net: amt, fee: 0, layer: "lightning".into(), txid: String::new(), hash, kind: "received".into(), memo: s(&i, "memo"), to: Vec::new(), height: 0, phantom: false });
        }
        for p in arr(&self.get(&format!("/v1/payments?include_incomplete=false&index_offset={}&max_payments=100000", m("payment")))?, "payments") {
            if s(&p, "status") != "SUCCEEDED" { continue; }
            let hash = s(&p, "payment_hash").to_lowercase(); let amt = n(&p, "value_sat"); let fee = n(&p, "fee_sat").max(0) as u64;
            let time = { let ns = n(&p, "creation_time_ns"); if ns > 0 { (ns / 1_000_000_000) as u32 } else { n(&p, "creation_date") as u32 } };
            if hash.is_empty() { continue; }
            out.push(Event { id: format!("{hash}:out"), time, net: -(amt + fee as i64), fee, layer: "lightning".into(), txid: String::new(), hash, kind: "paid".into(), memo: String::new(), to: Vec::new(), height: 0, phantom: false });
        }
        let fw = self.post("/v1/switch", serde_json::json!({ "start_time": (m("forward") + 1).to_string(), "num_max_events": 50000 }))?;
        for (k, f) in arr(&fw, "forwarding_events").iter().enumerate() {
            let fee = n(f, "fee"); if fee <= 0 { continue; }
            let time = { let ns = n(f, "timestamp_ns"); if ns > 0 { (ns / 1_000_000_000) as u32 } else { n(f, "timestamp") as u32 } };
            out.push(Event { id: format!("routed:{time}:{k}"), time, net: fee, fee: 0, layer: "lightning".into(), txid: String::new(), hash: String::new(), kind: "routed".into(), memo: String::new(), to: Vec::new(), height: 0, phantom: false });
        }
        let ev = fold_routed(out);
        let mut marks = self.marks_after(&ev)?;
        // a read with no new forwards keeps the forward mark it had, or the next read would fetch them all again
        if marks.get("forward").copied().unwrap_or(0) < m("forward") { marks.insert("forward".into(), m("forward")); }
        Ok((ev, marks))
    }
    /// The marks a read leaves: the chain tip, the last invoice and payment indexes, the last forward's time.
    fn marks_after(&self, ev: &[Event]) -> R<std::collections::BTreeMap<String, u64>> {
        let n = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| x.as_u64())).unwrap_or(0);
        let mut marks = std::collections::BTreeMap::new();
        marks.insert("height".into(), n(&self.get("/v1/getinfo")?, "block_height"));
        let inv = self.get("/v1/invoices?reversed=true&num_max_invoices=1")?; marks.insert("invoice".into(), n(&inv, "last_index_offset"));
        let pay = self.get("/v1/payments?include_incomplete=true&reversed=true&max_payments=1")?; marks.insert("payment".into(), n(&pay, "last_index_offset"));
        marks.insert("forward".into(), ev.iter().filter(|e| e.kind == "routed").map(|e| e.time as u64).max().unwrap_or(0));
        Ok(marks)
    }
}

/// Routing fees are many and small: one row per UTC day carries the day's total, its time the last forward's.
pub fn fold_routed(mut ev: Vec<Event>) -> Vec<Event> {
    let mut days: std::collections::BTreeMap<u32, (i64, u32)> = std::collections::BTreeMap::new();
    ev.retain(|e| { if e.kind == "routed" { let d = days.entry(e.time / 86400).or_insert((0, 0)); d.0 += e.net; d.1 = d.1.max(e.time); false } else { true } });
    for (day, (net, time)) in days { if net != 0 { ev.push(Event { id: format!("routed:{day}"), time, net, fee: 0, layer: "lightning".into(), txid: String::new(), hash: String::new(), kind: "routed".into(), memo: String::new(), to: Vec::new(), height: 0, phantom: false }); } }
    ev.sort_by_key(|e| (e.time, e.id.clone()));
    ev
}

/// New events folded into a ledger already held: a row seen again replaces its earlier reading (a transaction
/// confirmed, an invoice settled); a day's routing row adds to what the day already had.
pub fn merge_events(have: &[Event], new: Vec<Event>) -> Vec<Event> {
    let mut by_id: std::collections::BTreeMap<String, Event> = have.iter().map(|e| (e.id.clone(), e.clone())).collect();
    for e in new { if e.kind == "routed" { if let Some(h) = by_id.get_mut(&e.id) { h.net += e.net; h.time = h.time.max(e.time); continue; } } by_id.insert(e.id.clone(), e); }
    let mut out: Vec<Event> = by_id.into_values().collect();
    out.sort_by_key(|e| (e.time, e.id.clone()));
    out
}

/// A ledger re-read WHOLE (Core Lightning's bookkeeper gives everything each time): every row the node reports now
/// replaces its earlier reading, and a row the node no longer reports — a database rolled back, a history wiped —
/// is KEPT. The ledger never shrinks: what a node once said about money that moved is evidence, and a node that has
/// forgotten it is not a reason for this app to.
pub fn keep_and_replace(have: &[Event], new: Vec<Event>) -> Vec<Event> {
    let mut by_id: std::collections::BTreeMap<String, Event> = have.iter().map(|e| (e.id.clone(), e.clone())).collect();
    for e in new { by_id.insert(e.id.clone(), e); }
    let mut out: Vec<Event> = by_id.into_values().collect();
    out.sort_by_key(|e| (e.time, e.id.clone()));
    out
}

/// What the node says it holds, sats: on chain, in channels, pending (unconfirmed outputs, channels not yet usable).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Held { pub onchain: u64, pub channels: u64, pub pending: u64 }
impl Held { pub fn total(&self) -> i64 { (self.onchain + self.channels + self.pending) as i64 } }

/// The node's reported balance is the authority; the ledger explains it. When the ledger's rows add up to something
/// other than what the node holds, the difference is booked as ONE explicit row — "unaccounted" — so the sum is honest
/// again and the gap is visible, with its reason beside it, instead of a holding chart that disagrees with the balance
/// card. Money can leave a node without a row here: a channel closed with the funds delivered to someone else's
/// address, a payment made from another copy of the node's keys. There is only ever one such row: each reconciliation
/// recomputes the gap from the explained rows and rewrites it — keeping the date of the row already there (when the
/// money was found missing), or dated `at` if it is new — and removes it when the ledger explains the balance again.
pub fn reconcile(mut events: Vec<Event>, held: i64, at: u32, why: &str) -> Vec<Event> {
    let explained: i64 = events.iter().filter(|e| e.kind != "unaccounted").map(|e| e.net).sum();
    let diff = held - explained;
    let was = events.iter().filter(|e| e.kind == "unaccounted").map(|e| (e.time, e.memo.clone())).min_by_key(|x| x.0);
    events.retain(|e| e.kind != "unaccounted");
    if diff == 0 { return events; }
    let (time, memo) = match was { Some((t, m)) if t > 0 => (t.min(at), if why.is_empty() { m } else { why.to_string() }), _ => (at, why.to_string()) };
    events.push(Event { id: format!("unaccounted:{time}"), time, net: diff, fee: 0, layer: "node".into(), txid: String::new(), hash: String::new(), kind: "unaccounted".into(), memo, to: Vec::new(), height: 0, phantom: false });
    events.sort_by_key(|e| (e.time, e.id.clone()));
    events
}

/// A ledger from a FILE: the node's own answers, saved on a machine that could reach it when this one could not — an
/// offline restore of a backup, a node since rebuilt. One JSON object. For Core Lightning the output of
/// `bkpr-listincome` (with `getinfo` beside it under "getinfo", if the identity is to be checked). For LND an object
/// keyed by the lncli commands: listchaintxns, listinvoices, listpayments, fwdinghistory, listchannels,
/// closedchannels, pendingchannels, getinfo — and walletbalance / channelbalance when the balances are wanted (without
/// them, on-chain is the sum of listchaintxns, valid only for a complete history, and channels the sum of listchannels).
/// For Core Lightning, `listfunds` under "listfunds" gives the balances. Returns (kind, identity pubkey if the file
/// names one, events, what the node holds if the file can say).
/// What a Core Lightning file gives beyond its ledger: `listfunds` and `listclosedchannels` as the node answered them,
/// for the chain to check (cln_chain_truth). Taken from the file after ledger_from_file; empty when absent.
pub fn cln_claims_from_file(text: &str) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap_or(serde_json::Value::Null);
    serde_json::json!({ "listfunds": v.get("listfunds").cloned().unwrap_or(serde_json::Value::Null), "listclosedchannels": v.get("listclosedchannels").cloned().unwrap_or(serde_json::Value::Null) })
}

/// The chain over the node's word. A node whose database was rolled back, or whose keys someone else holds, lists
/// outputs it no longer has and channels whose closing it never booked. For every output `listfunds` calls unspent
/// and every channel `listclosedchannels` says ended with money on our side, the chain is asked: was it spent, in
/// which transaction, in which block, to which addresses? What it answers becomes a dated row with the destination
/// on it — a withdrawal for a spent output, a close for a channel whose closing transaction the bookkeeper never
/// saw — and the balance the node claimed is reduced by the outputs the chain shows gone. Returns the rows and how
/// many sats to take off the claimed on-chain balance. Failures leave things as they were: nothing is invented.
pub fn cln_chain_truth(claims: &serde_json::Value, events: &[Event], servers: &crate::chain::Servers, network: &str) -> (Vec<Event>, u64) {
    use std::str::FromStr;
    use bitcoin::{Txid, OutPoint};
    let net = crate::wallet::network_of(network);
    let mut want: Vec<(OutPoint, u64, &'static str, String)> = Vec::new();   // outpoint, sats on our side, what it is, a word about it
    for o in claims.get("listfunds").and_then(|f| f.get("outputs")).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
        let (Some(txid), Some(n)) = (o.get("txid").and_then(|x| x.as_str()).and_then(|t| Txid::from_str(t).ok()), o.get("output").and_then(|x| x.as_u64())) else { continue };
        let sats = o.get("amount_msat").and_then(|x| x.as_u64()).unwrap_or(0) / 1000;
        want.push((OutPoint { txid, vout: n as u32 }, sats, "output", format!("output {}:{n}", &txid.to_string()[..12])));
    }
    let booked: std::collections::HashSet<&str> = events.iter().filter(|e| !e.txid.is_empty()).map(|e| e.txid.as_str()).collect();
    for c in claims.get("listclosedchannels").and_then(|f| f.get("closedchannels")).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
        let ours = c.get("final_to_us_msat").and_then(|x| x.as_u64()).unwrap_or(0) / 1000; if ours == 0 { continue; }
        let (Some(txid), Some(n)) = (c.get("funding_txid").and_then(|x| x.as_str()).and_then(|t| Txid::from_str(t).ok()), c.get("funding_outnum").and_then(|x| x.as_u64())) else { continue };
        want.push((OutPoint { txid, vout: n as u32 }, ours, "channel", format!("channel {} ({}, closed {})", &txid.to_string()[..12], c.get("peer_id").and_then(|x| x.as_str()).map(|p| &p[..12.min(p.len())]).unwrap_or("?"), c.get("close_cause").and_then(|x| x.as_str()).unwrap_or("?"))));
    }
    if want.is_empty() { return (Vec::new(), 0); }
    // the scripts those outpoints pay, then each script's history, then the transaction in it that spends the outpoint
    let ids: Vec<Txid> = want.iter().map(|w| w.0.txid).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let Ok(txs) = crate::chain::transactions_kept(servers, network, &ids, &|_| true) else { return (Vec::new(), 0) };
    let by_id: std::collections::HashMap<Txid, &bitcoin::Transaction> = txs.iter().map(|t| (t.compute_txid(), t)).collect();
    let spks: Vec<bitcoin::ScriptBuf> = want.iter().filter_map(|w| by_id.get(&w.0.txid).and_then(|t| t.output.get(w.0.vout as usize)).map(|o| o.script_pubkey.clone())).collect();
    if spks.len() != want.len() { return (Vec::new(), 0); }
    let Ok(hists) = crate::chain::histories(servers, &spks) else { return (Vec::new(), 0) };
    let cand: Vec<Txid> = hists.iter().flatten().filter_map(|h| Txid::from_str(&h.txid).ok()).filter(|t| !by_id.contains_key(t)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let Ok(ctxs) = crate::chain::transactions_kept(servers, network, &cand, &|_| true) else { return (Vec::new(), 0) };
    let heights: std::collections::HashMap<Txid, i32> = hists.iter().flatten().filter_map(|h| Txid::from_str(&h.txid).ok().map(|t| (t, h.height))).collect();
    // a close whose money came home: some output of the closing transaction was later spent by a transaction the
    // bookkeeper knows (the sweep of a timelocked output, a withdrawal). Those closes are not losses. The outputs'
    // scripts are asked for their history and the spenders compared with the booked transactions.
    let closes: Vec<&bitcoin::Transaction> = want.iter().filter(|w| w.2 == "channel").filter_map(|w| ctxs.iter().find(|t| t.input.iter().any(|i| i.previous_output == w.0))).collect();
    let close_spks: Vec<bitcoin::ScriptBuf> = closes.iter().flat_map(|t| t.output.iter().map(|o| o.script_pubkey.clone())).collect();
    let came_home: std::collections::HashSet<Txid> = if close_spks.is_empty() { Default::default() } else {
        let hs = crate::chain::histories(servers, &close_spks).unwrap_or_default();
        // the booked transactions in those histories, fetched, and kept only if an input of theirs spends the close
        let cands: Vec<Txid> = hs.iter().flatten().filter(|i| booked.contains(i.txid.as_str())).filter_map(|i| Txid::from_str(&i.txid).ok()).filter(|t| !by_id.contains_key(t)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let spenders = crate::chain::transactions_kept(servers, network, &cands, &|_| true).unwrap_or_default();
        closes.iter().map(|t| t.compute_txid()).filter(|cid| spenders.iter().any(|sp| sp.input.iter().any(|i| i.previous_output.txid == *cid))).collect() };
    let mut out = Vec::new(); let mut gone = 0u64; let mut need_heights = std::collections::BTreeSet::new();
    for (w, _) in want.iter().zip(&spks) {
        let Some(sp) = ctxs.iter().find(|t| t.input.iter().any(|i| i.previous_output == w.0)) else { continue };
        let sid = sp.compute_txid().to_string();
        if w.2 == "channel" && came_home.contains(&sp.compute_txid()) { continue; }
        let h = heights.get(&sp.compute_txid()).copied().unwrap_or(0).max(0) as u32; if h > 0 { need_heights.insert(h); }
        let to: Vec<(String, u64)> = sp.output.iter().map(|o| (bitcoin::Address::from_script(&o.script_pubkey, net).map(|a| a.to_string()).unwrap_or_else(|_| format!("script {}", hex::encode(o.script_pubkey.as_bytes()))), o.value.to_sat())).collect();
        if w.2 == "output" {
            gone += w.1;
            out.push(Event { id: sid.clone(), time: 0, net: -(w.1 as i64), fee: 0, layer: "chain".into(), txid: sid, hash: String::new(), kind: "withdrawal".into(), memo: format!("the node still lists {} ({} sats) as its own; the chain shows it spent in block {h} by a key held elsewhere — where it went is under To", w.3, w.1), to, height: h, phantom: false });
        } else if !booked.contains(sid.as_str()) {
            out.push(Event { id: sid.clone(), time: 0, net: -(w.1 as i64), fee: 0, layer: "channel".into(), txid: sid, hash: String::new(), kind: "close".into(), memo: format!("{} with {} sats on this node's side; the closing transaction is in block {h} and the bookkeeper never booked it — the money was delivered to an address this node does not hold, listed under To", w.3, w.1), to, height: h, phantom: false });
        }
    }
    // dates from the blocks
    let hs: Vec<u32> = need_heights.into_iter().collect();
    if let Ok(times) = crate::chain::header_times(servers, &hs) { let at: std::collections::HashMap<u32, u32> = hs.iter().copied().zip(times).collect(); for e in out.iter_mut() { if let Some(t) = at.get(&e.height) { e.time = *t; } } }
    (out, gone)
}

/// The rows the chain found replace what the bookkeeper guessed about the same transaction: a "fee" of a channel's
/// whole balance, or a close row with no date, gives way to the dated row with the destination on it.
pub fn prefer_chain_rows(mut events: Vec<Event>, found: Vec<Event>) -> Vec<Event> {
    for f in found {
        // the same transaction under another id, or a bookkeeper's "fee"/close of exactly this amount with no
        // destination on it (its guess at the same money leaving): the chain's row stands in for it
        events.retain(|e| !(e.id != f.id && (e.layer == "fee" || e.layer == "channel") && e.net == f.net && (e.txid == f.txid || (e.to.is_empty() && f.kind == "close"))));
        match events.iter_mut().find(|e| e.id == f.id) { Some(e) => *e = f, None => events.push(f) }
    }
    events.sort_by_key(|e| (e.time, e.id.clone()));
    events
}

pub fn ledger_from_file(text: &str) -> R<(String, String, Vec<Event>, Option<Held>)> {
    let v: serde_json::Value = serde_json::from_str(text.trim()).map_err(|_| "this is not a JSON file")?;
    let o = v.as_object().ok_or("the file is JSON, but not one object")?;
    let take = |keys: &[&str]| -> serde_json::Value { keys.iter().find_map(|k| o.get(*k).cloned()).unwrap_or(serde_json::json!({})) };
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_lowercase();
    if o.contains_key("income_events") || o.contains_key("bkpr-listincome") {
        let income = if o.contains_key("income_events") { v.clone() } else { take(&["bkpr-listincome"]) };
        let ev = cln_income_events(&income);
        if ev.is_empty() { return Err("the file holds no income events".into()); }
        let held = o.get("listfunds").map(|f| {
            let msat = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0) / 1000;
            let mut h = Held::default();
            for out in f.get("outputs").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) { if out.get("status").and_then(|x| x.as_str()) == Some("confirmed") { h.onchain += msat(out, "amount_msat"); } else { h.pending += msat(out, "amount_msat"); } }
            for c in f.get("channels").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) { if c.get("state").and_then(|x| x.as_str()) == Some("CHANNELD_NORMAL") { h.channels += msat(c, "our_amount_msat"); } else { h.pending += msat(c, "our_amount_msat"); } }
            h
        });
        return Ok(("cln".into(), s(&take(&["getinfo"]), "id"), ev, held));
    }
    let lnd_keys = ["listchaintxns", "transactions", "listinvoices", "invoices", "listpayments", "payments", "fwdinghistory", "forwarding_events"];
    if lnd_keys.iter().any(|k| o.contains_key(*k)) {
        // each answer may sit under its command name, or the file may be one bare answer (then the others are empty)
        let bare = |k: &str| if o.contains_key(k) { v.clone() } else { serde_json::json!({}) };
        let d = LndDump {
            channels: if o.contains_key("listchannels") { take(&["listchannels"]) } else { bare("channels") },
            closed: take(&["closedchannels"]), pending: take(&["pendingchannels"]),
            transactions: if o.contains_key("listchaintxns") { take(&["listchaintxns"]) } else { bare("transactions") },
            invoices: if o.contains_key("listinvoices") { take(&["listinvoices"]) } else { bare("invoices") },
            payments: if o.contains_key("listpayments") { take(&["listpayments"]) } else { bare("payments") },
            forwards: if o.contains_key("fwdinghistory") { take(&["fwdinghistory"]) } else { bare("forwarding_events") },
        };
        let ev = lnd_events_from(&d);
        if ev.is_empty() { return Err("the file holds no transactions, invoices, payments or forwards".into()); }
        let n = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str().and_then(|t| t.parse::<i64>().ok()).or_else(|| x.as_i64())).unwrap_or(0);
        let arr = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
        let held = if o.contains_key("walletbalance") || (o.contains_key("listchaintxns") && o.contains_key("listchannels")) {
            let (onchain, unconfirmed) = match o.get("walletbalance") { Some(w) => (n(w, "confirmed_balance").max(0) as u64, n(w, "unconfirmed_balance").max(0) as u64),
                None => (arr(&d.transactions, "transactions").iter().map(|t| n(t, "amount")).sum::<i64>().max(0) as u64, 0) };
            let channels = arr(&d.channels, "channels").iter().map(|c| n(c, "local_balance").max(0) as u64).sum::<u64>();
            let pending_ch = arr(&d.pending, "pending_open_channels").iter().map(|c| c.get("channel").map(|ch| n(ch, "local_balance").max(0) as u64).unwrap_or(0)).sum::<u64>();
            Some(Held { onchain, channels, pending: unconfirmed + pending_ch })
        } else { None };
        return Ok(("lnd".into(), s(&take(&["getinfo"]), "identity_pubkey"), ev, held));
    }
    Err("not a ledger this app knows: save bkpr-listincome (Core Lightning) or the lncli answers (LND) as one JSON object".into())
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
        // the incremental read: from the marks a full read leaves, a second read brings back nothing new (or nearly)
        let (first, marks) = c.events_since(&Default::default()).unwrap();
        let t0 = std::time::Instant::now();
        let (again, marks2) = c.events_since(&marks).unwrap();
        eprintln!("first read {} rows; marks {:?}; again {} rows in {:.1}s; marks after {:?}", first.len(), marks, again.len(), t0.elapsed().as_secs_f64(), marks2);
        assert!(again.len() <= 12, "a refresh with nothing new re-reads only the transactions near the tip: {}", again.len());
        assert_eq!(node_merge_len(&first, again.clone()), first.len(), "merging what was re-read adds nothing");
    }
    fn node_merge_len(have: &[Event], new: Vec<Event>) -> usize { merge_events(have, new).len() }
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
    /// A real ledger file, when one is named: A2K_LEDGER_FILE=path cargo test --lib node::a_real_ledger_file -- --ignored --nocapture
    /// A_2K_LEDGER_FILE=<a CLN file with listfunds + listclosedchannels>: the chain is asked about the node's claims.
    #[test] #[ignore]
    fn the_chain_corrects_a_real_cln_file() {
        let Ok(p) = std::env::var("A2K_LEDGER_FILE") else { return };
        let text = std::fs::read_to_string(&p).unwrap();
        let (kind, _, ev, held) = ledger_from_file(&text).unwrap(); assert_eq!(kind, "cln");
        let servers = crate::chain::defaults(bitcoin::Network::Bitcoin);
        let (found, gone) = cln_chain_truth(&cln_claims_from_file(&text), &ev, &servers, "mainnet");
        println!("claimed {held:?}; the chain says {gone} sats of the listed outputs are gone; {} rows found", found.len());
        for e in &found { println!("  {} {}/{} net {} time {} height {} to {:?}\n    {}", &e.txid[..12], e.layer, e.kind, e.net, e.time, e.height, e.to, e.memo); }
        let all = prefer_chain_rows(ev, found); println!("ledger now {} rows, sums to {}", all.len(), all.iter().map(|e| e.net).sum::<i64>());
    }
    #[test] #[ignore]
    fn a_real_ledger_file_parses_and_its_rows_are_counted() {
        let p = std::env::var("A2K_LEDGER_FILE").expect("A2K_LEDGER_FILE");
        let (kind, pk, ev, held) = ledger_from_file(&std::fs::read_to_string(&p).unwrap()).unwrap();
        println!("held {held:?}; ledger sums to {}", ev.iter().map(|e| e.net).sum::<i64>());
        let mut kinds: std::collections::BTreeMap<String, (usize, i64)> = Default::default();
        for e in &ev { let k = kinds.entry(format!("{}/{}", e.layer, e.kind)).or_default(); k.0 += 1; k.1 += e.net; }
        println!("{kind} {pk} rows {} first {} last {}", ev.len(), ev.first().map(|e| e.time).unwrap_or(0), ev.last().map(|e| e.time).unwrap_or(0));
        for (k, (n, net)) in kinds { println!("  {k:22} {n:6} rows  net {net:>12} sats"); }
    }

    #[test]
    fn a_ledger_file_is_read_as_the_node_would_be_read() {
        // Core Lightning: bkpr-listincome saved as-is, with getinfo beside it
        let cln = serde_json::json!({ "getinfo": { "id": "02AB" }, "income_events": [
            { "account": "wallet", "tag": "deposit", "credit_msat": 100000000, "debit_msat": 0, "currency": "bc", "timestamp": 1700000000, "txid": "aa", "outpoint": "aa:0" },
            { "account": "wallet", "tag": "invoice", "credit_msat": 0, "debit_msat": "5000000msat", "currency": "bc", "timestamp": 1700000100, "payment_id": "cc", "description": "coffee" },
        ] });
        let (kind, pk, ev, held) = ledger_from_file(&cln.to_string()).unwrap();
        assert_eq!((kind.as_str(), pk.as_str(), ev.len(), held), ("cln", "02ab", 2, None));
        assert_eq!(ev.iter().find(|e| e.kind == "paid").unwrap().net, -5000, "an msat string is read like an msat number");
        // LND: lncli answers under their command names; r_hash as lncli prints it (hex), not as REST sends it (base64)
        let h = "11".repeat(32);
        let lnd = serde_json::json!({ "getinfo": { "identity_pubkey": "03CD" },
            "listchaintxns": { "transactions": [ { "tx_hash": "bb", "amount": "250000", "total_fees": "0", "time_stamp": "1700000000" } ] },
            "listinvoices": { "invoices": [ { "r_hash": h, "state": "SETTLED", "amt_paid_sat": "1200", "settle_date": "1700000200", "memo": "tip" } ] },
            "listpayments": { "payments": [] }, "fwdinghistory": { "forwarding_events": [ { "fee": "3", "timestamp": "1700000300" } ] } });
        let (kind, pk, ev, held) = ledger_from_file(&lnd.to_string()).unwrap();
        assert_eq!((kind.as_str(), pk.as_str(), ev.len()), ("lnd", "03cd", 3));
        assert_eq!(held, None, "no listchannels, no walletbalance: the file does not say what is held");
        assert_eq!(ev.iter().find(|e| e.kind == "received").unwrap().hash, h);
        assert!(ledger_from_file("{\"hello\": 1}").is_err());
    }

    #[test]
    fn the_node_is_the_authority_and_the_ledger_books_what_it_cannot_explain() {
        let ev = |id: &str, t: u32, net: i64| Event { id: id.into(), time: t, net, fee: 0, layer: "chain".into(), txid: String::new(), hash: String::new(), kind: "deposit".into(), memo: String::new(), to: Vec::new(), height: 0, phantom: false };
        let led = vec![ev("a", 10, 1_000_000), ev("b", 20, 500_000)];
        // the node holds 400k: 1.1M left without a row — booked, dated, named
        let r = reconcile(led.clone(), 400_000, 30, "left without a record");
        assert_eq!(r.len(), 3); let u = r.iter().find(|e| e.kind == "unaccounted").unwrap();
        assert_eq!((u.net, u.time, u.layer.as_str()), (-1_100_000, 30, "node"));
        assert_eq!(r.iter().map(|e| e.net).sum::<i64>(), 400_000, "the ledger now sums to what the node holds");
        // the same gap again books nothing more; the exact opposite gap (a late row arrived) removes the booking
        assert_eq!(reconcile(r.clone(), 400_000, 40, "x").len(), 3);
        let mut late = r.clone(); late.push(ev("c", 25, -1_100_000));
        let back = reconcile(late, 400_000, 50, "x");
        assert!(back.iter().all(|e| e.kind != "unaccounted"), "a row that explains the gap replaces the booking");
        assert_eq!(back.iter().map(|e| e.net).sum::<i64>(), 400_000);
    }

    #[test]
    fn a_whole_reread_replaces_rows_and_keeps_what_the_node_forgot() {
        let ev = |id: &str, net: i64| Event { id: id.into(), time: 10, net, fee: 0, layer: "lightning".into(), txid: String::new(), hash: String::new(), kind: "received".into(), memo: String::new(), to: Vec::new(), height: 0, phantom: false };
        let have = vec![ev("a", 100), ev("routed:0", 7), ev("b", 50)];
        // the node now reports a (changed) and the day's routing (whole, not an increment); b is gone from it
        let out = keep_and_replace(&have, vec![ev("a", 120), ev("routed:0", 9)]);
        assert_eq!(out.len(), 3, "a forgotten row is kept");
        assert_eq!(out.iter().find(|e| e.id == "a").unwrap().net, 120, "a row read again is replaced");
        assert_eq!(out.iter().find(|e| e.id == "routed:0").unwrap().net, 9, "a whole re-read replaces the day, it does not add to it");
        assert_eq!(out.iter().find(|e| e.id == "b").unwrap().net, 50);
    }

    #[test]
    fn routing_fees_fold_into_one_row_per_day_and_merge_adds_to_it() {
        let r = |t: u32, id: &str, net: i64| Event { id: id.into(), time: t, net, fee: 0, layer: "lightning".into(), txid: String::new(), hash: String::new(), kind: "routed".into(), memo: String::new(), to: Vec::new(), height: 0, phantom: false };
        let ev = fold_routed(vec![r(86400 * 100 + 10, "a", 3), r(86400 * 100 + 50, "b", 4), r(86400 * 101 + 1, "c", 5)]);
        assert_eq!(ev.iter().map(|e| (e.id.as_str(), e.net, e.time)).collect::<Vec<_>>(), vec![("routed:100", 7, 86400 * 100 + 50), ("routed:101", 5, 86400 * 101 + 1)]);
        let later = fold_routed(vec![r(86400 * 101 + 900, "d", 2)]);
        let all = merge_events(&ev, later);
        assert_eq!(all.iter().map(|e| (e.id.as_str(), e.net)).collect::<Vec<_>>(), vec![("routed:100", 7), ("routed:101", 7)], "a day's row grows; nothing is counted twice");
        let paid = Event { id: "h:out".into(), time: 5, net: -70, fee: 1, layer: "lightning".into(), txid: String::new(), hash: "h".into(), kind: "paid".into(), memo: String::new(), to: Vec::new(), height: 0, phantom: false };
        let again = merge_events(&[paid.clone()], vec![Event { net: -71, ..paid.clone() }]);
        assert_eq!(again.len(), 1); assert_eq!(again[0].net, -71, "a row seen again is replaced, not added");
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
