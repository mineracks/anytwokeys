//! Nitrokey 3 firmware: read the version over CTAPHID, and update it the way Nitrokey's own tool does — fetch the
//! signed release, reboot the key into its ROM bootloader (one touch), stream the SB2 image over HID, reboot.
//!
//! The wire formats are the nitrokey SDK's (pynitrokey 0.14), checked against a real key on 2026-10-09: the key
//! answers admin commands on the FIDO HID interface; the bootloader is NXP's mboot over HID. The image is only
//! accepted by the key's own ROM if it carries Nitrokey's signature (secure boot, RKTH) — that is the signature
//! check; this side checks the release's sha256 list and the manifest. Only the LPC55 keys (3A NFC, 3C NFC) are
//! flashed here; the nRF52 Mini has another bootloader and is sent to Nitrokey App 2.
//!
//! Nothing here is asked of anyone unless the person presses the button: the newest version known when this build
//! was made is compared offline, like the advisory list; the release fetch happens on request and sends nothing
//! about the key.
use serde::Serialize;
use std::io::{Cursor, Read};
use std::time::{Duration, Instant};

pub const VID: u16 = 0x20A0;
pub const PID_KEY: u16 = 0x42B2;
pub const PID_BOOTLOADER: u16 = 0x42DD;
const FIDO_USAGE_PAGE: u16 = 0xF1D0;
/// The newest release known when this build was made — the offline comparison, like the advisory list.
pub const LATEST_KNOWN: &str = "1.9.1";
pub const LATEST_KNOWN_AS_OF: &str = "9 October 2026";
const RELEASES: &str = "https://api.github.com/repos/Nitrokey/nitrokey-3-firmware/releases/latest";
const MAX_DOWNLOAD: usize = 16 << 20;

