//! Taking a PSBT out of the app and back in: BIP 174 bytes, UR `crypto-psbt` QR parts (BCR-2020-005/006,
//! the animated-QR transport Sparrow, Keystone, Passport, Jade and SeedSigner read), and a printable PDF.
use bitcoin::psbt::Psbt;
use std::str::FromStr;

// ---------- CBOR byte string: all the CBOR `crypto-psbt` needs ----------
fn cbor_wrap(data: &[u8]) -> Vec<u8> {
    let mut b = match data.len() {
        n if n < 24 => vec![0x40 | n as u8],
        n if n < 0x100 => vec![0x58, n as u8],
        n if n < 0x1_0000 => { let mut v = vec![0x59]; v.extend((n as u16).to_be_bytes()); v }
        n => { let mut v = vec![0x5A]; v.extend((n as u32).to_be_bytes()); v }
    };
    b.extend_from_slice(data);
    b
}
fn cbor_unwrap(b: &[u8]) -> Result<&[u8], String> {
    let bad = || "the QR did not carry a PSBT".to_string();
    let (len, at) = match *b.first().ok_or_else(bad)? {
        h @ 0x40..=0x57 => ((h & 0x1F) as usize, 1),
        0x58 => (*b.get(1).ok_or_else(bad)? as usize, 2),
        0x59 => (u16::from_be_bytes(b.get(1..3).ok_or_else(bad)?.try_into().unwrap()) as usize, 3),
        0x5A => (u32::from_be_bytes(b.get(1..5).ok_or_else(bad)?.try_into().unwrap()) as usize, 5),
        _ => return Err(bad()),
    };
    b.get(at..at + len).filter(|_| at + len == b.len()).ok_or_else(bad)
}

/// UR parts for a PSBT. One part if it fits in `fragment` bytes; otherwise the n plain fragments followed by n
/// fountain mixtures, so a camera that joins late still completes when the list is shown in a loop.
pub fn ur_parts(psbt: &[u8], fragment: usize, mixtures: bool) -> Result<Vec<String>, String> {
    let cbor = cbor_wrap(psbt);
    if cbor.len() <= fragment { return Ok(vec![ur::ur::encode(&cbor, &ur::Type::Custom("crypto-psbt")).to_uppercase()]); }
    let mut enc = ur::Encoder::new(&cbor, fragment, "crypto-psbt").map_err(|e| e.to_string())?;
    let n = enc.fragment_count();
    (0..if mixtures { n * 2 } else { n }).map(|_| enc.next_part().map(|p| p.to_uppercase()).map_err(|e| e.to_string())).collect()
}

