//! The little map on a transaction. Tiles come from ONE file on a server of ours (a Protomaps PMTiles archive, read by
//! HTTP range requests — https://tiles.mineracks.com), are fetched from here and never from the window (no cookies, no
//! referrer, no browser to fingerprint), at ONE coarse zoom so what the host can learn is a district, not a street,
//! and each tile is kept sealed so a place is asked for once. The picture is drawn here too: the window gets a PNG.
//!
//! PMTiles v3: a 127-byte header, a root directory within the first 16 KiB, optional leaf directories, tile data.
//! A tile is a gzip'd Mapbox Vector Tile (protobuf). Map data © OpenStreetMap contributors (ODbL); tiles by Protomaps.

use std::io::Read;

pub const ZOOM: u8 = 12;
const SPAN_M: f64 = 2600.0;          // how much ground the picture shows, left to right
pub const W: u32 = 640;
pub const H: u32 = 400;

// ------------------------------------------------------------- pmtiles --

fn varint(b: &[u8], i: &mut usize) -> Result<u64, String> {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let byte = *b.get(*i).ok_or("the tile index ended early")?; *i += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 { return Ok(v); }
        shift += 7; if shift > 63 { return Err("a number in the tile index is too long".into()); }
    }
}
fn gunzip(b: &[u8]) -> Result<Vec<u8>, String> { let mut out = Vec::new(); flate2::read::GzDecoder::new(b).read_to_end(&mut out).map_err(|e| format!("could not unpack: {e}"))?; Ok(out) }

#[derive(Debug, Clone, Copy)]
pub struct Header { root: (u64, u64), leaves: u64, data: u64, internal_gzip: bool, tile_gzip: bool, pub min_zoom: u8, pub max_zoom: u8 }

pub fn header(b: &[u8]) -> Result<Header, String> {
    if b.len() < 127 || &b[..7] != b"PMTiles" { return Err("the tile host did not answer with a PMTiles archive".into()); }
    if b[7] != 3 { return Err(format!("PMTiles version {} is not one this app reads", b[7])); }
    let u = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let comp = |c: u8| match c { 1 => Ok(false), 2 => Ok(true), other => Err(format!("compression {other} is not one this app reads")) };
    if b[99] != 1 { return Err("the archive does not hold vector tiles".into()); }
    Ok(Header { root: (u(8), u(16)), leaves: u(40), data: u(56), internal_gzip: comp(b[97])?, tile_gzip: comp(b[98])?, min_zoom: b[100], max_zoom: b[101] })
}

#[derive(Debug, Clone, Copy)]
struct Entry { id: u64, run: u64, off: u64, len: u64 }

fn directory(raw: &[u8], gz: bool) -> Result<Vec<Entry>, String> {
    let b = if gz { gunzip(raw)? } else { raw.to_vec() };
    let mut i = 0;
    let n = varint(&b, &mut i)? as usize;
    if n > 5_000_000 { return Err("the tile index is implausibly large".into()); }
    let mut e = vec![Entry { id: 0, run: 0, off: 0, len: 0 }; n];
    let mut last = 0u64;
    for x in e.iter_mut() { last += varint(&b, &mut i)?; x.id = last; }
    for x in e.iter_mut() { x.run = varint(&b, &mut i)?; }
    for x in e.iter_mut() { x.len = varint(&b, &mut i)?; }
    for k in 0..n { let v = varint(&b, &mut i)?; e[k].off = if v == 0 && k > 0 { e[k - 1].off + e[k - 1].len } else { v.checked_sub(1).ok_or("a tile offset of zero where none can be")? }; }
    Ok(e)
}
fn find(e: &[Entry], id: u64) -> Option<Entry> {
    let k = e.partition_point(|x| x.id <= id); if k == 0 { return None; }
    let x = e[k - 1];
    (x.run == 0 || id < x.id + x.run).then_some(x)
}