// admin app commands (CTAPHID vendor commands; STATUS goes through the app dispatcher 0x72)
const CMD_UPDATE: u8 = 0x51;
const CMD_VERSION: u8 = 0x61;
const CMD_UUID: u8 = 0x62;
const CMD_APP: u8 = 0x72;
const APP_STATUS: u8 = 0x80;
const CTAP_INIT: u8 = 0x06;
const CTAP_ERROR: u8 = 0x3F;
const CTAP_KEEPALIVE: u8 = 0x3B;
const CTAP_ERR_INVALID_LENGTH: u8 = 0x03;   // what the admin app answers when the touch does not come

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Firmware {
    pub uuid: String,
    pub version: String,
    pub variant: String,        // lpc55 · nrf52 · ?
    pub latest_known: String,
    pub latest_known_as_of: String,
    pub behind: bool,           // older than the newest known at build time
    pub flashable: bool,        // this app can flash it (LPC55 only)
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress { pub phase: &'static str, pub pct: u8, pub text: String }

// ---------- versions ----------

/// The 32-bit version the admin app answers: major in the top 10 bits, minor in 16, patch in 6 (runners/lpc55/build.rs).
pub fn version_from_int(v: u32) -> String { format!("{}.{}.{}", v >> 22, (v >> 6) & 0xFFFF, v & 0x3F) }
pub fn version_from_answer(v: &[u8]) -> Option<String> {
    if v.len() == 4 && !(v[0] == b'v' && v[1].is_ascii_digit()) { return Some(version_from_int(u32::from_be_bytes([v[0], v[1], v[2], v[3]]))); }
    let s = std::str::from_utf8(v).ok()?.trim().trim_start_matches('v');
    parse_version(s).map(|_| s.to_string())
}
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let core = s.trim().trim_start_matches('v').split(|c| c == '-' || c == '+').next()?;
    let mut it = core.split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}
pub fn is_behind(device: &str, latest: &str) -> bool { matches!((parse_version(device), parse_version(latest)), (Some(d), Some(l)) if d < l) }

// ---------- CTAPHID: the key's admin app ----------

const PKT: usize = 64;

/// The packets one command becomes: an initial packet (channel, command, length, 57 bytes) then continuations
/// (channel, sequence, 59 bytes). Pure, so it can be tested.
pub(crate) fn ctap_packets(cid: [u8; 4], cmd: u8, data: &[u8]) -> Vec<[u8; PKT]> {
    let mut out = Vec::new();
    let mut p = [0u8; PKT];
    p[..4].copy_from_slice(&cid); p[4] = 0x80 | cmd; p[5] = (data.len() >> 8) as u8; p[6] = data.len() as u8;
    let n = data.len().min(PKT - 7); p[7..7 + n].copy_from_slice(&data[..n]); out.push(p);
    let (mut off, mut seq) = (n, 0u8);
    while off < data.len() {
        let mut p = [0u8; PKT];
        p[..4].copy_from_slice(&cid); p[4] = seq;
        let n = (data.len() - off).min(PKT - 5); p[5..5 + n].copy_from_slice(&data[off..off + n]);
        out.push(p); off += n; seq = seq.wrapping_add(1);
    }
    out
}

#[derive(Debug)]
enum HidErr { Ctap(u8), Gone(String), Other(String) }
impl From<hidapi::HidError> for HidErr { fn from(e: hidapi::HidError) -> Self { HidErr::Gone(e.to_string()) } }
impl HidErr { fn text(&self) -> String { match self { HidErr::Ctap(c) => format!("the key answered error {c:#04x}"), HidErr::Gone(s) | HidErr::Other(s) => s.clone() } } }

struct Ctap { dev: hidapi::HidDevice, cid: [u8; 4] }
impl Ctap {
    fn open(api: &hidapi::HidApi, path: &std::ffi::CStr) -> Result<Ctap, HidErr> {
        let dev = api.open_path(path)?;
        let mut c = Ctap { dev, cid: [0xFF; 4] };
        let nonce: [u8; 8] = crate::lockbox::random::<8>();
        let r = c.call(CTAP_INIT, &nonce, 3000)?;
        if r.len() < 12 || r[..8] != nonce { return Err(HidErr::Other("the key did not answer the channel request".into())); }
        c.cid.copy_from_slice(&r[8..12]);
        Ok(c)
    }
    fn send(&self, cmd: u8, data: &[u8]) -> Result<(), HidErr> {
        for p in ctap_packets(self.cid, cmd, data) {
            let mut buf = [0u8; PKT + 1];   // report id 0 first: this key uses no numbered reports
            buf[1..].copy_from_slice(&p);
            self.dev.write(&buf)?;
        }
        Ok(())
    }
    fn read_packet(&self, timeout_ms: i32) -> Result<[u8; PKT], HidErr> {
        let mut buf = [0u8; PKT];
        let n = self.dev.read_timeout(&mut buf, timeout_ms)?;
        if n == 0 { return Err(HidErr::Other("the key did not answer in time".into())); }
        Ok(buf)
    }
    fn recv(&self, timeout_ms: i32) -> Result<(u8, Vec<u8>), HidErr> {
        let until = Instant::now() + Duration::from_millis(timeout_ms as u64);
        loop {
            let left = until.saturating_duration_since(Instant::now()).as_millis() as i32;
            if left <= 0 { return Err(HidErr::Other("the key did not answer in time".into())); }
            let p = self.read_packet(left)?;
            if p[..4] != self.cid || p[4] & 0x80 == 0 { continue; }   // another channel, or a stray continuation
            let cmd = p[4] & 0x7F;
            let len = ((p[5] as usize) << 8) | p[6] as usize;
            let mut out = p[7..7 + len.min(PKT - 7)].to_vec();
            let mut seq = 0u8;
            while out.len() < len {
                let q = self.read_packet(2000)?;
                if q[..4] != self.cid || q[4] != seq { continue; }
                let n = (len - out.len()).min(PKT - 5); out.extend_from_slice(&q[5..5 + n]); seq = seq.wrapping_add(1);
            }
            if cmd == CTAP_KEEPALIVE { continue; }
            return Ok((cmd, out));
        }
    }
    fn call(&self, cmd: u8, data: &[u8], timeout_ms: i32) -> Result<Vec<u8>, HidErr> {
        self.send(cmd, data)?;
        let (rc, out) = self.recv(timeout_ms)?;
        if rc == CTAP_ERROR { return Err(HidErr::Ctap(out.first().copied().unwrap_or(0))); }
        if rc != cmd { return Err(HidErr::Other(format!("the key answered command {rc:#04x} to {cmd:#04x}"))); }
        Ok(out)
    }
    /// Asked with 0x01, a current firmware answers its version as text ("v1.9.1", with any pre-release suffix); an
    /// old one answers the 32-bit word. Both are taken.
    fn version(&self) -> Result<String, HidErr> {
        let v = self.call(CMD_VERSION, &[0x01], 3000)?;
        version_from_answer(&v).ok_or_else(|| HidErr::Other("unreadable version answer".into()))
    }
    fn uuid(&self) -> Result<[u8; 16], HidErr> {
        let v = self.call(CMD_UUID, &[], 3000)?;
        if v.len() < 16 { return Err(HidErr::Other("short UUID answer".into())); }
        let mut u = [0u8; 16]; u.copy_from_slice(&v[..16]); Ok(u)
    }
    /// init status, free blocks, variant (1 LPC55 · 2 nRF52), model, revision — the variant decides who may flash.
    fn variant(&self) -> Result<&'static str, HidErr> {
        let s = self.call(CMD_APP, &[APP_STATUS], 3000)?;
        Ok(match s.get(4) { Some(1) => "lpc55", Some(2) => "nrf52", _ => "?" })
    }
    /// Reboot into the ROM bootloader. The key blinks white and waits for a touch; it reboots on the touch (the
    /// connection drops: that is success) and answers INVALID_LENGTH when the touch does not come.
    fn reboot_to_bootloader(&self) -> Result<(), String> {
        match self.call(CMD_UPDATE, &[], 30_000) {
            Ok(_) | Err(HidErr::Gone(_)) => Ok(()),
            Err(HidErr::Ctap(CTAP_ERR_INVALID_LENGTH)) => Err("the key was not touched in time".into()),
            Err(e) => Err(e.text()),
        }
    }
}

