//! Files kept with a transaction (or an address, or a coin): a receipt, an invoice, a photo.
//! Bytes live on this computer under `attachments/`, sealed under the store key (lockbox.rs) and named by a KEYED hash
//! of their content, so neither what they say nor which known file they are can be read off the disk; the record
//! that names them (by plain sha256) rides in the same synced record as the label (labels.rs). Photos are re-saved without their metadata unless the person says not
//! to: a receipt photographed at home carries the home's coordinates. See roadmap TRANSACTION-DETAIL.md.
use bitcoin::hashes::{sha256, Hash};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const MAX_FILE: u64 = 25 * 1024 * 1024;
const THUMB_EDGE: u32 = 360;

pub fn dir() -> PathBuf { crate::lockbox::dir().join("attachments") }
fn blob(hash: &str) -> Result<PathBuf, String> {
    // the path is built from the HASH and nothing else: a name never reaches the file system
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) { return Err("that is not an attachment id".into()); }
    let hash = hash.to_lowercase();
    Ok(dir().join(if crate::lockbox::sealed_mode() { crate::lockbox::blob_name(&hash)? } else { hash }))
}
fn read_blob(hash: &str) -> Result<Vec<u8>, String> {
    let p = blob(hash)?;
    if crate::lockbox::sealed_mode() { crate::lockbox::read_sealed(&p, "att")?.ok_or_else(|| "that file is not on this computer".to_string()) }
    else { std::fs::read(&p).map_err(|_| "that file is not on this computer".to_string()) }
}

fn put(bytes: &[u8]) -> Result<String, String> {
    let hash = hex::encode(sha256::Hash::hash(bytes));
    let path = blob(&hash)?;
    if !path.exists() {
        std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
        if crate::lockbox::sealed_mode() { crate::lockbox::write_sealed(&path, "att", bytes)?; }
        else {
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        }
    }
    Ok(hash)
}

/// Readable copies exist only while something outside this app is looking at them: a picture handed to the viewer,
/// a HEIC on its way through the system's converter. They live in one private folder that is emptied when the app
/// locks, quits and starts.
fn scratch() -> Result<PathBuf, String> {
    let d = dir().join("opened");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)); }
    Ok(d)
}
pub fn clear_opened() { let _ = std::fs::remove_dir_all(dir().join("opened")); }

/// What a file is, by its first bytes — never by the name it arrived with.
pub fn sniff(b: &[u8]) -> &'static str {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) { "image/jpeg" }
    else if b.starts_with(b"\x89PNG\r\n\x1a\n") { "image/png" }
    else if b.starts_with(b"GIF8") { "image/gif" }
    else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" { "image/webp" }
    else if b.len() > 12 && &b[4..8] == b"ftyp" && [&b"heic"[..], b"heix", b"mif1", b"heim", b"hevc"].contains(&&b[8..12]) { "image/heic" }
    else if b.starts_with(b"%PDF-") { "application/pdf" }
    else if b.len() < 2_000_000 && std::str::from_utf8(b).is_ok() { "text/plain" }
    else { "application/octet-stream" }
}

/// The extension a viewer needs, from the TYPE. `None` = this app will only ever show it in its folder.
fn ext_for(mime: &str) -> Option<&'static str> {
    Some(match mime { "image/jpeg" => "jpg", "image/png" => "png", "image/gif" => "gif", "image/webp" => "webp", "image/heic" => "heic", "application/pdf" => "pdf", "text/plain" => "txt", _ => return None })
}

/// A display name: what the person called it, with nothing that means something to a file system or a shell.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().map(|c| if c.is_alphanumeric() || " ._-()".contains(c) { c } else { '_' }).collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c == ' ' || c == '_').to_string();
    let cut: String = cleaned.chars().take(80).collect();
    if cut.is_empty() { "file".into() } else { cut }
}

#[derive(Debug, Serialize)]
pub struct Added { pub entry: serde_json::Value, pub note: Option<String> }

