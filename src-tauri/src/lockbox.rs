//! Everything this app keeps on this computer — the vaults and their descriptors, labels, notes, contacts, the seal
//! shares, the price cache, every attachment — is sealed under one random STORE KEY. The key is never on disk in the
//! clear. It sits in KEYSLOTS, each of which can give it up on its own:
//!
//!   - this Mac: a Keychain item only this signed app may read. With "ask for Touch ID" on, the app asks the system
//!     for a fingerprint (or the Mac's password) before it reads the item.
//!   - each YubiKey that has been added: the store key wrapped under a key derived from that card's PIN-protected
//!     secret (OpenPGP private object 0103, the same one that locks its seal share). The key plugged in, plus its
//!     PIN, opens the app — on a Mac with no Touch ID, or when the Keychain item is gone.
//!
//! A file:  "A2KS\x01" ‖ 24-byte nonce ‖ XChaCha20-Poly1305 ciphertext, with the file's ROLE as associated data, so a
//! sealed cache cannot be passed off as a sealed store.
//!
//! While no key is in memory the app is LOCKED: every read of the store is an error, the watcher does nothing, and
//! nothing is saved. A store that fails to open is an error too — never an empty store that the next save would
//! write over the real one.
//!
//! Tests and harnesses never touch the Keychain: `A2K_STORE_KEY` (64 hex) supplies the key.

use chacha20poly1305::{aead::{Aead, KeyInit, Payload}, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub const MAGIC: &[u8; 5] = b"A2KS\x01";
pub const LOCKED: &str = "locked: open Any Two Keys with Touch ID or one of your keys first";
const SLOT_INFO: &[u8] = b"a2k-store-slot-v1";

static KEY: RwLock<Option<[u8; 32]>> = RwLock::new(None);

pub fn random<const N: usize>() -> [u8; N] {
    use bitcoin::secp256k1::rand::RngCore;
    let mut b = [0u8; N];
    bitcoin::secp256k1::rand::rngs::OsRng.fill_bytes(&mut b);
    b
}

// ---------------------------------------------------------------- the box --

pub fn seal_with(key: &[u8; 32], role: &str, plain: &[u8], nonce: [u8; 24]) -> Vec<u8> {
    let ct = XChaCha20Poly1305::new(key.into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: role.as_bytes() }).expect("XChaCha20-Poly1305 encryption has no failing input");
    [MAGIC.as_slice(), &nonce, &ct].concat()
}

pub fn open_with(key: &[u8; 32], role: &str, blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < MAGIC.len() + 24 + 16 || &blob[..MAGIC.len()] != MAGIC { return Err("not a sealed Any Two Keys file".into()); }
    let (nonce, ct) = blob[MAGIC.len()..].split_at(24);
    XChaCha20Poly1305::new(key.into()).decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: role.as_bytes() })
        .map_err(|_| "it did not open with this computer's store key (wrong key, or the file was changed)".to_string())
}

/// The key in memory, or the one a test supplied. `Err(LOCKED)` otherwise.
pub fn key() -> Result<[u8; 32], String> {
    if let Some(k) = *KEY.read().map_err(|_| "store key lock poisoned".to_string())? { return Ok(k); }
    if let Ok(h) = std::env::var("A2K_STORE_KEY") {
        let b = hex::decode(h.trim()).map_err(|_| "A2K_STORE_KEY is not hex".to_string())?;
        return b.try_into().map_err(|_| "A2K_STORE_KEY is not 32 bytes".to_string());
    }
    Err(LOCKED.into())
}
pub fn unlocked() -> bool { key().is_ok() }
pub fn lock() { if let Ok(mut k) = KEY.write() { if let Some(old) = k.as_mut() { old.fill(0); } *k = None; } crate::attach::clear_opened(); }
fn hold(k: [u8; 32]) { if let Ok(mut g) = KEY.write() { *g = Some(k); } }

// ------------------------------------------------------------------ files --

