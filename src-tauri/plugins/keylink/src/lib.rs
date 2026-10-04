//! The wire to a YubiKey on a phone. The Rust side is only a doorway: `call` hands a command to the Swift plugin
//! (ios/Sources/KeylinkPlugin.swift) and waits for its answer. Nothing here is exposed to the window; the app's own
//! commands are the only way in, exactly as on a computer.
use std::sync::OnceLock;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::Wry;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_keylink);

static HANDLE: OnceLock<PluginHandle<Wry>> = OnceLock::new();

pub fn init() -> TauriPlugin<Wry> {
    Builder::new("keylink")
        .setup(|_app, _api| {
            #[cfg(target_os = "ios")]
            { let _ = HANDLE.set(_api.register_ios_plugin(init_plugin_keylink)?); }
            #[cfg(target_os = "android")]
            { let _ = HANDLE.set(_api.register_android_plugin("com.anytwokeys.keylink", "KeylinkPlugin")?); }
            Ok(())
        })
        .build()
}

/// One command to the Swift side, answered or refused. Blocks: call it from a worker thread, never the main one.
pub fn call<T: serde::de::DeserializeOwned>(command: &str, payload: serde_json::Value) -> Result<T, String> {
    let handle = HANDLE.get().ok_or("the key reader is not available on this device")?;
    handle.run_mobile_plugin(command, payload).map_err(|e| e.to_string())
}