/// Every Nitrokey 3 answering on the FIDO HID interface: (uuid, version, variant, path).
fn keys(api: &hidapi::HidApi) -> Vec<(String, String, &'static str, std::ffi::CString)> {
    let mut out = Vec::new();
    for d in api.device_list() {
        if d.vendor_id() != VID || d.product_id() != PID_KEY || d.usage_page() != FIDO_USAGE_PAGE { continue; }
        let Ok(c) = Ctap::open(api, d.path()) else { continue };
        if let (Ok(u), Ok(v), Ok(var)) = (c.uuid(), c.version(), c.variant()) { out.push((hex::encode_upper(u), v, var, d.path().to_owned())); }
    }
    out
}

/// The key whose OpenPGP serial is the first four bytes of its UUID (cards.rs: a Nitrokey writes its serial binary).
fn key_for(api: &hidapi::HidApi, serial: u32) -> Result<(String, String, &'static str, std::ffi::CString), String> {
    let want = hex::encode_upper(serial.to_be_bytes());
    keys(api).into_iter().find(|k| k.0.starts_with(&want)).ok_or_else(|| "that Nitrokey does not answer on its FIDO interface".to_string())
}

fn api() -> Result<hidapi::HidApi, String> { hidapi::HidApi::new().map_err(|e| format!("HID: {e}")) }

