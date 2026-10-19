//! The wire to a key. Everything the app says to a key is a sequence of APDUs written once, in `cards.rs` and
//! `keyinit.rs`; this is only how the bytes travel. On a computer that is PC/SC. On a phone it is the operating
//! system's own smart-card service for a key in the USB-C port, or an NFC session for a key held to the phone —
//! both behind one small Swift plugin (`plugins/keylink`).
use crate::cards::CardError;

/// One command out, one answer back (status word included).
pub trait Wire {
    fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError>;
    /// VERIFY with the PIN typed on the READER's own keypad (PC/SC part 10, VERIFY_PIN_DIRECT): the reader builds the
    /// command and sends it to the card; this computer never sees a digit. `None` when the reader has no such keypad.
    /// `p2` is the PIN wanted: 81 / 82 the key PIN, 83 the admin PIN.
    fn pinpad_verify(&self, p2: u8) -> Option<Result<(), CardError>> { let _ = p2; None }
    /// CHANGE REFERENCE DATA with BOTH PINs typed on the reader's keypad (MODIFY_PIN_DIRECT): the old one, the new one,
    /// the new one again. Neither ever reaches this computer — the way to replace a PIN that once did.
    fn pinpad_modify(&self, p2: u8) -> Option<Result<(), CardError>> { let _ = p2; None }
    /// Whether the reader this card sits in has that keypad.
    fn pinpad(&self) -> bool { false }
}
impl<T: Wire + ?Sized> Wire for &T {
    fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError> { (**self).send(cmd) }
    fn pinpad_verify(&self, p2: u8) -> Option<Result<(), CardError>> { (**self).pinpad_verify(p2) }
    fn pinpad_modify(&self, p2: u8) -> Option<Result<(), CardError>> { (**self).pinpad_modify(p2) }
    fn pinpad(&self) -> bool { (**self).pinpad() }
}

/// What was behind a reader's name: a card that answers, or something a smart-card service cannot talk to.
pub enum Opened { Card(Card), NotACard }

