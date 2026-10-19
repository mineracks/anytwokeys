//! The OpenPGP applet of a security key over PC/SC: the six APDUs a Bitcoin cosigner needs.
//! Ported byte-for-byte from the Python proof (yubikit) that signed on mainnet with a YubiKey. A Nitrokey 3 (opcard)
//! answers the same applet; where it answers differently, that was measured on a real one on 2026-10-09 and is noted
//! at the spot. A key is judged by what it ANSWERS, never by its reader's name: macOS called a Nitrokey 3 on firmware
//! 1.9.0 "8352 17074" and calls one on 1.9.1 "Nitrokey Nitrokey 3" — the same key, two names.
use crate::transport::{self, Card, Opened, Wire};
use serde::Serialize;
use thiserror::Error;

pub(crate) const AID_OPENPGP: &[u8] = &[0xD2, 0x76, 0x00, 0x01, 0x24, 0x01];
pub(crate) const OID_SECP256K1: &[u8] = &[0x2B, 0x81, 0x04, 0x00, 0x0A];
/// The form-factor nibble the screens draw for a Nitrokey 3 (Yubico's own shapes are 1–5, from its management applet).
pub const FORM_NITROKEY3: u8 = 14;

/// Who made the key, from the manufacturer bytes of the OpenPGP AID (the FSFE registry) — never from the reader's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Maker { Yubico, Nitrokey, Other(u16) }
impl Maker {
    /// AID: D2760001 2401 | OpenPGP spec version (2) | manufacturer (2) | serial (4) | RFU (2).
    pub fn of(aid: &[u8]) -> Maker {
        match aid.get(8..10) { Some([0x00, 0x06]) => Maker::Yubico, Some([0x00, 0x0F]) => Maker::Nitrokey, Some(&[a, b]) => Maker::Other(((a as u16) << 8) | b as u16), _ => Maker::Other(0) }
    }
    pub fn name(self) -> &'static str { match self { Maker::Yubico => "yubico", Maker::Nitrokey => "nitrokey", Maker::Other(_) => "other" } }
    /// The device id the screens use (devices.js); an unknown maker's OpenPGP card is shown as a plain card.
    pub fn device(self) -> &'static str { match self { Maker::Yubico => "yubikey", Maker::Nitrokey => "nitrokey3", Maker::Other(_) => "card" } }
    /// The serial in the AID. Yubico writes the DECIMAL serial as hex digits (BCD: 27446043 → 0x27446043); a Nitrokey 3
    /// writes a number (the first four bytes of its UUID), and so, by the specification, does everyone else.
    pub fn serial(self, aid: &[u8]) -> u32 {
        let Some(b) = aid.get(10..14) else { return 0 };
        let bin = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        match self { Maker::Yubico => u32::from_str_radix(&hex::encode(b), 10).unwrap_or(bin), _ => bin }
    }
    pub fn form(self) -> u8 { match self { Maker::Nitrokey => FORM_NITROKEY3, _ => 0 } }
    /// The longest certificate (7F21) the card really stores. A Nitrokey 3 advertises 4096 in C0 and answers 6500 to
    /// anything past 3240 bytes (found by bisection, firmware 1.9.0); the write is refused whole, nothing is half-kept.
    pub fn cert_room(self, advertised: usize) -> usize { match self { Maker::Nitrokey => advertised.min(3240), _ => advertised } }
}
const ORDER: [u8; 32] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
];