// ---------- BBQr (Coinkite's "Better Bitcoin QR"; the Coldcard Q reads and writes it) ----------
// Spec: github.com/coinkite/BBQr/BBQr.md. Each part: "B$" + encoding (H hex · 2 base32 · Z raw-deflate wbits=10 then
// base32) + file type (P psbt, T txn, J json, U text…) + total and index, two base-36 digits each; every part but the
// last the same length, each decoding to whole bytes.
const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
fn b32_encode(d: &[u8]) -> String {
    let (mut out, mut acc, mut bits) = (String::new(), 0u32, 0u32);
    for &b in d { acc = (acc << 8) | b as u32; bits += 8; while bits >= 5 { bits -= 5; out.push(B32[((acc >> bits) & 31) as usize] as char); } }
    if bits > 0 { out.push(B32[((acc << (5 - bits)) & 31) as usize] as char); }
    out
}
fn b32_decode(t: &str) -> Result<Vec<u8>, String> {
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for c in t.bytes() {
        let v = B32.iter().position(|&x| x == c).ok_or("a BBQr part holds a character that is not base32")? as u32;
        acc = (acc << 5) | v; bits += 5;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); }
        acc &= (1 << bits) - 1;
    }
    Ok(out)
}
fn b36(n: usize) -> String { let d = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"; format!("{}{}", d[n / 36] as char, d[n % 36] as char) }
fn from_b36(t: &str) -> Option<usize> { usize::from_str_radix(t, 36).ok() }

/// BBQr parts, base32 (no compression, which every reader must accept), `per` data characters to a part (a multiple of 8).
pub fn bbqr_parts(data: &[u8], kind: char, per: usize) -> Vec<String> {
    let per = (per / 8).max(1) * 8;
    let all = b32_encode(data);
    let chunks: Vec<&str> = all.as_bytes().chunks(per).map(|c| std::str::from_utf8(c).unwrap()).collect();
    let n = chunks.len();
    chunks.iter().enumerate().map(|(i, c)| format!("B$2{kind}{}{}{c}", b36(n), b36(i))).collect()
}

/// The file a set of BBQr parts carries: (file type, bytes). Parts may come in any order, repeated.
pub fn bbqr_decode(text: &str) -> Result<(char, Vec<u8>), String> {
    let mut parts: std::collections::BTreeMap<usize, &str> = Default::default();
    let (mut enc, mut kind, mut total) = (None, None, None);
    for p in text.split_whitespace() {
        let h = p.get(..8).filter(|h| h.starts_with("B$")).ok_or("that is not a BBQr code")?;
        let (e, k, n, i) = (h.as_bytes()[2] as char, h.as_bytes()[3] as char, from_b36(&h[4..6]).ok_or("bad BBQr header")?, from_b36(&h[6..8]).ok_or("bad BBQr header")?);
        if enc.get_or_insert(e) != &e || kind.get_or_insert(k) != &k || total.get_or_insert(n) != &n { return Err("those BBQr parts are from different files".into()); }
        if i >= n { return Err("bad BBQr header".into()); }
        parts.insert(i, &p[8..]);
    }
    let (enc, kind, total) = (enc.ok_or("nothing to read")?, kind.unwrap(), total.unwrap());
    if parts.len() < total { return Err(format!("some QR parts are missing ({} of {total}); scan until the code completes", parts.len())); }
    let body: String = parts.values().copied().collect();
    let bytes = match enc {
        'H' => hex::decode(&body).map_err(|e| format!("BBQr hex: {e}"))?,
        '2' => b32_decode(&body)?,
        'Z' => { use std::io::Read; let raw = b32_decode(&body)?; let mut out = Vec::new(); flate2::read::DeflateDecoder::new(raw.as_slice()).read_to_end(&mut out).map_err(|e| format!("BBQr: could not decompress: {e}"))?; out }
        e => return Err(format!("BBQr encoding '{e}' is not one this app reads")),
    };
    Ok((kind, bytes))
}

/// A PSBT from whatever the person has: the binary file, base64, hex, or UR text (one part, or every part on its own line).
pub fn parse(input: &[u8]) -> Result<Psbt, String> {
    if input.starts_with(b"psbt\xff") { return Psbt::deserialize(input).map_err(|e| format!("not a valid PSBT: {e}")); }
    let text = std::str::from_utf8(input).map_err(|_| "that file is neither a PSBT nor text".to_string())?.trim();
    if text.is_empty() { return Err("nothing to read".into()); }
    if text.starts_with("B$") {
        return match bbqr_decode(text)? {
            ('P', b) => Psbt::deserialize(&b).map_err(|e| format!("not a valid PSBT: {e}")),
            ('T', _) => Err("that is a finished transaction, not a PSBT: the device completed it. Broadcast it from the device's own wallet, or scan the PSBT instead".into()),
            (k, _) => Err(format!("that BBQr code carries a '{k}' file, not a PSBT")),
        };
    }
    if text.len() >= 3 && text[..3].eq_ignore_ascii_case("ur:") {
        let parts: Vec<String> = text.split_whitespace().map(|p| p.to_lowercase()).collect();
        // crypto-psbt (BCR-2020-006) and the newer registry's plain "psbt": the same CBOR byte string
        if parts.iter().any(|p| !p.starts_with("ur:crypto-psbt/") && !p.starts_with("ur:psbt/")) { return Err("that UR is not a PSBT (crypto-psbt)".into()); }
        let cbor = if parts.len() == 1 && parts[0].matches('/').count() == 1 {
            ur::ur::decode(&parts[0]).map_err(|e| e.to_string())?.1
        } else {
            let mut d = ur::Decoder::default();
            for p in &parts { d.receive(p).map_err(|e| e.to_string())?; }
            d.message().map_err(|e| e.to_string())?.ok_or("some QR parts are missing; scan until the code completes")?
        };
        return Psbt::deserialize(cbor_unwrap(&cbor)?).map_err(|e| format!("not a valid PSBT: {e}"));
    }
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() % 2 == 0 && compact.to_lowercase().starts_with("70736274ff") {
        return Psbt::deserialize(&hex::decode(&compact).map_err(|e| e.to_string())?).map_err(|e| format!("not a valid PSBT: {e}"));
    }
    Psbt::from_str(&compact).map_err(|e| format!("not a valid PSBT: {e}"))
}

/// The vault's recovery sheet: descriptor (and chain code) as one sturdy code the Restore screen scans, the keys, the line as text.
pub fn recovery_sheet(v: &crate::wallet::Vault) -> Result<Sheet, String> {
    let line = crate::wallet::recovery_line(v);
    let rotating = crate::wallet::rotates(v);
    // A sheet is read in an emergency, so for a vault of extended keys it must say what is true of THAT vault: no
    // chain code, no seal on any card, and signing done on the keys' own devices from their seed words.
    if v.ext.is_some() { return ext_sheet(v, line); }
    let sealed = v.keys.iter().filter(|k| v.seal.sealed.contains(&k.pubkey)).count();
    Ok(Sheet {
        title: "Vault recovery sheet (output descriptor)".into(),
        facts: vec![("Vault".into(), format!("{} \u{b7} {} of {} \u{b7} {}", v.name, v.threshold, v.keys.len(), v.network)),
            ("Addresses".into(), if rotating { "a fresh one for every deposit (the chain code below derives them)".into() } else { "one fixed address".into() }),
            ("First address".into(), crate::wallet::address_at(v, 0)?), ("Created".into(), ymd(v.created)),
            ("Sealed on keys".into(), format!("{sealed} of {}{}", v.keys.len(), if sealed == v.keys.len() { " \u{2014} any of them also rebuild this vault without this sheet".to_string() } else { String::new() }))],
        rows: v.keys.iter().enumerate().map(|(i, k)| (format!("Key {}", i + 1), k.pubkey.clone(), if k.serial > 0 { format!("{} \u{b7} YubiKey serial {}", k.name, k.serial) } else { k.name.clone() })).collect(),
        codes: vec![line.clone()], sturdy: true,
        code_hint: format!("To restore: Any Two Keys \u{203a} Restore \u{203a} Scan the sheet, then plug in any {} of the keys to spend.", v.threshold),
        text_heading: "The recovery line (the same as the code)".into(), text: line,
        note: format!("Public keys only: this sheet cannot spend. It does show whoever reads it what the vault holds and has done, so keep it private, and keep it away from the PINs. {}",
            if rotating { "The part before cc= is a standard output descriptor for the first address (Bitcoin Core: importdescriptors); the chain code after it derives every other address." }
            else { "The line is a standard output descriptor: Bitcoin Core imports it with importdescriptors as a watch-only wallet." }) })
}

/// The recovery sheet of a vault whose keys are extended keys held by other signers.
fn ext_sheet(v: &crate::wallet::Vault, line: String) -> Result<Sheet, String> {
    let fp = |k: &crate::wallet::KeyRef| k.pubkey.strip_prefix('[').and_then(|r| r.split('/').next()).unwrap_or("").to_string();
    // a card in a mixed vault is a bare key: it has no seed words, and lives only on the card
    let card = |k: &crate::wallet::KeyRef| !k.pubkey.starts_with('[');
    let cards = v.keys.iter().filter(|k| card(k)).count();
    if cards > 0 {
        return Ok(Sheet {
            title: "Vault recovery sheet (wallet descriptor)".into(),
            facts: vec![("Vault".into(), format!("{} \u{b7} {} of {} \u{b7} {}", v.name, v.threshold, v.keys.len(), v.network)),
                ("Addresses".into(), "a fresh one for every deposit, derived by BIP32 from the extended keys below".into()),
                ("First address".into(), crate::wallet::address_at(v, 0)?), ("Created".into(), ymd(v.created)),
                ("Signers".into(), format!("{} device{} from seed words, and {cards} card{} whose key exists only on the card \u{2014} any {} of them sign", v.keys.len() - cards, if v.keys.len() - cards == 1 { "" } else { "s" }, if cards == 1 { "" } else { "s" }, v.threshold))],
            rows: v.keys.iter().enumerate().map(|(i, k)| (format!("Key {}", i + 1), k.pubkey.clone(), if card(k) { format!("{} \u{b7} a card's key, fixed in this vault; it cannot be copied off the card", k.name) } else { format!("{} \u{b7} fingerprint {}", k.name, fp(k)) })).collect(),
            codes: vec![line.clone()], sturdy: true,
            code_hint: format!("To restore: paste or scan this line into Any Two Keys, or Bitcoin Core (importdescriptors). To spend, any {} of the keys sign: the devices from their seed words, the cards themselves.", v.threshold),
            text_heading: "The wallet descriptor (the same as the code)".into(), text: line,
            note: "Public keys only: this sheet cannot spend, but it shows whoever reads it every address and balance of this vault. A lost card is a lost key \u{2014} that is what the other keys are for. Whether Sparrow imports a card's plain key beside extended keys has not been tested.".into() });
    }
    Ok(Sheet {
        title: "Vault recovery sheet (wallet descriptor)".into(),
        facts: vec![("Vault".into(), format!("{} \u{b7} {} of {} \u{b7} {}", v.name, v.threshold, v.keys.len(), v.network)),
            ("Addresses".into(), "a fresh one for every deposit, derived by BIP32 from the extended keys below".into()),
            ("First address".into(), crate::wallet::address_at(v, 0)?), ("Created".into(), ymd(v.created)),
            ("Signers".into(), format!("other devices, each from its own seed words \u{2014} any {} of them sign", v.threshold))],
        rows: v.keys.iter().enumerate().map(|(i, k)| (format!("Key {}", i + 1), k.pubkey.clone(), format!("{} \u{b7} fingerprint {}", k.name, fp(k)))).collect(),
        codes: vec![line.clone()], sturdy: true,
        code_hint: format!("To restore: paste or scan this line into Any Two Keys, Sparrow or any wallet that reads descriptors. To spend, any {} of the keys sign on their own devices, from their seed words.", v.threshold),
        text_heading: "The wallet descriptor (the same as the code)".into(), text: line,
        note: "Extended public keys only: this sheet cannot spend. It is a complete, standard wallet descriptor \u{2014} Sparrow, Bitcoin Core (importdescriptors) and most wallets import it whole as a watch-only wallet \u{2014} so it shows whoever reads it everything this vault holds and has done. Keep it private, and keep it apart from the seed words: the seed words alone can spend if enough of them are found together, and this line tells a finder what they are for.".into() })
}

// ---------- a small PDF writer: three built-in fonts, text and filled squares ----------
/// A printable sheet. `rows` are two-line entries (label, a monospaced line, a plain line); `codes` are drawn as QR
/// codes (one large, or two to a row with `code_caption` numbering them); `text` repeats the payload for retyping.
pub struct Sheet { pub title: String, pub facts: Vec<(String, String)>, pub rows: Vec<(String, String, String)>, pub codes: Vec<String>, pub sturdy: bool,
    pub code_hint: String, pub text_heading: String, pub text: String, pub note: String }

fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for w in s.split_whitespace() {
        let last = lines.last_mut().unwrap();
        if !last.is_empty() && last.chars().count() + 1 + w.chars().count() > width { lines.push(w.to_string()); } else { if !last.is_empty() { last.push(' '); } last.push_str(w); }
    }
    lines
}