pub use imp::{open, readers, Card, Tx};

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod imp {
    use super::{Opened, Wire};
    use crate::cards::CardError;
    use pcsc::ffi;
    use pcsc::{Context, Scope};

    /// A card held for one whole conversation: connected shared, inside a PC/SC transaction from `open` until it is
    /// dropped, and let go with SCARD_LEAVE_CARD.
    ///
    /// Two things this replaces, both measured on 2026-10-19 with a YubiKey 5C Nano. (1) The pcsc crate's `Card`
    /// disconnects with SCARD_RESET_CARD when dropped, so every pass of the 2 s card-list poll RESET the key on its
    /// way out: a shared handle held by anyone else died within two seconds, a signature waiting for a touch was
    /// cut from under it, and a poll landing between another poll's SELECT and its public-key read made a key with
    /// 89 signatures behind it look empty ("ready to be initialised"). (2) Nothing kept another handle's commands
    /// out of a sequence that must not be interleaved: VERIFY then PSO, SELECT then GET DATA. A transaction does,
    /// and on macOS a second BeginTransaction on the same card blocks until the first ends (measured), so two polls
    /// or a poll and a signature simply take turns. A Nitrokey 3 has lost its key store to one interrupted write
    /// before, so the gentle disposition matters there as much as the serialising does.
    pub struct Card { ctx: ffi::SCARDCONTEXT, handle: ffi::SCARDHANDLE, proto: ffi::DWORD }
    /// Kept for the call sites that ask for one explicitly: the transaction is already held from `open`.
    pub struct Tx<'a>(&'a Card);
    impl Card {
        pub fn transaction(&mut self) -> Result<Tx<'_>, CardError> { Ok(Tx(self)) }

        /// Power the card down and up again, still ours. Measured 2026-10-19 on a REINER cyberJack RFID komfort: a
        /// Token2 card slid past the reader's antenna on its way into the slot, the reader opened a contactless
        /// session (T=1, the 3B 8F 80 01 pseudo-ATR) and kept it once the card was seated, and every command on that
        /// stale session came back 6F00 — the card looked like "no OpenPGP applet" for ten minutes. One cold reset
        /// re-powered it through the contacts (T=0) and it answered at once.
        pub fn repower(&mut self) -> Result<(), CardError> {
            unsafe { ffi::SCardEndTransaction(self.handle, ffi::SCARD_LEAVE_CARD) };
            let mut p: ffi::DWORD = 0;
            let rv = unsafe { ffi::SCardReconnect(self.handle, ffi::SCARD_SHARE_SHARED, ffi::SCARD_PROTOCOL_T0 | ffi::SCARD_PROTOCOL_T1, ffi::SCARD_UNPOWER_CARD, &mut p) };
            if rv != ffi::SCARD_S_SUCCESS { return Err(code(rv)); }
            self.proto = p;
            let rv = unsafe { ffi::SCardBeginTransaction(self.handle) };
            if rv != ffi::SCARD_S_SUCCESS { return Err(code(rv)); }
            Ok(())
        }
    }

    fn code(rv: ffi::LONG) -> CardError {
        match rv {
            ffi::SCARD_W_RESET_CARD => CardError::WasReset,
            ffi::SCARD_W_REMOVED_CARD => CardError::Link("the key was unplugged".into()),
            ffi::SCARD_E_SHARING_VIOLATION => CardError::Link("another program holds the key exclusively".into()),
            ffi::SCARD_E_NO_SERVICE => CardError::Link("the smart-card service is not running".into()),
            _ => CardError::Link(format!("PC/SC error {:#010x}", rv as u32)),
        }
    }
    fn transmit(card: &Card, cmd: &[u8]) -> Result<Vec<u8>, CardError> {
        let pci = ffi::SCARD_IO_REQUEST { dwProtocol: card.proto, cbPciLength: std::mem::size_of::<ffi::SCARD_IO_REQUEST>() as ffi::DWORD };
        let mut buf = vec![0u8; 4096];
        let mut len = buf.len() as ffi::DWORD;
        let rv = unsafe { ffi::SCardTransmit(card.handle, &pci, cmd.as_ptr(), cmd.len() as ffi::DWORD, std::ptr::null_mut(), buf.as_mut_ptr(), &mut len) };
        if rv != ffi::SCARD_S_SUCCESS { return Err(code(rv)); }
        buf.truncate(len as usize);
        Ok(buf)
    }
    impl Wire for Card {
        fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError> { transmit(self, cmd) }
        fn pinpad_verify(&self, p2: u8) -> Option<Result<(), CardError>> { pinpad_verify(self, p2) }
        fn pinpad_modify(&self, p2: u8) -> Option<Result<(), CardError>> { pinpad_modify(self, p2) }
        fn pinpad(&self) -> bool { pinpad_code(self).is_some() }
    }
    impl Wire for Tx<'_> {
        fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError> { transmit(self.0, cmd) }
        fn pinpad_verify(&self, p2: u8) -> Option<Result<(), CardError>> { pinpad_verify(self.0, p2) }
        fn pinpad_modify(&self, p2: u8) -> Option<Result<(), CardError>> { pinpad_modify(self.0, p2) }
        fn pinpad(&self) -> bool { pinpad_code(self.0).is_some() }
    }

    // ---- a PIN typed on the reader (PC/SC part 10) ----
    // Measured 2026-10-13 on a REINER SCT cyberJack RFID komfort: it advertises VERIFY_PIN_DIRECT (and MODIFY), takes
    // the PIN on its keypad for a card in its CONTACT slot only (a card on its antenna gets 6985 at once, no prompt),
    // and refuses a maximum PIN length above 12 (6B80). The LCD and keys are the firmware's own: the host can ask for
    // a PIN operation, never read a key. That is the point of it.
    /// The IOCTL for a part-10 control code: pcsc-lite (macOS, Linux) adds it to 0x42000000; Windows shifts it.
    fn ctl(code: ffi::DWORD) -> ffi::DWORD { if cfg!(windows) { (0x31 << 16) | (code << 2) } else { 0x42000000 + code } }
    const CM_IOCTL_GET_FEATURE_REQUEST: ffi::DWORD = 3400;
    const FEATURE_VERIFY_PIN_DIRECT: u8 = 0x06;
    const FEATURE_MODIFY_PIN_DIRECT: u8 = 0x07;
    fn control(card: &Card, code: ffi::DWORD, send: &[u8]) -> Result<Vec<u8>, CardError> {
        let mut buf = vec![0u8; 1024];
        let mut got: ffi::DWORD = 0;
        let rv = unsafe { ffi::SCardControl(card.handle, code, send.as_ptr() as *const _, send.len() as ffi::DWORD, buf.as_mut_ptr() as *mut _, buf.len() as ffi::DWORD, &mut got) };
        if rv != ffi::SCARD_S_SUCCESS { return Err(self::code(rv)); }
        // a failing driver has been seen to hand back a length past its buffer: never trust it past what was given
        buf.truncate((got as usize).min(1024));
        Ok(buf)
    }
    /// The reader's control code for one part-10 feature, from its feature list; `None` when it has not got it.
    fn feature_code(card: &Card, feature: u8) -> Option<ffi::DWORD> {
        let tlv = control(card, ctl(CM_IOCTL_GET_FEATURE_REQUEST), &[]).ok()?;
        let mut i = 0;
        while i + 6 <= tlv.len() {
            let (tag, len) = (tlv[i], tlv[i + 1] as usize);
            if tag == feature && len == 4 { return Some(u32::from_be_bytes([tlv[i + 2], tlv[i + 3], tlv[i + 4], tlv[i + 5]])); }
            i += 2 + len;
        }
        None
    }
    /// The reader's control code for VERIFY_PIN_DIRECT; `None` when it has no keypad.
    fn pinpad_code(card: &Card) -> Option<ffi::DWORD> { feature_code(card, FEATURE_VERIFY_PIN_DIRECT) }
    /// What the reader's keypad answered: the card's status word, or one of the reader's own.
    fn pinpad_answer(r: Result<Vec<u8>, CardError>) -> Result<(), CardError> {
        match r {
            Err(e) => Err(e),
            Ok(r) if r.len() < 2 => Err(CardError::Parse),
            Ok(r) => match u16::from_be_bytes([r[r.len() - 2], r[r.len() - 1]]) {
                0x9000 => Ok(()),
                0x6400 => Err(CardError::Link("no PIN was typed on the reader in time".into())),
                0x6401 => Err(CardError::Link("cancelled on the reader".into())),
                0x6402 => Err(CardError::Link("the new PIN was not typed the same way twice on the reader; nothing was changed".into())),
                0x6403 => Err(CardError::Link("the PIN typed on the reader was too short or too long".into())),
                0x6B80 => Err(CardError::Link("the reader did not accept the PIN request".into())),
                0x6985 => Err(CardError::Link("the reader takes a PIN on its keypad only for a card in its slot, not one held to its antenna".into())),
                sw => Err(CardError::Sw { sw, hint: String::new() }),
            },
        }
    }
    /// CHANGE REFERENCE DATA built by the reader from three entries on its keypad (old, new, new again).
    /// PIN_MODIFY_STRUCTURE: the VERIFY shape proven on the cyberJack, plus: old PIN at offset 0 and the new one
    /// straight after it (variable length, as GnuPG asks a class-3 reader for an OpenPGP card), bConfirmPIN 03 = the
    /// old PIN is asked for and the new one confirmed, the reader's own three prompts. The firmware refusing the
    /// request (6B80) spends no try, so a second shape with no PIN-block size is tried before giving up.
    fn pinpad_modify(card: &Card, p2: u8) -> Option<Result<(), CardError>> {
        let code = feature_code(card, FEATURE_MODIFY_PIN_DIRECT)?;
        let (min, max) = if p2 == 0x83 { (8u8, 12u8) } else { (6u8, 12u8) };
        let apdu = [0x00, 0x24, 0x00, p2, 0x00];
        let shape = |block: u8| {
            let mut s = vec![0x00, 0x00, 0x82, block, 0x00, 0x00, 0x00, max, min, 0x03, 0x02, 0xFF, 0x09, 0x04, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00];
            s.extend_from_slice(&(apdu.len() as u32).to_le_bytes());
            s.extend_from_slice(&apdu);
            s
        };
        let first = pinpad_answer(control(card, code, &shape(0x0F)));
        Some(match first {
            Err(CardError::Link(ref m)) if m.starts_with("the reader did not accept") => pinpad_answer(control(card, code, &shape(0x00))),
            r => r,
        })
    }
    fn pinpad_verify(card: &Card, p2: u8) -> Option<Result<(), CardError>> {
        let code = pinpad_code(card)?;
        // PIN_VERIFY_STRUCTURE: timeouts default · ASCII, left-justified, byte units · no length byte in the block ·
        // 6 to 12 digits for a key PIN, 8 to 12 for the admin PIN (the reader's own ceiling is 12) · validated by its
        // OK key · one message, English · T=0/1 prologue zero · then the bare VERIFY header the reader completes.
        // Shape proven on the cyberJack with a Token2 card (2026-10-19, "PIN korrekt" on its display, 9000 back):
        // PIN block size 15 in bmPINBlockString, the reader's default message, and the VERIFY header WITH its Lc
        // byte. The firmware ignores bTimeOut and waits about 30 s on its own clock.
        let (min, max) = if p2 == 0x83 { (8u8, 12u8) } else { (6u8, 12u8) };
        let apdu = [0x00, 0x20, 0x00, p2, 0x00];
        let mut s = vec![0x00, 0x00, 0x82, 0x0F, 0x00, max, min, 0x02, 0xFF, 0x09, 0x04, 0x00, 0x00, 0x00, 0x00];
        s.extend_from_slice(&(apdu.len() as u32).to_le_bytes());
        s.extend_from_slice(&apdu);
        Some(pinpad_answer(control(card, code, &s)))
    }
    impl Drop for Card {
        fn drop(&mut self) {
            unsafe {
                ffi::SCardEndTransaction(self.handle, ffi::SCARD_LEAVE_CARD);
                ffi::SCardDisconnect(self.handle, ffi::SCARD_LEAVE_CARD);
                ffi::SCardReleaseContext(self.ctx);
            }
        }
    }

    /// Every reader present — `cards::probe` decides by what answers. Nothing is picked by name: macOS named a
    /// Nitrokey 3's reader "8352 17074" on firmware 1.9.0 and "Nitrokey Nitrokey 3" on 1.9.1. No smart-card service,
    /// or no reader at all, is a real empty answer.
    pub fn readers() -> Result<Vec<String>, CardError> {
        let ctx = match Context::establish(Scope::User) { Ok(c) => c, Err(pcsc::Error::NoService) => return Ok(vec![]), Err(e) => return Err(CardError::Link(e.to_string())) };
        let mut buf = vec![0u8; 2048];
        let names = match ctx.list_readers(&mut buf) { Ok(n) => n, Err(pcsc::Error::NoReadersAvailable) | Err(pcsc::Error::NoService) => return Ok(vec![]), Err(e) => return Err(CardError::Link(e.to_string())) };
        Ok(names.map(|n| n.to_string_lossy().into_owned()).collect())
    }

    pub fn open(reader: &str) -> Result<Opened, CardError> {
        let cname = std::ffi::CString::new(reader).map_err(|_| CardError::Parse)?;
        let mut ctx: ffi::SCARDCONTEXT = 0;
        let rv = unsafe { ffi::SCardEstablishContext(ffi::SCARD_SCOPE_USER, std::ptr::null(), std::ptr::null(), &mut ctx) };
        if rv != ffi::SCARD_S_SUCCESS { return Err(code(rv)); }
        let mut handle: ffi::SCARDHANDLE = 0;
        let mut proto: ffi::DWORD = 0;
        let rv = unsafe { ffi::SCardConnect(ctx, cname.as_ptr(), ffi::SCARD_SHARE_SHARED, ffi::SCARD_PROTOCOL_T0 | ffi::SCARD_PROTOCOL_T1, &mut handle, &mut proto) };
        match rv {
            ffi::SCARD_S_SUCCESS => {}
            // listed a moment ago and gone now, or a reader with nothing a smart-card service can talk to: that is not
            // a key failing, it is not a key. (A YubiKey Bio showed up exactly this way on 2026-09-21.)
            ffi::SCARD_E_UNKNOWN_READER | ffi::SCARD_E_NO_SMARTCARD | ffi::SCARD_W_REMOVED_CARD
            | ffi::SCARD_E_READER_UNAVAILABLE | ffi::SCARD_W_UNPOWERED_CARD | ffi::SCARD_W_UNRESPONSIVE_CARD => {
                unsafe { ffi::SCardReleaseContext(ctx) };
                return Ok(Opened::NotACard);
            }
            _ => { unsafe { ffi::SCardReleaseContext(ctx) }; return Err(code(rv)); }
        }
        // Ours until drop. Another program resetting the card between connect and here is answered by one reconnect.
        let mut rv = unsafe { ffi::SCardBeginTransaction(handle) };
        if rv == ffi::SCARD_W_RESET_CARD {
            let mut p: ffi::DWORD = 0;
            if unsafe { ffi::SCardReconnect(handle, ffi::SCARD_SHARE_SHARED, ffi::SCARD_PROTOCOL_T0 | ffi::SCARD_PROTOCOL_T1, ffi::SCARD_LEAVE_CARD, &mut p) } == ffi::SCARD_S_SUCCESS {
                proto = p;
                rv = unsafe { ffi::SCardBeginTransaction(handle) };
            }
        }
        if rv != ffi::SCARD_S_SUCCESS {
            unsafe { ffi::SCardDisconnect(handle, ffi::SCARD_LEAVE_CARD); ffi::SCardReleaseContext(ctx); }
            return Err(code(rv));
        }
        Ok(Opened::Card(Card { ctx, handle, proto }))
    }
}