#[derive(Debug, Error)]
pub enum CardError {
    #[error("smartcard service: {0}")]
    Link(String),
    /// another program reset the card under us; `again` turns a second one into `Reset`
    #[error("the smart card was reset by another program")]
    WasReset,
    #[error("another program reset the key part-way through (YubiKey Manager, Nitrokey App or gpg-agent was talking to it at the same time); it was tried again and reset again. Close that program, unplug and replug the key, and try once more")]
    Reset,
    #[error("no key found")]
    NotFound,
    #[error("the card refused ({sw:04X}){hint}")]
    Sw { sw: u16, hint: String },
    #[error("wrong PIN, {0} tries left")]
    WrongPin(u8),
    #[error("the key was not touched in time")]
    NoTouch,
    #[error("malformed answer from the card")]
    Parse,
    #[error("the card's contents changed since they were read; nothing was written")]
    Changed,
    #[error("the card did not read back what was written")]
    Unverified,
    #[error("this key's private data object (0103) holds something that is not ours; it was left alone")]
    PrivateInUse,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardInfo {
    pub reader: String,
    pub serial: u32,
    pub version: String,
    pub pubkey: Option<String>,   // compressed secp256k1, hex — None if the SIG slot is empty or not secp256k1
    pub touch: String,            // OFF | ON | FIXED | ?
    pub form: u8,                 // YubiKey form factor: 1 USB-A · 2 USB-A Nano · 3 USB-C · 4 USB-C Nano · 5 USB-C/Lightning · 14 Nitrokey 3 · 0 unknown
    pub nfc: bool,                // the key has an NFC antenna (management config tag 0x0D; a Nitrokey does not say)
    pub firmware: String,         // e.g. 5.4.3 from Yubico's management applet, 1.9.1 from a Nitrokey's admin app over HID; empty if it would not say
    pub maker: String,            // yubico · nitrokey · other — from the AID, never the reader's name
    pub device: String,           // the devices.js id: yubikey · nitrokey3 · card
}


/// A shared card can be RESET under us by any other program that talks to it — YubiKey Manager polls, gpg-agent
/// probes — and PC/SC then refuses every command on the old handle ("the smart card has been reset, so any shared
/// state information is invalid"). Nothing is wrong with the key. The whole operation is done again from a fresh
/// connection, once; PIN and touch are part of it, so a half-done sequence is never resumed.
fn again<T>(f: impl Fn() -> Result<T, CardError>) -> Result<T, CardError> {
    match f() {
        Err(CardError::WasReset) => { std::thread::sleep(std::time::Duration::from_millis(250)); f().map_err(|e| if matches!(e, CardError::WasReset) { CardError::Reset } else { e }) }
        r => r,
    }
}

fn hint_for(sw: u16) -> String {
    match sw {
        0x6982 => " — security condition not satisfied (PIN first)".into(),
        0x6A88 => " — nothing there (empty slot?)".into(),
        0x6985 => " — conditions of use not satisfied".into(),
        _ => String::new(),
    }
}

/// Send one APDU, following 61xx GET RESPONSE chaining. Returns data (without SW).
pub(crate) fn apdu(card: &dyn Wire, cla: u8, ins: u8, p1: u8, p2: u8, data: &[u8]) -> Result<Vec<u8>, CardError> { apdu_more(card, cla, ins, p1, p2, data, 0xC0) }
/// The same, for an applet that asks for the rest of a long answer with its own instruction (OATH uses A5, not C0).
pub(crate) fn apdu_more(card: &dyn Wire, cla: u8, ins: u8, p1: u8, p2: u8, data: &[u8], more: u8) -> Result<Vec<u8>, CardError> {
    let mut cmd = vec![cla, ins, p1, p2];
    if !data.is_empty() {
        cmd.push(data.len() as u8);
        cmd.extend_from_slice(data);
    }
    cmd.push(0x00); // Le
    let mut out = Vec::new();
    let mut resp = card.send(&cmd)?;
    loop {
        if resp.len() < 2 {
            return Err(CardError::Parse);
        }
        let sw1 = resp[resp.len() - 2];
        let sw2 = resp[resp.len() - 1];
        out.extend_from_slice(&resp[..resp.len() - 2]);
        match sw1 {
            0x90 if sw2 == 0x00 => return Ok(out),
            0x61 => {
                resp = card.send(&[0x00, more, 0x00, 0x00, sw2])?;
            }
            0x63 if sw2 & 0xF0 == 0xC0 => return Err(CardError::WrongPin(sw2 & 0x0F)),
            0x66 if sw2 == 0x00 => return Err(CardError::NoTouch),
            _ => {
                let sw = ((sw1 as u16) << 8) | sw2 as u16;
                return Err(CardError::Sw { sw, hint: hint_for(sw) });
            }
        }
    }
}

/// Minimal BER-TLV: returns (tag, value, rest).
pub(crate) fn tlv(buf: &[u8]) -> Option<(u16, &[u8], &[u8])> {
    if buf.is_empty() {
        return None;
    }
    let (tag, mut i) = if buf[0] & 0x1F == 0x1F {
        (((buf[0] as u16) << 8) | *buf.get(1)? as u16, 2)
    } else {
        (buf[0] as u16, 1)
    };
    let l0 = *buf.get(i)?;
    i += 1;
    let len = match l0 {
        0x81 => { let l = *buf.get(i)? as usize; i += 1; l }
        0x82 => { let l = ((*buf.get(i)? as usize) << 8) | *buf.get(i + 1)? as usize; i += 2; l }
        n if n < 0x80 => n as usize,
        _ => return None,
    };
    let v = buf.get(i..i + len)?;
    Some((tag, v, &buf[i + len..]))
}

pub(crate) fn find(buf: &[u8], want: u16) -> Option<&[u8]> {
    let mut rest = buf;
    while let Some((t, v, r)) = tlv(rest) {
        if t == want {
            return Some(v);
        }
        rest = r;
    }
    None
}

pub(crate) fn select(card: &dyn Wire) -> Result<(), CardError> {
    apdu(card, 0x00, 0xA4, 0x04, 0x00, AID_OPENPGP).map(|_| ())
}

/// How many tries each PIN has left: [key PIN, reset code, admin PIN], from the PW status bytes (C4).
pub(crate) fn pin_tries(card: &dyn Wire) -> Option<[u8; 3]> {
    let ad = app_data(card).ok()?;
    let c4 = find(find(&ad, 0x73).unwrap_or(&ad), 0xC4).or_else(|| find(&ad, 0xC4))?;
    (c4.len() >= 7).then(|| [c4[4], c4[5], c4[6]])
}

/// What a YubiKey's refusal of a PIN means. It answers 6982 for a wrong PIN and 6983 for a blocked one, and never
/// says how many tries are left in the answer itself: that is read from the PW status bytes afterwards.
pub(crate) fn pin_refusal(card: &dyn Wire, p2: u8, e: CardError) -> CardError {
    match e {
        CardError::Sw { sw: 0x6982, .. } => match pin_tries(card) { Some(t) => CardError::WrongPin(t[if p2 == 0x83 { 2 } else { 0 }]), None => e },
        CardError::Sw { sw: 0x6983, .. } => CardError::WrongPin(0),
        e => e,
    }
}

/// VERIFY: 81 the key PIN for a signature, 82 the key PIN for everything else, 83 the admin PIN.
pub(crate) fn verify(card: &dyn Wire, p2: u8, pin: &str) -> Result<(), CardError> {
    apdu(card, 0x00, 0x20, 0x00, p2, pin.as_bytes()).map(|_| ()).map_err(|e| pin_refusal(card, p2, e))
}

/// Application related data — the card answers with the whole 6E object; return its contents.
pub(crate) fn app_data(card: &dyn Wire) -> Result<Vec<u8>, CardError> {
    let raw = apdu(card, 0x00, 0xCA, 0x00, 0x6E, &[])?;
    match tlv(&raw) {
        Some((0x6E, v, _)) => Ok(v.to_vec()),
        _ => Ok(raw),      // some firmwares answer unwrapped
    }
}

fn compress(point: &[u8]) -> Option<[u8; 33]> {
    if point.len() != 65 || point[0] != 0x04 {
        return None;
    }
    let mut out = [0u8; 33];
    out[0] = if point[64] & 1 == 1 { 0x03 } else { 0x02 };
    out[1..].copy_from_slice(&point[1..33]);
    Some(out)
}

pub(crate) fn read_sig_pubkey(card: &dyn Wire) -> Option<[u8; 33]> {
    let resp = apdu(card, 0x00, 0x47, 0x81, 0x00, &[0xB6, 0x00]).ok()?;
    let pk = find(&resp, 0x7F49)?;
    let point = find(pk, 0x86)?;
    compress(point)
}

fn connect(reader: &str) -> Result<Card, CardError> {
    match transport::open(reader)? { Opened::Card(card) => { select(&card)?; Ok(card) }, Opened::NotACard => Err(CardError::NotFound) }
}

/// What a plugged-in device turned out to be, judged by what it ANSWERED rather than by its name. Whether it can
/// hold a key for this app is decided by whether it answers the OpenPGP applet. A YubiKey Bio, a FIDO-only key, a
/// PIV-only card is plugged in and working perfectly well — it is just not a key this app can use, and must never be
/// mistaken for a key that is failing.
pub enum Probe { Key(Card), Other(String) }

#[derive(Debug, Clone, serde::Serialize)]
pub struct OtherDevice { pub reader: String, pub why: String }

pub(crate) fn probe(reader: &str) -> Result<Probe, CardError> {
    let card = match transport::open(reader)? {
        Opened::Card(c) => c,
        Opened::NotACard => return Ok(Probe::Other("the computer can see it, but it does not answer as a smart card".into())),
    };
    match select(&card) {
        Ok(()) => Ok(Probe::Key(card)),
        Err(_) => Ok(Probe::Other("it has no OpenPGP applet, which is what Any Two Keys signs with".into())),
    }
}

/// Every smart card plugged in that is NOT a key this app can use, and why.
pub fn others() -> Vec<OtherDevice> {
    let Ok(names) = readers() else { return vec![] };
    names.into_iter().filter_map(|reader| match probe(&reader) {
        Ok(Probe::Other(why)) => Some(OtherDevice { reader, why }),
        _ => None,
    }).collect()
}

pub(crate) fn readers() -> Result<Vec<String>, CardError> { transport::readers() }

/// The key's shape, NFC and firmware, from the YubiKey management applet (READ CONFIG). Leaves OpenPGP selected again.
/// Only a Yubico key has that applet: a Nitrokey 3 answers 6A82 to it (measured) and is drawn as its own shape.
fn hardware(card: &dyn Wire, maker: Maker, serial: u32) -> (u8, bool, String) {
    if maker == Maker::Nitrokey { return (maker.form(), false, crate::nkfw::firmware_cached(serial).unwrap_or_default()); }
    if maker != Maker::Yubico { return (maker.form(), false, String::new()); }
    const AID_MGMT: &[u8] = &[0xA0, 0x00, 0x00, 0x05, 0x27, 0x47, 0x11, 0x17];
    let cfg = apdu(card, 0x00, 0xA4, 0x04, 0x00, AID_MGMT).and_then(|_| apdu(card, 0x00, 0x1D, 0x00, 0x00, &[])).ok();
    let tlvs = cfg.as_deref().and_then(|c| c.get(1..)).unwrap_or(&[]);
    let form = find(tlvs, 0x04).and_then(|v| v.first().copied()).map(|f| f & 0x0F).unwrap_or(0);
    let nfc = find(tlvs, 0x0D).map(|v| v.iter().any(|b| *b != 0)).unwrap_or(false);
    let firmware = find(tlvs, 0x05).filter(|v| v.len() == 3).map(|v| format!("{}.{}.{}", v[0], v[1], v[2])).unwrap_or_default();
    let _ = select(card);
    (form, nfc, firmware)
}

pub fn list() -> Result<Vec<CardInfo>, CardError> { again(|| list_once()) }
fn list_once() -> Result<Vec<CardInfo>, CardError> {
    let mut out = Vec::new();
    for reader in readers()? {
        let card = match probe(&reader) { Ok(Probe::Key(c)) => c, _ => continue };
        let ad = app_data(&card)?;
        let aid = find(&ad, 0x4F).ok_or(CardError::Parse)?;
        if aid.len() < 14 {
            return Err(CardError::Parse);
        }
        let maker = Maker::of(aid);
        let version = format!("OpenPGP {}.{}", aid[6], aid[7]);
        let serial = maker.serial(aid);
        // algorithm attributes for SIG live inside 6E → 73 → C1; only a secp256k1 key counts
        let disc = find(&ad, 0x73).unwrap_or(&[]);
        let is_k1 = find(disc, 0xC1).map(|a| a.len() >= 6 && a[0] == 0x13 && &a[1..6] == OID_SECP256K1).unwrap_or(false);
        let pubkey = if is_k1 { read_sig_pubkey(&card).map(hex::encode) } else { None };
        let touch = apdu(&card, 0x00, 0xCA, 0x00, 0xD6, &[]).ok()
            .and_then(|v| v.first().copied())
            .map(|u| match u { 0 => "OFF", 1 => "ON", 2 => "FIXED", 3 => "CACHED", 4 => "CACHED_FIXED", _ => "?" }.to_string())
            .unwrap_or_else(|| "?".into());
        let (form, nfc, firmware) = hardware(&card, maker, serial);
        out.push(CardInfo { reader, serial, version, pubkey, touch, form, nfc, firmware, maker: maker.name().into(), device: maker.device().into() });
    }
    if out.is_empty() {
        return Err(CardError::NotFound);
    }
    Ok(out)
}

/// Generate a fresh secp256k1 key in the SIGNATURE slot and lock touch to FIXED.
/// Replaces whatever was in the slot. Needs the ADMIN PIN.
pub fn generate(reader: &str, admin_pin: &str) -> Result<String, CardError> { again(|| generate_once(reader, admin_pin)) }
fn generate_once(reader: &str, admin_pin: &str) -> Result<String, CardError> {
    let mut card = connect(reader)?;
    let card = card.transaction()?;   // nobody else's command, or disconnect, lands between ours (see write_sealed)
    verify(&card, 0x83, admin_pin)?;
    let mut attrs = vec![0x13];
    attrs.extend_from_slice(OID_SECP256K1);
    apdu(&card, 0x00, 0xDA, 0x00, 0xC1, &attrs)?;
    let resp = apdu(&card, 0x00, 0x47, 0x80, 0x00, &[0xB6, 0x00])?;
    let point = find(&resp, 0x7F49).and_then(|pk| find(pk, 0x86)).ok_or(CardError::Parse)?;
    let comp = compress(point).ok_or(CardError::Parse)?;
    // touch: FIXED (2), button (0x20). One-way: the card refuses to lower it afterwards.
    apdu(&card, 0x00, 0xDA, 0x00, 0xD6, &[0x02, 0x20])?;
    let ok = apdu(&card, 0x00, 0xCA, 0x00, 0xD6, &[])?.first() == Some(&2);
    forget_admin(&card);
    if !ok { return Err(CardError::Unverified); }
    Ok(hex::encode(comp))
}

/// ECDSA over a 32-byte digest with the SIG key: user PIN, then a touch (waited for up to ~60 s).
/// Returns (r, s) with low-S.
pub fn sign_digest(reader: &str, pin: &str, digest: &[u8; 32]) -> Result<([u8; 32], [u8; 32]), CardError> { again(|| sign_digest_once(reader, pin, digest)) }
fn sign_digest_once(reader: &str, pin: &str, digest: &[u8; 32]) -> Result<([u8; 32], [u8; 32]), CardError> {
    let card = connect(reader)?;
    sign_on_card(&card, pin, digest)
}

/// Recheck the public key and uncached FIXED touch policy on the SAME connection
/// that verifies the PIN and signs; never sign after a card swap or policy change.
pub fn sign_digest_checked(reader: &str, pin: &str, digest: &[u8;32], expected: &[u8;33]) -> Result<([u8;32], [u8;32]), CardError> { again(|| sign_digest_checked_once(reader, pin, digest, expected)) }
fn sign_digest_checked_once(reader: &str, pin: &str, digest: &[u8;32], expected: &[u8;33]) -> Result<([u8;32], [u8;32]), CardError> {
    let card = connect(reader)?;
    if read_sig_pubkey(&card).as_ref() != Some(expected) { return Err(CardError::Changed); }
    if apdu(&card, 0x00, 0xCA, 0x00, 0xD6, &[])?.first() != Some(&2) { return Err(CardError::Unverified); }
    sign_on_card(&card, pin, digest)
}

/// Spark setup is never allowed to replace a slot, including a non-secp256k1 key.
pub fn generate_empty(reader: &str, admin_pin: &str, serial: u32) -> Result<String, CardError> { again(|| generate_empty_once(reader, admin_pin, serial)) }
fn generate_empty_once(reader: &str, admin_pin: &str, serial: u32) -> Result<String, CardError> {
    let mut card = connect(reader)?;
    let card = card.transaction()?;
    require_empty(&card, serial)?;
    verify(&card, 0x83, admin_pin)?;
    let mut attrs = vec![0x13]; attrs.extend_from_slice(OID_SECP256K1);
    apdu(&card, 0x00, 0xDA, 0x00, 0xC1, &attrs)?;
    let resp = apdu(&card, 0x00, 0x47, 0x80, 0x00, &[0xB6, 0x00])?;
    let point = find(&resp, 0x7F49).and_then(|pk| find(pk, 0x86)).ok_or(CardError::Parse)?;
    let comp = compress(point).ok_or(CardError::Parse)?;
    apdu(&card, 0x00, 0xDA, 0x00, 0xD6, &[0x02, 0x20])?;
    let ok = apdu(&card, 0x00, 0xCA, 0x00, 0xD6, &[])?.first() == Some(&2) && read_sig_pubkey(&card) == Some(comp);
    forget_admin(&card);
    if !ok { return Err(CardError::Unverified); }
    Ok(hex::encode(comp))
}

/// The slot must be EMPTY: 6A88 by the specification, 6581 from a YubiKey 5.4, and a key whose public key can be
/// read is occupied even with its fingerprints unset.
fn require_empty(card: &dyn Wire, serial: u32) -> Result<(), CardError> {
    let ad = app_data(card)?;
    if serial_of(&ad) != serial { return Err(CardError::Changed); }
    let disc = find(&ad, 0x73).ok_or(CardError::Parse)?;
    let fingerprints = find(disc, 0xC5).ok_or(CardError::Parse)?;
    if fingerprints.len() < 20 || fingerprints[..20].iter().any(|b| *b != 0) { return Err(CardError::Changed); }
    match apdu(card, 0x00, 0x47, 0x81, 0x00, &[0xB6, 0x00]) {
        Err(CardError::Sw { sw: 0x6A88 | 0x6581, .. }) => Ok(()),
        Ok(_) => Err(CardError::Changed),
        Err(e) => Err(e),
    }
}

/// Write a secret made OUTSIDE the key into the EMPTY signature slot (key import, PUT DATA 4D), lock touch to
/// FIXED, and read back the public key, which must be the one the secret implies. Needs the ADMIN PIN.
/// The template carries the scalar AND the public point: opcard (a Nitrokey 3) insists on both (measured 2026-10-10;
/// "wrong data" without the point); the specification allows it, and a YubiKey takes the scalar-only form, which is
/// tried second if the first is refused.
pub fn import_empty(reader: &str, admin_pin: &str, serial: u32, secret: &[u8; 32], point: &[u8; 65]) -> Result<String, CardError> { again(|| import_empty_once(reader, admin_pin, serial, secret, point)) }
fn import_empty_once(reader: &str, admin_pin: &str, serial: u32, secret: &[u8; 32], point: &[u8; 65]) -> Result<String, CardError> {
    let mut card = connect(reader)?;
    let card = card.transaction()?;
    require_empty(&card, serial)?;
    let want = compress(point).ok_or(CardError::Parse)?;
    verify(&card, 0x83, admin_pin)?;
    let mut attrs = vec![0x13]; attrs.extend_from_slice(OID_SECP256K1);
    apdu(&card, 0x00, 0xDA, 0x00, 0xC1, &attrs)?;
    // extended header list 4D { B6 00 | 7F48 { 92 20 [99 41] } | 5F48 { d [‖ Q] } }
    let template = |with_point: bool| -> Vec<u8> {
        let (tpl, mut data) = if with_point { (vec![0x7F, 0x48, 0x04, 0x92, 0x20, 0x99, 0x41], [&secret[..], &point[..]].concat()) } else { (vec![0x7F, 0x48, 0x02, 0x92, 0x20], secret.to_vec()) };
        let mut body = vec![0x5F, 0x48]; if data.len() > 127 { body.push(0x81); } body.push(data.len() as u8); body.append(&mut data);
        let inner = [&[0xB6, 0x00][..], &tpl, &body].concat();
        let mut out = vec![0x4D]; if inner.len() > 127 { out.push(0x81); } out.push(inner.len() as u8); out.extend_from_slice(&inner); out
    };
    match apdu(&card, 0x00, 0xDB, 0x3F, 0xFF, &template(true)) {
        Ok(_) => {}
        Err(CardError::Sw { sw: 0x6A80, .. }) => { apdu(&card, 0x00, 0xDB, 0x3F, 0xFF, &template(false))?; }
        Err(e) => { forget_admin(&card); return Err(e); }
    }
    let got = read_sig_pubkey(&card);
    if got != Some(want) { forget_admin(&card); return Err(CardError::Unverified); }
    apdu(&card, 0x00, 0xDA, 0x00, 0xD6, &[0x02, 0x20])?;
    let ok = apdu(&card, 0x00, 0xCA, 0x00, 0xD6, &[])?.first() == Some(&2);
    forget_admin(&card);
    if !ok { return Err(CardError::Unverified); }
    Ok(hex::encode(want))
}

pub(crate) fn sign_on_card(card: &dyn Wire, pin: &str, digest: &[u8;32]) -> Result<([u8;32], [u8;32]), CardError> {
    let mut raw = None;
    for _ in 0..4 {
        // verified before EVERY try: a Nitrokey 3 spends the PIN on a signature that times out, so a retry without
        // it would answer 6982 at once instead of waiting for the finger again. A YubiKey does not mind the repeat.
        verify(&card, 0x81, pin)?;
        match apdu(&card, 0x00, 0x2A, 0x9E, 0x9A, digest) {
            Ok(r) => { raw = Some(r); break; }
            Err(CardError::NoTouch) => continue,
            // A Nitrokey 3 answers 6982 when the touch does not come (about 15 s; its source says 6600 is "not
            // available" to it). The PIN was verified a moment ago on this very connection, so here 6982 is the touch.
            Err(CardError::Sw { sw: 0x6982, .. }) => continue,
            // A YubiKey closes its 15 s touch window with 6985 "conditions of use not satisfied" (seen 2026-10-19 on a
            // 5C Nano whose tap landed before the key was waiting). Same meaning, same answer: the next window.
            Err(CardError::Sw { sw: 0x6985, .. }) => continue,
            Err(e) => return Err(e),
        }
    }
    let raw = raw.ok_or(CardError::NoTouch)?;
    let (r, mut s) = split_rs(&raw).ok_or(CardError::Parse)?;
    if !is_low_s(&s) {
        s = neg_mod_n(&s);
    }
    Ok((r, s))
}

/// r and s from what the card answered. A YubiKey gives r‖s in 64 bytes; a Nitrokey 3 (opcard over its SE050) gives
/// each in a 64-byte field, right-aligned — 128 bytes in all (measured 2026-10-09: 30 of 30 verified read that way).
pub(crate) fn split_rs(raw: &[u8]) -> Option<([u8; 32], [u8; 32])> {
    if raw.len() != 64 && raw.len() != 128 { return None; }
    let half = raw.len() / 2;
    let fit = |x: &[u8]| -> Option<[u8; 32]> {
        let (pad, v) = x.split_at(half - 32);
        if pad.iter().any(|b| *b != 0) { return None; }
        let mut o = [0u8; 32]; o.copy_from_slice(v); Some(o)
    };
    let (a, b) = raw.split_at(half);
    Some((fit(a)?, fit(b)?))
}

/// Forget the key PIN on the card (VERIFY with P1 = FF). A Nitrokey 3 honours it; without it, its verified state lives
/// until the key is unplugged — a PC/SC reset does not clear it (measured) — so any other program on the computer
/// could read 0103 after us. A YubiKey answers 6B00 to it and resets on its own; the answer is not looked at.
pub(crate) fn forget_pin(card: &dyn Wire) {
    let _ = apdu(card, 0x00, 0x20, 0xFF, 0x82, &[]);
    let _ = apdu(card, 0x00, 0x20, 0xFF, 0x81, &[]);
}
/// The same for the ADMIN PIN, after any admin work. Measured 2026-10-10: a Nitrokey kept the admin verification the
/// app's setup walk had left, and a later script's TERMINATE DF rode on it — anything on the computer could have.
pub(crate) fn forget_admin(card: &dyn Wire) { let _ = apdu(card, 0x00, 0x20, 0xFF, 0x83, &[]); }

/// What one plugged-in card holds for the seal: readable with no PIN.
#[derive(Debug, Clone, Serialize)]
pub struct Sealed { pub reader: String, pub serial: u32, pub pubkey: Option<String>, pub container: Vec<u8>, pub room: usize }

pub(crate) fn serial_of(ad: &[u8]) -> u32 {
    find(ad, 0x4F).filter(|a| a.len() >= 14).map(|a| Maker::of(a).serial(a)).unwrap_or(0)
}
pub(crate) fn maker_of(ad: &[u8]) -> Maker { find(ad, 0x4F).map(Maker::of).unwrap_or(Maker::Other(0)) }

/// The cardholder-certificate object (7F21) of every plugged-in card, and how many bytes it can hold.
pub fn read_sealed() -> Result<Vec<Sealed>, CardError> { again(|| read_sealed_once()) }
fn read_sealed_once() -> Result<Vec<Sealed>, CardError> {
    let mut out = Vec::new();
    for reader in readers()? {
        // A KEY that is plugged in but unreadable is an error, not an absent key — so a failing vault key is never
        // quietly treated as missing. But a device that is not an OpenPGP key at all (a Bio, a FIDO-only key) is not
        // a key failing: it is skipped, and reported as plugged in and unusable by `others()`.
        let card = match probe(&reader)? { Probe::Key(c) => c, Probe::Other(_) => continue };
        let ad = app_data(&card)?;
        // extended capabilities (C0): bytes 4..6 = the longest certificate the card says it will store — capped at what
        // its maker's cards were measured to take
        let room = maker_of(&ad).cert_room(find(&ad, 0x73).and_then(|d| find(d, 0xC0)).filter(|c| c.len() >= 6).map(|c| ((c[4] as usize) << 8) | c[5] as usize).unwrap_or(0));
        let container = apdu(&card, 0x00, 0xCA, 0x7F, 0x21, &[])?;
        out.push(Sealed { serial: serial_of(&ad), pubkey: read_sig_pubkey(&card).map(hex::encode), reader, container, room });
    }
    Ok(out)
}

/// Replace the certificate object. `expect` is what the caller read before deciding: if the card holds anything
/// else by now, nothing is written. Needs the ADMIN PIN. The write is read back and compared before returning.
/// The whole thing runs inside one PC/SC transaction: the write is a CHAIN of commands, and on macOS another
/// program closing its own connection resets the card between two of them. A Nitrokey 3 caught that way (2026-10-09,
/// by this app's own 2 s poll under a test script) hung on both its interfaces until it was unplugged.
pub fn write_sealed(reader: &str, admin_pin: &str, expect: &[u8], data: &[u8]) -> Result<(), CardError> { again(|| write_sealed_once(reader, admin_pin, expect, data)) }
fn write_sealed_once(reader: &str, admin_pin: &str, expect: &[u8], data: &[u8]) -> Result<(), CardError> {
    let mut card = connect(reader)?;
    let card = card.transaction()?;
    if apdu(&card, 0x00, 0xCA, 0x7F, 0x21, &[])? != expect { return Err(CardError::Changed); }
    verify(&card, 0x83, admin_pin)?;
    // command chaining: every block but the last carries CLA bit 0x10
    let blocks: Vec<&[u8]> = data.chunks(255).collect();
    for (i, b) in blocks.iter().enumerate() {
        apdu(&card, if i + 1 < blocks.len() { 0x10 } else { 0x00 }, 0xDA, 0x7F, 0x21, b)?;
    }
    let ok = apdu(&card, 0x00, 0xCA, 0x7F, 0x21, &[])? == data;
    forget_admin(&card);
    if !ok { return Err(CardError::Unverified); }
    Ok(())
}

/// Factory-reset the OpenPGP applet: TERMINATE DF then ACTIVATE FILE, the documented reset on a YubiKey and the one
/// that brought a crashed Nitrokey 3 back on 2026-10-10. The signing key, both PINs, the certificate object (a seal
/// share) and the private objects (the store's unlock slot) are gone; passkeys and the other applets are untouched.
/// Needs the ADMIN PIN — a key whose admin PIN has no tries left accepts the reset without it, by the specification.
/// Checked afterwards: the slot empty, the attributes back to the factory, and nothing left in the certificate.
pub fn reset_openpgp(reader: &str, admin_pin: &str, serial: u32) -> Result<(), CardError> { again(|| reset_openpgp_once(reader, admin_pin, serial)) }
fn reset_openpgp_once(reader: &str, admin_pin: &str, serial: u32) -> Result<(), CardError> {
    let mut card = connect(reader)?;
    let card = card.transaction()?;
    let ad = app_data(&card)?;
    if serial_of(&ad) != serial { return Err(CardError::Changed); }
    let blocked = pin_tries(&card).map(|t| t[2] == 0).unwrap_or(false);
    if !blocked { verify(&card, 0x83, admin_pin)?; }
    apdu(&card, 0x00, 0xE6, 0x00, 0x00, &[])?;
    apdu(&card, 0x00, 0x44, 0x00, 0x00, &[])?;
    select(&card)?;
    let fresh = read_sig_pubkey(&card).is_none() && apdu(&card, 0x00, 0xCA, 0x7F, 0x21, &[]).map(|c| c.is_empty()).unwrap_or(false);
    if !fresh { return Err(CardError::Unverified); }
    Ok(())
}

/// The secret that locks this key's shares. It lives in the OpenPGP private object 0103, which the card gives up only
/// after the USER PIN (VERIFY with P2 = 82, the mode for everything that is not a signature) and lets be written under
/// the same PIN. `create`: make and store a fresh 32 random bytes if the object is empty. Anything else found there is
/// left alone. The write is read back before it is trusted.
pub fn seal_key(reader: &str, pin: &str, create: bool) -> Result<Option<[u8; 32]>, CardError> { again(|| seal_key_once(reader, pin, create)) }
fn seal_key_once(reader: &str, pin: &str, create: bool) -> Result<Option<[u8; 32]>, CardError> {
    let mut card = connect(reader)?;
    let card = card.transaction()?;
    verify(&card, 0x82, pin)?;
    let out = seal_key_verified(&card, create);
    forget_pin(&card);   // the secret stays behind the PIN once we have it
    out
}
fn seal_key_verified(card: &dyn Wire, create: bool) -> Result<Option<[u8; 32]>, CardError> {
    let parse = |v: &[u8]| -> Result<Option<[u8; 32]>, CardError> {
        if v.is_empty() { return Ok(None); }
        if v.len() != 36 || &v[..4] != crate::seal::KEY_MAGIC { return Err(CardError::PrivateInUse); }
        let mut k = [0u8; 32]; k.copy_from_slice(&v[4..]); Ok(Some(k))
    };
    if let Some(k) = parse(&apdu(&card, 0x00, 0xCA, 0x01, 0x03, &[])?)? { return Ok(Some(k)); }
    if !create { return Ok(None); }
    use bitcoin::secp256k1::rand::RngCore;
    let mut k = [0u8; 32];
    bitcoin::secp256k1::rand::rngs::OsRng.fill_bytes(&mut k);
    let body = [&crate::seal::KEY_MAGIC[..], &k[..]].concat();
    apdu(&card, 0x00, 0xDA, 0x01, 0x03, &body)?;
    if apdu(&card, 0x00, 0xCA, 0x01, 0x03, &[])? != body { return Err(CardError::Unverified); }
    Ok(Some(k))
}

fn is_low_s(s: &[u8; 32]) -> bool {
    // s <= n/2  ⇔  s < n - s  (compare against half order)
    const HALF: [u8; 32] = [
        0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0x5D, 0x57, 0x6E, 0x73, 0x57, 0xA4, 0x50, 0x1D, 0xDF, 0xE9, 0x2F, 0x46, 0x68, 0x1B, 0x20, 0xA0,
    ];
    s <= &HALF
}

fn neg_mod_n(s: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut borrow = 0i16;
    for i in (0..32).rev() {
        let d = ORDER[i] as i16 - s[i] as i16 - borrow;
        if d < 0 { out[i] = (d + 256) as u8; borrow = 1 } else { out[i] = d as u8; borrow = 0 }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maker_serial_and_device_come_from_the_aid_not_the_reader() {
        let aid = |maker: [u8; 2], serial: [u8; 4]| [&[0xD2, 0x76, 0x00, 0x01, 0x24, 0x01, 0x03, 0x04][..], &maker, &serial, &[0, 0]].concat();
        let yk = aid([0x00, 0x06], [0x27, 0x44, 0x60, 0x43]);   // the 5C Nano, serial 27446043 in BCD
        let nk = aid([0x00, 0x0F], [0x18, 0x16, 0xB5, 0xF4]);   // the Nitrokey 3, serial = the first bytes of its UUID
        assert_eq!(Maker::of(&yk), Maker::Yubico); assert_eq!(Maker::of(&yk).serial(&yk), 27446043); assert_eq!(Maker::of(&yk).device(), "yubikey");
        assert_eq!(Maker::of(&nk), Maker::Nitrokey); assert_eq!(Maker::of(&nk).serial(&nk), 0x1816B5F4); assert_eq!(Maker::of(&nk).device(), "nitrokey3"); assert_eq!(Maker::of(&nk).form(), FORM_NITROKEY3);
        // a Nitrokey whose serial happens to be all decimal digits must NOT be read as BCD
        let nk2 = aid([0x00, 0x0F], [0x12, 0x34, 0x56, 0x78]);
        assert_eq!(Maker::of(&nk2).serial(&nk2), 0x12345678);
        assert_eq!(Maker::of(&[0u8; 8]), Maker::Other(0));
        assert_eq!(Maker::Nitrokey.cert_room(4096), 3240); assert_eq!(Maker::Nitrokey.cert_room(2000), 2000); assert_eq!(Maker::Yubico.cert_room(2048), 2048);
    }

    #[test]
    fn a_signature_is_read_from_64_or_128_bytes_and_from_nothing_else() {
        let r = [0x11u8; 32]; let s = [0x22u8; 32];
        assert_eq!(split_rs(&[r, s].concat()), Some((r, s)));
        let padded = [[0u8; 32].as_slice(), &r, &[0u8; 32], &s].concat();
        assert_eq!(split_rs(&padded), Some((r, s)));
        let mut bad = padded.clone(); bad[3] = 1;   // a non-zero pad byte is a value that does not fit
        assert_eq!(split_rs(&bad), None);
        assert_eq!(split_rs(&[0u8; 63]), None); assert_eq!(split_rs(&[0u8; 70]), None); assert_eq!(split_rs(&[]), None);
    }

    /// The whole Nitrokey 3 path on a FACTORY-FRESH key with its factory PINs, undone by a factory reset at the end:
    /// `A2K_NITROKEY_FACTORY_TEST=1 cargo test hardware_nitrokey -- --ignored --nocapture`. It refuses any key that
    /// is not a Nitrokey, or not fresh. Touch stays OFF so no finger is needed; the touch path is proven in the app.
    #[test]
    #[ignore]
    fn hardware_nitrokey_factory_roundtrip() {
        use bitcoin::secp256k1::{ecdsa::Signature, Message, PublicKey, Secp256k1};
        if std::env::var("A2K_NITROKEY_FACTORY_TEST").as_deref() != Ok("1") { eprintln!("set A2K_NITROKEY_FACTORY_TEST=1 to run this against a factory-fresh Nitrokey 3"); return; }
        let (reader, card) = readers().unwrap().into_iter().find_map(|r| match probe(&r) {
            Ok(Probe::Key(c)) => { let ad = app_data(&c).ok()?; (maker_of(&ad) == Maker::Nitrokey).then_some((r, c)) }
            _ => None,
        }).expect("a Nitrokey 3 plugged in");
        let ad = app_data(&card).unwrap();
        let (facts, ours) = crate::keyinit::pgp_facts(&card).unwrap();
        assert_eq!(crate::keyinit::judge(&facts, ours.is_some()).0, "fresh", "not a factory-fresh key: {facts:?}");
        let serial = serial_of(&ad);
        println!("Nitrokey 3 serial {serial:#x} on {reader}");
        // a secp256k1 key, touch left off, the PIN good for many signatures
        verify(&card, 0x83, "12345678").unwrap();
        let mut attrs = vec![0x13]; attrs.extend_from_slice(OID_SECP256K1);
        apdu(&card, 0x00, 0xDA, 0x00, 0xC1, &attrs).unwrap();
        apdu(&card, 0x00, 0x47, 0x80, 0x00, &[0xB6, 0x00]).unwrap();
        apdu(&card, 0x00, 0xDA, 0x00, 0xC4, &[0x01]).unwrap();
        let pk = PublicKey::from_slice(&read_sig_pubkey(&card).unwrap()).unwrap();
        let secp = Secp256k1::verification_only();
        let compact = |r: [u8; 32], s: [u8; 32]| { let mut c = [0u8; 64]; c[..32].copy_from_slice(&r); c[32..].copy_from_slice(&s); Signature::from_compact(&c).unwrap() };
        // eight signatures of one digest: every one verifies, and (random nonces) they differ
        let z = [7u8; 32]; let mut seen = std::collections::HashSet::new();
        for _ in 0..8 { let (r, s) = sign_on_card(&card, "123456", &z).unwrap(); secp.verify_ecdsa(&Message::from_digest(z), &compact(r, s), &pk).unwrap(); seen.insert(r); }
        assert!(seen.len() > 1, "eight signatures of one digest were all the same");
        // the rotated child: the card signs z·T⁻¹, the lifted s verifies under the child key
        let cc = [9u8; 32]; let t = crate::hd::child_scalar(&cc, &pk, 5);
        let (r, sc) = sign_on_card(&card, "123456", &crate::hd::card_digest(&z, &t)).unwrap();
        secp.verify_ecdsa(&Message::from_digest(z), &compact(r, crate::hd::lift_s(&sc, &t)), &crate::hd::child_pubkey(&cc, &pk, 5)).unwrap();
        drop(card);
        // the seal's objects through the app's own doors: the real room, a full write, the PIN-locked secret
        let me = read_sealed().unwrap().into_iter().find(|s| s.reader == reader).unwrap();
        assert_eq!((me.room, me.serial, me.container.len()), (3240, serial, 0));
        write_sealed(&reader, "12345678", &[], &vec![0xA5; 3240]).unwrap();
        let k = seal_key(&reader, "123456", true).unwrap().unwrap();
        assert_eq!(seal_key(&reader, "123456", false).unwrap(), Some(k));
        let c2 = connect(&reader).unwrap();
        assert!(matches!(apdu(&c2, 0x00, 0xCA, 0x01, 0x03, &[]), Err(CardError::Sw { sw: 0x6982, .. })), "0103 must be behind the PIN again once seal_key is done");
        // back to the factory: TERMINATE DF under the admin PIN, ACTIVATE FILE
        verify(&c2, 0x83, "12345678").unwrap();
        apdu(&c2, 0x00, 0xE6, 0x00, 0x00, &[]).unwrap(); apdu(&c2, 0x00, 0x44, 0x00, 0x00, &[]).unwrap(); select(&c2).unwrap();
        assert!(read_sig_pubkey(&c2).is_none()); assert_eq!(apdu(&c2, 0x00, 0xCA, 0x7F, 0x21, &[]).unwrap().len(), 0);
        let (facts, _) = crate::keyinit::pgp_facts(&c2).unwrap(); assert_eq!(crate::keyinit::judge(&facts, false).0, "fresh");
        println!("round trip done; the key is back at the factory");
    }

    /// Import on a FACTORY-FRESH Nitrokey with its factory PINs, undone by a factory reset:
    /// `A2K_NITROKEY_FACTORY_TEST=1 cargo test hardware_nitrokey_import -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn hardware_nitrokey_import_roundtrip() {
        if std::env::var("A2K_NITROKEY_FACTORY_TEST").as_deref() != Ok("1") { eprintln!("set A2K_NITROKEY_FACTORY_TEST=1"); return; }
        let (reader, card) = readers().unwrap().into_iter().find_map(|r| match probe(&r) {
            Ok(Probe::Key(c)) => { let ad = app_data(&c).ok()?; (maker_of(&ad) == Maker::Nitrokey).then_some((r, c)) }
            _ => None,
        }).expect("a Nitrokey 3 plugged in");
        let ad = app_data(&card).unwrap(); let serial = serial_of(&ad);
        let (facts, ours) = crate::keyinit::pgp_facts(&card).unwrap();
        assert_eq!(crate::keyinit::judge(&facts, ours.is_some()).0, "fresh", "not a factory-fresh key: {facts:?}");
        drop(card);
        let secret = crate::secret::secret_of(&crate::secret::from_hex("1e99423a4ed27608a15a2616a2b0e9e52ced330ac530edcc32c8ffc6a526aedd").unwrap()).unwrap();
        let pk = import_empty(&reader, "12345678", serial, &secret, &crate::secret::point_of(&secret)).unwrap();
        assert_eq!(pk, crate::secret::pubkey_of(&secret));
        assert!(matches!(import_empty(&reader, "12345678", serial, &secret, &crate::secret::point_of(&secret)), Err(CardError::Changed)), "an occupied slot is refused");
        // admin forgotten: a fresh connection cannot write C4 without the PIN
        let c2 = connect(&reader).unwrap();
        assert!(matches!(apdu(&c2, 0x00, 0xDA, 0x00, 0xC4, &[0x01]), Err(CardError::Sw { sw: 0x6982, .. })), "the admin PIN must be forgotten after the import");
        verify(&c2, 0x83, "12345678").unwrap(); apdu(&c2, 0x00, 0xE6, 0x00, 0x00, &[]).unwrap(); apdu(&c2, 0x00, 0x44, 0x00, 0x00, &[]).unwrap(); select(&c2).unwrap();
        assert!(read_sig_pubkey(&c2).is_none()); println!("import round trip done; key back at the factory");
    }

    /// Needs a key plugged in: `cargo test hardware_read -- --ignored --nocapture`. Reads only.
    #[test]
    #[ignore]
    fn hardware_read_sealed() {
        for c in super::read_sealed().unwrap() { println!("serial {} pubkey {:?} container {} bytes, room {}", c.serial, c.pubkey, c.container.len(), c.room); assert!(c.room >= 255); }
    }

    /// The 2026-10-19 fault, as a check that needs a key plugged in: one handle held open and asked every 300 ms
    /// while two threads run the card-list poll flat out for 8 s. Before transport.rs held a transaction and left
    /// the card alone on drop, the held handle died within two seconds ("the smart card was reset").
    /// `cargo test hardware_polls_do_not_reset_a_held_card -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn hardware_polls_do_not_reset_a_held_card() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        let reader = readers().unwrap().into_iter().find(|r| matches!(probe(r), Ok(Probe::Key(_)))).expect("plug in a key");
        let stop = Arc::new(AtomicBool::new(false));
        let passes = Arc::new(AtomicUsize::new(0));
        let pollers: Vec<_> = (0..2).map(|_| { let (stop, passes) = (stop.clone(), passes.clone()); std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) { if list().is_ok() { passes.fetch_add(1, Ordering::Relaxed); } }
        }) }).collect();
        std::thread::sleep(Duration::from_millis(300));
        let held = connect(&reader).expect("open the key");
        let start = Instant::now();
        let mut asks = 0;
        while start.elapsed() < Duration::from_secs(8) {
            apdu(&held, 0x00, 0xCA, 0x00, 0x4F, &[]).unwrap_or_else(|e| panic!("held handle failed after {asks} asks: {e}"));
            asks += 1;
            std::thread::sleep(Duration::from_millis(300));
        }
        drop(held);
        stop.store(true, Ordering::Relaxed);
        for p in pollers { p.join().unwrap(); }
        println!("held handle answered {asks} times over 8 s while the poll completed {} passes on two threads", passes.load(Ordering::Relaxed));
        assert!(passes.load(Ordering::Relaxed) > 0, "the polls never completed a pass");
    }
}
