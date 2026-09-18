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

/// A PSBT from whatever the person has: the binary file, base64, hex, or UR text (one part, or every part on its own line).
pub fn parse(input: &[u8]) -> Result<Psbt, String> {
    if input.starts_with(b"psbt\xff") { return Psbt::deserialize(input).map_err(|e| format!("not a valid PSBT: {e}")); }
    let text = std::str::from_utf8(input).map_err(|_| "that file is neither a PSBT nor text".to_string())?.trim();
    if text.is_empty() { return Err("nothing to read".into()); }
    if text.len() >= 3 && text[..3].eq_ignore_ascii_case("ur:") {
        let parts: Vec<String> = text.split_whitespace().map(|p| p.to_lowercase()).collect();
        if parts.iter().any(|p| !p.starts_with("ur:crypto-psbt/")) { return Err("that UR is not a PSBT (crypto-psbt)".into()); }
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

// ---------- a small PDF writer: three built-in fonts, text and filled squares ----------
pub struct Sheet { pub title: String, pub facts: Vec<(String, String)>, pub pays: Vec<(String, String)>, pub note: String, pub parts: Vec<String>, pub base64: String }

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
    fn qr(&mut self, x: f32, top: f32, size: f32, data: &str) -> Result<(), String> {
        let code = qrcode::QrCode::with_error_correction_level(data, qrcode::EcLevel::L).map_err(|e| format!("QR: {e}"))?;
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
    for (i, (addr, amt)) in s.pays.iter().enumerate() { page!().text(2, 9.0, M, y, if i == 0 { "PAYS" } else { "" }); page!().text(3, 9.0, M + 110.0, y, addr); page!().text(1, 10.0, M + 110.0, y - 12.0, amt); y -= 29.0; }
    page!().rule(y); y -= 16.0;
    // the codes: one large, or two to a row
    let n = s.parts.len();
    let (size, cols) = if n == 1 { (300.0, 1) } else { (240.0, 2) };
    for (i, part) in s.parts.iter().enumerate() {
        let col = i % cols;
        if col == 0 && y - size - 16.0 < M { pages.push(Page(Vec::new())); y = H - M; }
        let x = if cols == 1 { (W - size) / 2.0 } else { M + col as f32 * (size + (W - 2.0 * M - 2.0 * size)) };
        page!().qr(x, y, size, part)?;
        if n > 1 { page!().text(2, 9.0, x + 16.0, y - size - 2.0, &format!("PART {} OF {}", i + 1, n)); }
        if col == cols - 1 || i == n - 1 { y -= size + 18.0; }
    }
    if n > 1 { if y < M + 30.0 { pages.push(Page(Vec::new())); y = H - M; } page!().text(1, 9.5, M, y, "Scan the parts in any order with a wallet that reads UR (crypto-psbt) codes."); y -= 18.0; }
    // the same PSBT as text
    let lines: Vec<&str> = s.base64.as_bytes().chunks(112).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    if y - 30.0 - lines.len() as f32 * 9.0 < M { pages.push(Page(Vec::new())); y = H - M; }
    page!().text(2, 9.0, M, y, "THE SAME PSBT AS TEXT (BASE64, BIP 174)"); y -= 14.0;
    for l in lines { if y < M + 24.0 { pages.push(Page(Vec::new())); y = H - M; } page!().text(3, 7.0, M, y, l); y -= 9.0; }
    y -= 10.0;
    if y < M + 24.0 { pages.push(Page(Vec::new())); y = H - M; }
    page!().text(1, 9.5, M, y, &s.note);

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

#[cfg(test)]
mod tests {
    use super::*;
    // a real 2-of-3 spend from the signet vault with ONE YubiKey signature on it (mornay's 2026-09-17 proof); public data
    fn sample() -> Vec<u8> { include_bytes!("../../test/fixtures/sample.psbt").to_vec() }

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
    #[test]
    fn the_pdf_is_a_pdf_with_every_part_on_it() {
        let raw = sample();
        let parts = ur_parts(&raw, 120, false).unwrap();
        let sheet = Sheet { title: "Partially signed transaction".into(), facts: vec![("Vault".into(), "f\u{e4}mily (vault) \u{20bf}".into())], pays: vec![("tb1qexample".into(), "5,000 sats".into())],
            note: "Public data.".into(), base64: Psbt::deserialize(&raw).unwrap().to_string(), parts: parts.clone() };
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
        for (name, frag) in [("multi", 120usize), ("single", usize::MAX)] {
            let sheet = super::Sheet { title: "Partially signed transaction (PSBT, BIP 174)".into(),
                facts: vec![("Vault".into(), "vault \u{b7} 2 of 3 \u{b7} signet".into()), ("Signatures".into(), "1 of 2 on every input".into()), ("Signed by".into(), "laptop".into()), ("Not yet".into(), "keyring, off-site".into()),
                    ("Transaction".into(), "bcd3fb3da2e6abd3381b2dd5cde0b3cc0357c0eb999bcba5027eec2786a391e1".into()), ("Inputs \u{b7} fee".into(), "1 \u{b7} 190 sats".into())],
                pays: vec![("tb1q2n7mza35dzyea6du9dh4amzqxjg6ysgtc45kfklfd7revryppuxqfegkkp".into(), "4,810 sats \u{b7} back to this vault (change)".into())],
                note: "Needs 1 more signature. Load it in Any Two Keys (Send \u{203a} Load a PSBT) or any PSBT wallet. It cannot spend as it is.".into(),
                parts: super::ur_parts(raw, frag, false).unwrap(), base64: b64.clone() };
            std::fs::write(dir.join(format!("{name}.pdf")), super::pdf(&sheet).unwrap()).unwrap();
        }
    }
}
