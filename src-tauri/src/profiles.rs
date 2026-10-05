//! Profiles: one app, several sealed data trees, one open at a time.
//!
//! Everything this app keeps lives in a folder (`lockbox::dir()`). A profile IS such a folder, moved one level down:
//!
//!   Any Two Keys/
//!     profiles.json            unsealed index: ids, names, classes, pictures, timestamps — nothing else
//!     profiles/<id>/           exactly the tree the app has always kept: store.enc, slots.json, chain.db, attachments/…
//!
//! Each profile has its OWN store key in its own Keychain item (service `com.anytwokeys.app.store-key`, account per
//! profile), so a demo profile can be copied elsewhere without ever having been sealed under the personal key, and the
//! personal profile can ask for Touch ID and a card while a demo opens on a click. `lockbox::dir()` answers "the
//! current profile's folder", which is the one indirection everything else runs through; while no profile is open it
//! answers the root, as it always did, so tests and harnesses that point `A2K_DATA_DIR` at a folder keep working.
//!
//! Classes decide behaviour, names are the person's:
//!   private — the real data: always asked, never opened on start, the window uncapturable, a curtain while another
//!             display is attached (lib.rs `guard_*`), discreet mode on by default
//!   work    — demo wallets for development: opens on a click (Touch ID only if the person turns it on)
//!   fiction — a made-up person's holdings for an audience: opens on a click
//!
//! The index holds nothing sensitive. A profile's NAME is as exposed as the app's name in the Dock; its contents are not.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(not(test))] use std::sync::RwLock;

pub const LEGACY_ACCOUNT: &str = "store key";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Class { Private, Work, Fiction }

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub class: Class,
    /// `glyph:<one or two characters>` or `img:<base64 PNG/JPEG thumbnail, ≤ 48 KB>` — a picture is as public as the name
    #[serde(default)] pub picture: Option<String>,
    pub created: u64,
    #[serde(default)] pub last_opened: u64,
    /// open this one on start without asking (never true for a private profile)
    #[serde(default)] pub start: bool,
    /// amounts, names and addresses drawn blurred until held (remembered per profile)
    #[serde(default)] pub discreet: bool,
    /// a fiction profile: the seed its persona was generated from, so it can be rebuilt or extended
    #[serde(default)] pub persona_seed: Option<String>,
    /// the Keychain account its store key sits under; the migrated profile keeps the one the app always used
    #[serde(default = "legacy_account")] pub keychain_account: String,
    /// set while this profile is synced (sync.rs): the name it goes by on every device. As public as the profile's
    /// own name; it lets the picker say "already on this device" without opening anything.
    #[serde(default)] pub sync_id: Option<String>,
}
fn legacy_account() -> String { LEGACY_ACCOUNT.into() }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Index {
    #[serde(default = "one")] pub v: u8,
    #[serde(default)] pub profiles: Vec<Profile>,
}
fn one() -> u8 { 1 }

// the open profile. Under test it is per thread: cargo runs tests in parallel, and a test that opens a profile must not
// move every other test's files while it does.
#[cfg(not(test))] static CURRENT: RwLock<Option<Profile>> = RwLock::new(None);
#[cfg(test)] thread_local! { static CURRENT_T: std::cell::RefCell<Option<Profile>> = const { std::cell::RefCell::new(None) }; }
fn cur_get() -> Option<Profile> {
    #[cfg(not(test))] { CURRENT.read().ok().and_then(|c| c.clone()) }
    #[cfg(test)] { CURRENT_T.with(|c| c.borrow().clone()) }
}
fn cur_set(p: Option<Profile>) {
    #[cfg(not(test))] { if let Ok(mut c) = CURRENT.write() { *c = p; } }
    #[cfg(test)] { CURRENT_T.with(|c| *c.borrow_mut() = p); }
}