/// z/x/y → the archive's tile number: every tile of the lower zooms, then the position along a Hilbert curve.
pub fn tile_id(z: u8, x: u32, y: u32) -> u64 {
    let base: u64 = (0..z).map(|i| 1u64 << (2 * i as u32)).sum();
    // the reference implementation's arithmetic, signs and all: the reflected coordinates go negative and only their low bits are read
    let (mut x, mut y, mut d) = (x as i64, y as i64, 0u64);
    let mut a = (1i64 << z) / 2;
    while a > 0 {
        let (rx, ry) = (((x & a) > 0) as u64, ((y & a) > 0) as u64);
        d += (a * a) as u64 * ((3 * rx) ^ ry);
        if ry == 0 { if rx == 1 { x = a - 1 - x; y = a - 1 - y; } std::mem::swap(&mut x, &mut y); }
        a /= 2;
    }
    base + d
}

/// Something that can return bytes `[from, from+len)` of the archive.
pub trait Source { fn range(&self, from: u64, len: u64) -> Result<Vec<u8>, String>; }

pub struct Http { pub url: String }
impl Source for Http {
    fn range(&self, from: u64, len: u64) -> Result<Vec<u8>, String> {
        crate::net::out("the map")?;
        let c = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(20)).user_agent("AnyTwoKeys").build().map_err(|e| e.to_string())?;
        let r = c.get(&self.url).header("Range", format!("bytes={}-{}", from, from + len - 1)).send().map_err(|_| "the map host could not be reached".to_string())?;
        if r.status().as_u16() != 206 { return Err(format!("the map host answered {}", r.status().as_u16())); }
        let b = r.bytes().map_err(|e| e.to_string())?;
        if b.len() as u64 != len { return Err("the map host sent a different amount than was asked for".into()); }
        Ok(b.to_vec())
    }
}

/// One tile's vector data, unpacked. `Ok(None)`: the archive has no such tile (open sea at a high zoom).
pub fn tile(src: &dyn Source, head: &[u8], z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, String> {
    let h = header(head)?;
    let id = tile_id(z, x, y);
    let (mut off, mut len) = h.root;
    let mut dir = if off + len <= head.len() as u64 { head[off as usize..(off + len) as usize].to_vec() } else { src.range(off, len)? };
    for _ in 0..4 {
        match find(&directory(&dir, h.internal_gzip)?, id) {
            None => return Ok(None),
            Some(e) if e.run > 0 => { let raw = src.range(h.data + e.off, e.len)?; return Ok(Some(if h.tile_gzip { gunzip(&raw)? } else { raw })); }
            Some(e) => { off = h.leaves + e.off; len = e.len; dir = src.range(off, len)?; }
        }
    }
    Err("the tile index goes deeper than it should".into())
}

// ----------------------------------------------------------------- mvt --

struct Pb<'a> { b: &'a [u8], i: usize }
impl<'a> Pb<'a> {
    fn next(&mut self) -> Option<Result<(u64, u8, u64, &'a [u8]), String>> {
        if self.i >= self.b.len() { return None; }
        Some((|| { let key = varint(self.b, &mut self.i)?; let (f, w) = (key >> 3, (key & 7) as u8);
            match w { 0 => Ok((f, w, varint(self.b, &mut self.i)?, &[][..])),
                2 => { let n = varint(self.b, &mut self.i)? as usize; let s = self.b.get(self.i..self.i + n).ok_or("a map tile ended early")?; self.i += n; Ok((f, w, 0, s)) }
                1 => { self.i += 8; Ok((f, w, 0, &[][..])) } 5 => { self.i += 4; Ok((f, w, 0, &[][..])) }
                _ => Err("a map tile has a field this app cannot read".into()) } })())
    }
}

pub struct Feature { pub kind: String, pub geom_type: u8, pub rings: Vec<Vec<(f32, f32)>> }
pub struct Layer { pub name: String, pub extent: f32, pub features: Vec<Feature> }