#[cfg(any(target_os = "ios", target_os = "android"))]
mod imp {
    use super::{Opened, Wire};
    use crate::cards::CardError;
    use serde::Deserialize;

    /// A key reached through the phone: `id` names the open session on the Swift side, closed when this is dropped.
    pub struct Card { id: u32 }
    /// A phone session is the app's alone, so there is nothing to hold; this keeps the same shape as the desktop.
    pub struct Tx<'a>(&'a Card);
    impl Card {
        pub fn transaction(&mut self) -> Result<Tx<'_>, CardError> { Ok(Tx(self)) }
        /// A phone session is one tag read or one slot; there is no stale link to clear.
        pub fn repower(&mut self) -> Result<(), CardError> { Ok(()) }
    }

    #[derive(Deserialize)] struct Readers { readers: Vec<String> }
    #[derive(Deserialize)] struct Session { id: Option<u32> }
    #[derive(Deserialize)] struct Answer { data: String }
    #[derive(Deserialize)] struct Nothing {}

    fn call<T: serde::de::DeserializeOwned>(cmd: &str, payload: serde_json::Value) -> Result<T, CardError> {
        tauri_plugin_keylink::call(cmd, payload).map_err(CardError::Link)
    }
    impl Wire for Card {
        fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError> {
            let a: Answer = call("transmit", serde_json::json!({ "id": self.id, "apdu": hex::encode(cmd) }))?;
            hex::decode(a.data).map_err(|_| CardError::Parse)
        }
    }
    impl Wire for Tx<'_> { fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError> { self.0.send(cmd) } }
    impl Drop for Card { fn drop(&mut self) { let _ = call::<Nothing>("close", serde_json::json!({ "id": self.id })); } }

    /// Keys in the USB-C port. A key reached by NFC is never "present": it is opened by name (`NFC`) when wanted.
    pub fn readers() -> Result<Vec<String>, CardError> { Ok(call::<Readers>("readers", serde_json::json!({}))?.readers) }

    pub fn open(reader: &str) -> Result<Opened, CardError> {
        let s: Session = call("open", serde_json::json!({ "reader": reader }))?;
        Ok(match s.id { Some(id) => Opened::Card(Card { id }), None => Opened::NotACard })
    }
}
