// Reading a QR code off a camera frame — including a code ENGRAVED IN METAL, in poor light, where one frame handed
// to jsQR as it comes often fails: glare burns out part of the code, the engraving is faint, the plate is small in
// the picture. Each frame is tried more than one way, in rotation, and the frame itself is measured so the screen
// can say WHY nothing is reading and what would help.
//
// The thresholds below are a first calibration, checked on synthetic codes (test/ui-signers.py: dark, glare, blur,
// faint); real plates under real light are what will tune them.
const SCAN_HINT_AFTER = 2500;   // ms without a read before a hint is shown

function makeQrReader(note) {
  const full = document.createElement('canvas'), fctx = full.getContext('2d', { willReadFrequently: true });
  const work = document.createElement('canvas'), wctx = work.getContext('2d', { willReadFrequently: true });
  const tiny = document.createElement('canvas'), tctx = tiny.getContext('2d', { willReadFrequently: true });
  let tick = 0, since = Date.now(), hintEl = null;
  const r = { last: null, attempts: [] };

  const gray = (d) => { const g = new Uint8ClampedArray(d.length / 4); for (let i = 0, j = 0; i < d.length; i += 4, j++) g[j] = (d[i] * 77 + d[i + 1] * 150 + d[i + 2] * 29) >> 8; return g; };
  const toRGBA = (g, img) => { const d = img.data; for (let i = 0, j = 0; j < g.length; i += 4, j++) { d[i] = d[i + 1] = d[i + 2] = g[j]; d[i + 3] = 255; } return img; };
  // faint engraving: stretch the 2nd–98th percentile of brightness to the full range
  const stretch = (g) => { const h = new Uint32Array(256); for (const v of g) h[v]++; let lo = 0, hi = 255, acc = 0; const n = g.length;
    for (let v = 0; v < 256; v++) { acc += h[v]; if (acc > n * 0.02) { lo = v; break; } } acc = 0;
    for (let v = 255; v >= 0; v--) { acc += h[v]; if (acc > n * 0.02) { hi = v; break; } }
    const s = 255 / Math.max(1, hi - lo), o = new Uint8ClampedArray(n); for (let i = 0; i < n; i++) o[i] = (g[i] - lo) * s; return o; };
  // glare and uneven light: each pixel against the mean of its neighbourhood (an integral image), not one level for all
  const threshold = (g, w, h) => { const I = new Float64Array((w + 1) * (h + 1));
    for (let y = 0; y < h; y++) { let row = 0; for (let x = 0; x < w; x++) { row += g[y * w + x]; I[(y + 1) * (w + 1) + x + 1] = I[y * (w + 1) + x + 1] + row; } }
    const R = Math.max(8, Math.round(Math.min(w, h) / 24)), o = new Uint8ClampedArray(g.length);
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      const x0 = Math.max(0, x - R), y0 = Math.max(0, y - R), x1 = Math.min(w, x + R + 1), y1 = Math.min(h, y + R + 1);
      const sum = I[y1 * (w + 1) + x1] - I[y0 * (w + 1) + x1] - I[y1 * (w + 1) + x0] + I[y0 * (w + 1) + x0];
      o[y * w + x] = g[y * w + x] < sum / ((x1 - x0) * (y1 - y0)) - 6 ? 0 : 255; }
    return o; };
  const decode = (img) => { const c = jsQR(img.data, img.width, img.height, { inversionAttempts: 'attemptBoth' }); return c && c.data ? c.data : null; };

  /// What is wrong with the picture, if anything, measured on the framed middle at 160 px.
  function measure(src, sx, sy, side) {
    tiny.width = tiny.height = 160; tctx.drawImage(src, sx, sy, side, side, 0, 0, 160, 160);
    const g = gray(tctx.getImageData(0, 0, 160, 160).data), n = g.length;
    let sum = 0, sq = 0, burnt = 0, lap = 0;
    for (let i = 0; i < n; i++) { sum += g[i]; sq += g[i] * g[i]; if (g[i] >= 250) burnt++; }
    for (let y = 1; y < 159; y++) for (let x = 1; x < 159; x++) { const i = y * 160 + x; lap += Math.abs(4 * g[i] - g[i - 1] - g[i + 1] - g[i - 160] - g[i + 160]); }
    const mean = sum / n, std = Math.sqrt(Math.max(0, sq / n - mean * mean)), glare = burnt / n, sharp = lap / (158 * 158);
    // sharpness is judged against contrast, so a dim but crisp code is not called blurred (synthetic: crisp 0.72, blurred 0.17)
    const why = mean < 50 ? 'dark' : glare > 0.04 ? 'glare' : std < 20 ? 'faint' : sharp / Math.max(std, 1) < 0.3 ? 'blur' : null;
    return { mean: Math.round(mean), std: Math.round(std), glare: Math.round(glare * 1000) / 1000, sharp: Math.round(sharp * 10) / 10, why };
  }
  const HINTS = {
    dark: 'Too dark to read. Add light, or turn the plate toward a window or lamp.',
    glare: 'Glare on the metal is hiding part of the code. Tilt the plate a little, so the light does not bounce straight into the camera.',
    faint: 'The engraving is faint in this light. Light the plate from one side, low and raking, so the marks cast shadows.',
    blur: 'The picture is soft. Hold the plate still, a little further from the camera, and give it a moment to focus.',
    small: 'Fill the gold frame with the code, flat and square to the camera.',
  };
  function hint(why) {
    if (!note) return;
    if (!hintEl) { hintEl = document.createElement('div'); hintEl.className = 'scanhint'; hintEl.setAttribute('role', 'status'); note.parentNode.insertBefore(hintEl, note.nextSibling); }
    hintEl.hidden = !why; if (why) hintEl.textContent = HINTS[why];
  }

  /// One frame from a video (or any image source): the code's text, or null.
  r.read = (src) => {
    const w = src.videoWidth || src.width, h = src.videoHeight || src.height; if (!w || !h) return null;
    const side = Math.round(Math.min(w, h) * 0.62), sx = Math.round((w - side) / 2), sy = Math.round((h - side) / 2);
    // 1. the whole frame, as it comes (at most 1280 wide: jsQR's cost grows with the pixels)
    const k = Math.min(1, 1280 / w); full.width = Math.round(w * k); full.height = Math.round(h * k); fctx.drawImage(src, 0, 0, full.width, full.height);
    let got = decode(fctx.getImageData(0, 0, full.width, full.height)); r.attempts = ['frame'];
    // 2. the framed middle at full resolution, one way per frame in turn: as it is, stretched, thresholded
    if (!got) {
      work.width = work.height = Math.min(side, 1100); wctx.drawImage(src, sx, sy, side, side, 0, 0, work.width, work.height);
      const img = wctx.getImageData(0, 0, work.width, work.height), way = ['middle', 'stretched', 'thresholded'][tick % 3];
      if (way !== 'middle') { const g = gray(img.data); toRGBA(way === 'stretched' ? stretch(g) : threshold(stretch(g), work.width, work.height), img); }
      got = decode(img); r.attempts.push(way);
      if (got) r.way = way;
    } else r.way = 'frame';
    if (tick++ % 3 === 0 || got) r.last = measure(src, sx, sy, side);
    if (got) { since = Date.now(); hint(null); return got; }
    if (Date.now() - since > SCAN_HINT_AFTER) hint((r.last && r.last.why) || (Date.now() - since > 6000 ? 'small' : null));
    return null;
  };
  /// Try every way on one image at once (for tests): which of them read it.
  r.tryAll = (src) => { const out = {}; const t0 = tick; for (let i = 0; i < 3; i++) { tick = i; out[['middle', 'stretched', 'thresholded'][i]] = !!r.read(src); } tick = t0; return out; };
  r.reset = () => { since = Date.now(); hint(null); };
  r.age = (ms) => { since -= ms; };   // tests: as if nothing had read for this long
  return r;
}
