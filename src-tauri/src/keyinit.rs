//! Setting up a brand-new key — a YubiKey 5 or a Nitrokey 3 — from inside the app, so nobody is sent to the maker's
//! software to change two PINs. Both ship with the same factory PINs and the same empty OpenPGP applet; what differs
//! (Yubico's management, OTP and OATH applets) is asked of a Yubico key only.
//!
//! Three rules shape everything here:
//!  * LOOKING never costs anything. `survey` sends no PIN and writes nothing, to any applet.
//!  * Only the OpenPGP applet's two PINs, its signature-PIN policy and its empty signature slot are ever written,
//!    plus (if asked) the USB switch for the Yubico OTP keyboard. PIV, the authenticator codes, the OTP slots and the
//!    passkeys are read at most, never written, and nothing here can reset an applet.
//!  * A factory PIN is only ever tried on a key that LOOKS untouched in every way that can be read without a PIN.
//!    Changing the PIN is itself the test: a key PIN try that misses is repaired by the next right one, an admin PIN
//!    try that misses three times is repaired by nothing — so the key PIN goes first.
use crate::cards::{self, apdu, apdu_more, app_data, find, maker_of, pin_refusal, probe, read_sig_pubkey, readers, select, serial_of, tlv, verify, CardError, Maker, Probe, OID_SECP256K1};
use crate::transport::{Card, Wire};
use serde::Serialize;

const FACTORY_PIN: &str = "123456";
const FACTORY_ADMIN: &str = "12345678";
const AID_MGMT: &[u8] = &[0xA0, 0x00, 0x00, 0x05, 0x27, 0x47, 0x11, 0x17];
const AID_OTP: &[u8] = &[0xA0, 0x00, 0x00, 0x05, 0x27, 0x20, 0x01];
const AID_OATH: &[u8] = &[0xA0, 0x00, 0x00, 0x05, 0x27, 0x21, 0x01];
const AID_PIV: &[u8] = &[0xA0, 0x00, 0x00, 0x03, 0x08];
// the management applet's capability bits
const CAP_OTP: u16 = 0x01;
const CAP_OPENPGP: u16 = 0x08;

/// What the OpenPGP applet says about itself with no PIN given.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct OpenPgp {
    pub tries: [u8; 3],        // key PIN, reset code, admin PIN
    pub sig_pin_each: bool,    // the key PIN is asked for every signature (PW status byte 0 = 00)
    pub slots: [bool; 3],      // signature, decryption, authentication: holds a key
    pub fingerprints: bool,    // any fingerprint recorded
    pub sig_count: u32,
    pub name: bool, pub url: bool, pub login: bool,
    pub cert: usize,           // bytes in the cardholder certificate (where this app keeps a seal share)
    pub private: bool,         // anything in the private objects readable without a PIN (0101, 0102)
    pub kdf: bool,             // PINs are hashed before they are sent: a plain PIN would be a wrong one
    pub unread: Vec<String>,   // what would not answer; an unread fact is never taken for an empty one
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Piv { pub keys: Option<u8>, pub certs: u8, pub pin_default: Option<bool> }
#[derive(Debug, Clone, Default, Serialize)]
pub struct Oath { pub password: bool, pub accounts: Option<usize> }
#[derive(Debug, Clone, Default, Serialize)]
pub struct Otp { pub slot1: bool, pub slot2: bool }

#[derive(Debug, Clone, Serialize)]
pub struct Survey {
    pub reader: String, pub serial: u32, pub firmware: String, pub form: u8, pub nfc: bool,
    pub maker: String, pub device: String,   // from the AID (cards::Maker): yubico/yubikey · nitrokey/nitrokey3
    pub pubkey: Option<String>,
    /// fresh — untouched, can be set up · ours — already holds a signing key this app can use ·
    /// blocked — a PIN has run out of tries · used — something else lives on its OpenPGP applet
    pub state: String,
    pub why: Vec<String>,
    pub pgp: OpenPgp,
    pub piv: Option<Piv>, pub oath: Option<Oath>, pub otp: Option<Otp>,
    pub otp_usb: Option<bool>,     // the OTP keyboard is switched on over USB
    pub fido: Option<bool>,        // passkeys / security-key part switched on; its contents are never read
    pub config_locked: bool,
    pub pin_complexity: bool,
}