/// YYYY-MM-DD for a Unix time, UTC (civil-from-days; no date crate for one line on a sheet).
pub fn ymd(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097); let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    format!("{:04}-{:02}-{:02}", yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

const W: f32 = 595.0; const H: f32 = 842.0; const M: f32 = 48.0;

fn winansi(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for c in s.chars() {
        let b = match c { '(' | ')' | '\\' => { out.push(b'\\'); c as u8 } ' '..='~' => c as u8, '\u{b7}' => 0xB7, '\u{2026}' => 0x85, '\u{2014}' => 0x97, '\u{2013}' => 0x96, '\u{203a}' => 0x9B, '\u{2019}' => 0x92,
            c if (0xA0..=0xFF).contains(&(c as u32)) => c as u32 as u8, _ => b'?' };
        out.push(b);
    }
    out
}
struct Page(Vec<u8>);
impl Page {
    fn text(&mut self, font: u8, size: f32, x: f32, y: f32, s: &str) {
        self.0.extend(format!("BT /F{font} {size} Tf {x:.1} {y:.1} Td (").bytes()); self.0.extend(winansi(s)); self.0.extend(b") Tj ET\n");
    }
    fn rule(&mut self, y: f32) { self.0.extend(format!("0.75 G 0.5 w {M} {y:.1} m {:.1} {y:.1} l S 0 G\n", W - M).bytes()); }
    /// The code with its quiet zone inside a `size`-point square whose top-left corner is (x, top).
    fn qr(&mut self, x: f32, top: f32, size: f32, data: &str, sturdy: bool) -> Result<(), String> {
        let code = qrcode::QrCode::with_error_correction_level(data, if sturdy { qrcode::EcLevel::M } else { qrcode::EcLevel::L }).map_err(|e| format!("QR: {e}"))?;
        let (n, cells) = (code.width(), code.to_colors());
        let m = size / (n as f32 + 8.0);
        for r in 0..n {
            let mut c = 0;
            while c < n {
                if cells[r * n + c] == qrcode::Color::Dark {
                    let start = c; while c < n && cells[r * n + c] == qrcode::Color::Dark { c += 1; }
                    self.0.extend(format!("{:.2} {:.2} {:.2} {:.2} re\n", x + (start as f32 + 4.0) * m, top - (r as f32 + 5.0) * m, (c - start) as f32 * m + 0.05, m + 0.05).bytes());
                } else { c += 1; }
            }
        }
        self.0.extend(b"f\n");
        Ok(())
    }
}

pub fn pdf(s: &Sheet) -> Result<Vec<u8>, String> {
    let mut pages = vec![Page(Vec::new())];
    let mut y = H - M - 8.0;
    macro_rules! page { () => { pages.last_mut().unwrap() } }
    page!().text(2, 20.0, M, y, "Any Two Keys"); y -= 22.0;
    page!().text(1, 12.0, M, y, &s.title); y -= 14.0;
    page!().rule(y); y -= 20.0;
    for (k, v) in &s.facts { page!().text(2, 9.0, M, y, &k.to_uppercase()); page!().text(1, 11.0, M + 110.0, y, v); y -= 17.0; }
    y -= 4.0;
    for (label, mono, plain) in &s.rows { page!().text(2, 9.0, M, y, &label.to_uppercase()); page!().text(3, 8.5, M + 110.0, y, mono); page!().text(1, 10.0, M + 110.0, y - 12.0, plain); y -= 29.0; }
    page!().rule(y); y -= 16.0;
    // the codes: one large, or two to a row
    let n = s.codes.len();
    let (size, cols) = if n == 1 { (300.0, 1) } else { (240.0, 2) };
    for (i, part) in s.codes.iter().enumerate() {
        let col = i % cols;
        if col == 0 && y - size - 16.0 < M { pages.push(Page(Vec::new())); y = H - M; }
        let x = if cols == 1 { (W - size) / 2.0 } else { M + col as f32 * (size + (W - 2.0 * M - 2.0 * size)) };
        page!().qr(x, y, size, part, s.sturdy)?;
        if n > 1 { page!().text(2, 9.0, x + 16.0, y - size - 2.0, &format!("PART {} OF {}", i + 1, n)); }
        if col == cols - 1 || i == n - 1 { y -= size + 18.0; }
    }
    if !s.code_hint.is_empty() { if y < M + 30.0 { pages.push(Page(Vec::new())); y = H - M; } for l in wrap(&s.code_hint, 104) { page!().text(1, 9.5, M, y, &l); y -= 13.0; } y -= 6.0; }
    // the same PSBT as text
    let lines: Vec<&str> = s.text.as_bytes().chunks(112).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    if y - 30.0 - lines.len() as f32 * 9.0 < M { pages.push(Page(Vec::new())); y = H - M; }
    page!().text(2, 9.0, M, y, &s.text_heading.to_uppercase()); y -= 14.0;
    for l in lines { if y < M + 24.0 { pages.push(Page(Vec::new())); y = H - M; } page!().text(3, 7.0, M, y, l); y -= 9.0; }
    y -= 10.0;
    for l in wrap(&s.note, 104) { if y < M + 12.0 { pages.push(Page(Vec::new())); y = H - M; } page!().text(1, 9.5, M, y, &l); y -= 13.0; }

    // assemble: 1 catalog, 2 pages, 3-5 fonts, then a page object and a content stream per page
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| { offsets.push(out.len()); let id = offsets.len(); out.extend(format!("{id} 0 obj\n").bytes()); out.extend(body); out.extend(b"\nendobj\n"); };
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 6 + i * 2)).collect();
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, format!("<< /Type /Pages /Count {} /Kids [{}] >>", pages.len(), kids.join(" ")).as_bytes());
    for f in ["Helvetica", "Helvetica-Bold", "Courier"] { obj(&mut out, format!("<< /Type /Font /Subtype /Type1 /BaseFont /{f} /Encoding /WinAnsiEncoding >>").as_bytes()); }
    for (i, p) in pages.iter().enumerate() {
        obj(&mut out, format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {W} {H}] /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R >> >> /Contents {} 0 R >>", 7 + i * 2).as_bytes());
        let mut body = format!("<< /Length {} >>\nstream\n", p.0.len()).into_bytes(); body.extend(&p.0); body.extend(b"endstream");
        obj(&mut out, &body);
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).bytes());
    for o in &offsets { out.extend(format!("{o:010} 00000 n \n").bytes()); }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).bytes());
    Ok(out)
}