pub fn dir() -> PathBuf {
    // a test never looks at this computer's real folder: what is or is not sealed there is none of its business
    #[cfg(test)] let home = std::env::temp_dir().join("a2k-tests-empty");
    #[cfg(not(test))] let home = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("Any Two Keys");
    let dir = match std::env::var_os("A2K_DATA_DIR") { Some(d) => PathBuf::from(d), None => home };
    std::fs::create_dir_all(&dir).ok();
    dir
}
fn slots_path() -> PathBuf { dir().join("slots.json") }

/// This computer seals what it keeps: there is a keyslot file, or a sealed store (the file that matters most) exists.
pub fn sealed_mode() -> bool { slots_path().exists() || dir().join("store.enc").exists() }

fn write_atomic(p: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = p.with_extension(format!("{}.tmp", p.extension().and_then(|e| e.to_str()).unwrap_or("x")));
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)); }
    std::fs::rename(&tmp, p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Seal `plain` into `path` and PROVE it before returning: the file is read back and opened, and must give the same
/// bytes. Everything that replaces a readable file with a sealed one goes through here.
pub fn write_sealed(path: &Path, role: &str, plain: &[u8]) -> Result<(), String> {
    let k = key()?;
    write_atomic(path, &seal_with(&k, role, plain, random()))?;
    let back = open_with(&k, role, &std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)?;
    if back != plain { return Err(format!("{}: what was written did not read back the same", path.display())); }
    Ok(())
}

/// `Ok(None)`: no such file. `Err`: there is one and it cannot be opened (locked, damaged, wrong key).
pub fn read_sealed(path: &Path, role: &str) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
        Ok(blob) => open_with(&key()?, role, &blob).map(Some).map_err(|e| format!("{}: {e}", path.display())),
    }
}

/// The name an attachment has on disk: not its content hash (which anyone holding the same file could look for), but
/// a keyed hash of it.
pub fn blob_name(sha256: &str) -> Result<String, String> {
    use bitcoin::hashes::{sha256, Hash, HashEngine, Hmac, HmacEngine};
    let mut e = HmacEngine::<sha256::Hash>::new(&key()?);
    e.input(b"a2k-attachment-name-v1"); e.input(sha256.as_bytes());
    Ok(hex::encode(&Hmac::<sha256::Hash>::from_engine(e).to_byte_array()[..20]))
}

// ------------------------------------------------------------------ slots --

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct CardSlot { pub pubkey: String, pub name: String, pub nonce: String, pub wrapped: String, pub added: u64 }

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Slots {
    pub v: u8,
    /// random, public: salts the card slots so a slot from one computer means nothing on another
    pub id: String,
    /// ask the system for Touch ID (or the Mac's password) before reading the Keychain item
    #[serde(default = "yes")] pub touch: bool,
    /// lock again after this many idle minutes; 0 = stay open until the app quits
    #[serde(default)] pub idle_minutes: u32,
    #[serde(default)] pub cards: Vec<CardSlot>,
    /// every readable file has been sealed and removed
    #[serde(default)] pub migrated: bool,
}
fn yes() -> bool { true }

pub fn slots() -> Result<Option<Slots>, String> {
    match std::fs::read(slots_path()) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("slots.json: {e}")),
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|e| format!("slots.json is damaged: {e}")),
    }
}
pub fn save_slots(s: &Slots) -> Result<(), String> { write_atomic(&slots_path(), &serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?) }

fn wrap_key(id: &str, card_secret: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(id.as_bytes()), card_secret).expand(SLOT_INFO, &mut out).expect("32 bytes is a valid HKDF-SHA256 length");
    out
}
fn slot_aad(id: &str, pubkey: &str) -> Vec<u8> { [SLOT_INFO, b"|", id.as_bytes(), b"|", pubkey.as_bytes()].concat() }