/// The whole judgement, from facts alone, so it can be tested without a key.
pub fn judge(p: &OpenPgp, ours: bool) -> (&'static str, Vec<String>) {
    if ours { return ("ours", vec![]); }
    let mut why = Vec::new();
    if p.tries[0] == 0 || p.tries[2] == 0 { return ("blocked", vec![if p.tries[2] == 0 { "its admin PIN has no tries left".into() } else { "its key PIN has no tries left".into() }]); }
    if p.tries[0] != 3 || p.tries[2] != 3 { why.push("a PIN has been tried and missed, so somebody has set one".into()); }
    if p.tries[1] != 0 { why.push("a reset code has been set".into()); }
    if p.slots.iter().any(|s| *s) || p.fingerprints { why.push("it already holds a key".into()); }
    if p.sig_count != 0 { why.push(format!("it has signed {} time{}", p.sig_count, if p.sig_count == 1 { "" } else { "s" })); }
    if p.name || p.url || p.login { why.push("a name, a login or a key address has been written to it".into()); }
    if p.cert != 0 || p.private { why.push("it holds a certificate or private data".into()); }
    if p.kdf { why.push("its PINs are hashed (KDF), which this app does not speak".into()); }
    for u in &p.unread { why.push(format!("its {u} could not be read")); }
    if why.is_empty() { ("fresh", why) } else { ("used", why) }
}

fn be(v: &[u8]) -> u16 { v.iter().fold(0u16, |a, b| (a << 8) | *b as u16) }
/// The value inside `tag`, whether the card answered wrapped in the tag or bare.
fn inside(raw: &[u8], tag: u16) -> &[u8] { match tlv(raw) { Some((t, v, _)) if t == tag => v, _ => raw } }

pub(crate) fn pgp_facts(card: &dyn Wire) -> Result<(OpenPgp, Option<[u8; 33]>), CardError> {
    let ad = app_data(card)?;
    let disc = find(&ad, 0x73).unwrap_or(&ad);
    let mut p = OpenPgp::default();
    match find(disc, 0xC4).or_else(|| find(&ad, 0xC4)).filter(|c| c.len() >= 7) {
        Some(c4) => { p.tries = [c4[4], c4[5], c4[6]]; p.sig_pin_each = c4[0] == 0; }
        None => p.unread.push("PIN counters".into()),
    }
    // C5 is three 20-byte fingerprints; newer firmware adds a fourth for the attestation key Yubico puts there at the
    // factory, which is on every untouched key and says nothing about use.
    match find(disc, 0xC5) { Some(f) => p.fingerprints = f[..f.len().min(60)].iter().any(|b| *b != 0), None => p.unread.push("fingerprints".into()) }
    // Key information (DE, firmware 5.2 and newer): pairs of slot (1 signature, 2 decryption, 3 authentication) and
    // state (0 none, 1 generated, 2 imported). Without it, ask each slot for its public key: an empty one answers
    // 6A88, or 6581 on a YubiKey 5.4 (measured), and anything else is taken for occupied.
    let info = find(disc, 0xDE).filter(|d| d.len() >= 6);
    for (i, crt) in [0xB6u8, 0xB8, 0xA4].into_iter().enumerate() {
        if let Some(st) = info.and_then(|d| d.chunks(2).find(|c| c.len() == 2 && c[0] == i as u8 + 1)) { p.slots[i] = st[1] != 0; continue; }
        match apdu(card, 0x00, 0x47, 0x81, 0x00, &[crt, 0x00]) {
            Err(CardError::Sw { sw: 0x6A88 | 0x6581, .. }) => {}
            Ok(_) => p.slots[i] = true,
            Err(_) => { p.slots[i] = true; p.unread.push(["signature slot", "decryption slot", "authentication slot"][i].into()); }
        }
    }
    match apdu(card, 0x00, 0xCA, 0x00, 0x7A, &[]) {
        Ok(raw) => p.sig_count = find(inside(&raw, 0x7A), 0x93).map(|v| v.iter().fold(0u32, |a, b| (a << 8) | *b as u32)).unwrap_or(0),
        Err(_) => p.unread.push("signature counter".into()),
    }
    match apdu(card, 0x00, 0xCA, 0x00, 0x65, &[]) {
        Ok(raw) => p.name = find(inside(&raw, 0x65), 0x5B).map(|v| !v.is_empty()).unwrap_or(false),
        Err(_) => p.unread.push("cardholder name".into()),
    }
    p.url = apdu(card, 0x00, 0xCA, 0x5F, 0x50, &[]).map(|raw| !inside(&raw, 0x5F50).is_empty()).unwrap_or(false);
    p.login = apdu(card, 0x00, 0xCA, 0x00, 0x5E, &[]).map(|raw| !inside(&raw, 0x5E).is_empty()).unwrap_or(false);
    match apdu(card, 0x00, 0xCA, 0x7F, 0x21, &[]) { Ok(c) => p.cert = c.len(), Err(CardError::Sw { sw: 0x6A88, .. }) => {}, Err(_) => p.unread.push("certificate".into()) }
    p.private = [0x01u8, 0x02].iter().any(|n| apdu(card, 0x00, 0xCA, 0x01, *n, &[]).map(|v| !v.is_empty()).unwrap_or(false));
    // KDF (F9): 81 01 03 means iterated-salted hashing is on. A card too old to have the object has none.
    p.kdf = apdu(card, 0x00, 0xCA, 0x00, 0xF9, &[]).ok().and_then(|raw| find(inside(&raw, 0xF9), 0x81).map(|v| v.first().copied().unwrap_or(0) != 0)).unwrap_or(false);
    let is_k1 = find(disc, 0xC1).map(|a| a.len() >= 6 && a[0] == 0x13 && &a[1..6] == OID_SECP256K1).unwrap_or(false);
    let ours = if is_k1 { read_sig_pubkey(card) } else { None };
    Ok((p, ours))
}