/// What this key runs, against the newest release known at build time. One HID conversation, no network.
pub fn firmware(serial: u32) -> Result<Firmware, String> {
    let api = api()?;
    let (uuid, version, variant, _) = key_for(&api, serial)?;
    Ok(Firmware { behind: is_behind(&version, LATEST_KNOWN), flashable: variant == "lpc55", uuid, version, variant: variant.into(), latest_known: LATEST_KNOWN.into(), latest_known_as_of: LATEST_KNOWN_AS_OF.into() })
}

/// The firmware version for the card list, remembered per key for a while so the 2 s poll does not open HID each time.
pub fn firmware_cached(serial: u32) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<u32, (Option<String>, Instant)>>> = Mutex::new(None);
    let mut g = CACHE.lock().ok()?;
    let m = g.get_or_insert_with(HashMap::new);
    if let Some((v, at)) = m.get(&serial) {
        let ttl = if v.is_some() { 600 } else { 30 };
        if at.elapsed().as_secs() < ttl { return v.clone(); }
    }
    let v = firmware(serial).ok().map(|f| f.version);
    m.insert(serial, (v.clone(), Instant::now()));
    v
}

// ---------- the release ----------

#[derive(serde::Deserialize)] struct Release { tag_name: String, assets: Vec<Asset> }
#[derive(serde::Deserialize)] struct Asset { name: String, browser_download_url: String }

fn http() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder().user_agent("anytwokeys").timeout(Duration::from_secs(60)).build().map_err(|e| e.to_string())
}

/// The newest release on GitHub: (version, URL of the firmware container). Sends nothing about the key.
pub fn latest_release() -> Result<(String, String), String> {
    let r: Release = http()?.get(RELEASES).send().map_err(|e| format!("could not reach the release list: {e}"))?.error_for_status().map_err(|e| e.to_string())?.json().map_err(|e| e.to_string())?;
    let version = r.tag_name.trim_start_matches('v').to_string();
    let want = format!("firmware-nk3-{}.zip", r.tag_name);
    let url = r.assets.into_iter().find(|a| a.name == want).map(|a| a.browser_download_url).ok_or_else(|| format!("release {} has no {want}", r.tag_name))?;
    Ok((version, url))
}

pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut resp = http()?.get(url).send().map_err(|e| format!("download: {e}"))?.error_for_status().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    resp.by_ref().take(MAX_DOWNLOAD as u64 + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() > MAX_DOWNLOAD { return Err("the download is larger than any firmware release".into()); }
    Ok(out)
}

pub struct Container { pub version: String, pub lpc55: Vec<u8> }

/// The firmware container: a zip with `sha256sums`, `manifest.json` and one image per variant. Every file used is
/// checked against the list, the way Nitrokey's own tool does.
pub fn parse_container(bytes: &[u8]) -> Result<Container, String> {
    use sha2::{Digest, Sha256};
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a firmware container: {e}"))?;
    let read = |z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str| -> Result<Vec<u8>, String> {
        let mut f = z.by_name(name).map_err(|_| format!("the container has no {name}"))?;
        let mut v = Vec::new(); f.read_to_end(&mut v).map_err(|e| e.to_string())?; Ok(v)
    };
    let sums = String::from_utf8(read(&mut z, "sha256sums")?).map_err(|_| "sha256sums is not text")?;
    let sums: std::collections::HashMap<String, String> = sums.lines().filter_map(|l| { let (h, n) = l.split_once("  ")?; Some((n.trim().to_string(), h.trim().to_lowercase())) }).collect();
    let check = |name: &str, data: &[u8]| -> Result<(), String> {
        let want = sums.get(name).ok_or_else(|| format!("{name} is not in the container's checksum list"))?;
        if hex::encode(Sha256::digest(data)) != *want { return Err(format!("{name} does not match its checksum")); }
        Ok(())
    };
    let manifest = read(&mut z, "manifest.json")?; check("manifest.json", &manifest)?;
    let m: serde_json::Value = serde_json::from_slice(&manifest).map_err(|e| format!("manifest: {e}"))?;
    if m["device"].as_str() != Some("Nitrokey 3") { return Err("this container is not for a Nitrokey 3".into()); }
    let version = m["version"].as_str().ok_or("manifest has no version")?.trim_start_matches('v').to_string();
    let image = m["images"]["lpc55"].as_str().ok_or("manifest has no lpc55 image")?.to_string();
    let lpc55 = read(&mut z, &image)?; check(&image, &lpc55)?;
    if &lpc55.get(20..24).unwrap_or(&[]) != b"STMP" { return Err("the lpc55 image is not an SB2 file".into()); }
    Ok(Container { version, lpc55 })
}