// ------------------------------------------------------------ documents --
// A printable record of one transaction: facts, where and on what (with the map), the note, and the pictures.

#[derive(Debug)]
pub enum Block { Heading(String), Facts(Vec<(String, String)>), Para(String), Mono(String), Jpeg { data: Vec<u8>, w: u32, h: u32, caption: String, wide: bool }, Rule }

/// Any picture the `image` crate reads → a JPEG no wider than `max_w` pixels, with its size. Metadata does not survive.
pub fn jpeg_for_print(bytes: &[u8], max_w: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("picture: {e}"))?;
    let img = if img.width() > max_w { img.resize(max_w, u32::MAX, image::imageops::FilterType::Triangle) } else { img };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 86).encode_image(&img.to_rgb8()).map_err(|e| e.to_string())?;
    Ok((out, img.width(), img.height()))
}

pub fn doc(title: &str, subtitle: &str, blocks: &[Block]) -> Result<Vec<u8>, String> {
    let mut pages = vec![Page(Vec::new())];
    let mut images: Vec<(Vec<u8>, u32, u32)> = Vec::new();
    let mut used: Vec<Vec<usize>> = vec![Vec::new()];   // which pictures each page actually draws
    let mut y = H - M - 8.0;
    macro_rules! page { () => { pages.last_mut().unwrap() } }
    macro_rules! need { ($h:expr) => { if y - $h < M { pages.push(Page(Vec::new())); used.push(Vec::new()); y = H - M; } } }
    page!().text(2, 20.0, M, y, "Any Two Keys"); y -= 22.0;
    for l in wrap(title, 60) { page!().text(2, 13.0, M, y, &l); y -= 16.0; }
    if !subtitle.is_empty() { page!().text(1, 10.0, M, y, subtitle); y -= 14.0; }
    page!().rule(y); y -= 18.0;
    for b in blocks {
        match b {
            Block::Rule => { need!(16.0); y -= 4.0; page!().rule(y); y -= 16.0; }
            Block::Heading(h) => { need!(30.0); y -= 4.0; page!().text(2, 9.0, M, y, &h.to_uppercase()); y -= 15.0; }
            Block::Facts(kv) => for (k, v) in kv {
                let lines = wrap(v, 78); need!(14.0 * lines.len() as f32 + 3.0);
                page!().text(2, 8.5, M, y, &k.to_uppercase());
                for (i, l) in lines.iter().enumerate() { page!().text(1, 10.5, M + 110.0, y, l); if i + 1 < lines.len() { y -= 13.0; } }
                y -= 16.0;
            },
            Block::Para(t) => { for para in t.split('\n') { let lines = wrap(para, 96); if lines.len() == 1 && lines[0].is_empty() { y -= 6.0; continue; } for l in lines { need!(13.0); page!().text(1, 10.0, M, y, &l); y -= 13.0; } } y -= 4.0; }
            Block::Mono(t) => { for l in t.as_bytes().chunks(104).map(|c| std::str::from_utf8(c).unwrap_or("")) { need!(10.0); page!().text(3, 7.5, M, y, l); y -= 10.0; } y -= 6.0; }
            Block::Jpeg { data, w, h, caption, wide } => {
                let maxw = if *wide { W - 2.0 * M } else { 260.0 };
                let (pw, ph) = { let sc = (maxw / *w as f32).min(320.0 / *h as f32).min(1.0); (*w as f32 * sc, *h as f32 * sc) };
                need!(ph + 22.0);
                images.push((data.clone(), *w, *h));
                let n = images.len();
                used.last_mut().unwrap().push(n);
                page!().0.extend(format!("q {pw:.1} 0 0 {ph:.1} {M} {:.1} cm /Im{n} Do Q\n", y - ph).bytes());
                y -= ph + 3.0;
                if !caption.is_empty() { page!().text(1, 8.5, M, y - 6.0, caption); y -= 12.0; }
                y -= 8.0;
            }
        }
    }
    // assemble: catalog, pages, three fonts, the images, then a page object and a content stream per page
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| { offsets.push(out.len()); let id = offsets.len(); out.extend(format!("{id} 0 obj\n").bytes()); out.extend(body); out.extend(b"\nendobj\n"); };
    let first_page = 6 + images.len();
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", first_page + i * 2)).collect();
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, format!("<< /Type /Pages /Count {} /Kids [{}] >>", pages.len(), kids.join(" ")).as_bytes());
    for f in ["Helvetica", "Helvetica-Bold", "Courier"] { obj(&mut out, format!("<< /Type /Font /Subtype /Type1 /BaseFont /{f} /Encoding /WinAnsiEncoding >>").as_bytes()); }
    for (data, w, h) in &images {
        let mut body = format!("<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend(data); body.extend(b"\nendstream"); obj(&mut out, &body);
    }
    for (i, p) in pages.iter().enumerate() {
        // a page names only the pictures it draws, so a long document does not make every page carry all of them
        let xobjects: Vec<String> = used[i].iter().map(|n| format!("/Im{n} {} 0 R", 5 + n)).collect();
        let res = if xobjects.is_empty() { String::new() } else { format!(" /XObject << {} >>", xobjects.join(" ")) };
        obj(&mut out, format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {W} {H}] /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R >>{res} >> /Contents {} 0 R >>", first_page + 1 + i * 2).as_bytes());
        let mut body = format!("<< /Length {} >>\nstream\n", p.0.len()).into_bytes(); body.extend(&p.0); body.extend(b"endstream");
        obj(&mut out, &body);
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).bytes());
    for o in &offsets { out.extend(format!("{o:010} 00000 n \n").bytes()); }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    // a real 2-of-3 spend from the signet vault with ONE YubiKey signature on it (mornay's 2026-09-17 proof); public data
    fn sample() -> Vec<u8> { include_bytes!("../../test/fixtures/sample.psbt").to_vec() }

    /// A document with everything in it, written to $TMPDIR/a2k-pdf for `test/pdf-check.py` to open with a library
    /// that is not ours. Always runs: a PDF this app cannot write is a PDF nobody can read.
    #[test]
    fn a_document_carries_its_pictures_and_points_at_its_own_objects() {
        let mut px = image::RgbImage::new(400, 300);
        for (x, y, p) in px.enumerate_pixels_mut() { *p = image::Rgb([(x / 2) as u8, (y / 2) as u8, 140]); }
        let mut png = Vec::new(); image::DynamicImage::ImageRgb8(px).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let (jpeg, w, h) = jpeg_for_print(&png, 1400).unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]) && (w, h) == (400, 300), "a picture for print is a JPEG at its own size");
        let blocks = vec![
            Block::Facts(vec![("Amount".into(), "+9,171 sats".into()), ("Worth then".into(), "A$10.07 AUD  (hourly 2026-09-18 08:00 UTC)".into())]),
            Block::Heading("Your note".into()), Block::Para("Ordered 17 Sep, invoice 4471.\nAsk Sam about the warranty.".into()),
            Block::Jpeg { data: jpeg.clone(), w, h, caption: "IMG_0412.jpg".into(), wide: true },
            Block::Rule, Block::Heading("The transaction itself".into()), Block::Mono("0200000000010".repeat(90)),
            Block::Jpeg { data: jpeg, w, h, caption: "receipt.jpg".into(), wide: false },
        ];
        let out = doc("Lets check if labelling still works", "mineracks vault · printed 2026-09-20", &blocks).unwrap();
        assert!(out.starts_with(b"%PDF-1.4"));
        assert_eq!(out.windows(9).filter(|w| *w == b"DCTDecode").count(), 2, "both pictures are embedded");
        // every offset in the table must land on that object's own header: the classic way a hand-written PDF breaks
        let whole = String::from_utf8_lossy(&out).to_string();
        let xref = &whole[whole.rfind("xref\n0 ").expect("an xref table")..];
        let offsets: Vec<usize> = xref.lines().skip(2).skip(1).take_while(|l| l.ends_with(" 00000 n ")).map(|l| l[..10].parse().unwrap()).collect();
        assert!(offsets.len() > 8, "{} objects", offsets.len());
        for (i, o) in offsets.iter().enumerate() { assert!(out[*o..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "offset {i} does not point at object {}", i + 1); }
        let dir = std::env::temp_dir().join("a2k-pdf"); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("doc.pdf"), &out).unwrap();
        println!("{}", dir.join("doc.pdf").display());
    }

    /// The real thing: `A2K_STORE_KEY=… cargo test live_tx_pdf -- --ignored --nocapture` — every transaction of every
    /// vault, written to $TMPDIR/a2k-pdf.
    #[test]
    #[ignore]
    fn live_tx_pdf() {
        let s = crate::wallet::load().unwrap();
        let dir = std::env::temp_dir().join("a2k-pdf"); std::fs::create_dir_all(&dir).unwrap();
        for v in &s.vaults {
            let servers = crate::wallet::servers_for(&s, &v.network);
            let sc = crate::wallet::scan(v, &servers, s.settings.gap).unwrap();
            let mut hist: Vec<(String, i32)> = sc.history.iter().map(|(t, h, _)| (t.clone(), *h)).collect();
            hist.sort(); hist.dedup();
            for (txid, h) in hist {
                let bytes = crate::tx_sheet(&v.name, &txid, h, "AUD").unwrap();
                let p = dir.join(format!("{}-{}.pdf", v.name.replace(' ', "-"), &txid[..8]));
                std::fs::write(&p, &bytes).unwrap();
                println!("{} ({} KB)", p.display(), bytes.len() / 1024);
            }
        }
    }

    #[test]
    fn a_psbt_survives_every_way_out_and_back_in() {
        let raw = sample();
        let psbt = Psbt::deserialize(&raw).unwrap();
        assert_eq!(parse(&raw).unwrap(), psbt);
        assert_eq!(parse(psbt.to_string().as_bytes()).unwrap(), psbt);
        assert_eq!(parse(hex::encode(&raw).as_bytes()).unwrap(), psbt);
        let one = ur_parts(&raw, 10_000, false).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(parse(one[0].as_bytes()).unwrap(), psbt);
        let many = ur_parts(&raw, 100, true).unwrap();
        assert!(many.len() >= 4 && many.len() % 2 == 0);
        assert_eq!(parse(many[..many.len() / 2].join("\n").as_bytes()).unwrap(), psbt);
        // a camera that joins late sees only mixtures plus whatever plain parts come round again
        let late: Vec<String> = many[many.len() / 2..].iter().chain(many[1..many.len() / 2].iter()).cloned().collect();
        assert_eq!(parse(late.join("\n").as_bytes()).unwrap(), psbt);
        assert!(parse(many[..1].join("\n").as_bytes()).is_err());
        assert!(parse(b"ur:crypto-seed/oeadgdaxbt").is_err());
    }
    /// Coinkite's own vectors: a real scan off a Coldcard (8 parts, compressed, out of order and repeated) and a PSBT.
    #[test]
    fn bbqr_reads_a_real_coldcard_scan_and_round_trips_a_psbt() {
        let scan = std::fs::read_to_string(format!("{}/tests/fixtures/bbqr-real-scan.txt", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let (kind, bytes) = bbqr_decode(&scan).unwrap();
        assert_eq!(kind, 'U');
        let text = String::from_utf8(bytes).expect("the scan decompresses to text");
        use bitcoin::hashes::{sha256, Hash};   // the digest python's zlib (raw deflate, wbits=10) gives for the same scan
        assert_eq!(sha256::Hash::hash(text.as_bytes()).to_string(), include_str!("../tests/fixtures/bbqr-real-scan.sha256").trim());
        // a missing part is said, not guessed
        let short: Vec<&str> = scan.split_whitespace().filter(|p| !p.starts_with("B$ZU0803")).collect();
        assert!(bbqr_decode(&short.join("\n")).unwrap_err().contains("missing"));
        let raw = std::fs::read(format!("{}/tests/fixtures/bbqr-1in2out.psbt", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let parts = bbqr_parts(&raw, 'P', 400);
        assert!(parts.len() > 1 && parts.iter().all(|p| p.starts_with("B$2P")));
        let back: Vec<String> = parts.iter().rev().cloned().collect();
        assert_eq!(parse(back.join("\n").as_bytes()).unwrap().serialize(), raw, "any order");
    }
    pub(super) fn vault(cc: Option<&str>) -> crate::wallet::Vault {
        let keys = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe", "038410f240613b33fe73d968f331ca82eb1fbd018c6189f89f8c5f9b9fb26f6a47"];
        let descriptor = crate::wallet::descriptor_for(2, &keys.iter().map(|k| k.to_string()).collect::<Vec<_>>()).unwrap();
        crate::wallet::Vault { name: "f\u{e4}mily vault".into(), network: "signet".into(), threshold: 2, keys: keys.iter().zip(["laptop", "keyring", "off-site"]).enumerate().map(|(i, (k, n))| crate::wallet::KeyRef { name: n.to_string(), serial: 27446043 + i as u32, pubkey: k.to_string(), device: None }).collect(),
            descriptor, address: String::new(), created: 1_789_700_000, chain_code: cc.map(|c| c.to_string()), next: 0, scan_max: 0, next_change: 0, scan_max_change: 0, labels: Default::default(), label_meta: Default::default(), notes: Default::default(), files: Default::default(), more: Default::default(), labels_synced: 0, seal: Default::default(), ext: None, cost_method: None }
    }
    #[test]
    fn the_recovery_sheet_carries_the_line_the_restore_screen_reads() {
        assert_eq!(ymd(0), "1970-01-01"); assert_eq!(ymd(951_782_400), "2000-02-29"); assert_eq!(ymd(1_789_700_000), "2026-09-18");
        for cc in [None, Some("ab".repeat(32))] {
            let v = vault(cc.as_deref());
            let sheet = recovery_sheet(&v).unwrap();
            assert_eq!(sheet.codes, vec![crate::wallet::recovery_line(&v)]);
            // what the code says is what descriptor_parse / vault_restore take
            let (desc, got_cc) = crate::wallet::split_recovery(&sheet.codes[0]);
            assert_eq!(crate::wallet::keys_of(&desc).unwrap(), (2, v.keys.iter().map(|k| k.pubkey.clone()).collect::<Vec<_>>()));
            assert_eq!(got_cc, v.chain_code);
            let pdf = pdf(&sheet).unwrap();
            let text = String::from_utf8_lossy(&pdf);
            for k in &v.keys { assert!(text.contains(&k.pubkey) && text.contains(&k.name)); }
            assert!(text.contains("2026-09-18") && text.contains("cannot spend"));
        }
    }
    #[test]
    fn the_pdf_is_a_pdf_with_every_part_on_it() {
        let raw = sample();
        let parts = ur_parts(&raw, 120, false).unwrap();
        let sheet = Sheet { title: "Partially signed transaction".into(), facts: vec![("Vault".into(), "f\u{e4}mily (vault) \u{20bf}".into())], rows: vec![("Pays".into(), "tb1qexample".into(), "5,000 sats".into())], sturdy: false, code_hint: "Scan the parts in any order.".into(), text_heading: "The same PSBT as text".into(),
            note: "Public data.".into(), text: Psbt::deserialize(&raw).unwrap().to_string(), codes: parts.clone() };
        let pdf = pdf(&sheet).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&pdf);
        assert_eq!(text.matches("PART ").count(), parts.len());
        assert!(text.contains("f\u{fffd}mily \\(vault\\) ?") || text.contains("mily \\(vault\\) ?"));
        // the xref offsets point at the objects they claim to
        let xref_at: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(pdf[xref_at..].starts_with(b"xref"));
    }
}

#[cfg(test)]
mod dump {
    /// `A2K_DUMP=/some/dir cargo test dump_for_inspection -- --ignored`: writes a PDF and the screen parts for outside tools to read.
    #[test]
    #[ignore]
    fn dump_for_inspection() {
        let dir = std::path::PathBuf::from(std::env::var("A2K_DUMP").expect("set A2K_DUMP"));
        let raw = include_bytes!("../../test/fixtures/sample.psbt");
        let b64 = bitcoin::psbt::Psbt::deserialize(raw).unwrap().to_string();
        std::fs::write(dir.join("parts.txt"), super::ur_parts(raw, 160, true).unwrap().join("\n")).unwrap();
        std::fs::write(dir.join("single.txt"), super::ur_parts(raw, usize::MAX, false).unwrap().join("\n")).unwrap();
        for (name, cc) in [("recovery-fixed", None), ("recovery-rotating", Some("ab".repeat(32)))] {
            let v = super::tests::vault(cc.as_deref());
            std::fs::write(dir.join(format!("{name}.pdf")), super::pdf(&super::recovery_sheet(&v).unwrap()).unwrap()).unwrap();
            std::fs::write(dir.join(format!("{name}.txt")), crate::wallet::recovery_line(&v)).unwrap();
        }
        for (name, frag) in [("multi", 120usize), ("single", usize::MAX)] {
            let sheet = super::Sheet { title: "Partially signed transaction (PSBT, BIP 174)".into(),
                facts: vec![("Vault".into(), "vault \u{b7} 2 of 3 \u{b7} signet".into()), ("Signatures".into(), "1 of 2 on every input".into()), ("Signed by".into(), "laptop".into()), ("Not yet".into(), "keyring, off-site".into()),
                    ("Transaction".into(), "bcd3fb3da2e6abd3381b2dd5cde0b3cc0357c0eb999bcba5027eec2786a391e1".into()), ("Inputs \u{b7} fee".into(), "1 \u{b7} 190 sats".into())],
                rows: vec![("Pays".into(), "tb1q2n7mza35dzyea6du9dh4amzqxjg6ysgtc45kfklfd7revryppuxqfegkkp".into(), "4,810 sats \u{b7} back to this vault (change)".into())], sturdy: false, code_hint: String::new(), text_heading: "The same PSBT as text (base64, BIP 174)".into(),
                note: "Needs 1 more signature. Load it in Any Two Keys (Send \u{203a} Load a PSBT) or any PSBT wallet. It cannot spend as it is.".into(),
                codes: super::ur_parts(raw, frag, false).unwrap(), text: b64.clone() };
            std::fs::write(dir.join(format!("{name}.pdf")), super::pdf(&sheet).unwrap()).unwrap();
        }
    }
}