struct Mgmt { usb_enabled: Option<u16>, locked: bool, firmware: String, form: u8, nfc: bool, pin_complexity: bool }
fn mgmt(card: &dyn Wire) -> Option<Mgmt> {
    apdu(card, 0x00, 0xA4, 0x04, 0x00, AID_MGMT).ok()?;
    let cfg = apdu(card, 0x00, 0x1D, 0x00, 0x00, &[]).ok()?;
    let t = cfg.get(1..)?;
    Some(Mgmt {
        usb_enabled: find(t, 0x03).map(be),
        locked: find(t, 0x0A).map(|v| v.iter().any(|b| *b != 0)).unwrap_or(false),
        firmware: find(t, 0x05).filter(|v| v.len() == 3).map(|v| format!("{}.{}.{}", v[0], v[1], v[2])).unwrap_or_default(),
        form: find(t, 0x04).and_then(|v| v.first().copied()).map(|f| f & 0x0F).unwrap_or(0),
        nfc: find(t, 0x0D).map(|v| v.iter().any(|b| *b != 0)).unwrap_or(false),
        pin_complexity: find(t, 0x16).map(|v| v == [1]).unwrap_or(false),
    })
}

/// The other things a key can hold, as far as each will say with no PIN. `None` = switched off or would not answer.
/// The OTP and OATH applets are Yubico's own and are not asked of another maker's key; PIV is a standard and is.
fn neighbours(card: &dyn Wire, maker: Maker) -> (Option<Piv>, Option<Oath>, Option<Otp>) {
    let yubico = maker == Maker::Yubico;
    let otp = if yubico { apdu(card, 0x00, 0xA4, 0x04, 0x00, AID_OTP).ok().filter(|s| s.len() >= 6).map(|s| Otp { slot1: s[4] & 0x01 != 0, slot2: s[4] & 0x02 != 0 }) } else { None };
    let oath = if !yubico { None } else { apdu(card, 0x00, 0xA4, 0x04, 0x00, AID_OATH).ok().map(|sel| {
        let password = find(&sel, 0x74).is_some();
        let accounts = if password { None } else { apdu_more(card, 0x00, 0xA1, 0x00, 0x00, &[], 0xA5).ok().map(|l| { let (mut n, mut rest) = (0, &l[..]); while let Some((t, _, r)) = tlv(rest) { if t == 0x72 { n += 1; } rest = r; } n }) };
        Oath { password, accounts }
    }) };
    let piv = apdu(card, 0x00, 0xA4, 0x04, 0x00, AID_PIV).ok().map(|_| {
        // certificates are readable by anyone: 9A, 9C, 9D, 9E
        let certs = [[0x5F, 0xC1, 0x05u8], [0x5F, 0xC1, 0x0A], [0x5F, 0xC1, 0x0B], [0x5F, 0xC1, 0x01]].iter()
            .filter(|id| apdu(card, 0x00, 0xCB, 0x3F, 0xFF, &[0x5C, 0x03, id[0], id[1], id[2]]).map(|v| !v.is_empty()).unwrap_or(false)).count() as u8;
        // metadata (firmware 5.3 and newer) says which slots hold a key, and whether the PIN is still the factory one
        let pin = apdu(card, 0x00, 0xF7, 0x00, 0x80, &[]).ok();
        let pin_default = pin.as_deref().and_then(|m| find(m, 0x05)).map(|v| v.first().copied().unwrap_or(0) != 0);
        // a slot's metadata is non-empty when it holds a key; a Nitrokey's PIV answers 9000 and nothing to every slot
        let keys = pin.as_ref().map(|_| [0x9Au8, 0x9C, 0x9D, 0x9E].into_iter().chain(0x82..=0x95).filter(|s| apdu(card, 0x00, 0xF7, 0x00, *s, &[]).map(|m| !m.is_empty()).unwrap_or(false)).count() as u8);
        Piv { keys, certs, pin_default }
    });
    (piv, oath, otp)
}