pub fn layers(tile: &[u8]) -> Result<Vec<Layer>, String> {
    let mut out = Vec::new();
    let mut t = Pb { b: tile, i: 0 };
    while let Some(f) = t.next() { let (field, _, _, body) = f?; if field != 3 { continue; }
        let (mut name, mut extent, mut keys, mut vals, mut feats) = (String::new(), 4096f32, Vec::new(), Vec::new(), Vec::new());
        let mut l = Pb { b: body, i: 0 };
        while let Some(f) = l.next() { let (field, _, v, s) = f?; match field {
            1 => name = String::from_utf8_lossy(s).into_owned(), 2 => feats.push(s), 3 => keys.push(String::from_utf8_lossy(s).into_owned()), 5 => extent = v as f32,
            4 => { let mut val = Pb { b: s, i: 0 }; let mut text = String::new(); while let Some(f) = val.next() { let (vf, _, _, vs) = f?; if vf == 1 { text = String::from_utf8_lossy(vs).into_owned(); } } vals.push(text); }
            _ => {} } }
        let mut features = Vec::new();
        for fb in feats {
            let (mut tags, mut gtype, mut geom) = (Vec::new(), 0u8, Vec::new());
            let mut p = Pb { b: fb, i: 0 };
            while let Some(f) = p.next() { let (field, w, v, s) = f?; match (field, w) {
                (2, 2) => { let mut i = 0; while i < s.len() { tags.push(varint(s, &mut i)? as usize); } }
                (3, 0) => gtype = v as u8,
                (4, 2) => { let mut i = 0; while i < s.len() { geom.push(varint(s, &mut i)? as u32); } }
                _ => {} } }
            let kind = tags.chunks(2).find(|c| c.len() == 2 && keys.get(c[0]).map(|k| k == "kind").unwrap_or(false)).and_then(|c| vals.get(c[1]).cloned()).unwrap_or_default();
            // geometry: MoveTo(1) / LineTo(2) / ClosePath(7), counts in the high bits, zig-zag deltas
            let (mut rings, mut cur, mut x, mut y, mut i) = (Vec::new(), Vec::new(), 0i32, 0i32, 0usize);
            while i < geom.len() { let (cmd, count) = (geom[i] & 7, geom[i] >> 3); i += 1;
                match cmd { 1 | 2 => for _ in 0..count { if i + 1 >= geom.len() { break; } let dz = |v: u32| ((v >> 1) as i32) ^ -((v & 1) as i32); x += dz(geom[i]); y += dz(geom[i + 1]); i += 2;
                        if cmd == 1 && !cur.is_empty() { rings.push(std::mem::take(&mut cur)); } cur.push((x as f32, y as f32)); },
                    7 => { if let Some(&first) = cur.first() { cur.push(first); } rings.push(std::mem::take(&mut cur)); }
                    _ => break } }
            if !cur.is_empty() { rings.push(cur); }
            features.push(Feature { kind, geom_type: gtype, rings });
        }
        out.push(Layer { name, extent, features });
    }
    Ok(out)
}

// ---------------------------------------------------------------- draw --