/// Decode, turn upright, and save again: what comes out has pixels and nothing else. Returns (bytes, mime, thumbnail).
fn resave(bytes: &[u8], mime: &str) -> Result<(Vec<u8>, &'static str, Vec<u8>), String> {
    use image::{ImageDecoder, ImageReader};
    let mut dec = ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?.into_decoder().map_err(|e| format!("that picture could not be read: {e}"))?;
    let orientation = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(dec).map_err(|e| format!("that picture could not be read: {e}"))?;
    img.apply_orientation(orientation);   // the rotation lived in the metadata that is about to go
    let mut out = Vec::new();
    let kind = if mime == "image/png" || mime == "image/gif" {
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| e.to_string())?; "image/png"
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&img.to_rgb8()).map_err(|e| e.to_string())?; "image/jpeg"
    };
    Ok((out, kind, thumb_of(&img)?))
}
fn thumb_of(img: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut t = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut t, 78).encode_image(&img.thumbnail(THUMB_EDGE, THUMB_EDGE).to_rgb8()).map_err(|e| e.to_string())?;
    Ok(t)
}

/// HEIC is what an iPhone makes and no pure-Rust decoder reads: on a Mac the system converts it; elsewhere it is kept as it is.
fn heic_to_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    if !cfg!(target_os = "macos") { return None; }
    let tmp = scratch().ok()?.join(format!("convert-{}.heic", hex::encode(&sha256::Hash::hash(bytes)[..8])));
    let out = tmp.with_extension("jpg");
    std::fs::write(&tmp, bytes).ok()?;
    let ok = std::process::Command::new("/usr/bin/sips").args(["-s", "format", "jpeg"]).arg(&tmp).arg("--out").arg(&out).output().map(|o| o.status.success()).unwrap_or(false);
    let got = if ok { std::fs::read(&out).ok() } else { None };
    let _ = std::fs::remove_file(&tmp); let _ = std::fs::remove_file(&out);
    got
}

/// Take a file in. `keep_original`: store a picture byte for byte, metadata and all.
pub fn add(name: &str, bytes: &[u8], keep_original: bool, now: u64) -> Result<Added, String> {
    if bytes.is_empty() { return Err("that file is empty".into()); }
    if bytes.len() as u64 > MAX_FILE { return Err(format!("{} is larger than 25 MB", safe_name(name))); }
    let mut mime = sniff(bytes);
    let mut note = None;
    let (stored, thumb, meta): (Vec<u8>, Option<Vec<u8>>, &str) = if mime.starts_with("image/") {
        let source = if mime == "image/heic" { heic_to_jpeg(bytes) } else { Some(bytes.to_vec()) };
        match (keep_original, source) {
            (false, Some(src)) => match resave(&src, if mime == "image/heic" { "image/jpeg" } else { mime }) {
                Ok((b, m, t)) => { mime = m; (b, Some(t), "stripped") }
                Err(e) => return Err(e),
            },
            (false, None) => { note = Some("This computer cannot re-save HEIC pictures, so it was kept exactly as it is, location data included.".into()); (bytes.to_vec(), None, "original") }
            (true, src) => { let t = src.and_then(|s| image::load_from_memory(&s).ok()).and_then(|i| thumb_of(&i).ok()); (bytes.to_vec(), t, "original") }
        }
    } else { (bytes.to_vec(), None, "n/a") };
    let sha = put(&stored)?;
    let mut entry = serde_json::json!({ "name": safe_name(name), "mime": mime, "size": stored.len(), "sha256": sha, "t": now, "meta": meta });
    if let Some(t) = thumb { entry["thumb"] = serde_json::Value::String(put(&t)?); }
    // a re-saved picture changes type: say so in the name the person sees
    if meta == "stripped" { if let (Some(ext), Some(stem)) = (ext_for(mime), Path::new(&safe_name(name)).file_stem().and_then(|s| s.to_str())) { entry["name"] = format!("{stem}.{ext}").into(); } }
    Ok(Added { entry, note })
}

pub fn have(hash: &str) -> bool { blob(hash).map(|p| p.exists()).unwrap_or(false) }

/// A thumbnail's bytes, for the window to draw. Only small things come back this way.
pub fn small(hash: &str) -> Result<Vec<u8>, String> {
    if std::fs::metadata(blob(hash)?).map_err(|_| "not on this computer".to_string())?.len() > 400_000 { return Err("too large to preview".into()); }
    read_blob(hash)
}