fn survey_card(card: &dyn Wire, reader: String) -> Result<Survey, CardError> {
    let (pgp, ours) = pgp_facts(card)?;
    let ad = app_data(card)?;
    let (serial, maker) = (serial_of(&ad), maker_of(&ad));
    let m = if maker == Maker::Yubico { mgmt(card) } else { None };
    let (piv, oath, otp) = neighbours(card, maker);
    select(card)?;   // leave it where every other part of the app expects it
    let (state, why) = judge(&pgp, ours.is_some());
    Ok(Survey {
        reader, serial, pubkey: ours.map(hex::encode), state: state.into(), why, pgp, piv, oath, otp,
        maker: maker.name().into(), device: maker.device().into(),
        firmware: m.as_ref().map(|m| m.firmware.clone()).or_else(|| (maker == Maker::Nitrokey).then(|| crate::nkfw::firmware_cached(serial)).flatten()).unwrap_or_default(),
        form: m.as_ref().map(|m| m.form).unwrap_or(maker.form()), nfc: m.as_ref().map(|m| m.nfc).unwrap_or(false),
        otp_usb: m.as_ref().and_then(|m| m.usb_enabled).map(|e| e & CAP_OTP != 0),
        fido: m.as_ref().and_then(|m| m.usb_enabled).map(|e| e & 0x202 != 0),
        config_locked: m.as_ref().map(|m| m.locked).unwrap_or(false), pin_complexity: m.as_ref().map(|m| m.pin_complexity).unwrap_or(false),
    })
}

/// Every plugged-in key that answers OpenPGP, looked at and not touched.
pub fn survey() -> Result<Vec<Survey>, CardError> {
    let mut out = Vec::new();
    for reader in readers()? {
        let Ok(Probe::Key(card)) = probe(&reader) else { continue };
        if let Ok(s) = survey_card(&card, reader) { out.push(s); }
    }
    Ok(out)
}

/// The key with this serial, wherever it is plugged in. Turning its OTP keyboard off changes the reader's NAME
/// ("OTP+FIDO+CCID" becomes "FIDO+CCID"), so a key being set up is always found by its serial, never by its reader.
fn open(serial: u32) -> Result<(Card, String), CardError> {
    for reader in readers()? {
        let Ok(Probe::Key(card)) = probe(&reader) else { continue };
        if app_data(&card).map(|ad| serial_of(&ad)).unwrap_or(0) == serial { return Ok((card, reader)); }
    }
    // on a phone, a key that is not in the port is asked for by NFC; a different key held up is not the one wanted
    #[cfg(target_os = "ios")]
    if let Probe::Key(card) = probe(NFC)? {
        return if app_data(&card).map(|ad| serial_of(&ad)).unwrap_or(0) == serial { Ok((card, NFC.to_string())) } else { Err(CardError::Changed) };
    }
    Err(CardError::NotFound)
}