/// The root the app has always used: `A2K_DATA_DIR`, else the user's data folder.
#[cfg(test)] thread_local! { static ROOT_T: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) }; }
/// Under test, this thread's root — so a profiles test never touches the process-wide `A2K_DATA_DIR` that other tests use.
#[cfg(test)] pub fn test_root(p: Option<PathBuf>) { ROOT_T.with(|r| *r.borrow_mut() = p); }
/// Where the system says this app keeps its files, told once at start. A computer and an iPhone answer through the
/// usual folders; an Android phone has none of those, and without this the app would write into "." and fail.
static SYSTEM_DIRS: std::sync::OnceLock<(PathBuf, PathBuf)> = std::sync::OnceLock::new();
pub fn set_system_dirs(data: PathBuf, documents: PathBuf) { let _ = SYSTEM_DIRS.set((data, documents)); }
/// The folder a phone's own Files app shows for this app: what is saved from here lands there.
#[allow(dead_code)]
pub fn phone_documents() -> Option<PathBuf> {
    let d = match SYSTEM_DIRS.get() { Some((_, docs)) => docs.clone(), None => dirs::home_dir()?.join("Documents") };
    std::fs::create_dir_all(&d).ok()?; Some(d)
}
pub fn root() -> PathBuf {
    #[cfg(test)] if let Some(r) = ROOT_T.with(|r| r.borrow().clone()) { std::fs::create_dir_all(&r).ok(); return r; }
    #[cfg(test)] let home = std::env::temp_dir().join("a2k-tests-empty");
    #[cfg(not(test))] let home = SYSTEM_DIRS.get().map(|(data, _)| data.clone()).or_else(dirs::data_dir).unwrap_or_else(|| PathBuf::from(".")).join("Any Two Keys");
    let dir = match std::env::var_os("A2K_DATA_DIR") { Some(d) => PathBuf::from(d), None => home };
    std::fs::create_dir_all(&dir).ok();
    dir
}
fn index_path() -> PathBuf { root().join("profiles.json") }
pub fn dir_for(id: &str) -> PathBuf { root().join("profiles").join(id) }

/// Where the app's files are RIGHT NOW: the open profile's folder, or the root while none is open.
pub fn dir() -> PathBuf {
    match cur_get().map(|p| p.id) {
        Some(id) => { let d = dir_for(&id); std::fs::create_dir_all(&d).ok(); d }
        None => root(),
    }
}
pub fn current() -> Option<Profile> { cur_get() }
pub fn keychain_account() -> String { current().map(|p| p.keychain_account).unwrap_or_else(legacy_account) }
/// Slots made for a demo profile do not ask for Touch ID unless the person turns it on; everything else does.
pub fn touch_default() -> bool { current().map(|p| p.class == Class::Private).unwrap_or(true) }

pub fn index() -> Result<Index, String> {
    match std::fs::read(index_path()) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Index::default()),
        Err(e) => Err(format!("profiles.json: {e}")),
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("profiles.json is damaged: {e}")),
    }
}
pub fn save_index(ix: &Index) -> Result<(), String> {
    let p = index_path();
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(ix).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)); }
    std::fs::rename(&tmp, &p).map_err(|e| format!("{}: {e}", p.display()))
}
pub fn has_index() -> bool { index_path().exists() }

fn new_id() -> String { hex::encode(crate::lockbox::random::<6>()) }