// ---------- NXP mboot over HID: the ROM bootloader ----------

const RPT_CMD_OUT: u8 = 0x01;
const RPT_DATA_OUT: u8 = 0x02;
const TAG_GET_PROPERTY: u8 = 0x07;
const TAG_RECEIVE_SB: u8 = 0x08;
const TAG_RESET: u8 = 0x0B;
const RESP_GENERIC: u8 = 0xA0;
const RESP_GET_PROPERTY: u8 = 0xA7;
const PROP_MAX_PACKET: u32 = 0x0B;
const FLAG_DATA_PHASE: u8 = 0x01;

/// A report: id, a zero, the payload length little-endian, the payload.
pub(crate) fn mboot_frame(report: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![report, 0, payload.len() as u8, (payload.len() >> 8) as u8]; v.extend_from_slice(payload); v
}
/// A command packet: tag, flags, reserved, parameter count, then the parameters little-endian.
pub(crate) fn mboot_command(tag: u8, flags: u8, params: &[u32]) -> Vec<u8> {
    let mut v = vec![tag, flags, 0, params.len() as u8]; for p in params { v.extend_from_slice(&p.to_le_bytes()); } v
}
/// A response report parsed: (tag, parameters). The first parameter is the status; 0 is success.
pub(crate) fn mboot_parse(report: &[u8]) -> Option<(u8, Vec<u32>)> {
    if report.len() < 8 { return None; }
    let len = report[2] as usize | ((report[3] as usize) << 8);
    let body = report.get(4..4 + len)?;
    let (tag, count) = (body[0], body[3] as usize);
    let params = (0..count).filter_map(|i| body.get(4 + i * 4..8 + i * 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))).collect();
    Some((tag, params))
}
/// Where the SDK pauses 100 ms while streaming: just past the first boot tag block of the SB2 header.
pub(crate) fn sb2_pause_point(image: &[u8]) -> Option<usize> {
    (image.len() >= 36 && &image[20..24] == b"STMP").then(|| u32::from_le_bytes([image[32], image[33], image[34], image[35]]) as usize * 16)
}