/// The reader's name for a key held to a phone (transport.rs).
pub const NFC: &str = "NFC";

/// One key, held to the phone, looked at and not touched.
pub fn survey_nfc() -> Result<Survey, CardError> {
    match probe(NFC)? { Probe::Key(card) => survey_card(&card, NFC.to_string()), Probe::Other(_) => Err(CardError::NotFound) }
}

// ---------- which PINs are refused before the key ever sees them ----------

const COMMON: &[&str] = &["password", "passw0rd", "qwerty", "qwertyui", "qwertyuiop", "asdfgh", "asdfghjk", "zxcvbn", "1qaz2wsx", "letmein", "iloveyou", "welcome", "abc123", "admin123",
    "yubikey", "yubico", "nitrokey", "bitcoin", "satoshi", "nakamoto", "anytwokeys", "147258", "147258369", "159753", "123321", "12344321", "112233", "11223344", "102030", "10203040", "696969", "314159", "31415926"];

/// Why this PIN will not do, or `None`. `admin`: the admin PIN (8 or more) rather than the key PIN (6 or more).
pub fn refuse(pin: &str, admin: bool, serial: u32) -> Option<String> {
    let what = if admin { "admin PIN" } else { "key PIN" };
    let n = pin.chars().count();
    if n >= 32 && pin.chars().all(|c| "cbdefghijklnrtuv".contains(c)) { return Some("That was the key typing a one-time code, not a PIN. Nothing was sent to the key.".into()); }
    if !pin.chars().all(|c| c.is_ascii_graphic() || c == ' ') { return Some(format!("Keep the {what} to the letters, digits and symbols of a plain keyboard, so it can be typed on any computer.")); }
    let min = if admin { 8 } else { 6 };
    if n < min { return Some(format!("The {what} needs at least {min} characters.")); }
    if n > 127 { return Some(format!("The {what} can be at most 127 characters.")); }
    if pin == FACTORY_PIN || pin == FACTORY_ADMIN { return Some("That is the factory PIN every key ships with, YubiKey or Nitrokey. Anyone who picks the key up would try it first.".into()); }
    let c: Vec<char> = pin.chars().collect();
    if c.iter().all(|x| *x == c[0]) { return Some("One character repeated is among the first things anyone would try.".into()); }
    let step = |a: char, b: char, d: i32| -> bool { let (a, b) = (a as i32, b as i32); b - a == d || (a >= 48 && a <= 57 && b >= 48 && b <= 57 && (b - 48) == (a - 48 + d).rem_euclid(10)) };
    if c.windows(2).all(|w| step(w[0], w[1], 1)) || c.windows(2).all(|w| step(w[0], w[1], -1)) { return Some("A run like 123456 or 87654321 is among the first things anyone would try.".into()); }
    for block in 1..=n / 2 { if n % block == 0 && c.chunks(block).all(|k| k == &c[..block]) { return Some("A short pattern repeated is easy to guess.".into()); } }
    let low = pin.to_lowercase();
    if COMMON.contains(&low.as_str()) { return Some("That one is on every list of common PINs and passwords.".into()); }
    if serial != 0 && low.contains(&serial.to_string()) { return Some("That is the serial number printed on the key itself.".into()); }
    None
}

/// Both PINs judged together: each on its own, and never the same as one another.
pub fn refuse_pair(user: &str, admin: &str, serial: u32) -> (Option<String>, Option<String>) {
    let u = refuse(user, false, serial);
    let a = refuse(admin, true, serial).or_else(|| (!user.is_empty() && (admin == user || admin.starts_with(user) && admin.len() - user.len() <= 2)).then(|| "The admin PIN must be different from the key PIN: it is the one that rescues a forgotten key PIN.".to_string()));
    (u, a)
}

// ---------- writing ----------