pub fn wrap_for_card(id: &str, pubkey: &str, name: &str, card_secret: &[u8; 32], store_key: &[u8; 32], nonce: [u8; 24], now: u64) -> CardSlot {
    let ct = XChaCha20Poly1305::new((&wrap_key(id, card_secret)).into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: store_key, aad: &slot_aad(id, pubkey) }).expect("encryption has no failing input");
    CardSlot { pubkey: pubkey.into(), name: name.into(), nonce: hex::encode(nonce), wrapped: hex::encode(ct), added: now }
}
pub fn unwrap_from_card(id: &str, slot: &CardSlot, card_secret: &[u8; 32]) -> Result<[u8; 32], String> {
    let nonce = hex::decode(&slot.nonce).map_err(|_| "slot nonce is not hex".to_string())?;
    let ct = hex::decode(&slot.wrapped).map_err(|_| "slot is not hex".to_string())?;
    if nonce.len() != 24 { return Err("slot nonce is the wrong length".into()); }
    let k = XChaCha20Poly1305::new((&wrap_key(id, card_secret)).into()).decrypt(XNonce::from_slice(&nonce), Payload { msg: &ct, aad: &slot_aad(id, &slot.pubkey) })
        .map_err(|_| "this key's secret does not open its slot (the key was resealed or reset since it was added)".to_string())?;
    k.try_into().map_err(|_| "the slot did not hold a 32-byte key".to_string())
}

/// A candidate key is only trusted once it has opened the store itself.
fn proves(k: &[u8; 32]) -> Result<(), String> {
    match std::fs::read(dir().join("store.enc")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),   // nothing sealed yet (mid-migration): nothing to disprove it
        Err(e) => Err(format!("store.enc: {e}")),
        Ok(blob) => open_with(k, "store", &blob).map(|_| ()).map_err(|_| "that key does not open this computer's store".to_string()),
    }
}

// ----------------------------------------------------------------- unlock --

/// With this Mac: Touch ID (or the Mac's password) first when that is on, then the Keychain item.
pub fn unlock_mac(reason: &str) -> Result<(), String> {
    let touch = slots()?.map(|s| s.touch).unwrap_or(true);
    if touch { mac::presence(reason)?; }
    let k = mac::get()?.ok_or("this Mac's Keychain has no key for Any Two Keys. Open it with one of your keys instead.")?;
    proves(&k)?;
    hold(k);
    Ok(())
}

/// With a key: `card_secret` is what the plugged-in card gave up after its PIN.
pub fn unlock_card(pubkey: &str, card_secret: &[u8; 32]) -> Result<(), String> {
    let s = slots()?.ok_or("this computer has no keyslots yet")?;
    let slot = s.cards.iter().find(|c| c.pubkey == pubkey).ok_or("that key has not been added as a way to open this app")?;
    let k = unwrap_from_card(&s.id, slot, card_secret)?;
    proves(&k)?;
    hold(k);
    // the Keychain item may be what was lost: put it back, quietly
    if mac::get().ok().flatten() != Some(k) { let _ = mac::set(&k); }
    Ok(())
}

pub fn add_card(pubkey: &str, name: &str, card_secret: &[u8; 32], now: u64) -> Result<Slots, String> {
    let k = key()?;
    let mut s = slots()?.ok_or("this computer has no keyslots yet")?;
    let slot = wrap_for_card(&s.id, pubkey, name, card_secret, &k, random(), now);
    if unwrap_from_card(&s.id, &slot, card_secret)? != k { return Err("the new slot did not give the key back".into()); }
    s.cards.retain(|c| c.pubkey != pubkey);
    s.cards.push(slot);
    save_slots(&s)?;
    Ok(s)
}

// -------------------------------------------------------------- migration --

#[derive(serde::Serialize, Default, Debug)]
pub struct Migration { pub store: bool, pub cache: bool, pub attachments: usize, pub left: Vec<String> }

