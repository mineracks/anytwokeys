//! Spark reader sessions. Protocol and authorization reviewed at buildonspark/spark
//! 0b3a32a05c9ac06cc411683551dd1f1bde9d0caa: protos/spark_authn.proto,
//! connection.ts authenticate(), wallet_setting_handler.go, authz/authz.go,
//! base_transfer_handler.go and coop_exit_handler.go. Master identity permits reads;
//! settings target the session's own identity; transfers/exits require owner identity
//! when operator authz is enabled. This is an operator-enforced permission, not a
//! cryptographic guarantee about all future servers. This module exposes no spend RPC.
use bitcoin::{bip32::{ChildNumber, Xpriv}, secp256k1::{ecdsa, Message, PublicKey, Secp256k1, SecretKey}};
use prost::Message as ProtoMessage;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::{Mutex, OnceLock, atomic::{AtomicU64, Ordering}}, time::{Duration, Instant}};
use zeroize::Zeroizing;
use crate::{cards, spark, wallet};
type R<T> = Result<T, String>;

// These field numbers/types follow the checked-in upstream protocol, not JSON.
#[derive(Clone, PartialEq, prost::Message)]
struct Challenge { #[prost(int32, tag="1")] version: i32, #[prost(int64, tag="2")] timestamp: i64, #[prost(bytes="vec", tag="3")] nonce: Vec<u8>, #[prost(bytes="vec", tag="4")] public_key: Vec<u8> }
#[derive(Clone, PartialEq, prost::Message)]
struct Protected { #[prost(int32, tag="1")] version: i32, #[prost(message, optional, tag="2")] challenge: Option<Challenge>, #[prost(bytes="vec", tag="3")] server_hmac: Vec<u8> }
#[derive(Clone, PartialEq, prost::Message)]
struct GetChallenge { #[prost(bytes="vec", tag="1")] public_key: Vec<u8> }
#[derive(Clone, PartialEq, prost::Message)]
struct ChallengeReply { #[prost(message, optional, tag="1")] protected_challenge: Option<Protected> }
#[derive(Clone, PartialEq, prost::Message)]
struct Verify { #[prost(message, optional, tag="1")] protected_challenge: Option<Protected>, #[prost(bytes="vec", tag="2")] signature: Vec<u8>, #[prost(bytes="vec", tag="3")] public_key: Vec<u8> }
#[derive(Clone, PartialEq, prost::Message)]
struct Verified { #[prost(string, tag="1")] session_token: String, #[prost(int64, tag="2")] expiration_timestamp: i64 }
#[derive(Clone, PartialEq, prost::Message)]
struct Setting { #[prost(bytes="vec", tag="1")] owner: Vec<u8>, #[prost(bool, tag="2")] private: bool, #[prost(bytes="vec", optional, tag="3")] master: Option<Vec<u8>> }
#[derive(Clone, PartialEq, prost::Message)]
struct SettingReply { #[prost(message, optional, tag="1")] wallet_setting: Option<Setting> }
#[derive(Clone, PartialEq, prost::Message)]
struct UpdateSetting { #[prost(bool, optional, tag="1")] private: Option<bool>, #[prost(bytes="vec", optional, tag="2")] master: Option<Vec<u8>> }

pub struct Identity(SecretKey);
impl Drop for Identity { fn drop(&mut self) { self.0.non_secure_erase(); } }
impl Identity {
    pub fn public(&self) -> PublicKey { self.0.public_key(&Secp256k1::new()) }
    fn sign(&self, digest: &[u8;32]) -> R<Vec<u8>> { Ok(Secp256k1::new().sign_ecdsa(&Message::from_digest(*digest), &self.0).serialize_der().to_vec()) }
}
struct Root(Xpriv);
impl Drop for Root { fn drop(&mut self) { self.0.private_key.non_secure_erase(); self.0.chain_code = [0u8;32].into(); } }
fn derive_path(words: &str, account: u32) -> R<Identity> {
    if account >= 1 << 31 { return Err("account index must be below 2147483648".into()); }
    let mut norm = Zeroizing::new(String::new());
    for word in words.split_whitespace() { if !norm.is_empty() { norm.push(' '); } for c in word.chars() { norm.push(c.to_ascii_lowercase()); } }
    let mnemonic = bip39::Mnemonic::parse_normalized(&norm).map_err(|_| "check the seed words and their checksum")?;
    let seed = Zeroizing::new(mnemonic.to_seed(""));
    drop(mnemonic);
    let mut root = Root(Xpriv::new_master(bitcoin::Network::Bitcoin, &seed[..]).map_err(|_| "could not derive the wallet")?);
    drop(seed);
    let secp = Secp256k1::new();
    for i in [spark::PURPOSE, account, 0] {
        let next = Root(root.0.derive_priv(&secp, &[ChildNumber::from_hardened_idx(i).map_err(|_| "invalid account")?]).map_err(|_| "could not derive the wallet")?);
        root = next; // previous private key and chain code erased on every step, including error paths
    }
    Ok(Identity(root.0.private_key))
}
/// Blitz recovery (3126b04ff7522558438ecaa5080d978cef04a8f9) first converts
/// 16 bytes of a derived key to a NEW mnemonic, then opens Spark account 1 of it.
/// It is not evidence that Wallet of Satoshi uses Blitz's namespaces.
pub fn derive_identity(words: &str, account: u32, scheme: &str) -> R<Identity> {
    match scheme {
        "direct" => derive_path(words, account),
        "blitz" => {
            let key = derive_path(words, account)?;
            let bytes = Zeroizing::new(key.0.secret_bytes()); drop(key);
            let mnemonic = bip39::Mnemonic::from_entropy(&bytes[..16]).map_err(|_| "could not derive the child wallet")?;
            drop(bytes);
            let phrase = Zeroizing::new(mnemonic.to_string()); drop(mnemonic);
            derive_path(&phrase, 1)
        }
        _ => Err("unknown wallet derivation scheme".into()),
    }
}

pub struct Session { pub operator: String, token: Zeroizing<String>, until: Instant, pub expires: i64, epoch: u64 }
impl Session { pub fn live(&self) -> bool { Instant::now() < self.until && self.epoch == EPOCH.load(Ordering::SeqCst) } pub(crate) fn token(&self) -> &str { &self.token } }
fn challenge_digest(p: &Protected, public: &[u8]) -> R<[u8;32]> {
    let c = p.challenge.as_ref().ok_or("Spark omitted its challenge")?;
    if p.version != 1 || c.version != 1 || c.timestamp <= 0 || c.nonce.len() != 32 || p.server_hmac.len() != 32 || c.public_key != public {
        return Err("Spark returned a challenge with the wrong identity, version or shape; nothing was signed".into());
    }
    Ok(Sha256::digest(c.encode_to_vec()).into())
}
fn authenticate(base: &str, public: &PublicKey, mut sign: impl FnMut(&[u8;32]) -> R<Vec<u8>>) -> R<Session> {
    const SERVICE: &str = "spark_authn.SparkAuthnService";
    let public = public.serialize().to_vec(); // current SDK uses compressed; server accepts both
    let started = Instant::now();
    let epoch = EPOCH.load(Ordering::SeqCst);
    let raw = spark::rpc(base, SERVICE, "get_challenge", GetChallenge { public_key: public.clone() }.encode_to_vec(), None)?;
    let p = ChallengeReply::decode(raw.as_slice()).map_err(|_| "invalid Spark challenge")?.protected_challenge.ok_or("Spark omitted its protected challenge")?;
    let digest = challenge_digest(&p, &public)?;
    let issued = p.challenge.as_ref().unwrap().timestamp;
    let signature = sign(&digest)?;
    // Verify locally as well, especially for a card swapped while a dialog was open.
    let sig = ecdsa::Signature::from_der(&signature).map_err(|_| "invalid reader signature")?;
    Secp256k1::new().verify_ecdsa(&Message::from_digest(digest), &sig, &PublicKey::from_slice(&public).unwrap()).map_err(|_| "the card signed with a different key")?;
    let raw = Zeroizing::new(spark::rpc(base, SERVICE, "verify_challenge", Verify { protected_challenge: Some(p), signature, public_key: public }.encode_to_vec(), None)?);
    let mut verified = Verified::decode(raw.as_slice()).map_err(|_| "invalid Spark session response")?;
    let token = Zeroizing::new(std::mem::take(&mut verified.session_token));
    let lifetime = verified.expiration_timestamp.checked_sub(issued).filter(|n| *n > 5).ok_or("Spark returned an expired session")?;
    let until = started.checked_add(Duration::from_secs((lifetime - 5) as u64)).ok_or("invalid session expiry")?;
    if token.is_empty() || token.len() > 16384 || !token.bytes().all(|b| b.is_ascii_graphic()) || until <= Instant::now() { return Err("Spark returned an unusable session".into()); }
    if epoch != EPOCH.load(Ordering::SeqCst) { return Err("the app locked while signing in to Spark".into()); }
    Ok(Session { operator: base.into(), token, until, expires: verified.expiration_timestamp, epoch })
}
fn owner_session(key: &Identity) -> R<Session> {
    let mut last = String::new();
    for base in spark::OPERATORS { match authenticate(base, &key.public(), |d| key.sign(d)) { Ok(s) => return Ok(s), Err(e) => last = e } }
    Err(last)
}
fn settings(s: &Session, owner: &[u8]) -> R<Setting> {
    let raw = spark::rpc(&s.operator, "spark.SparkService", "query_wallet_setting", vec![], Some(s))?;
    let setting = SettingReply::decode(raw.as_slice()).map_err(|_| "invalid wallet settings")?.wallet_setting.ok_or("Spark omitted wallet settings")?;
    if setting.owner != owner { return Err("Spark returned settings for a different wallet".into()); }
    Ok(setting)
}

#[derive(Clone, Serialize)]
pub struct Candidate { pub id: String, pub account: u32, pub scheme: String, pub identity: String, pub address: String, pub balance: u64, pub private: bool, pub existing_reader: String, pub network: String }
struct Pending { candidate: Candidate, owner: Session, until: Instant, epoch: u64, discovery: u64 }
struct Cached { session: std::sync::Arc<Session>, epoch: u64 }
#[derive(Default)]
struct Runtime { pending: HashMap<String, Pending>, readers: HashMap<String, Cached>, next_operator: HashMap<String, usize> }
static RUNTIME: OnceLock<Mutex<Runtime>> = OnceLock::new();
static EPOCH: AtomicU64 = AtomicU64::new(0);
static DISCOVERY: AtomicU64 = AtomicU64::new(0);
fn runtime() -> &'static Mutex<Runtime> { RUNTIME.get_or_init(|| Mutex::new(Runtime::default())) }
fn key_for(sp: &wallet::Spark, network: &str) -> String { format!("{network}:{}:{}:{}", sp.identity, sp.reader_pubkey, sp.reader_card) }
pub fn clear() { EPOCH.fetch_add(1, Ordering::SeqCst); if let Ok(mut r) = runtime().lock() { *r = Runtime::default(); } }
pub fn prune() { if let Ok(mut r) = runtime().lock() { r.pending.retain(|_, p| p.until > Instant::now() && p.owner.live()); r.readers.retain(|_, c| c.session.live()); } }
pub fn cancel_discovery() { DISCOVERY.fetch_add(1, Ordering::SeqCst); if let Ok(mut r) = runtime().lock() { r.pending.clear(); } }
pub fn session(sp: &wallet::Spark, network: &str) -> R<Option<std::sync::Arc<Session>>> {
    if !sp.authenticated { return Ok(None); }
    prune();
    let r = runtime().lock().map_err(|_| "Spark session lock failed")?;
    r.readers.get(&key_for(sp, network)).filter(|c| c.epoch == EPOCH.load(Ordering::SeqCst)).map(|c| Some(c.session.clone())).ok_or_else(|| "SPARK_UNLOCK: insert the registered YubiKey and unlock this wallet's reader".into())
}
pub fn read_result<T>(sp: &wallet::Spark, network: &str, result: R<T>) -> R<T> {
    if sp.authenticated && result.is_err() {
        let mut r = runtime().lock().map_err(|_| "Spark session lock failed")?;
        if let Some(c) = r.readers.remove(&key_for(sp, network)) {
            let n = spark::OPERATORS.iter().position(|b| *b == c.session.operator).unwrap_or(0);
            r.next_operator.insert(key_for(sp, network), (n + 1) % spark::OPERATORS.len());
        }
        return Err(format!("SPARK_UNLOCK: {}. Unlock again to retry with the next operator.", result.err().unwrap()));
    }
    result
}

#[derive(Serialize)]
pub struct Discovery { pub wallets: Vec<Candidate>, pub checked: u32, pub errors: Vec<String> }
pub fn discover(words: Zeroizing<String>, start: u32, count: u32, scheme: String, expected: String, network: String) -> R<Discovery> {
    wallet::load()?; // refuse when the app is locked
    if network != "mainnet" { return Err("authenticated Spark reads currently support mainnet only".into()); }
    if count == 0 || count > 20 || start.checked_add(count).filter(|n| *n <= 1 << 31).is_none() { return Err("scan 1–20 accounts at a time, below index 2147483648".into()); }
    let expected = if expected.trim().is_empty() { None } else if expected.trim().starts_with("spark") { let (id, n) = spark::identity_of(expected.trim())?; if n != network { return Err("the expected address is for another network".into()); } Some(hex::encode(id)) } else { Some(hex::encode(PublicKey::from_slice(&hex::decode(expected.trim()).map_err(|_| "expected identity must be a Spark address or public key")?).map_err(|_| "invalid expected public key")?.serialize())) };
    cancel_discovery();
    let discovery = DISCOVERY.load(Ordering::SeqCst);
    let epoch = EPOCH.load(Ordering::SeqCst);
    // Derive all keys before the first network request, then release words.
    let keys = (start..start + count).map(|a| derive_identity(&words, a, &scheme).map(|k| (a, k))).collect::<R<Vec<_>>>()?;
    drop(words);
    let mut report = Discovery { wallets: vec![], checked: 0, errors: vec![] };
    for (account, key) in keys {
        if epoch != EPOCH.load(Ordering::SeqCst) || discovery != DISCOVERY.load(Ordering::SeqCst) { return Err("Spark discovery was cancelled or the app locked".into()); }
        report.checked += 1;
        let public = key.public().serialize();
        let identity = hex::encode(public);
        if expected.as_ref().map(|e| e != &identity).unwrap_or(false) { continue; }
        crate::net::emit("spark-discovery", serde_json::json!({ "account": account, "checked": report.checked, "count": count }));
        let attempt: R<Option<Pending>> = (|| {
            let mut owner = owner_session(&key)?;
            drop(key);
            let balance = spark::balance_with(&public, &network, Some(&owner))?;
            // Positive ownership evidence before any setting can change. If empty,
            // history proves a used wallet, but an unused identity cannot be enrolled.
            if balance == 0 && spark::transfers_with(&public, &network, 1, 0, Some(&owner))?.0.is_empty() { return Ok(None); }
            let setting = settings(&owner, &public)?;
            use bitcoin::secp256k1::rand::{RngCore, rngs::OsRng};
            let mut nonce = [0u8;16]; OsRng.fill_bytes(&mut nonce);
            let c = Candidate { id: hex::encode(nonce), account, scheme: scheme.clone(), identity, address: spark::address_of(&public, &network)?, balance, private: setting.private, existing_reader: setting.master.map(hex::encode).unwrap_or_default(), network: network.clone() };
            let until = Instant::now() + Duration::from_secs(600);
            owner.until = owner.until.min(until);
            Ok(Some(Pending { candidate: c, owner, until, epoch, discovery }))
        })();
        match attempt {
            Ok(Some(p)) => {
                let mut r = runtime().lock().map_err(|_| "Spark session lock failed")?;
                if epoch != EPOCH.load(Ordering::SeqCst) || discovery != DISCOVERY.load(Ordering::SeqCst) { return Err("discovery was cancelled or the app locked".into()); }
                report.wallets.push(p.candidate.clone()); r.pending.insert(p.candidate.id.clone(), p);
            }
            Ok(None) => {},
            Err(e) => report.errors.push(format!("account {account}: {e}")),
        }
    }
    Ok(report)
}

fn checked_card(reader: &str, expected: &str) -> R<cards::CardInfo> {
    let c = cards::list().map_err(|e| e.to_string())?.into_iter().find(|c| c.reader == reader).ok_or("insert the selected YubiKey")?;
    if c.pubkey.as_deref() != Some(expected) { return Err("the selected card does not hold the expected reader key".into()); }
    if c.touch != "FIXED" { return Err("this reader requires touch locked to FIXED; configure the card before enrolling".into()); }
    Ok(c)
}
fn card_session(base: &str, c: &cards::CardInfo, pin: &str) -> R<Session> {
    let public = PublicKey::from_slice(&hex::decode(c.pubkey.as_ref().ok_or("card has no reader key")?).map_err(|_| "invalid card public key")?).map_err(|_| "invalid card public key")?;
    authenticate(base, &public, |digest| {
        let (r, s) = cards::sign_digest_checked(&c.reader, pin, digest, &public.serialize()).map_err(|e| e.to_string())?;
        let mut compact = [0u8;64]; compact[..32].copy_from_slice(&r); compact[32..].copy_from_slice(&s);
        Ok(ecdsa::Signature::from_compact(&compact).map_err(|_| "invalid card signature")?.serialize_der().to_vec())
    })
}
#[derive(Serialize)]
pub struct Unlocked { pub expires: i64, pub operator: String }
pub fn unlock(name: String, reader: String, pin: Zeroizing<String>) -> R<Unlocked> {
    let store = wallet::load()?;
    let v = store.vaults.iter().find(|v| v.name == name).ok_or("no such vault")?;
    let sp = v.spark.as_ref().filter(|s| s.authenticated).ok_or("this is not an enrolled Spark wallet")?;
    if let Some(s) = session(sp, &v.network).ok().flatten() { return Ok(Unlocked { expires: s.expires, operator: s.operator.clone() }); }
    let epoch = EPOCH.load(Ordering::SeqCst);
    let c = checked_card(&reader, &sp.reader_pubkey)?;
    if c.serial.to_string() != sp.reader_card { return Err("this is not the registered reader card".into()); }
    let k = key_for(sp, &v.network);
    let n = *runtime().lock().map_err(|_| "Spark session lock failed")?.next_operator.get(&k).unwrap_or(&0);
    // One touch attempt per click. Never automatically retry a wrong PIN.
    let opened = card_session(spark::OPERATORS[n], &c, &pin);
    drop(pin);
    let s = match opened { Ok(s) => s, Err(e) => { runtime().lock().map_err(|_| "Spark session lock failed")?.next_operator.insert(k, (n + 1) % spark::OPERATORS.len()); return Err(e); } };
    let result = Unlocked { expires: s.expires, operator: s.operator.clone() };
    let mut r = runtime().lock().map_err(|_| "Spark session lock failed")?;
    if epoch != EPOCH.load(Ordering::SeqCst) { return Err("the app locked while the reader was opening".into()); }
    r.readers.insert(k, Cached { session: std::sync::Arc::new(s), epoch });
    Ok(result)
}

pub fn enroll(candidate_id: String, name: String, reader: String, pubkey: String, pin: Zeroizing<String>, make_private: bool, replace_reader: bool) -> R<wallet::Vault> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 { return Err("give the wallet a name of up to 60 characters".into()); }
    let store = wallet::load()?;
    let p = runtime().lock().map_err(|_| "Spark session lock failed")?.pending.remove(&candidate_id).ok_or("discovery expired; enter the words and find the wallet again")?;
    if p.until <= Instant::now() || !p.owner.live() || (p.epoch != EPOCH.load(Ordering::SeqCst) || p.discovery != DISCOVERY.load(Ordering::SeqCst)) { return Err("discovery expired; find the wallet again".into()); }
    let c = checked_card(&reader, &pubkey)?;
    if pubkey == p.candidate.identity { return Err("the reader must be a different key from the wallet owner".into()); }
    if store.vaults.iter().any(|v| v.name == name && v.spark.as_ref().map(|s| s.identity.as_str()) != Some(&p.candidate.identity)) { return Err("that name belongs to another vault".into()); }
    // Touch and authenticate the actual card BEFORE changing the wallet.
    let reader_session = card_session(&p.owner.operator, &c, &pin)?;
    drop(pin);
    let owner = hex::decode(&p.candidate.identity).map_err(|_| "invalid discovered identity")?;
    let current = settings(&p.owner, &owner)?;
    let reader_key = hex::decode(&pubkey).map_err(|_| "invalid reader public key")?;
    if current.master.as_ref().map(|k| k != &reader_key).unwrap_or(false) && !replace_reader { return Err("this wallet already has a different designated reader; review and explicitly allow replacement".into()); }
    if current.master.as_ref().map(hex::encode).unwrap_or_default() != p.candidate.existing_reader { return Err("the designated reader changed since discovery; discover and review again".into()); }
    let balance = spark::balance_with(&owner, &p.candidate.network, Some(&p.owner))?;
    let history = spark::transfers_with(&owner, &p.candidate.network, 10, 0, Some(&p.owner))?.0;
    if balance == 0 && history.is_empty() { return Err("this identity has no visible funds or transfers; enrolment stopped".into()); }
    if p.epoch != EPOCH.load(Ordering::SeqCst) || p.discovery != DISCOVERY.load(Ordering::SeqCst) { return Err("the app locked; enrolment stopped".into()); }
    let update = UpdateSetting { private: if make_private { Some(true) } else { None }, master: Some(reader_key.clone()) };
    spark::rpc(&p.owner.operator, "spark.SparkService", "update_wallet_setting", update.encode_to_vec(), Some(&p.owner))
        .map_err(|e| format!("{e}. The operator may have applied the reader setting; discover again to check it before retrying."))?;
    let verify = (|| {
        let after = settings(&p.owner, &owner)?;
        if after.master.as_ref() != Some(&reader_key) || (make_private && !after.private) { return Err("Spark did not confirm the reader setting".into()); }
        let read_balance = spark::balance_with(&owner, &p.candidate.network, Some(&reader_session))?;
        let read_history = spark::transfers_with(&owner, &p.candidate.network, 10, 0, Some(&reader_session))?.0;
        if balance > 0 && read_balance == 0 || !history.is_empty() && read_history.is_empty() { return Err("the reader could not see the wallet's known funds/history".into()); }
        Ok(())
    })();
    drop(p.owner); // owner credential is never placed in the vault's reader-session cache
    verify.map_err(|e: String| format!("{e}. The reader setting was updated, but verification did not complete. Discover again to check and finish enrolment."))?;
    let mut store = wallet::load()?; // fresh store: do not overwrite concurrent edits
    if p.epoch != EPOCH.load(Ordering::SeqCst) || p.discovery != DISCOVERY.load(Ordering::SeqCst) { return Err("the app locked after updating Spark; discover again to finish saving".into()); }
    let sp = wallet::Spark { address: p.candidate.address.clone(), identity: p.candidate.identity.clone(), reader_pubkey: pubkey, reader_card: c.serial.to_string(), authenticated: true };
    let v = if let Some(v) = store.vaults.iter_mut().find(|v| v.network == p.candidate.network && v.spark.as_ref().map(|s| &s.identity) == Some(&sp.identity)) { v.spark = Some(sp.clone()); v.clone() } else {
        if store.vaults.iter().any(|v| v.name == name) { return Err("that vault name was used while enrolling; discover again to save under another name".into()); }
        let v = crate::new_spark_vault(name, sp.clone(), p.candidate.network.clone()); store.vaults.push(v.clone()); v
    };
    wallet::save(&store).map_err(|e| format!("Spark accepted the reader, but saving the vault failed: {e}. Discover again to finish saving."))?;
    let mut r = runtime().lock().map_err(|_| "Spark session lock failed")?;
    if p.epoch == EPOCH.load(Ordering::SeqCst) { r.readers.insert(key_for(&sp, &p.candidate.network), Cached { session: std::sync::Arc::new(reader_session), epoch: p.epoch }); }
    r.pending.clear();
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::{Read, Write}, net::TcpListener};
    const WORDS: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    fn identity() -> Identity { derive_identity(WORDS, 1, "direct").unwrap() }
    fn protected(public: Vec<u8>) -> Protected { Protected { version: 1, challenge: Some(Challenge { version: 1, timestamp: 1700000000, nonce: (0..32).collect(), public_key: public }), server_hmac: vec![42;32] } }
    type Step = Box<dyn Fn(String, Vec<u8>) -> (Vec<u8>, u8) + Send>;
    fn mock(steps: Vec<Step>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            for step in steps {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop { match listener.accept() { Ok((s,_)) => break s, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => { assert!(Instant::now() < deadline, "mock request timed out"); std::thread::sleep(Duration::from_millis(10)); }, Err(e) => panic!("{e}") } };
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") { let mut b = [0]; stream.read_exact(&mut b).unwrap(); head.push(b[0]); assert!(head.len() < 32768); }
                let headers = String::from_utf8(head).unwrap();
                let len: usize = headers.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length: ").map(str::to_owned)).unwrap().parse().unwrap();
                let mut framed = vec![0;len]; stream.read_exact(&mut framed).unwrap();
                assert_eq!(framed[0], 0); assert_eq!(u32::from_be_bytes(framed[1..5].try_into().unwrap()) as usize, framed.len()-5);
                let (body, status) = step(headers, framed[5..].to_vec());
                let trailers = format!("grpc-status: {status}\r\n");
                let mut response = vec![0]; response.extend_from_slice(&(body.len() as u32).to_be_bytes()); response.extend(body);
                response.push(0x80); response.extend_from_slice(&(trailers.len() as u32).to_be_bytes()); response.extend(trailers.as_bytes());
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/grpc-web+proto\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).unwrap();
                stream.write_all(&response).unwrap();
            }
        });
        (url, thread)
    }
    #[test]
    fn challenge_checks_identity_version_and_shape_before_signing() {
        let public = identity().public().serialize().to_vec();
        let p = protected(public.clone());
        let d = challenge_digest(&p, &public).unwrap();
        // Independent wire/hash fixture calculated from the published proto fields.
        assert_eq!(hex::encode(d), "1353b9fe056f2c937e754095111aa54c4c86c6543787dc0fa9ca4523de3afd64");
        let mut bad = p.clone(); bad.challenge.as_mut().unwrap().public_key[0] ^= 1;
        assert!(challenge_digest(&bad, &public).is_err());
        let mut bad = p.clone(); bad.version = 2; assert!(challenge_digest(&bad, &public).is_err());
        let mut bad = p.clone(); bad.challenge.as_mut().unwrap().nonce.pop(); assert!(challenge_digest(&bad, &public).is_err());
        let mut bad = p; bad.server_hmac.clear(); assert!(challenge_digest(&bad, &public).is_err());
    }
    #[test]
    fn blitz_child_seed_is_not_a_direct_spark_account() {
        // Fixtures from BlitzWallet/spark-recover derivedWallets.test.js.
        for (index, child) in [(4, "name exile social van valid whale behind danger project language coast tiger"), (200000, "right monkey model escape peasant change puzzle today able helmet early change"), (100000, "soul ridge oven yellow truck coral document ski fitness swarm galaxy pony"), (1001, "other october draft boring leopard sail mechanic peace jump athlete three tumble"), (300000, "gentle hurt salmon wife wish aware outdoor furnace dutch pistol wing palm")] {
            assert_eq!(derive_identity(WORDS, index, "blitz").unwrap().public(), derive_identity(child, 1, "direct").unwrap().public());
            assert_ne!(derive_identity(WORDS, index, "blitz").unwrap().public(), derive_identity(WORDS, index, "direct").unwrap().public());
        }
        assert!(derive_identity(WORDS, 1 << 31, "direct").is_err());
        assert!(derive_identity(WORDS, 1, "unknown").is_err());
    }
    #[test]
    fn software_handshake_and_authenticated_reads_use_one_operator_and_bearer() {
        let key = identity(); let public = key.public().serialize().to_vec();
        let expected = public.clone();
        let (base, worker) = mock(vec![
            Box::new(move |head, req| {
                assert!(head.starts_with("POST /spark_authn.SparkAuthnService/get_challenge "));
                assert!(!head.to_lowercase().contains("authorization:"));
                let req = GetChallenge::decode(req.as_slice()).unwrap(); assert_eq!(req.public_key, expected);
                (ChallengeReply { protected_challenge: Some(protected(req.public_key)) }.encode_to_vec(), 0)
            }),
            Box::new(move |head, req| {
                assert!(head.starts_with("POST /spark_authn.SparkAuthnService/verify_challenge "));
                let req = Verify::decode(req.as_slice()).unwrap();
                let p = req.protected_challenge.unwrap(); assert_eq!(p.server_hmac, vec![42;32]);
                let digest = Sha256::digest(p.challenge.unwrap().encode_to_vec());
                let sig = ecdsa::Signature::from_der(&req.signature).unwrap();
                Secp256k1::new().verify_ecdsa(&Message::from_digest(digest.into()), &sig, &PublicKey::from_slice(&req.public_key).unwrap()).unwrap();
                (Verified { session_token: "test-reader-token".into(), expiration_timestamp: 1700000300 }.encode_to_vec(), 0)
            }),
            Box::new(|head, _| {
                assert!(head.starts_with("POST /spark.SparkService/query_nodes "));
                assert!(head.to_lowercase().contains("authorization: bearer test-reader-token\r\n"));
                // QueryNodesResponse: one map entry with a TreeNode value of 512 sats; offset -1.
                (vec![10,10,10,1,b'x',18,5,24,128,4,32,0,16,255,255,255,255,255,255,255,255,255,1], 0)
            }),
            Box::new(|head, _| {
                assert!(head.to_lowercase().contains("authorization: bearer test-reader-token\r\n"));
                (vec![], 16) // authenticated rejection must NOT retry anonymously
            }),
        ]);
        let s = authenticate(&base, &key.public(), |d| key.sign(d)).unwrap();
        assert!(s.live()); assert_eq!(s.expires, 1700000300);
        assert_eq!(spark::balance_with(&public, "mainnet", Some(&s)).unwrap(), 512);
        let err = spark::balance_with(&public, "mainnet", Some(&s)).unwrap_err();
        assert!(err.contains("RPC status 16"));
        worker.join().unwrap();
    }
    #[test]
    fn malformed_challenge_never_reaches_signer() {
        let key = identity();
        let (base, worker) = mock(vec![Box::new(|_,_| (ChallengeReply { protected_challenge: Some(protected(vec![0;33])) }.encode_to_vec(),0))]);
        let mut calls = 0;
        assert!(authenticate(&base, &key.public(), |_| { calls += 1; Err("must not sign".into()) }).is_err());
        assert_eq!(calls, 0); worker.join().unwrap();
    }
    #[test]
    fn absent_or_expired_reader_never_becomes_anonymous_and_lock_drops_credentials() {
        clear();
        let sp = wallet::Spark { identity: hex::encode(identity().public().serialize()), address: "spark1test".into(), reader_pubkey: "reader".into(), reader_card: "123".into(), authenticated: true };
        assert!(session(&sp,"mainnet").is_err());
        let epoch = EPOCH.load(Ordering::SeqCst);
        runtime().lock().unwrap().readers.insert(key_for(&sp, "mainnet"), Cached { session: std::sync::Arc::new(Session { operator: spark::OPERATORS[0].into(), token: Zeroizing::new("not-persisted".into()), until: Instant::now() + Duration::from_secs(30), expires: 1, epoch }), epoch });
        assert!(session(&sp,"mainnet").unwrap().is_some());
        assert!(session(&sp,"signet").is_err());
        assert!(!serde_json::to_string(&sp).unwrap().contains("not-persisted"));
        assert!(read_result::<()>(&sp, "mainnet", Err("Spark RPC status 16".into())).unwrap_err().starts_with("SPARK_UNLOCK:"));
        assert_eq!(runtime().lock().unwrap().next_operator[&key_for(&sp,"mainnet")], 1);
        assert!(session(&sp,"mainnet").is_err());
        runtime().lock().unwrap().readers.insert(key_for(&sp,"mainnet"), Cached { session: std::sync::Arc::new(Session { operator: spark::OPERATORS[0].into(), token: Zeroizing::new("expired".into()), until: Instant::now(), expires: 1, epoch }), epoch });
        assert!(session(&sp,"mainnet").is_err());
        clear(); assert!(runtime().lock().unwrap().readers.is_empty());
        let stale = Session { operator: "unused".into(), token: Zeroizing::new("stale".into()), until: Instant::now() + Duration::from_secs(60), expires: 1, epoch };
        assert!(!stale.live(), "a session held by an in-flight reader cannot issue more requests after lock");
        let legacy: wallet::Spark = serde_json::from_str(r#"{"address":"spark1test","identity":"02"}"#).unwrap();
        assert!(!legacy.authenticated); assert!(session(&legacy,"mainnet").unwrap().is_none());
    }
    #[test]
    fn enrollment_setting_preserves_privacy_unless_requested() {
        assert_eq!(UpdateSetting { private: None, master: Some(vec![2;33]) }.encode_to_vec()[0], 18);
        assert_eq!(&UpdateSetting { private: Some(true), master: Some(vec![2;33]) }.encode_to_vec()[..2], &[8,1]);
    }
    fn page(ids: &[u8], next: i64) -> Vec<u8> {
        let mut out = Vec::new();
        for id in ids { let t = [10,1,*id,32,5,40,100,80,2]; out.extend_from_slice(&[10,t.len() as u8]); out.extend_from_slice(&t); }
        out.push(16); let mut n = next as u64; loop { let b = (n & 127) as u8; n >>= 7; out.push(if n > 0 { b | 128 } else { b }); if n == 0 { break; } }
        out
    }
    #[test]
    fn history_pages_directions_independently_and_deduplicates() {
        #[derive(Clone, PartialEq, prost::Message)]
        struct Query { #[prost(bytes="vec", tag="1")] receiver: Vec<u8>, #[prost(bytes="vec", tag="2")] sender: Vec<u8>, #[prost(uint64, tag="50")] offset: u64 }
        let (base, worker) = mock(vec![
            Box::new(|_,body| { let q = Query::decode(body.as_slice()).unwrap(); assert!(!q.sender.is_empty()); assert_eq!(q.offset,0); (page(b"A",1),0) }),
            Box::new(|_,body| { let q = Query::decode(body.as_slice()).unwrap(); assert!(!q.sender.is_empty()); assert_eq!(q.offset,1); (page(b"B",-1),0) }),
            Box::new(|_,body| { let q = Query::decode(body.as_slice()).unwrap(); assert!(!q.receiver.is_empty()); assert_eq!(q.offset,0); (page(b"AC",-1),0) }),
        ]);
        let s = Session { operator: base, token: Zeroizing::new("reader".into()), until: Instant::now()+Duration::from_secs(30), expires:1, epoch:EPOCH.load(Ordering::SeqCst) };
        let rows = spark::history_with(&identity().public().serialize(),"mainnet",Some(&s)).unwrap();
        assert_eq!(rows.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),vec!["A","B","C"]); worker.join().unwrap();
    }
    #[test]
    fn repeated_history_cursor_fails_instead_of_returning_partial_accounts() {
        let (base, worker) = mock(vec![Box::new(|_,_| (page(b"A",0),0))]);
        let s = Session { operator: base, token: Zeroizing::new("reader".into()), until: Instant::now()+Duration::from_secs(30), expires:1, epoch:EPOCH.load(Ordering::SeqCst) };
        assert!(spark::history_with(&identity().public().serialize(),"mainnet",Some(&s)).unwrap_err().contains("incomplete accounting")); worker.join().unwrap();
    }
    /// Read-only smoke test with a fresh random identity: establishes handshake
    /// interoperability only, NOT funded-wallet or YubiKey authorization correctness.
    #[test] #[ignore]
    fn live_ephemeral_authentication_smoke() {
        let key = Identity(SecretKey::new(&mut bitcoin::secp256k1::rand::thread_rng()));
        let session = owner_session(&key).expect("published authentication protocol should work");
        assert!(session.live());
        let public = key.public().serialize(); drop(key);
        assert_eq!(spark::balance_with(&public, "mainnet", Some(&session)).unwrap(), 0);
        println!("Spark authentication and authenticated query_nodes succeeded at {}", session.operator);
    }
}