/// Delete bytes nothing refers to any more. `in_use`: every hash any record of any vault still names.
pub fn sweep(gone: &[String], in_use: &std::collections::HashSet<String>) {
    for h in gone { if !in_use.contains(h) { if let Ok(p) = blob(h) { let _ = std::fs::remove_file(p); } } }
}

/// Hand a file to the system's own viewer — only the kinds that are looked at, never run — under a name built from its
/// TYPE. Anything else is shown in its folder. Returns what was done.
pub fn open(hash: &str, name: &str, mime: &str) -> Result<&'static str, String> {
    let bytes = read_blob(hash)?;
    let folder = scratch()?.join(&hash[..16]);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let sniffed = sniff(&bytes);   // what it IS, not what its record says
    let stem = Path::new(&safe_name(name)).file_stem().and_then(|s| s.to_str()).unwrap_or("file").to_string();
    let (shown, view) = match ext_for(sniffed) { Some(ext) if sniffed == mime || mime.starts_with("image/") => (folder.join(format!("{stem}.{ext}")), true), _ => (folder.join(format!("{stem}.bin")), false) };
    std::fs::write(&shown, &bytes).map_err(|e| e.to_string())?;
    if view { ::open::that(&shown).map_err(|e| e.to_string())?; Ok("opened") }
    else { reveal(&shown)?; Ok("shown in its folder: this kind of file is not opened from here") }
}
fn reveal(p: &Path) -> Result<(), String> {
    if cfg!(target_os = "macos") { std::process::Command::new("/usr/bin/open").arg("-R").arg(p).status().map_err(|e| e.to_string())?; Ok(()) }
    else { ::open::that(p.parent().unwrap_or(p)).map_err(|e| e.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_name_cannot_reach_the_file_system_and_a_type_comes_from_the_bytes() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\Users\\x\\inv:oice?.pdf"), "inv_oice_.pdf");
        assert_eq!(safe_name(".hidden"), "hidden");
        assert_eq!(safe_name("  "), "file");
        assert_eq!(safe_name(&"a".repeat(300)).chars().count(), 80);
        assert!(blob("../../x").is_err() && blob(&"g".repeat(64)).is_err() && blob(&"ab".repeat(32)).is_ok());
        assert_eq!(sniff(b"%PDF-1.7 ..."), "application/pdf");
        assert_eq!(sniff(b"#!/bin/sh\nrm -rf ~"), "text/plain");          // a script is text: it is shown, never run
        assert_eq!(sniff(&[0xCF, 0xFA, 0xED, 0xFE, 7, 0, 0, 1]), "application/octet-stream");   // a Mach-O binary
        assert_eq!(ext_for("application/octet-stream"), None);
    }
    #[test]
    fn a_photo_comes_out_without_its_metadata_and_the_right_way_up() {
        // a 2×1 JPEG, red then blue, with an EXIF block saying "rotate 90°" and carrying a GPS-looking string
        let mut px = image::RgbImage::new(2, 1); px.put_pixel(0, 0, image::Rgb([250, 5, 5])); px.put_pixel(1, 0, image::Rgb([5, 5, 250]));
        let mut plain = Vec::new(); image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 100).encode_image(&px).unwrap();
        let tiff: Vec<u8> = [&b"MM\x00\x2a\x00\x00\x00\x08\x00\x01"[..], &[0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00], &[0, 0, 0, 0], b"GPS -27.4698,153.0251"].concat();
        let exif: Vec<u8> = [&b"Exif\x00\x00"[..], &tiff].concat();
        let seg: Vec<u8> = [&[0xFF, 0xE1][..], &((exif.len() + 2) as u16).to_be_bytes(), &exif].concat();
        let with_exif: Vec<u8> = [&plain[..2], &seg, &plain[2..]].concat();
        assert!(with_exif.windows(3).any(|w| w == b"GPS"));
        let (out, mime, thumb) = resave(&with_exif, "image/jpeg").unwrap();
        assert_eq!(mime, "image/jpeg");
        assert!(!out.windows(3).any(|w| w == b"GPS") && !out.windows(4).any(|w| w == b"Exif"), "metadata survived");
        let back = image::load_from_memory(&out).unwrap();
        assert_eq!((back.width(), back.height()), (1, 2), "orientation 6 means the stored picture is 1 wide and 2 tall");
        assert!(image::load_from_memory(&thumb).is_ok());
    }
}