struct Boot { dev: hidapi::HidDevice }
impl Boot {
    fn find(api: &hidapi::HidApi) -> Option<Boot> {
        api.device_list().find(|d| d.vendor_id() == VID && d.product_id() == PID_BOOTLOADER).and_then(|d| api.open_path(d.path()).ok()).map(|dev| Boot { dev })
    }
    fn read(&self, timeout_ms: i32) -> Result<(u8, Vec<u32>), String> {
        let mut buf = [0u8; 1024];
        let n = self.dev.read_timeout(&mut buf, timeout_ms).map_err(|e| format!("bootloader: {e}"))?;
        if n == 0 { return Err("the bootloader did not answer in time".into()); }
        mboot_parse(&buf[..n]).ok_or_else(|| "malformed answer from the bootloader".into())
    }
    fn command(&self, tag: u8, flags: u8, params: &[u32], timeout_ms: i32) -> Result<Vec<u32>, String> {
        self.dev.write(&mboot_frame(RPT_CMD_OUT, &mboot_command(tag, flags, params))).map_err(|e| format!("bootloader: {e}"))?;
        let (rtag, p) = self.read(timeout_ms)?;
        let want = if tag == TAG_GET_PROPERTY { RESP_GET_PROPERTY } else { RESP_GENERIC };
        if rtag != want { return Err(format!("the bootloader answered {rtag:#04x} to command {tag:#04x}")); }
        if p.first().copied().unwrap_or(1) != 0 { return Err(format!("the bootloader refused command {tag:#04x} (status {})", p[0])); }
        Ok(p)
    }
    fn max_packet(&self) -> usize { self.command(TAG_GET_PROPERTY, 0, &[PROP_MAX_PACKET, 0], 2000).ok().and_then(|p| p.get(1).copied()).filter(|n| *n > 0).unwrap_or(32) as usize }
    /// Stream the signed image. The ROM checks the signature as it goes and refuses anything not Nitrokey's.
    fn receive_sb_file(&self, image: &[u8], progress: &dyn Fn(usize, usize)) -> Result<(), String> {
        let max = self.max_packet();
        self.command(TAG_RECEIVE_SB, FLAG_DATA_PHASE, &[image.len() as u32], 5000)?;
        let pause = sb2_pause_point(image);
        let (mut sent, mut paused) = (0usize, false);
        for chunk in image.chunks(max) {
            self.dev.write(&mboot_frame(RPT_DATA_OUT, chunk)).map_err(|e| format!("bootloader: {e} after {sent} bytes"))?;
            sent += chunk.len(); progress(sent, image.len());
            if !paused && pause.map_or(false, |p| sent > p) { std::thread::sleep(Duration::from_millis(100)); paused = true; }
        }
        let (tag, p) = self.read(60_000)?;
        if tag != RESP_GENERIC || p.first().copied().unwrap_or(1) != 0 { return Err(format!("the bootloader refused the image (status {})", p.first().copied().unwrap_or(0))); }
        Ok(())
    }
    fn reset(&self) -> Result<(), String> { self.command(TAG_RESET, 0, &[], 3000).map(|_| ()) }
}

fn wait_for<T>(secs: u64, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until { if let Some(t) = f() { return Some(t); } std::thread::sleep(Duration::from_millis(500)); }
    None
}

