//! YubiKey OpenPGP applet over PC/SC: the six APDUs a Bitcoin cosigner needs.
//! Ported byte-for-byte from the Python proof (yubikit) that signed on mainnet.
use pcsc::{Card, Context, Protocols, Scope, ShareMode};
use serde::Serialize;
use thiserror::Error;

const AID_OPENPGP: &[u8] = &[0xD2, 0x76, 0x00, 0x01, 0x24, 0x01];
const OID_SECP256K1: &[u8] = &[0x2B, 0x81, 0x04, 0x00, 0x0A];
const ORDER: [u8; 32] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
];

#[derive(Debug, Error)]
pub enum CardError {
    #[error("smartcard service: {0}")]
    Pcsc(#[from] pcsc::Error),
    #[error("no YubiKey found")]
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
    pub form: u8,                 // YubiKey form factor: 1 USB-A · 2 USB-A Nano · 3 USB-C · 4 USB-C Nano · 5 USB-C/Lightning · 0 unknown
    pub nfc: bool,                // the key has an NFC antenna (management config tag 0x0D)
    pub firmware: String,         // e.g. 5.4.3, from the management applet; empty if it would not say
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
fn apdu(card: &Card, cla: u8, ins: u8, p1: u8, p2: u8, data: &[u8]) -> Result<Vec<u8>, CardError> {
    let mut cmd = vec![cla, ins, p1, p2];
    if !data.is_empty() {
        cmd.push(data.len() as u8);
        cmd.extend_from_slice(data);
    }
    cmd.push(0x00); // Le
    let mut out = Vec::new();
    let mut buf = vec![0u8; 4096];
    let mut resp = card.transmit(&cmd, &mut buf)?.to_vec();
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
                let mut b = vec![0u8; 4096];
                resp = card.transmit(&[0x00, 0xC0, 0x00, 0x00, sw2], &mut b)?.to_vec();
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
fn tlv(buf: &[u8]) -> Option<(u16, &[u8], &[u8])> {
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

fn find(buf: &[u8], want: u16) -> Option<&[u8]> {
    let mut rest = buf;
    while let Some((t, v, r)) = tlv(rest) {
        if t == want {
            return Some(v);
        }
        rest = r;
    }
    None
}

fn select(card: &Card) -> Result<(), CardError> {
    apdu(card, 0x00, 0xA4, 0x04, 0x00, AID_OPENPGP).map(|_| ())
}

/// Application related data — the card answers with the whole 6E object; return its contents.
fn app_data(card: &Card) -> Result<Vec<u8>, CardError> {
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

fn read_sig_pubkey(card: &Card) -> Option<[u8; 33]> {
    let resp = apdu(card, 0x00, 0x47, 0x81, 0x00, &[0xB6, 0x00]).ok()?;
    let pk = find(&resp, 0x7F49)?;
    let point = find(pk, 0x86)?;
    compress(point)
}

fn connect(ctx: &Context, reader: &str) -> Result<Card, CardError> {
    let cname = std::ffi::CString::new(reader).map_err(|_| CardError::Parse)?;
    let card = ctx.connect(&cname, ShareMode::Shared, Protocols::ANY)?;
    select(&card)?;
    Ok(card)
}

/// What a plugged-in Yubico device turned out to be, judged by what it ANSWERED rather than by its name.
/// A reader with "yubi" in its name is only a Yubico device; whether it can hold a key for this app is decided by
/// whether it answers the OpenPGP applet. A YubiKey Bio, a FIDO-only key or a PIV-only one is plugged in and
/// working perfectly well — it is just not a key this app can use, and must never be mistaken for a key that is
/// failing.
pub enum Probe { Key(Card), Other(String) }

#[derive(Debug, Clone, serde::Serialize)]
pub struct OtherDevice { pub reader: String, pub why: String }

fn probe(ctx: &Context, reader: &str) -> Result<Probe, CardError> {
    let cname = std::ffi::CString::new(reader).map_err(|_| CardError::Parse)?;
    let card = match ctx.connect(&cname, ShareMode::Shared, Protocols::ANY) {
        Ok(c) => c,
        // listed a moment ago and gone now, or a reader with nothing a smart-card service can talk to: that is not
        // a key failing, it is not a key. (A YubiKey Bio showed up exactly this way on 2026-09-21.)
        Err(pcsc::Error::UnknownReader) | Err(pcsc::Error::NoSmartcard) | Err(pcsc::Error::RemovedCard)
        | Err(pcsc::Error::ReaderUnavailable) | Err(pcsc::Error::UnpoweredCard) | Err(pcsc::Error::UnresponsiveCard) =>
            return Ok(Probe::Other("the computer can see it, but it does not answer as a smart card".into())),
        Err(e) => return Err(e.into()),
    };
    match select(&card) {
        Ok(()) => Ok(Probe::Key(card)),
        Err(_) => Ok(Probe::Other("it has no OpenPGP applet, which is what Any Two Keys signs with".into())),
    }
}

/// Every Yubico device plugged in that is NOT a key this app can use, and why.
pub fn others() -> Vec<OtherDevice> {
    let Ok(ctx) = Context::establish(Scope::User) else { return vec![] };
    let Ok(names) = readers(&ctx) else { return vec![] };
    names.into_iter().filter_map(|reader| match probe(&ctx, &reader) {
        Ok(Probe::Other(why)) => Some(OtherDevice { reader, why }),
        _ => None,
    }).collect()
}

fn readers(ctx: &Context) -> Result<Vec<String>, CardError> {
    let mut buf = vec![0u8; 2048];
    let names = ctx.list_readers(&mut buf)?;
    Ok(names.map(|n| n.to_string_lossy().into_owned()).filter(|n| n.to_lowercase().contains("yubi")).collect())
}

/// The key's shape, NFC and firmware, from the YubiKey management applet (READ CONFIG). Leaves OpenPGP selected again.
fn hardware(card: &Card) -> (u8, bool, String) {
    const AID_MGMT: &[u8] = &[0xA0, 0x00, 0x00, 0x05, 0x27, 0x47, 0x11, 0x17];
    let cfg = apdu(card, 0x00, 0xA4, 0x04, 0x00, AID_MGMT).and_then(|_| apdu(card, 0x00, 0x1D, 0x00, 0x00, &[])).ok();
    let tlvs = cfg.as_deref().and_then(|c| c.get(1..)).unwrap_or(&[]);
    let form = find(tlvs, 0x04).and_then(|v| v.first().copied()).map(|f| f & 0x0F).unwrap_or(0);
    let nfc = find(tlvs, 0x0D).map(|v| v.iter().any(|b| *b != 0)).unwrap_or(false);
    let firmware = find(tlvs, 0x05).filter(|v| v.len() == 3).map(|v| format!("{}.{}.{}", v[0], v[1], v[2])).unwrap_or_default();
    let _ = select(card);
    (form, nfc, firmware)
}

pub fn list() -> Result<Vec<CardInfo>, CardError> {
    let ctx = Context::establish(Scope::User)?;
    let mut out = Vec::new();
    for reader in readers(&ctx)? {
        let card = match probe(&ctx, &reader) { Ok(Probe::Key(c)) => c, _ => continue };
        let ad = app_data(&card)?;
        let aid = find(&ad, 0x4F).ok_or(CardError::Parse)?;
        if aid.len() < 14 {
            return Err(CardError::Parse);
        }
        // AID: D2760001 2401 | OpenPGP spec version (2) | manufacturer (2) | serial (4) | RFU (2).
        // YubiKeys write the DECIMAL serial as hex digits (BCD): 27446043 → 0x27446043.
        let version = format!("OpenPGP {}.{}", aid[6], aid[7]);
        let serial = u32::from_str_radix(&hex::encode(&aid[10..14]), 10).unwrap_or(u32::from_be_bytes([aid[10], aid[11], aid[12], aid[13]]));
        // algorithm attributes for SIG live inside 6E → 73 → C1; only a secp256k1 key counts
        let disc = find(&ad, 0x73).unwrap_or(&[]);
        let is_k1 = find(disc, 0xC1).map(|a| a.len() >= 6 && a[0] == 0x13 && &a[1..6] == OID_SECP256K1).unwrap_or(false);
        let pubkey = if is_k1 { read_sig_pubkey(&card).map(hex::encode) } else { None };
        let touch = apdu(&card, 0x00, 0xCA, 0x00, 0xD6, &[]).ok()
            .and_then(|v| v.first().copied())
            .map(|u| match u { 0 => "OFF", 1 => "ON", 2 => "FIXED", 3 => "CACHED", 4 => "CACHED_FIXED", _ => "?" }.to_string())
            .unwrap_or_else(|| "?".into());
        let (form, nfc, firmware) = hardware(&card);
        out.push(CardInfo { reader, serial, version, pubkey, touch, form, nfc, firmware });
    }
    if out.is_empty() {
        return Err(CardError::NotFound);
    }
    Ok(out)
}

/// Generate a fresh secp256k1 key in the SIGNATURE slot and lock touch to FIXED.
/// Replaces whatever was in the slot. Needs the ADMIN PIN.
pub fn generate(reader: &str, admin_pin: &str) -> Result<String, CardError> {
    let ctx = Context::establish(Scope::User)?;
    let card = connect(&ctx, reader)?;
    apdu(&card, 0x00, 0x20, 0x00, 0x83, admin_pin.as_bytes())?;
    let mut attrs = vec![0x13];
    attrs.extend_from_slice(OID_SECP256K1);
    apdu(&card, 0x00, 0xDA, 0x00, 0xC1, &attrs)?;
    let resp = apdu(&card, 0x00, 0x47, 0x80, 0x00, &[0xB6, 0x00])?;
    let point = find(&resp, 0x7F49).and_then(|pk| find(pk, 0x86)).ok_or(CardError::Parse)?;
    let comp = compress(point).ok_or(CardError::Parse)?;
    // touch: FIXED (2), button (0x20). One-way: the card refuses to lower it afterwards.
    apdu(&card, 0x00, 0xDA, 0x00, 0xD6, &[0x02, 0x20])?;
    if apdu(&card, 0x00, 0xCA, 0x00, 0xD6, &[])?.first() != Some(&2) { return Err(CardError::Unverified); }
    Ok(hex::encode(comp))
}

/// ECDSA over a 32-byte digest with the SIG key: user PIN, then a touch (waited for up to ~60 s).
/// Returns (r, s) with low-S.
pub fn sign_digest(reader: &str, pin: &str, digest: &[u8; 32]) -> Result<([u8; 32], [u8; 32]), CardError> {
    let ctx = Context::establish(Scope::User)?;
    let card = connect(&ctx, reader)?;
    sign_on_card(&card, pin, digest)
}

/// Recheck the public key and uncached FIXED touch policy on the SAME connection
/// that verifies the PIN and signs; never sign after a card swap or policy change.
pub fn sign_digest_checked(reader: &str, pin: &str, digest: &[u8;32], expected: &[u8;33]) -> Result<([u8;32], [u8;32]), CardError> {
    let ctx = Context::establish(Scope::User)?;
    let card = connect(&ctx, reader)?;
    if read_sig_pubkey(&card).as_ref() != Some(expected) { return Err(CardError::Changed); }
    if apdu(&card, 0x00, 0xCA, 0x00, 0xD6, &[])?.first() != Some(&2) { return Err(CardError::Unverified); }
    sign_on_card(&card, pin, digest)
}

/// Spark setup is never allowed to replace a slot, including a non-secp256k1 key.
pub fn generate_empty(reader: &str, admin_pin: &str, serial: u32) -> Result<String, CardError> {
    let ctx = Context::establish(Scope::User)?;
    let card = connect(&ctx, reader)?;
    let ad = app_data(&card)?;
    if serial_of(&ad) != serial { return Err(CardError::Changed); }
    let disc = find(&ad, 0x73).ok_or(CardError::Parse)?;
    let fingerprints = find(disc, 0xC5).ok_or(CardError::Parse)?;
    if fingerprints.len() < 20 || fingerprints[..20].iter().any(|b| *b != 0) { return Err(CardError::Changed); }
    // Even a key with unset fingerprints is occupied if its public key can be read.
    match apdu(&card, 0x00, 0x47, 0x81, 0x00, &[0xB6, 0x00]) {
        Err(CardError::Sw { sw: 0x6A88, .. }) => {},
        Ok(_) => return Err(CardError::Changed),
        Err(e) => return Err(e),
    }
    apdu(&card, 0x00, 0x20, 0x00, 0x83, admin_pin.as_bytes())?;
    let mut attrs = vec![0x13]; attrs.extend_from_slice(OID_SECP256K1);
    apdu(&card, 0x00, 0xDA, 0x00, 0xC1, &attrs)?;
    let resp = apdu(&card, 0x00, 0x47, 0x80, 0x00, &[0xB6, 0x00])?;
    let point = find(&resp, 0x7F49).and_then(|pk| find(pk, 0x86)).ok_or(CardError::Parse)?;
    let comp = compress(point).ok_or(CardError::Parse)?;
    apdu(&card, 0x00, 0xDA, 0x00, 0xD6, &[0x02, 0x20])?;
    if apdu(&card, 0x00, 0xCA, 0x00, 0xD6, &[])?.first() != Some(&2) || read_sig_pubkey(&card) != Some(comp) { return Err(CardError::Unverified); }
    Ok(hex::encode(comp))
}

fn sign_on_card(card: &Card, pin: &str, digest: &[u8;32]) -> Result<([u8;32], [u8;32]), CardError> {
    apdu(&card, 0x00, 0x20, 0x00, 0x81, pin.as_bytes())?;
    let mut raw = None;
    for _ in 0..4 {
        match apdu(&card, 0x00, 0x2A, 0x9E, 0x9A, digest) {
            Ok(r) => { raw = Some(r); break; }
            Err(CardError::NoTouch) => continue,
            Err(e) => return Err(e),
        }
    }
    let raw = raw.ok_or(CardError::NoTouch)?;
    if raw.len() != 64 {
        return Err(CardError::Parse);
    }
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&raw[..32]);
    s.copy_from_slice(&raw[32..]);
    if !is_low_s(&s) {
        s = neg_mod_n(&s);
    }
    Ok((r, s))
}

/// What one plugged-in card holds for the seal: readable with no PIN.
#[derive(Debug, Clone, Serialize)]
pub struct Sealed { pub reader: String, pub serial: u32, pub pubkey: Option<String>, pub container: Vec<u8>, pub room: usize }

fn serial_of(ad: &[u8]) -> u32 {
    find(ad, 0x4F).filter(|a| a.len() >= 14).map(|a| u32::from_str_radix(&hex::encode(&a[10..14]), 10).unwrap_or(0)).unwrap_or(0)
}

/// The cardholder-certificate object (7F21) of every plugged-in card, and how many bytes it can hold.
pub fn read_sealed() -> Result<Vec<Sealed>, CardError> {
    let ctx = Context::establish(Scope::User)?;
    let mut out = Vec::new();
    for reader in readers(&ctx)? {
        // A KEY that is plugged in but unreadable is an error, not an absent key — so a failing vault key is never
        // quietly treated as missing. But a device that is not an OpenPGP key at all (a Bio, a FIDO-only key) is not
        // a key failing: it is skipped, and reported as plugged in and unusable by `others()`.
        let card = match probe(&ctx, &reader)? { Probe::Key(c) => c, Probe::Other(_) => continue };
        let ad = app_data(&card)?;
        // extended capabilities (C0): bytes 4..6 = the longest certificate the card will store
        let room = find(&ad, 0x73).and_then(|d| find(d, 0xC0)).filter(|c| c.len() >= 6).map(|c| ((c[4] as usize) << 8) | c[5] as usize).unwrap_or(0);
        let container = apdu(&card, 0x00, 0xCA, 0x7F, 0x21, &[])?;
        out.push(Sealed { serial: serial_of(&ad), pubkey: read_sig_pubkey(&card).map(hex::encode), reader, container, room });
    }
    Ok(out)
}

/// Replace the certificate object. `expect` is what the caller read before deciding: if the card holds anything
/// else by now, nothing is written. Needs the ADMIN PIN. The write is read back and compared before returning.
pub fn write_sealed(reader: &str, admin_pin: &str, expect: &[u8], data: &[u8]) -> Result<(), CardError> {
    let ctx = Context::establish(Scope::User)?;
    let card = connect(&ctx, reader)?;
    if apdu(&card, 0x00, 0xCA, 0x7F, 0x21, &[])? != expect { return Err(CardError::Changed); }
    apdu(&card, 0x00, 0x20, 0x00, 0x83, admin_pin.as_bytes())?;
    // command chaining: every block but the last carries CLA bit 0x10
    let blocks: Vec<&[u8]> = data.chunks(255).collect();
    for (i, b) in blocks.iter().enumerate() {
        apdu(&card, if i + 1 < blocks.len() { 0x10 } else { 0x00 }, 0xDA, 0x7F, 0x21, b)?;
    }
    if apdu(&card, 0x00, 0xCA, 0x7F, 0x21, &[])? != data { return Err(CardError::Unverified); }
    Ok(())
}

/// The secret that locks this key's shares. It lives in the OpenPGP private object 0103, which the card gives up only
/// after the USER PIN (VERIFY with P2 = 82, the mode for everything that is not a signature) and lets be written under
/// the same PIN. `create`: make and store a fresh 32 random bytes if the object is empty. Anything else found there is
/// left alone. The write is read back before it is trusted.
pub fn seal_key(reader: &str, pin: &str, create: bool) -> Result<Option<[u8; 32]>, CardError> {
    let ctx = Context::establish(Scope::User)?;
    let card = connect(&ctx, reader)?;
    apdu(&card, 0x00, 0x20, 0x00, 0x82, pin.as_bytes())?;
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
    /// Needs a key plugged in: `cargo test hardware_read -- --ignored --nocapture`. Reads only.
    #[test]
    #[ignore]
    fn hardware_read_sealed() {
        for c in super::read_sealed().unwrap() { println!("serial {} pubkey {:?} container {} bytes, room {}", c.serial, c.pubkey, c.container.len(), c.room); assert!(c.room >= 255); }
    }
}