#[derive(Debug, Clone, Default, Serialize)]
pub struct PinsSet {
    pub user_done: bool, pub admin_done: bool,
    /// "user" / "admin": the factory PIN was refused, so this key is not in its factory state; `tries` are left on that PIN
    pub not_factory: Option<String>, pub tries: u8,
    pub error: Option<String>,
}

fn change(card: &dyn Wire, p2: u8, old: &str, new: &str) -> Result<(), CardError> {
    let mut body = zeroize::Zeroizing::new(Vec::with_capacity(old.len() + new.len()));
    body.extend_from_slice(old.as_bytes()); body.extend_from_slice(new.as_bytes());
    apdu(card, 0x00, 0x24, 0x00, p2, &body).map(|_| ()).map_err(|e| pin_refusal(card, p2, e))
}

/// Replace the two factory PINs. Never retried by itself and never resumed blind: what was done is reported, and
/// `user_done` says the key PIN is already the new one (a cable pulled between the two changes), in which case it is
/// checked rather than changed. Everything happens inside one PC/SC transaction, so no other program's command
/// lands between ours.
pub fn set_pins(serial: u32, user: &str, admin: &str, user_done: bool) -> Result<PinsSet, String> {
    if let (Some(e), _) | (None, Some(e)) = refuse_pair(user, admin, serial) { return Err(e); }
    let (mut card, _) = open(serial).map_err(|e| e.to_string())?;
    let tx = card.transaction().map_err(|e| e.to_string())?;
    let (facts, ours) = pgp_facts(&tx).map_err(|e| e.to_string())?;
    let (state, why) = judge(&facts, ours.is_some());
    if state != "fresh" { return Err(format!("This key is no longer as it left the factory ({}). Nothing was written.", if why.is_empty() { "it holds a key".to_string() } else { why.join("; ") })); }
    let mut out = PinsSet { user_done, ..Default::default() };
    if user_done {
        if let Err(e) = verify(&tx, 0x81, user) { out.user_done = false; out.error = Some(format!("The key PIN did not check: {e}")); return Ok(out); }
    } else {
        match change(&tx, 0x81, FACTORY_PIN, user) {
            Ok(()) => {}
            Err(CardError::WrongPin(n)) => { out.not_factory = Some("user".into()); out.tries = n; return Ok(out); }
            Err(e) => { out.error = Some(e.to_string()); return Ok(out); }
        }
        out.user_done = true;
        if let Err(e) = verify(&tx, 0x81, user) { out.error = Some(format!("The key PIN was changed but did not check afterwards: {e}")); return Ok(out); }
    }
    match change(&tx, 0x83, FACTORY_ADMIN, admin) {
        Ok(()) => {}
        Err(CardError::WrongPin(n)) => { out.not_factory = Some("admin".into()); out.tries = n; return Ok(out); }
        Err(e) => { out.error = Some(e.to_string()); return Ok(out); }
    }
    out.admin_done = true;
    if let Err(e) = verify(&tx, 0x83, admin) { out.error = Some(format!("The admin PIN was changed but did not check afterwards: {e}")); }
    cards::forget_pin(&tx); cards::forget_admin(&tx);
    Ok(out)
}

/// "fresh" if the factory key PIN opens the key (it was reset since this computer saw its PINs set), "begun" if not.
pub fn resume_check(serial: u32) -> Result<String, String> {
    let (mut card, _) = open(serial).map_err(|e| e.to_string())?;
    let tx = card.transaction().map_err(|e| e.to_string())?;
    let r = match verify(&tx, 0x81, FACTORY_PIN) { Ok(()) => "fresh", Err(CardError::WrongPin(_)) => "begun", Err(e) => return Err(e.to_string()) };
    cards::forget_pin(&tx);
    Ok(r.into())
}

