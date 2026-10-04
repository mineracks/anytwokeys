//! The wire to a key. Everything the app says to a YubiKey is a sequence of APDUs written once, in `cards.rs` and
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
    use pcsc::{Context, Protocols, Scope, ShareMode};

    pub struct Card(pcsc::Card);
    /// Held while a sequence must not be interleaved with another program's commands; ends when dropped.
    pub struct Tx<'a>(pcsc::Transaction<'a>);

    fn err(e: pcsc::Error) -> CardError { match e { pcsc::Error::ResetCard => CardError::WasReset, e => CardError::Link(e.to_string()) } }
    fn transmit(card: &pcsc::Card, cmd: &[u8]) -> Result<Vec<u8>, CardError> { let mut buf = vec![0u8; 4096]; Ok(card.transmit(cmd, &mut buf).map_err(err)?.to_vec()) }
    impl Wire for Card { fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError> { transmit(&self.0, cmd) } }
    impl Wire for Tx<'_> { fn send(&self, cmd: &[u8]) -> Result<Vec<u8>, CardError> { transmit(&self.0, cmd) } }
    impl Card { pub fn transaction(&mut self) -> Result<Tx<'_>, CardError> { Ok(Tx(self.0.transaction().map_err(err)?)) } }

    /// Yubico readers present. No smart-card service, or no reader at all, is a real empty answer.
    pub fn readers() -> Result<Vec<String>, CardError> {
        let ctx = match Context::establish(Scope::User) { Ok(c) => c, Err(pcsc::Error::NoService) => return Ok(vec![]), Err(e) => return Err(err(e)) };
        let mut buf = vec![0u8; 2048];
        let names = match ctx.list_readers(&mut buf) { Ok(n) => n, Err(pcsc::Error::NoReadersAvailable) | Err(pcsc::Error::NoService) => return Ok(vec![]), Err(e) => return Err(err(e)) };
        Ok(names.map(|n| n.to_string_lossy().into_owned()).filter(|n| n.to_lowercase().contains("yubi")).collect())
    }

    pub fn open(reader: &str) -> Result<Opened, CardError> {
        let ctx = Context::establish(Scope::User).map_err(err)?;
        let cname = std::ffi::CString::new(reader).map_err(|_| CardError::Parse)?;
        match ctx.connect(&cname, ShareMode::Shared, Protocols::ANY) {
            Ok(c) => Ok(Opened::Card(Card(c))),
            // listed a moment ago and gone now, or a reader with nothing a smart-card service can talk to: that is not
            // a key failing, it is not a key. (A YubiKey Bio showed up exactly this way on 2026-09-21.)
            Err(pcsc::Error::UnknownReader) | Err(pcsc::Error::NoSmartcard) | Err(pcsc::Error::RemovedCard)
            | Err(pcsc::Error::ReaderUnavailable) | Err(pcsc::Error::UnpoweredCard) | Err(pcsc::Error::UnresponsiveCard) => Ok(Opened::NotACard),
            Err(e) => Err(err(e)),
        }
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
