//! One gate every outbound request passes through.
//!
//! Two things want the same choke point, so they share it. The light in the sidebar should pulse when the app is
//! actually talking to something, not when the window guesses it might be; and "work offline" has to be a promise
//! the app can keep, which means one place that refuses rather than a switch each caller remembers to read.
//!
//! What offline means here is exactly what this app can promise: no request leaves Any Two Keys. It is not a
//! firewall, and it does not speak for the rest of the computer — the wording in the window says so.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static OFFLINE: AtomicBool = AtomicBool::new(false);
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// The window is told about traffic so the light can pulse; nothing else depends on this being set.
pub fn attach(app: tauri::AppHandle) { let _ = APP.set(app); }

pub fn offline() -> bool { OFFLINE.load(Ordering::Relaxed) }

/// Take the setting as the truth — on a store read at start, and whenever it is changed.
pub fn seed(v: bool) { OFFLINE.store(v, Ordering::Relaxed); }

/// Called immediately before a request goes out. Refuses while offline, and otherwise says what is being talked to
/// so the window can show it. `what` is a plain word for the person, not a URL: no host or address is ever emitted,
/// because the window would then hold a record of where the app has been.
pub fn out(what: &'static str) -> Result<(), String> {
    if offline() { return Err(format!("Any Two Keys is offline: {what} needs the network. Turn it back on from the light at the bottom of the sidebar, or in App settings.")); }
    if let Some(app) = APP.get() { use tauri::Emitter; let _ = app.emit("net", what); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_refuses_before_a_request_goes_out_and_says_what_wanted_it() {
        seed(true);
        let e = out("the node").unwrap_err();
        assert!(e.contains("offline") && e.contains("the node"), "{e}");
        seed(false);
        assert!(out("the node").is_ok());
    }

    #[test]
    fn the_gate_never_carries_a_host_or_an_address_to_the_window() {
        // `out` is given a word for a person, and the only thing emitted is that word. A URL here would put a record
        // of everywhere the app has been into the window's reach.
        seed(false);
        for what in ["the node", "prices", "your labels", "the map", "place search"] {
            assert!(!what.contains("://") && !what.contains('.'), "{what} reads like a host");
        }
    }
}