/// Both PINs asked of the key itself. The window compares what was typed with what was set BEFORE calling this, so a
/// slip of the finger costs no try; this is the key's own word that the record on paper opens it.
pub fn check_pins(serial: u32, user: &str, admin: &str) -> Result<(), String> {
    let (mut card, _) = open(serial).map_err(|e| e.to_string())?;
    let tx = card.transaction().map_err(|e| e.to_string())?;
    verify(&tx, 0x81, user).map_err(|e| format!("key PIN: {e}"))?;
    let r = verify(&tx, 0x83, admin).map_err(|e| format!("admin PIN: {e}"));
    cards::forget_pin(&tx); cards::forget_admin(&tx);
    r
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Hardened { pub sig_pin_each: bool, pub otp_off: bool, pub notes: Vec<String> }

/// Two switches, both reversible and neither erasing anything: ask the key PIN for every signature, and stop the
/// key typing Yubico one-time codes over USB (the OTP slots keep whatever they hold). The second one restarts the
/// key, so it goes last and the answer to it may never arrive.
pub fn harden(serial: u32, admin: &str, sig_pin_each: bool, otp_off: bool) -> Result<Hardened, String> {
    let (mut card, _) = open(serial).map_err(|e| e.to_string())?;
    let tx = card.transaction().map_err(|e| e.to_string())?;
    let mut out = Hardened::default();
    if sig_pin_each {
        verify(&tx, 0x83, admin).map_err(|e| format!("admin PIN: {e}"))?;
        apdu(&tx, 0x00, 0xDA, 0x00, 0xC4, &[0x00]).map_err(|e| e.to_string())?;
        let ad = app_data(&tx).map_err(|e| e.to_string())?;
        out.sig_pin_each = find(find(&ad, 0x73).unwrap_or(&ad), 0xC4).and_then(|c| c.first().copied()) == Some(0);
        if !out.sig_pin_each { return Err(CardError::Unverified.to_string()); }
    }
    if otp_off {
        match mgmt(&tx) {
            None => out.notes.push("The key would not show its settings, so its one-time-code keyboard was left as it is.".into()),
            Some(m) if m.locked => out.notes.push("This key's settings are locked with a code, so its one-time-code keyboard was left as it is.".into()),
            Some(Mgmt { usb_enabled: None, .. }) => out.notes.push("The key did not say what is switched on, so its one-time-code keyboard was left as it is.".into()),
            Some(Mgmt { usb_enabled: Some(e), .. }) if e & CAP_OTP == 0 => out.otp_off = true,
            Some(Mgmt { usb_enabled: Some(e), .. }) => {
                let want = e & !CAP_OTP;
                // never a write that could switch off the part this app talks to
                if want & CAP_OPENPGP == 0 { return Err("refusing to write settings that would switch OpenPGP off".into()); }
                // [length] reboot (0C, empty) · USB enabled (03, two bytes)
                let body = [6u8, 0x0C, 0x00, 0x03, 0x02, (want >> 8) as u8, want as u8];
                match apdu(&tx, 0x00, 0x1C, 0x00, 0x00, &body) {
                    // the key restarts to apply it; a dropped connection here is the change happening
                    Ok(_) | Err(CardError::Link(_)) | Err(CardError::WasReset) => out.otp_off = true,
                    Err(e) => out.notes.push(format!("The key would not switch its one-time-code keyboard off ({e}). Nothing else is affected.")),
                }
                return Ok(out);
            }
        }
        let _ = select(&tx);
    }
    cards::forget_admin(&tx);
    Ok(out)
}

/// A fresh secp256k1 key in the EMPTY signature slot, touch fixed on. Refuses an occupied slot.
pub fn make_key(serial: u32, admin: &str) -> Result<String, String> {
    let (card, reader) = open(serial).map_err(|e| e.to_string())?;
    drop(card);
    cards::generate_empty(&reader, admin, serial).map_err(|e| e.to_string())
}

/// A key made OUTSIDE the card, written into its EMPTY signature slot: the one way a card key can have a backup or a
/// spare. The card must report the public key the secret implies before anything else is done.
pub fn make_key_from(serial: u32, admin: &str, secret: &crate::secret::Secret) -> Result<String, String> {
    let (card, reader) = open(serial).map_err(|e| e.to_string())?;
    drop(card);
    cards::import_empty(&reader, admin, serial, secret, &crate::secret::point_of(secret)).map_err(|e| e.to_string())
}

/// The first signature: key PIN, a touch, and the answer checked against the public key the key just made.
pub fn prove(serial: u32, user: &str, pubkey: &str) -> Result<(), String> {
    use std::str::FromStr;
    let (card, reader) = open(serial).map_err(|e| e.to_string())?;
    drop(card);
    let pk = bitcoin::secp256k1::PublicKey::from_str(pubkey).map_err(|_| "that public key is damaged")?;
    let digest: [u8; 32] = crate::lockbox::random();
    let (r, s) = cards::sign_digest_checked(&reader, user, &digest, &pk.serialize()).map_err(|e| e.to_string())?;
    let mut compact = [0u8; 64]; compact[..32].copy_from_slice(&r); compact[32..].copy_from_slice(&s);
    let sig = bitcoin::secp256k1::ecdsa::Signature::from_compact(&compact).map_err(|_| "the key's signature could not be read")?;
    bitcoin::secp256k1::Secp256k1::verification_only().verify_ecdsa(&bitcoin::secp256k1::Message::from_digest(digest), &sig, &pk)
        .map_err(|_| "the key did not prove itself: its signature does not match its public key".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn factory() -> OpenPgp { OpenPgp { tries: [3, 0, 3], ..Default::default() } }

    #[test]
    fn a_factory_key_is_fresh_and_nothing_else_is() {
        assert_eq!(judge(&factory(), false).0, "fresh");
        assert_eq!(judge(&factory(), true).0, "ours");
        let not: Vec<Box<dyn Fn(&mut OpenPgp)>> = vec![
            Box::new(|p| p.tries = [2, 0, 3]), Box::new(|p| p.tries = [3, 0, 2]), Box::new(|p| p.tries = [3, 3, 3]),
            Box::new(|p| p.slots = [false, true, false]), Box::new(|p| p.fingerprints = true), Box::new(|p| p.sig_count = 1),
            Box::new(|p| p.name = true), Box::new(|p| p.url = true), Box::new(|p| p.login = true), Box::new(|p| p.cert = 40),
            Box::new(|p| p.private = true), Box::new(|p| p.kdf = true), Box::new(|p| p.unread.push("signature counter".into())),
        ];
        for f in not { let mut p = factory(); f(&mut p); let (s, why) = judge(&p, false); assert_eq!(s, "used", "{p:?}"); assert!(!why.is_empty()); }
        // the signature-PIN policy is a preference, not a sign of use: gpg and a factory key differ on it
        let mut p = factory(); p.sig_pin_each = true; assert_eq!(judge(&p, false).0, "fresh");
        let mut p = factory(); p.tries = [0, 0, 3]; assert_eq!(judge(&p, false).0, "blocked");
        let mut p = factory(); p.tries = [3, 0, 0]; assert_eq!(judge(&p, false).0, "blocked");
    }

    #[test]
    fn weak_pins_are_refused_before_the_key_sees_them() {
        for (pin, admin) in [("123456", false), ("12345678", true), ("12345678", false), ("654321", false), ("000000", false), ("11111111", true), ("234567", false), ("7890123", false),
            ("abcdef", false), ("121212", false), ("123123", false), ("abcabcabc", true), ("password", true), ("QWERTYUI", true), ("12345", false), ("1234567", true),
            ("pin 27446043", true), ("27446043", true), ("cccccbcgtrdjkbhivnnfgrlnhjrcdtbjvjtlhurkdhtd", false), ("caf\u{e9}caf\u{e9}1", true)] {
            assert!(refuse(pin, admin, 27446043).is_some(), "{pin} should be refused");
        }
        for (pin, admin) in [("493817", false), ("correct horse 9", true), ("T4ble-lamp", true), ("830194", false), ("5827 1946", true)] {
            assert_eq!(refuse(pin, admin, 27446043), None, "{pin} should pass");
        }
        assert!(refuse_pair("49381726", "49381726", 0).1.is_some(), "admin equal to the key PIN");
        assert!(refuse_pair("493817", "4938171", 0).1.is_some(), "too short anyway");
        assert!(refuse_pair("493817", "49381700", 0).1.is_some(), "the key PIN with two characters added");
        assert_eq!(refuse_pair("493817", "60215934", 0), (None, None));
    }

    /// Needs a key plugged in: `cargo test hardware_survey -- --ignored --nocapture`. Reads only: no PIN, no write.
    #[test]
    #[ignore]
    fn hardware_survey() {
        for s in survey().unwrap() { println!("{}", serde_json::to_string_pretty(&s).unwrap()); }
    }
}
