//! The wire to a key. Everything the app says to a key is a sequence of APDUs written once, in `cards.rs` and
//! `keyinit.rs`; this is only how the bytes travel. On a computer that is PC/SC. On a phone it is the operating
//! system's own smart-card service for a key in the USB-C port, or an NFC session for a key held to the phone —
//! both behind one small Swift plugin (`plugins/keylink`).
use crate::cards::CardError;

/// One command out, one answer back (status word included).
pub trait Wire { fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError>; }
impl<T: Wire + ?Sized> Wire for &T { fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError> { (**self).send(cmd) } }

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
    impl Card { pub fn transaction(&mut self) -> Result<Tx<'_>, CardError> { Ok(Tx(self)) } }

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
    impl Wire for Card { fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError> { transmit(self, cmd) } }
    impl Wire for Tx<'_> { fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError> { transmit(self.0, cmd) } }
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
    impl Card { pub fn transaction(&mut self) -> Result<Tx<'_>, CardError> { Ok(Tx(self)) } }

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