/// What the first launch with profiles does to a folder that holds the app's files directly: every entry moves into
/// `profiles/<id>/` and becomes the private profile "Me", keeping its Keychain item (re-labelled, not re-keyed).
/// A folder with nothing of the app's in it gets an empty index. Safe to run again: it does nothing once the index exists.
#[derive(Serialize, Debug, Default, Clone)]
pub struct Moved { pub moved: bool, pub files: usize, pub id: Option<String> }
pub fn migrate_legacy(now: u64) -> Result<Moved, String> {
    if has_index() { return Ok(Moved::default()); }
    let r = root();
    let entries: Vec<PathBuf> = std::fs::read_dir(&r).map_err(|e| format!("{}: {e}", r.display()))?.flatten().map(|e| e.path())
        .filter(|p| { let n = p.file_name().and_then(|n| n.to_str()).unwrap_or(""); n != "profiles" && n != "profiles.json" && !n.starts_with('.') }).collect();
    let has_app_files = entries.iter().any(|p| { let n = p.file_name().and_then(|n| n.to_str()).unwrap_or(""); matches!(n, "store.enc" | "store.json" | "slots.json" | "chain.db" | "attachments" | "cache.enc" | "cache.json") });
    let mut ix = Index::default();
    let mut m = Moved::default();
    if has_app_files {
        let id = new_id();
        let d = dir_for(&id);
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        for p in &entries {
            let to = d.join(p.file_name().unwrap());
            std::fs::rename(p, &to).map_err(|e| format!("moving {} into the profile: {e}", p.display()))?;
            m.files += 1;
        }
        ix.profiles.push(Profile { id: id.clone(), name: "Me".into(), class: Class::Private, picture: Some("glyph:lock".into()), created: now, last_opened: 0, start: false, discreet: true, persona_seed: None, sync_id: None, keychain_account: LEGACY_ACCOUNT.into() });
        m.moved = true; m.id = Some(id);
    }
    save_index(&ix)?;
    Ok(m)
}

/// Open a profile: whatever was open is locked and forgotten first. Nothing from one profile is ever in memory while
/// another is. The caller then drives the lock screen exactly as on a plain start.
pub fn open(id: &str, now: u64) -> Result<Profile, String> {
    let mut ix = index()?;
    let p = ix.profiles.iter_mut().find(|p| p.id == id).ok_or("no such profile")?;
    crate::lockbox::lock();
    p.last_opened = now;
    let prof = p.clone();
    std::fs::create_dir_all(dir_for(id)).map_err(|e| e.to_string())?;
    cur_set(Some(prof.clone()));
    save_index(&ix)?;
    Ok(prof)
}
/// Back to the picker: lock, and no folder is current.
pub fn close() {
    crate::lockbox::lock();
    cur_set(None);
}
/// For tests and harnesses: `A2K_PROFILE=<id>` opens that profile before the window asks for anything.
pub fn open_from_env(now: u64) { if let Some(id) = std::env::var_os("A2K_PROFILE") { let _ = open(&id.to_string_lossy(), now); } }

pub fn create(name: &str, class: Class, picture: Option<String>, now: u64) -> Result<Profile, String> {
    let name = name.trim();
    if name.is_empty() { return Err("a profile needs a name".into()); }
    if name.chars().count() > 40 { return Err("a profile's name is at most 40 characters".into()); }
    check_picture(picture.as_deref())?;
    let mut ix = index()?;
    if ix.profiles.iter().any(|p| p.name.eq_ignore_ascii_case(name)) { return Err(format!("there is already a profile called {name:?}")); }
    let id = new_id();
    std::fs::create_dir_all(dir_for(&id)).map_err(|e| e.to_string())?;
    let p = Profile { id: id.clone(), name: name.into(), class, picture: picture.or_else(|| Some(match class { Class::Private => "glyph:lock", Class::Work => "glyph:keys", Class::Fiction => "glyph:entity" }.into())),
        created: now, last_opened: 0, start: false, discreet: class == Class::Private, persona_seed: None, sync_id: None, keychain_account: format!("profile {id}") };
    ix.profiles.push(p.clone());
    save_index(&ix)?;
    Ok(p)
}