/// Seal everything readable that this app keeps. Safe to run again after a crash at any point: each step proves the
/// sealed copy (read back, opened, compared) before the readable one is removed, and a failure leaves the rest as it was.
pub fn migrate() -> Result<Migration, String> {
    let d = dir();
    let mut s = match slots()? { Some(s) => s, None => {
        // the key exists in the Keychain — and has been read back from it — before a single file is sealed under it
        if std::env::var_os("A2K_STORE_KEY").is_none() {
            let k = match mac::get()? { Some(k) => k, None => { let k: [u8; 32] = random(); mac::set(&k)?; k } };
            if mac::get()? != Some(k) { return Err("the Keychain did not give back the key that was just stored in it; nothing was changed".into()); }
            proves(&k)?;
            hold(k);
        }
        let s = Slots { v: 1, id: hex::encode(random::<8>()), touch: true, idle_minutes: 0, cards: vec![], migrated: false };
        save_slots(&s)?; s
    } };
    let mut m = Migration::default();
    if s.migrated { return Ok(m); }
    key()?;
    let plain = d.join("store.json");
    if plain.exists() {
        let bytes = std::fs::read(&plain).map_err(|e| format!("store.json: {e}"))?;
        crate::wallet::parse_store(&bytes).map_err(|e| format!("store.json is damaged and was left alone: {e}"))?;
        if !d.join("store.enc").exists() { write_sealed(&d.join("store.enc"), "store", &bytes)?; }
        // what is sealed must hold everything the readable file holds before that file goes
        let sealed = read_sealed(&d.join("store.enc"), "store")?.ok_or("store.enc vanished")?;
        let (a, b): (serde_json::Value, serde_json::Value) = (serde_json::from_slice(&bytes).map_err(|e| e.to_string())?, serde_json::from_slice(&sealed).map_err(|e| e.to_string())?);
        if a != b { return Err("store.enc exists but does not match store.json; both were left alone".into()); }
        std::fs::remove_file(&plain).map_err(|e| format!("store.json: {e}"))?;
        m.store = true;
    }
    let cache = d.join("cache.json");
    if cache.exists() {
        let bytes = std::fs::read(&cache).map_err(|e| format!("cache.json: {e}"))?;
        write_sealed(&d.join("cache.enc"), "cache", &bytes)?;
        std::fs::remove_file(&cache).map_err(|e| format!("cache.json: {e}"))?;
        m.cache = true;
    }
    let att = d.join("attachments");
    if let Ok(rd) = std::fs::read_dir(&att) {
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            if !p.is_file() || name.starts_with('.') { continue; }
            // readable blobs are named by their 64-hex content hash; sealed ones by a 40-hex keyed name
            if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) { if name.len() != 40 { m.left.push(name); } continue; }
            let bytes = std::fs::read(&p).map_err(|e| format!("{name}: {e}"))?;
            write_sealed(&att.join(blob_name(&name)?), "att", &bytes)?;
            std::fs::remove_file(&p).map_err(|e| format!("{name}: {e}"))?;
            m.attachments += 1;
        }
    }
    let _ = std::fs::remove_file(d.join("watch.log"));   // it named vaults and amounts; the new one does not
    s.migrated = true;
    save_slots(&s)?;
    Ok(m)
}

// -------------------------------------------------------------------- mac --

#[cfg(target_os = "macos")]
pub mod mac {
    const SERVICE: &str = "com.anytwokeys.app.store-key";
    const ACCOUNT: &str = "store key";
    // errSecItemNotFound
    const NOT_FOUND: i32 = -25300;

    pub fn get() -> Result<Option<[u8; 32]>, String> {
        if std::env::var_os("A2K_STORE_KEY").is_some() || std::env::var_os("A2K_NO_KEYCHAIN").is_some() { return Ok(None); }
        match security_framework::passwords::get_generic_password(SERVICE, ACCOUNT) {
            Ok(b) => b.try_into().map(Some).map_err(|_| "the Keychain item for Any Two Keys is not a 32-byte key".to_string()),
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(format!("the Keychain would not give up the key for Any Two Keys: {e}")),
        }
    }
    pub fn set(k: &[u8; 32]) -> Result<(), String> {
        if std::env::var_os("A2K_NO_KEYCHAIN").is_some() { return Err("the Keychain is switched off for this run".into()); }
        security_framework::passwords::set_generic_password(SERVICE, ACCOUNT, k).map_err(|e| format!("the Keychain would not store the key: {e}"))
    }