/// Where a point falls in tile units at `z`: (tile x, tile y, fraction across, fraction down).
pub fn locate(lat: f64, lon: f64, z: u8) -> (u32, u32, f64, f64) {
    let n = (1u32 << z) as f64;
    let fx = (lon + 180.0) / 360.0 * n;
    let fy = (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * n;
    let (tx, ty) = (fx.floor().clamp(0.0, n - 1.0), fy.floor().clamp(0.0, n - 1.0));
    (tx as u32, ty as u32, fx - tx, fy - ty)
}

pub struct Palette { land: [u8; 3], water: [u8; 3], green: [u8; 3], built: [u8; 3], road: [u8; 3], major: [u8; 3], rail: [u8; 3] }
pub fn palette(dark: bool) -> Palette {
    if dark { Palette { land: [0x17, 0x20, 0x1d], water: [0x0c, 0x2a, 0x36], green: [0x1a, 0x2b, 0x22], built: [0x22, 0x2d, 0x29], road: [0x3a, 0x47, 0x43], major: [0x6b, 0x5a, 0x2c], rail: [0x2c, 0x36, 0x33] } }
    else { Palette { land: [0xf3, 0xef, 0xe6], water: [0xb9, 0xd3, 0xdc], green: [0xdc, 0xe5, 0xcf], built: [0xe6, 0xdf, 0xd2], road: [0xff, 0xff, 0xff], major: [0xee, 0xdc, 0xae], rail: [0xc9, 0xc1, 0xb4] } }
}

/// Draw the picture: `tiles` are (dx, dy, unpacked tile) for the tile under the point (0,0) and any neighbours the
/// window reaches. Returns PNG bytes. No marker is drawn: the window puts its own dot and precision ring on top.
pub fn draw(lat: f64, tiles: &[(i32, i32, Vec<u8>)], fx: f64, fy: f64, dark: bool) -> Result<Vec<u8>, String> {
    use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};
    let p = palette(dark);
    let mut pm = Pixmap::new(W, H).ok_or("no picture")?;
    let rgb = |c: [u8; 3]| tiny_skia::Color::from_rgba8(c[0], c[1], c[2], 255);
    pm.fill(rgb(p.water));   // what no land polygon covers is sea
    let tile_m = 40_075_016.686 * lat.to_radians().cos() / (1u32 << ZOOM) as f64;
    let px_per_tile = (W as f64 / SPAN_M * tile_m) as f32;
    let mut paint = Paint::default(); paint.anti_alias = true;
    let parsed: Vec<(i32, i32, Vec<Layer>)> = tiles.iter().map(|(dx, dy, b)| layers(b).map(|l| (*dx, *dy, l))).collect::<Result<_, _>>()?;
    let to_px = |dx: i32, dy: i32, extent: f32, (x, y): (f32, f32)| -> (f32, f32) {
        (W as f32 / 2.0 + ((dx as f32 + x / extent) - fx as f32) * px_per_tile, H as f32 / 2.0 + ((dy as f32 + y / extent) - fy as f32) * px_per_tile) };
    let mut pass = |layer: &str, want: &dyn Fn(&Feature) -> Option<([u8; 3], f32)>| {
        for (dx, dy, ls) in &parsed { for l in ls.iter().filter(|l| l.name == layer) { for f in &l.features {
            let Some((color, width)) = want(f) else { continue };
            let mut pb = PathBuilder::new();
            for ring in &f.rings { for (k, pt) in ring.iter().enumerate() { let (x, y) = to_px(*dx, *dy, l.extent, *pt); if k == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) } } if f.geom_type == 3 { pb.close(); } }
            let Some(path) = pb.finish() else { continue };
            paint.set_color(rgb(color));
            if f.geom_type == 3 { pm.fill_path(&path, &paint, FillRule::EvenOdd, Transform::identity(), None); }
            else if f.geom_type == 2 { pm.stroke_path(&path, &paint, &Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() }, Transform::identity(), None); }
        } } }
    };
    pass("earth", &|_| Some((p.land, 0.0)));
    pass("landcover", &|f| matches!(f.kind.as_str(), "forest" | "grassland" | "scrub" | "farmland").then_some((p.green, 0.0)));
    pass("landuse", &|f| Some((if matches!(f.kind.as_str(), "park" | "forest" | "wood" | "nature_reserve" | "grass" | "garden" | "golf_course" | "cemetery" | "pitch" | "recreation_ground" | "national_park" | "protected_area") { p.green } else { p.built }, 0.0)));
    pass("water", &|f| (f.geom_type == 3).then_some((p.water, 0.0)));
    pass("water", &|f| (f.geom_type == 2).then_some((p.water, 2.0)));
    pass("buildings", &|_| Some((p.built, 0.0)));
    pass("roads", &|f| matches!(f.kind.as_str(), "rail").then_some((p.rail, 1.5)));
    pass("roads", &|f| matches!(f.kind.as_str(), "minor_road" | "other" | "path").then_some((p.road, if f.kind == "path" { 1.0 } else { 2.2 })));
    pass("roads", &|f| matches!(f.kind.as_str(), "medium_road").then_some((p.road, 3.4)));
    pass("roads", &|f| matches!(f.kind.as_str(), "major_road").then_some((p.major, 3.8)));
    pass("roads", &|f| matches!(f.kind.as_str(), "highway").then_some((p.major, 5.5)));
    pm.encode_png().map_err(|e| e.to_string())
}