/// The whole update of one key, reporting as it goes. Returns what the key runs afterwards.
pub fn update(serial: u32, report: &dyn Fn(Progress)) -> Result<Firmware, String> { update_with(serial, report, None) }
/// The same with a container already in hand (tests), in which case the key's own version is not a bar.
pub fn update_with(serial: u32, report: &dyn Fn(Progress), given: Option<Container>) -> Result<Firmware, String> {
    let say = |phase: &'static str, pct: u8, text: String| report(Progress { phase, pct, text });
    // A key already sitting in its loader (an earlier attempt stopped after the touch) is taken from there: no
    // version to read, nothing to confirm, just the image to write.
    let in_loader = api().ok().and_then(|a| Boot::find(&a)).is_some();
    let before = match firmware(serial) { Ok(f) => Some(f), Err(e) if in_loader => { say("flash", 12, "The key is already in its loader".into()); let _ = e; None }, Err(e) => return Err(e) };
    if before.as_ref().map_or(false, |b| !b.flashable) { return Err("this Nitrokey is the nRF52 kind (a 3A Mini); update it with Nitrokey App 2".into()); }
    let container = match given {
        Some(c) => c,
        None => {
            say("fetch", 0, "Asking Nitrokey's release list".into());
            let (version, url) = latest_release()?;
            if let Some(b) = &before { if !is_behind(&b.version, &version) { return Err(format!("the key already runs {}, and {version} is the newest release", b.version)); } }
            say("fetch", 5, format!("Downloading firmware {version}"));
            let c = parse_container(&download(&url)?)?;
            if c.version != version { return Err(format!("the container says {} but the release is {version}", c.version)); }
            c
        }
    };
    let version = container.version.clone();
    if let Some(b) = &before {
        if is_behind(&version, &b.version) { return Err(format!("{version} is older than the {} on the key; a Nitrokey does not go backwards", b.version)); }
        say("touch", 10, format!("Touch the key when it blinks white: it restarts into its loader to take {version}"));
        let api = api()?;
        let (_, _, _, path) = key_for(&api, serial)?;
        let c = Ctap::open(&api, &path).map_err(|e| e.text())?;
        c.reboot_to_bootloader()?;
        say("flash", 12, "Waiting for the loader".into());
        std::thread::sleep(Duration::from_secs(1));
    }
    let boot = wait_for(30, || api().ok().and_then(|a| Boot::find(&a))).ok_or("the key did not come back as a loader; unplug it, plug it in and try again")?;
    boot.receive_sb_file(&container.lpc55, &|sent, total| say("flash", 12 + (sent * 80 / total.max(1)) as u8, format!("Writing {version}: {} of {} KB", sent / 1024, total / 1024)))?;
    boot.reset()?;
    drop(boot);
    say("wait", 95, "The key is restarting".into());
    let after = wait_for(40, || firmware(serial).ok()).ok_or("the key did not come back after the update; unplug it and plug it in again")?;
    if after.version != version { return Err(format!("the update went through but the key reports {}", after.version)); }
    say("done", 100, format!("Firmware {version}"));
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> { std::fs::read(format!("{}/tests/fixtures/firmware-nk3-v1.9.1.zip", env!("CARGO_MANIFEST_DIR"))).unwrap() }

    #[test]
    fn the_admin_apps_version_word_and_release_tags_read_the_same_way() {
        assert_eq!(version_from_int((1 << 22) | (9 << 6) | 1), "1.9.1");
        assert_eq!(version_from_int((1 << 22) | (8 << 6) | 2), "1.8.2");
        assert_eq!(version_from_answer(b"v1.9.1"), Some("1.9.1".into())); assert_eq!(version_from_answer(b"v1.9.0-rc.2"), Some("1.9.0-rc.2".into()));
        assert_eq!(version_from_answer(&((1u32 << 22) | (9 << 6) | 1).to_be_bytes()), Some("1.9.1".into())); assert_eq!(version_from_answer(b"junky"), None);
        assert_eq!(parse_version("v1.9.1"), Some((1, 9, 1))); assert_eq!(parse_version("1.9.0-rc.2"), Some((1, 9, 0))); assert_eq!(parse_version("x"), None);
        assert!(is_behind("1.9.0", "1.9.1")); assert!(!is_behind("1.9.1", "1.9.1")); assert!(!is_behind("1.10.0", "1.9.1")); assert!(!is_behind("?", "1.9.1"));
    }

    #[test]
    fn a_command_is_cut_into_64_byte_packets_with_the_channel_and_sequence_numbers() {
        let cid = [1, 2, 3, 4];
        let one = ctap_packets(cid, 0x61, &[0x01]);
        assert_eq!(one.len(), 1); assert_eq!(&one[0][..8], &[1, 2, 3, 4, 0xE1, 0, 1, 1]);
        let data: Vec<u8> = (0..130u8).collect();
        let p = ctap_packets(cid, 0x72, &data);
        assert_eq!(p.len(), 3);
        assert_eq!(&p[0][4..7], &[0xF2, 0, 130]); assert_eq!(&p[0][7..], &data[..57]);
        assert_eq!(p[1][4], 0); assert_eq!(&p[1][5..], &data[57..116]);
        assert_eq!(p[2][4], 1); assert_eq!(&p[2][5..5 + 14], &data[116..130]); assert!(p[2][19..].iter().all(|b| *b == 0));
    }

    #[test]
    fn bootloader_frames_commands_and_answers_are_the_sdks_byte_for_byte() {
        assert_eq!(mboot_command(TAG_RECEIVE_SB, FLAG_DATA_PHASE, &[508928]), vec![0x08, 0x01, 0, 1, 0x00, 0xC4, 0x07, 0x00]);
        assert_eq!(mboot_command(TAG_GET_PROPERTY, 0, &[PROP_MAX_PACKET, 0]), vec![0x07, 0, 0, 2, 0x0B, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(mboot_frame(RPT_CMD_OUT, &[0x0B, 0, 0, 0]), vec![1, 0, 4, 0, 0x0B, 0, 0, 0]);
        // a generic response: report 3, length 12, header A0 00 00 02, status 0, command tag 8
        let r = [3u8, 0, 12, 0, 0xA0, 0, 0, 2, 0, 0, 0, 0, 8, 0, 0, 0];
        assert_eq!(mboot_parse(&r), Some((0xA0, vec![0, 8])));
        // a get-property answer: max packet size 1016 → [status 0, 1016]
        let r = [3u8, 0, 12, 0, 0xA7, 0, 0, 2, 0, 0, 0, 0, 0xF8, 3, 0, 0];
        assert_eq!(mboot_parse(&r), Some((0xA7, vec![0, 1016])));
        assert_eq!(mboot_parse(&[3, 0]), None);
    }

    #[test]
    fn the_real_release_container_parses_and_every_file_in_it_matches_its_checksum() {
        let c = parse_container(&fixture()).unwrap();
        assert_eq!(c.version, "1.9.1"); assert_eq!(c.lpc55.len(), 508928);
        assert_eq!(sb2_pause_point(&c.lpc55), Some(228 * 16));
        // a flipped byte in the image is caught by the checksum list
        let mut z = fixture(); let pos = z.len() / 3; z[pos] ^= 1;   // a third of the way in: inside the lpc55 image
        assert!(parse_container(&z).is_err());
        assert!(parse_container(b"not a zip").is_err());
    }

    /// Reads only: `cargo test hardware_nk_firmware_read -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn hardware_nk_firmware_read() {
        let api = api().unwrap();
        for (uuid, version, variant, _) in keys(&api) { println!("Nitrokey 3 {uuid} firmware {version} ({variant})"); let serial = u32::from_be_bytes([u8::from_str_radix(&uuid[0..2], 16).unwrap(), u8::from_str_radix(&uuid[2..4], 16).unwrap(), u8::from_str_radix(&uuid[4..6], 16).unwrap(), u8::from_str_radix(&uuid[6..8], 16).unwrap()]); println!("{:?}", firmware(serial)); }
    }

    /// The real update from the release list, touch and all:
    /// `A2K_NK_UPDATE_TEST=<serial> cargo test hardware_nk_firmware_update -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn hardware_nk_firmware_update() {
        let Ok(serial) = std::env::var("A2K_NK_UPDATE_TEST") else { eprintln!("set A2K_NK_UPDATE_TEST to the key's OpenPGP serial (decimal)"); return };
        let r = update(serial.parse().unwrap(), &|p| println!("{:>5} {:3}% {}", p.phase, p.pct, p.text));
        println!("{r:?}"); r.unwrap();
    }

    /// The flasher itself, proven on a key that already runs 1.9.1: the fixture container is written over it, touch
    /// and all. `A2K_NK_FLASH_TEST=<serial> cargo test hardware_nk_flash_same -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn hardware_nk_flash_same_version() {
        let Ok(serial) = std::env::var("A2K_NK_FLASH_TEST") else { eprintln!("set A2K_NK_FLASH_TEST to the key's OpenPGP serial (decimal)"); return };
        let c = parse_container(&fixture()).unwrap();
        let r = update_with(serial.parse().unwrap(), &|p| println!("{:>5} {:3}% {}", p.phase, p.pct, p.text), Some(c));
        println!("{r:?}"); assert_eq!(r.unwrap().version, "1.9.1");
    }
}