    /// Ask the system to confirm someone entitled is here: Touch ID, an unlocked Apple Watch, or the Mac's password.
    /// This is a gate the app puts in front of its own Keychain read, not something the Keychain enforces: that needs
    /// the data-protection keychain, which a Developer ID build without a provisioning profile cannot use.
    pub fn presence(reason: &str) -> Result<(), String> {
        use block2::RcBlock;
        use objc2::runtime::Bool;
        use objc2_foundation::{NSError, NSString};
        use objc2_local_authentication::{LAContext, LAPolicy};
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        unsafe {
            let ctx = LAContext::new();
            if let Err(e) = ctx.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication) { return Err(format!("this Mac cannot ask for Touch ID or a password: {}", e.localizedDescription())); }
            let reply = RcBlock::new(move |ok: Bool, err: *mut NSError| {
                let r = if ok.as_bool() { Ok(()) } else if err.is_null() { Err("not confirmed".to_string()) } else { Err((*err).localizedDescription().to_string()) };
                let _ = tx.send(r);
            });
            ctx.evaluatePolicy_localizedReason_reply(LAPolicy::DeviceOwnerAuthentication, &NSString::from_str(reason), &reply);
            rx.recv().map_err(|_| "the system never answered".to_string())?
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub mod mac {
    pub fn get() -> Result<Option<[u8; 32]>, String> { Ok(None) }
    pub fn set(_: &[u8; 32]) -> Result<(), String> { Err("no system keychain on this platform yet: add one of your keys instead".into()) }
    pub fn presence(_: &str) -> Result<(), String> { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole move, on a copy of a store in a scratch folder, with the key from the environment (never the Keychain).
    #[test]
    fn sealing_a_live_store_loses_nothing_and_leaves_nothing_readable() {
        let d = std::env::temp_dir().join(format!("a2k-lockbox-{}", hex::encode(random::<6>())));
        std::fs::create_dir_all(d.join("attachments")).unwrap();
        std::env::set_var("A2K_DATA_DIR", &d);
        std::env::set_var("A2K_STORE_KEY", "11".repeat(32));
        let store = { let mut st = crate::wallet::Store::default(); st.settings.network = "mainnet".into();
            st.contacts.push(crate::wallet::Contact { name: "CANARY-CONTACT".into(), address: "bc1qcanary".into(), network: "mainnet".into(), added: 1, used: 3, last_used: 0 });
            let mut v = serde_json::to_value(&st).unwrap(); v["a_field_this_build_does_not_know"] = serde_json::json!({ "keep": "CANARY-UNKNOWN" }); v.to_string() };
        let store = store.as_str();
        std::fs::write(d.join("store.json"), store).unwrap();
        std::fs::write(d.join("cache.json"), br#"{"CANARY-CACHE":1}"#).unwrap();
        let receipt = b"%PDF-1.7 CANARY-RECEIPT".to_vec();
        let hash = { use bitcoin::hashes::Hash; hex::encode(bitcoin::hashes::sha256::Hash::hash(&receipt)) };
        std::fs::write(d.join("attachments").join(&hash), &receipt).unwrap();
        std::fs::write(d.join("attachments").join("not-ours.txt"), b"left alone").unwrap();
        std::fs::write(d.join("watch.log"), b"08:00:00 CANARY-VAULT: 9171 sats arrived").unwrap();
        assert!(!sealed_mode());
        assert_eq!(crate::wallet::load().unwrap().contacts[0].name, "CANARY-CONTACT");

        let m = migrate().unwrap();
        assert!(m.store && m.cache && m.attachments == 1 && m.left == vec!["not-ours.txt".to_string()], "{m:?}");
        assert!(sealed_mode() && slots().unwrap().unwrap().migrated);
        assert!(!d.join("store.json").exists() && !d.join("cache.json").exists() && !d.join("attachments").join(&hash).exists() && !d.join("watch.log").exists());
        // nothing is lost: the same JSON, fields this build does not know included; the attachment, byte for byte
        let back: serde_json::Value = serde_json::from_slice(&read_sealed(&d.join("store.enc"), "store").unwrap().unwrap()).unwrap();
        assert_eq!(back, serde_json::from_str::<serde_json::Value>(store).unwrap());
        assert_eq!(crate::wallet::load().unwrap().contacts[0].name, "CANARY-CONTACT");
        assert_eq!(crate::attach::small(&hash).unwrap(), receipt);
        assert!(crate::attach::have(&hash));
        // nothing readable is left: no canary in any file, no file named by a content hash
        fn walk(p: &Path, out: &mut Vec<PathBuf>) { for e in std::fs::read_dir(p).unwrap().flatten() { let p = e.path(); if p.is_dir() { walk(&p, out) } else { out.push(p) } } }
        let mut files = Vec::new(); walk(&d, &mut files);
        for f in &files {
            if f.ends_with("not-ours.txt") { continue; }
            let b = std::fs::read(f).unwrap();
            assert!(!b.windows(6).any(|w| w == b"CANARY"), "{} is readable", f.display());
            assert!(!f.to_string_lossy().contains(&hash), "{} is named by its content", f.display());
        }
        // running it again changes nothing
        let again = migrate().unwrap(); assert!(!again.store && again.attachments == 0);
        // a save goes to the sealed file, and a readable store planted afterwards is neither read nor obeyed
        let mut s = crate::wallet::load().unwrap(); s.contacts[0].used = 4; crate::wallet::save(&s).unwrap();
        std::fs::write(d.join("store.json"), store.replace("CANARY-CONTACT", "PLANTED")).unwrap();
        let s = crate::wallet::load().unwrap();
        assert!(s.contacts.len() == 1 && s.contacts[0].name == "CANARY-CONTACT" && s.contacts[0].used == 4, "a planted store.json must not be read once the move is finished");
        // …not even with the sealed store taken away, which is how someone who can write files would try it
        std::fs::rename(d.join("store.enc"), d.join("store.enc.aside")).unwrap();
        assert!(crate::wallet::load().unwrap().contacts.is_empty(), "with the sealed store gone the app is empty, not the planted store's");
        std::fs::rename(d.join("store.enc.aside"), d.join("store.enc")).unwrap();
        std::fs::remove_file(d.join("store.json")).unwrap();
        // the wrong key: an error, never an empty store — and a save with it must not get to write over the real one
        let sealed_before = std::fs::read(d.join("store.enc")).unwrap();
        std::env::set_var("A2K_STORE_KEY", "22".repeat(32));
        assert!(crate::wallet::load().is_err());
        assert!(crate::wallet::update(|s| s.contacts.clear()).is_err());
        assert_eq!(std::fs::read(d.join("store.enc")).unwrap(), sealed_before);
        std::env::set_var("A2K_STORE_KEY", "11".repeat(32));

        // a crash between sealing and removing: both files there, same content → the second run finishes the job
        let d2 = d.join("crash"); std::fs::create_dir_all(&d2).unwrap(); std::env::set_var("A2K_DATA_DIR", &d2);
        std::fs::write(d2.join("store.json"), store).unwrap();
        save_slots(&Slots { v: 1, id: "0011223344556677".into(), touch: true, idle_minutes: 0, cards: vec![], migrated: false }).unwrap();
        write_sealed(&d2.join("store.enc"), "store", store.as_bytes()).unwrap();
        assert!(crate::wallet::save(&crate::wallet::load().unwrap()).is_err(), "nothing is saved while the move is half done");
        assert!(migrate().unwrap().store && !d2.join("store.json").exists());
        // …and if the two DIFFER, neither is touched
        let d3 = d.join("differ"); std::fs::create_dir_all(&d3).unwrap(); std::env::set_var("A2K_DATA_DIR", &d3);
        std::fs::write(d3.join("store.json"), store).unwrap();
        save_slots(&Slots { v: 1, id: "0011223344556677".into(), touch: true, idle_minutes: 0, cards: vec![], migrated: false }).unwrap();
        write_sealed(&d3.join("store.enc"), "store", br#"{"vaults":[]}"#).unwrap();
        assert!(migrate().is_err() && d3.join("store.json").exists() && d3.join("store.enc").exists());
        std::env::remove_var("A2K_DATA_DIR"); std::env::remove_var("A2K_STORE_KEY");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The move, on a COPY of this computer's real store: `A2K_COPY=<folder holding a copy> cargo test dry_run_on_a_copy -- --ignored --nocapture`.
    /// Run it alone: it sets the process's environment.
    #[test]
    #[ignore]
    fn dry_run_on_a_copy() {
        let d = PathBuf::from(std::env::var("A2K_COPY").expect("A2K_COPY"));
        assert!(!d.to_string_lossy().contains("Application Support"), "a COPY, not the real folder");
        std::env::set_var("A2K_DATA_DIR", &d); std::env::set_var("A2K_STORE_KEY", "33".repeat(32));
        let before = crate::wallet::load().unwrap();
        let m = migrate().unwrap();
        let after = crate::wallet::load().unwrap();
        assert_eq!(serde_json::to_value(&before).unwrap(), serde_json::to_value(&after).unwrap());
        assert!(!d.join("store.json").exists() && d.join("store.enc").exists());
        println!("{m:?}: {} vaults, {} labels, {} notes read back the same", after.vaults.len(), after.vaults.iter().map(|v| v.labels.len()).sum::<usize>(), after.vaults.iter().map(|v| v.notes.len()).sum::<usize>());
    }

    #[test]
    fn a_sealed_file_opens_only_with_its_key_its_role_and_unchanged() {
        let (k, other) = ([7u8; 32], [8u8; 32]);
        let blob = seal_with(&k, "store", b"{\"vaults\":[]}", [1u8; 24]);
        assert_eq!(open_with(&k, "store", &blob).unwrap(), b"{\"vaults\":[]}");
        assert!(open_with(&other, "store", &blob).is_err(), "another key must not open it");
        assert!(open_with(&k, "cache", &blob).is_err(), "a sealed store must not pass as a sealed cache");
        let mut bent = blob.clone(); let n = bent.len() - 1; bent[n] ^= 1;
        assert!(open_with(&k, "store", &bent).is_err(), "one flipped bit must be noticed");
        assert!(open_with(&k, "store", b"{\"vaults\":[]}").is_err(), "a readable file is not a sealed one");
        assert!(!blob.windows(6).any(|w| w == b"vaults"), "the content must not be readable in the file");
    }

    /// The format is fixed: a build that cannot open this has changed it, and would strand every store in the field.
    #[test]
    fn frozen_vector() {
        use bitcoin::hashes::Hash;
        // the expected hash was made by libsodium (PyNaCl), not by this code: tools/lockbox-vector.sha256
        let blob = seal_with(&[0x42; 32], "store", b"any two keys", [0x24; 24]);
        assert_eq!(hex::encode(&blob[..5]), "41324b5301");
        assert_eq!(hex::encode(bitcoin::hashes::sha256::Hash::hash(&blob)), include_str!("../../tools/lockbox-vector.sha256").trim());
    }

    #[test]
    fn a_card_slot_gives_the_key_back_to_that_card_only() {
        let (store_key, secret, id, pk) = ([9u8; 32], [3u8; 32], "00112233aabbccdd", "02aa");
        let slot = wrap_for_card(id, pk, "laptop", &secret, &store_key, [5u8; 24], 1);
        assert_eq!(unwrap_from_card(id, &slot, &secret).unwrap(), store_key);
        assert!(unwrap_from_card(id, &slot, &[4u8; 32]).is_err(), "another card's secret");
        assert!(unwrap_from_card("ffffffffffffffff", &slot, &secret).is_err(), "a slot copied to another computer");
        let moved = CardSlot { pubkey: "02bb".into(), ..slot.clone() };
        assert!(unwrap_from_card(id, &moved, &secret).is_err(), "a slot relabelled as another key's");
        assert!(!slot.wrapped.contains(&hex::encode(store_key)), "the key is not in the slot in the clear");
    }
}