/// Which neighbours of the tile under the point the picture reaches, as (dx, dy).
pub fn needed(lat: f64, fx: f64, fy: f64) -> Vec<(i32, i32)> {
    let tile_m = 40_075_016.686 * lat.to_radians().cos() / (1u32 << ZOOM) as f64;
    let (hw, hh) = (SPAN_M / 2.0 / tile_m, SPAN_M * H as f64 / W as f64 / 2.0 / tile_m);
    let mut v = vec![(0, 0)];
    for dx in -1..=1 { for dy in -1..=1 { if (dx, dy) == (0, 0) { continue; }
        let reach_x = dx == 0 || (dx < 0 && fx - hw < 0.0) || (dx > 0 && fx + hw > 1.0);
        let reach_y = dy == 0 || (dy < 0 && fy - hh < 0.0) || (dy > 0 && fy + hh > 1.0);
        if reach_x && reach_y { v.push((dx, dy)); } } }
    v
}

/// Metres per picture pixel, for the window's precision ring.
pub fn metres_per_px() -> f64 { SPAN_M / W as f64 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tile_numbers_follow_the_published_examples() {
        // from the PMTiles v3 specification's own table
        assert_eq!(tile_id(0, 0, 0), 0); assert_eq!(tile_id(1, 0, 0), 1); assert_eq!(tile_id(1, 0, 1), 2); assert_eq!(tile_id(1, 1, 1), 3); assert_eq!(tile_id(1, 1, 0), 4); assert_eq!(tile_id(2, 0, 0), 5);
    }
    #[test]
    fn a_point_lands_in_the_right_tile() {
        let (x, y, fx, fy) = locate(-27.4689, 153.0235, 12);
        assert_eq!((x, y), (3789, 2373)); assert!((0.0..1.0).contains(&fx) && (0.0..1.0).contains(&fy));
    }
    /// Against a real archive: `A2K_TILES=https://…/x.pmtiles cargo test live_map -- --ignored --nocapture` writes map-*.png to $TMPDIR.
    #[test]
    #[ignore]
    fn live_map() {
        let src = Http { url: std::env::var("A2K_TILES").expect("A2K_TILES") };
        let head = src.range(0, 16384).unwrap();
        println!("{:?}", header(&head).unwrap());
        // the tile numbering is right if the z8 tile over Brisbane names Brisbane
        let (x, y, _, _) = locate(-27.4689, 153.0235, 8);
        let t = tile(&src, &head, 8, x, y).unwrap().expect("a z8 tile over land");
        assert!(t.windows(8).any(|w| w == b"Brisbane"), "the z8 tile fetched for Brisbane does not mention it: wrong tile");
        for (name, lat, lon) in [("brisbane", -27.4689, 153.0235), ("geneva", 46.2044, 6.1432)] {
            let (x, y, fx, fy) = locate(lat, lon, ZOOM);
            let tiles: Vec<(i32, i32, Vec<u8>)> = needed(lat, fx, fy).into_iter().filter_map(|(dx, dy)| tile(&src, &head, ZOOM, (x as i32 + dx) as u32, (y as i32 + dy) as u32).unwrap().map(|b| (dx, dy, b))).collect();
            for l in layers(&tiles[0].2).unwrap() { let mut kinds: Vec<&str> = l.features.iter().map(|f| f.kind.as_str()).collect(); kinds.sort(); kinds.dedup(); println!("{name} {}: {} features, kinds {:?}", l.name, l.features.len(), &kinds[..kinds.len().min(12)]); }
            for dark in [false, true] { let png = draw(lat, &tiles, fx, fy, dark).unwrap(); let p = std::env::temp_dir().join(format!("map-{name}-{}.png", if dark { "dark" } else { "light" })); std::fs::write(&p, png).unwrap(); println!("{}", p.display()); }
        }
    }
}