fn check_picture(p: Option<&str>) -> Result<(), String> {
    match p {
        None => Ok(()),
        Some(s) if s.starts_with("glyph:") => if s.len() <= 24 && s[6..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-') { Ok(()) } else { Err("a glyph is one of the app's own marks".into()) },
        Some(s) if s.starts_with("img:") => if s.len() <= 48 * 1024 * 4 / 3 + 8 { Ok(()) } else { Err("a profile picture is a small thumbnail, at most 48 KB".into()) },
        Some(_) => Err("a picture is glyph:… or img:…".into()),
    }
}

pub struct Changes { pub name: Option<String>, pub picture: Option<Option<String>>, pub start: Option<bool>, pub discreet: Option<bool>, pub persona_seed: Option<Option<String>> }
pub fn update(id: &str, c: Changes) -> Result<Profile, String> {
    let mut ix = index()?;
    if let Some(n) = &c.name { let n = n.trim(); if n.is_empty() { return Err("a profile needs a name".into()); } if ix.profiles.iter().any(|p| p.id != id && p.name.eq_ignore_ascii_case(n)) { return Err(format!("there is already a profile called {n:?}")); } }
    if let Some(Some(p)) = &c.picture { check_picture(Some(p))?; }
    let start_on = c.start == Some(true);
    let p = ix.profiles.iter_mut().find(|p| p.id == id).ok_or("no such profile")?;
    if let Some(n) = c.name { p.name = n.trim().to_string(); }
    if let Some(pic) = c.picture { p.picture = pic; }
    if let Some(s) = c.start { if s && p.class == Class::Private { return Err("a private profile never opens on start: the first screen in a room is never the real numbers".into()); } p.start = s; }
    if let Some(d) = c.discreet { p.discreet = d; }
    if let Some(seed) = c.persona_seed { p.persona_seed = seed; }
    let out = p.clone();
    if start_on { for q in ix.profiles.iter_mut() { if q.id != id { q.start = false; } } }   // one profile opens on start
    save_index(&ix)?;
    if current().map(|c| c.id == id).unwrap_or(false) { cur_set(Some(out.clone())); }
    // a synced profile's name and picture travel with it: however they were changed, the next tick says so
    crate::sync::DIRTY.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(out)
}

/// Shred the Keychain item first, then the folder. Never the open one.
pub fn set_sync_id(id: &str, sync_id: Option<String>) -> Result<(), String> {
    let mut ix = index()?;
    let p = ix.profiles.iter_mut().find(|p| p.id == id).ok_or("no such profile")?;
    p.sync_id = sync_id;
    save_index(&ix)
}

pub fn delete(id: &str) -> Result<Index, String> {
    if current().map(|c| c.id == id).unwrap_or(false) { return Err("close this profile before deleting it".into()); }
    let mut ix = index()?;
    let p = ix.profiles.iter().find(|p| p.id == id).ok_or("no such profile")?.clone();
    crate::lockbox::mac::forget(&p.keychain_account)?;
    let d = dir_for(id);
    if d.exists() { std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?; }
    ix.profiles.retain(|p| p.id != id);
    save_index(&ix)?;
    Ok(ix)
}

// ---------------------------------------------------------------- the screen guard (macOS) --

/// What the private profile must know about the displays: how many are attached, and whether any is mirrored.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Displays { pub count: u32, pub mirrored: bool, pub external: bool }

#[cfg(target_os = "macos")]
pub mod guard {
    use super::Displays;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayIsInMirrorSet(d: u32) -> i32;
        fn CGDisplayIsBuiltin(d: u32) -> i32;
    }
    pub fn displays() -> Displays {
        let mut ids = [0u32; 16]; let mut n = 0u32;
        if unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut n) } != 0 { return Displays { count: 1, mirrored: false, external: false }; }
        let ids = &ids[..n.min(16) as usize];
        Displays { count: n, mirrored: ids.iter().any(|&d| unsafe { CGDisplayIsInMirrorSet(d) } != 0), external: ids.iter().any(|&d| unsafe { CGDisplayIsBuiltin(d) } == 0) }
    }
    /// NSWindowSharingNone (0): the window is black in screenshots, recordings and screen shares. NSWindowSharingReadOnly (1)
    /// is the default. Must run on the main thread with the window's NSWindow pointer.
    pub unsafe fn set_sharing(ns_window: usize, none: bool) {
        let w: &objc2::runtime::AnyObject = &*(ns_window as *const objc2::runtime::AnyObject);
        let kind: usize = if none { 0 } else { 1 };
        let _: () = objc2::msg_send![w, setSharingType: kind];
    }
}
#[cfg(not(target_os = "macos"))]
pub mod guard {
    use super::Displays;
    pub fn displays() -> Displays { Displays { count: 1, mirrored: false, external: false } }
    pub unsafe fn set_sharing(_: usize, _: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("a2k-profiles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap();
        test_root(Some(d.clone())); d
    }

    #[test]
    fn a_legacy_folder_becomes_the_private_profile_me_and_keeps_its_keychain_account() {
        let d = fresh("legacy");
        std::fs::write(d.join("store.enc"), b"sealed").unwrap();
        std::fs::write(d.join("slots.json"), b"{}").unwrap();
        std::fs::create_dir_all(d.join("attachments")).unwrap(); std::fs::write(d.join("attachments/aa"), b"x").unwrap();
        std::fs::write(d.join("chain.db"), b"db").unwrap();
        let m = migrate_legacy(1_000).unwrap();
        assert!(m.moved); assert_eq!(m.files, 4);
        let ix = index().unwrap();
        assert_eq!(ix.profiles.len(), 1);
        let p = &ix.profiles[0];
        assert_eq!((p.name.as_str(), p.class, p.keychain_account.as_str(), p.discreet, p.start), ("Me", Class::Private, LEGACY_ACCOUNT, true, false));
        let pd = dir_for(&p.id);
        assert!(pd.join("store.enc").exists() && pd.join("attachments/aa").exists() && pd.join("chain.db").exists());
        assert!(!d.join("store.enc").exists(), "nothing of the app's is left at the root");
        // running it again changes nothing
        let m2 = migrate_legacy(2_000).unwrap(); assert!(!m2.moved);
        assert_eq!(index().unwrap().profiles.len(), 1);
        // dir() follows the open profile and falls back to the root
        assert_eq!(dir(), d);
        open(&p.id, 3_000).unwrap();
        assert_eq!(dir(), pd);
        assert_eq!(keychain_account(), LEGACY_ACCOUNT);
        assert!(touch_default());
        close(); assert_eq!(dir(), d); assert_eq!(keychain_account(), LEGACY_ACCOUNT);
        test_root(None);
    }

    #[test]
    fn an_empty_folder_gets_an_empty_index_and_profiles_are_made_named_and_deleted() {
        let _d = fresh("empty");
        let m = migrate_legacy(1).unwrap(); assert!(!m.moved); assert!(has_index());
        assert!(index().unwrap().profiles.is_empty());
        let w = create("YubiKey bench", Class::Work, None, 10).unwrap();
        assert_eq!(w.picture.as_deref(), Some("glyph:keys")); assert!(!w.discreet); assert!(w.keychain_account.starts_with("profile "));
        assert!(create(" yubikey BENCH ", Class::Fiction, None, 11).is_err(), "names are unique, case aside");
        assert!(create("x", Class::Work, Some("png:…".into()), 12).is_err());
        let f = create("Maya", Class::Fiction, Some("glyph:entity".into()), 12).unwrap();
        open(&w.id, 20).unwrap(); assert!(!touch_default()); assert_eq!(keychain_account(), w.keychain_account);
        let w2 = update(&w.id, Changes { name: None, picture: None, start: Some(true), discreet: None, persona_seed: None }).unwrap();
        assert!(w2.start); assert_eq!(current().unwrap().start, true);
        let f2 = update(&f.id, Changes { name: Some("Maya Lindqvist".into()), picture: None, start: Some(true), discreet: None, persona_seed: Some(Some("7f3a".into())) }).unwrap();
        assert!(f2.start && !index().unwrap().profiles.iter().find(|p| p.id == w.id).unwrap().start, "one profile opens on start");
        let pr = create("Me", Class::Private, None, 13).unwrap();
        assert!(update(&pr.id, Changes { name: None, picture: None, start: Some(true), discreet: None, persona_seed: None }).is_err(), "a private profile never opens on start");
        assert!(delete(&w.id).is_err(), "not the open one");
        close();
        let ix = delete(&f.id).unwrap();
        assert_eq!(ix.profiles.len(), 2); assert!(!dir_for(&f.id).exists());
        test_root(None);
    }
}
