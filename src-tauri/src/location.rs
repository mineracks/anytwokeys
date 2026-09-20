//! Where this computer is, from the system's Location Services — only when the person has turned it on in App
//! settings, and only after the system's own prompt. One manager lives on the main thread; each answer replaces the
//! last in `LAST`, and a transaction takes whatever is there if it is fresh. Nothing here talks to a network: the
//! system does the locating, and the coordinates go nowhere but the transaction's sealed record.

use crate::context::Fix;
use std::sync::Mutex;

static LAST: Mutex<Option<Fix>> = Mutex::new(None);
static STATE: Mutex<&'static str> = Mutex::new("off");

/// "off" | "asking" | "allowed" | "denied" | "unavailable: …" (never a coordinate: this goes to the log and the screen)
pub fn state() -> String { STATE.lock().map(|s| s.to_string()).unwrap_or_else(|_| "off".into()) }
fn set_state(s: &'static str) { if let Ok(mut g) = STATE.lock() { *g = s; } }

/// A fix no older than `max_age` seconds.
pub fn fresh(now: u64, max_age: u64) -> Option<Fix> { LAST.lock().ok().and_then(|g| *g).filter(|f| now.saturating_sub(f.t) <= max_age) }
pub fn forget() { if let Ok(mut g) = LAST.lock() { *g = None; } set_state("off"); }

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
    use objc2_core_location::{CLAuthorizationStatus, CLLocation, CLLocationManager, CLLocationManagerDelegate};
    use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "A2KLocationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl CLLocationManagerDelegate for Delegate {
            #[unsafe(method(locationManager:didUpdateLocations:))]
            fn did_update(&self, _m: &CLLocationManager, locs: &NSArray<CLLocation>) {
                if let Some(l) = locs.lastObject() {
                    let (c, acc) = unsafe { (l.coordinate(), l.horizontalAccuracy()) };
                    if acc >= 0.0 {
                        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                        if let Ok(mut g) = LAST.lock() { *g = Some(Fix { lat: c.latitude, lon: c.longitude, acc, t }); }
                        set_state("allowed");
                    }
                }
            }
            #[unsafe(method(locationManager:didFailWithError:))]
            fn did_fail(&self, _m: &CLLocationManager, err: &NSError) {
                // kCLErrorDenied = 1; anything else is "could not tell just now" and the next update may succeed
                if err.code() == 1 { set_state("denied"); }
            }
            #[unsafe(method(locationManagerDidChangeAuthorization:))]
            fn auth_changed(&self, m: &CLLocationManager) {
                let st = unsafe { m.authorizationStatus() };
                if st == CLAuthorizationStatus::Denied || st == CLAuthorizationStatus::Restricted { set_state("denied"); }
                else if st == CLAuthorizationStatus::NotDetermined { set_state("asking"); }
                else { set_state("allowed"); unsafe { m.startUpdatingLocation(); } }
            }
        }
    );

    thread_local! { static KEEP: std::cell::RefCell<Option<(Retained<CLLocationManager>, Retained<Delegate>)>> = const { std::cell::RefCell::new(None) }; }

    /// Main thread only. Asks the system (which asks the person, once) and starts listening.
    pub fn start() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        KEEP.with(|k| {
            if k.borrow().is_some() { return; }
            let delegate: Retained<Delegate> = unsafe { msg_send![super(Delegate::alloc(mtm).set_ivars(())), init] };
            let m = unsafe { CLLocationManager::new() };
            unsafe {
                m.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                m.setDesiredAccuracy(100.0);      // a hundred metres is all that is ever kept
                m.setDistanceFilter(200.0);
                set_state("asking");
                m.requestWhenInUseAuthorization();
                m.startUpdatingLocation();
            }
            *k.borrow_mut() = Some((m, delegate));
        });
    }
    pub fn stop() {
        KEEP.with(|k| { if let Some((m, _)) = k.borrow_mut().take() { unsafe { m.stopUpdatingLocation(); m.setDelegate(None); } } });
        super::forget();
    }
}

#[cfg(not(target_os = "macos"))]
mod imp { pub fn start() { *super::STATE.lock().unwrap() = "unavailable: no location service on this platform yet"; } pub fn stop() { super::forget(); } }

pub use imp::{start, stop};
