// Metal: engraving plates on a SeedHammer v1 from this computer.
//
// Two jobs come here: a vault's recovery line split into Shamir shares (one SH-03 plate per
// share, both sides), and anything at all - text and line art - on an SH-02 or SH-03.
// Layout is done by the companion engine (vendor/composer/composer.wasm, the same Go code the
// SeedHammer's own firmware runs), so the preview is the plate by code identity; the Rust side
// (engraver.rs) only speaks the wire protocol. A pen-up trace of every design is offered
// before the hammer touches metal, and the engrave button has to be armed.
(() => {
  const { invoke } = window.__TAURI__.core;
  const PLATE = { 1: { name: 'SH-02 square', w: 85, h: 85 }, 2: { name: 'SH-03 large', w: 85, h: 134 } };
  let wasmReady = null;
  let jobs = [];            // [{ label, plate, sh1e: Uint8Array, program: {commands, count, strokes, bounds_mm, warnings} }]
  let current = 0;
  let traced = new Set();   // job indexes that have had a pen-up trace since they were built
  let port = null;          // the engraver chosen
  let running = false;

  function engine() {
    if (wasmReady) return wasmReady;
    wasmReady = (async () => {
      if (!window.Go) { await new Promise((ok, no) => { const s = document.createElement('script'); s.src = 'vendor/composer/wasm_exec.js'; s.onload = ok; s.onerror = () => no(new Error('the engraving engine did not load')); document.head.appendChild(s); }); }
      const go = new Go();
      const resp = await fetch('vendor/composer/composer.wasm');
      if (!resp.ok) throw new Error('the engraving engine is missing (vendor/composer/composer.wasm)');
      const r = await WebAssembly.instantiate(await resp.arrayBuffer(), go.importObject);   // not instantiateStreaming: some hosts serve wasm with the wrong type
      go.run(r.instance);
      for (let i = 0; i < 50 && !globalThis.composerProgram; i++) await new Promise(ok => setTimeout(ok, 20));
      if (!globalThis.composerProgram) throw new Error('the engraving engine did not start');
    })();
    return wasmReady;
  }
  const check = (r) => { if (r && r.error) throw new Error(r.error); return r; };

  // ---------- building jobs ----------
  async function sharesJobs(vaultName, k, n) {
    await engine();
    const shares = await invoke('vault_shares_plates', { name: vaultName, k, n });
    const out = [];
    for (const s of shares) {
      const d = check(globalThis.composerShareDesign(JSON.stringify({ title: vaultName, ur_type: s.ur_type, fingerprint: s.set, k, n, index: s.index, payload: b64(s.payload) })));
      for (const [side, bytes] of [['A', d.side_a], ['B', d.side_b]]) out.push({ label: `Share ${s.index} of ${n} · side ${side}`, plate: 2, sh1e: bytes, side, program: check(globalThis.composerProgram(bytes)) });
    }
    return out;
  }
  async function customJob(plate, lines, sizePt, paths) {
    await engine();
    const w = PLATE[plate].w, h = PLATE[plate].h;
    const textBlocks = [];
    const lineMM = Math.ceil(sizePt * 0.33) + 1;
    const top = Math.max(10, Math.round((h - lines.length * lineMM) / 2));
    lines.forEach((t, i) => { if (t.trim()) textBlocks.push({ size_pt: sizePt, x_mm: Math.round(w / 2), y_mm: top + i * lineMM, align: 1, text: t.trim() }); });
    const svgPaths = (paths || []).map(p => ({ x_mm: p.x, y_mm: p.y, scale_pct: p.scale, d: p.d }));
    const bytes = check(globalThis.composerBuildDesign(JSON.stringify({ plate, text_blocks: textBlocks, svg_paths: svgPaths, qr_blocks: [] })));
    return [{ label: textBlocks.length ? textBlocks[0].text : 'Line art', plate, sh1e: bytes, program: check(globalThis.composerProgram(bytes)) }];
  }
  const b64 = (arr) => btoa(String.fromCharCode(...arr));

  /// A key's words on an SH-03, two to a line with their numbers, from the setup walk or the Backup card.
  window.metalEngraveWords = async (words, title) => {
    const lines = []; for (let i = 0; i < words.length; i += 2) lines.push(`${i + 1} ${words[i]}` + (words[i + 1] ? `   ${i + 2} ${words[i + 1]}` : ''));
    open(await customJob(2, [title.slice(0, 24)].concat(lines), words.length > 18 ? 13 : 15, []), `${words.length} words \u00b7 ${title}`);
    document.querySelector('#metal').scrollIntoView({ block: 'start', behavior: 'smooth' });
  };

  // ---------- the panel ----------
  const $m = (s) => document.querySelector('#metal ' + s);
  function open(newJobs, what) {
    jobs = newJobs; current = 0; traced = new Set();
    const box = document.querySelector('#metal'); box.hidden = false;
    // the panel lives on whichever screen asked for it, at the end of that screen's content
    const screen = document.querySelector('.screen.on'); if (screen && box.parentElement !== screen) screen.appendChild(box);
    $m('.cer-act-title').textContent = what;
    $m('#metal-say').textContent = 'Trace first: the head draws the design with the pen up, so you can see it land on the plate before anything is hammered.';
    paint(); findEngraver();
    box.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
  function paint() {
    const j = jobs[current]; if (!j) return;
    $m('#metal-chips').innerHTML = jobs.map((x, i) => `<button type="button" class="vchip" aria-pressed="${i === current}" data-i="${i}">${esc(x.label)}${traced.has(i) ? ' ✓' : ''}</button>`).join('');
    $m('#metal-preview').innerHTML = check(globalThis.composerPreview(j.sh1e));
    const p = j.program, b = p.bounds_mm;
    $m('#metal-facts').innerHTML = `<div><b>${PLATE[j.plate].name}</b> · ${p.strokes} strokes, ${p.count} moves · ${b.x0.toFixed(0)}–${b.x1.toFixed(0)} mm across, ${b.y0.toFixed(0)}–${b.y1.toFixed(0)} mm down</div>`
      + (p.warnings.length ? `<div class="warn">${esc(p.warnings.join(' '))}</div>` : '');
    $m('#metal-mount').textContent = j.plate === 2 ? 'Mount an SH-03 plate in the long slot, nuts tight.' : 'Mount an SH-02 plate in the square position, nuts tight.';
    const armed = traced.has(current) && !!port && !running;
    $m('#metal-trace').disabled = !port || running;
    $m('#metal-go').disabled = !armed;
    $m('#metal-go').classList.toggle('armed', armed);
    $m('#metal-go').title = traced.has(current) ? '' : 'Trace it first: the head draws the design with the pen up so you can see where it lands';
    $m('#metal-cancel').hidden = !running;
    $m('#metal-bar').hidden = !running;
  }
  async function findEngraver() {
    const row = $m('#metal-engraver');
    row.innerHTML = `<div class="tp-what"><div class="tp-head"><b>Looking for the engraver…</b></div></div>`;
    try {
      const ports = await invoke('engraver_find');
      port = ports[0] ? ports[0].path : null;
      row.innerHTML = port
        ? `<div class="tp-what"><div class="tp-head"><b>Engraver on ${esc(port)}</b></div><div class="muted" style="font-size:13px">${esc(ports[0].product)}${ports[0].serial ? ' · ' + esc(ports[0].serial) : ''}${ports.length > 1 ? ' · ' + (ports.length - 1) + ' more port' + (ports.length > 2 ? 's' : '') + ' ignored' : ''}</div></div><div class="tp-act"><button type="button" class="btn small" id="metal-jog">Jog test</button><button type="button" class="btn small" id="metal-again">Look again</button></div>`
        : `<div class="tp-what"><div class="tp-head"><b>No engraver on a USB port</b></div><div class="muted" style="font-size:13px">Plug the SeedHammer's USB cable into this computer and switch it on. The FTDI serial adapter inside it is what the app looks for.</div></div><div class="tp-act"><button type="button" class="btn small" id="metal-again">Look again</button></div>`;
      const again = row.querySelector('#metal-again'); if (again) again.onclick = findEngraver;
      const jog = row.querySelector('#metal-jog'); if (jog) jog.onclick = async () => { jog.disabled = true; try { toast(await invoke('engraver_jog', { port })); } catch (e) { fail(e); } finally { jog.disabled = false; } };
    } catch (e) { port = null; row.innerHTML = `<div class="tp-what"><div class="warn">${esc(String(e))}</div></div>`; }
    paint();
  }
  async function run(dry) {
    const j = jobs[current]; if (!j || !port || running) return;
    running = true; paint();
    const bar = $m('#metal-bar'), say = $m('#metal-say');
    say.textContent = dry ? 'Tracing with the pen up…' : 'Engraving — loud; hearing protection.';
    bar.classList.add('live'); bar.style.setProperty('--pct', '0%');
    const un = await window.__TAURI__.event.listen('engrave-progress', (ev) => { const p = ev.payload; bar.style.setProperty('--pct', `${Math.round(100 * p.done / Math.max(1, p.total))}%`); say.textContent = `${dry ? 'Tracing' : 'Engraving'} · ${p.done} of ${p.total}`; });
    try {
      const r = await invoke('engraver_run', { port, plate: j.plate, commands: Array.from(j.program.commands), dry });
      if (dry) traced.add(current);
      say.textContent = r + (dry ? '. If the head stayed on the plate, the engrave button is armed.' : '.');
      toast(dry ? 'Traced' : 'Engraved');
    } catch (e) { say.textContent = String(e); fail(e); }
    finally { un(); running = false; bar.classList.remove('live'); paint(); }
  }

  document.addEventListener('DOMContentLoaded', () => {
    const box = document.querySelector('#metal'); if (!box) return;
    box.addEventListener('click', (e) => { const c = e.target.closest('.vchip[data-i]'); if (c) { current = +c.dataset.i; paint(); } });
    $m('#metal-trace').onclick = () => run(true);
    $m('#metal-cancel').onclick = () => invoke('engraver_cancel').catch(() => {});
    $m('#metal-close').onclick = () => { box.hidden = true; };
    askTwice($m('#metal-go'), 'Engrave this side now? The hammer marks the plate permanently, and the process is loud.', 'Engrave it', () => run(false));

    // shares -> plates, from the vault's settings
    const sharesBtn = document.querySelector('#vs-shares-metal');
    if (sharesBtn) sharesBtn.onclick = async () => {
      sharesBtn.disabled = true;
      try { const k = +document.querySelector('#vs-shares-k').value, n = +document.querySelector('#vs-shares-n').value; open(await sharesJobs(vault.name, k, n), `${n} share plates for ${vault.name}`); }
      catch (e) { fail(e); } finally { sharesBtn.disabled = false; }
    };

    // anything -> a plate, from settings
    const anyBtn = document.querySelector('#any-go'); if (!anyBtn) return;
    let svgPaths = [];
    const plateOf = () => +(document.querySelector('#any-plate button[aria-pressed="true"]') || {}).dataset?.plate || 1;
    document.querySelector('#any-plate').addEventListener('click', (e) => { const b = e.target.closest('button'); if (!b) return; document.querySelectorAll('#any-plate button').forEach(x => x.setAttribute('aria-pressed', String(x === b))); });
    document.querySelector('#any-size').addEventListener('click', (e) => { const b = e.target.closest('.vchip'); if (!b) return; document.querySelectorAll('#any-size .vchip').forEach(x => x.setAttribute('aria-pressed', String(x === b))); });
    const dz = document.querySelector('#any-svg');
    const readSVG = async (file) => {
      try {
        const doc = new DOMParser().parseFromString(await file.text(), 'image/svg+xml');
        if (doc.querySelector('parsererror')) throw new Error('that is not valid SVG');
        rewriteShapesToPaths(doc, doc.documentElement);
        const ds = [...doc.documentElement.querySelectorAll('path')].map(p => p.getAttribute('d')).filter(Boolean);
        if (!ds.length) throw new Error('no drawable shapes in the file (text must be converted to outlines first; raster images cannot be engraved)');
        // fit the drawing's bounding box into the plate's safe area, centred
        const probe = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); probe.style.cssText = 'position:absolute;width:0;height:0;visibility:hidden'; document.body.appendChild(probe);
        let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
        for (const d of ds) { const p = document.createElementNS('http://www.w3.org/2000/svg', 'path'); p.setAttribute('d', d); probe.appendChild(p); const b = p.getBBox(); x0 = Math.min(x0, b.x); y0 = Math.min(y0, b.y); x1 = Math.max(x1, b.x + b.width); y1 = Math.max(y1, b.y + b.height); }
        probe.remove();
        const plate = plateOf(), w = PLATE[plate].w - 24, h = PLATE[plate].h - 24;
        const scale = Math.min(w / Math.max(1e-6, x1 - x0), h / Math.max(1e-6, y1 - y0));
        const pct = Math.max(1, Math.min(1000, Math.round(scale * 100)));
        const dx = Math.round((PLATE[plate].w - (x1 - x0) * scale) / 2 - x0 * scale), dy = Math.round((PLATE[plate].h - (y1 - y0) * scale) / 2 - y0 * scale);
        svgPaths = ds.map(d => ({ d, x: dx, y: dy, scale: pct }));
        dz.querySelector('.dz-main').textContent = `${file.name} — ${ds.length} path${ds.length === 1 ? '' : 's'}, fitted to the plate`;
        dz.classList.add('done');
      } catch (e) { svgPaths = []; dz.classList.remove('done'); fail(e); }
    };
    dz.querySelector('input').onchange = (e) => { if (e.target.files[0]) readSVG(e.target.files[0]); };
    dz.addEventListener('dragover', (e) => { e.preventDefault(); dz.classList.add('over'); }); dz.addEventListener('dragleave', () => dz.classList.remove('over'));
    dz.addEventListener('drop', (e) => { e.preventDefault(); dz.classList.remove('over'); if (e.dataTransfer.files[0]) readSVG(e.dataTransfer.files[0]); });
    anyBtn.onclick = async () => {
      anyBtn.disabled = true;
      try {
        const lines = document.querySelector('#any-text').value.split('\n').slice(0, 12);
        const size = +(document.querySelector('#any-size .vchip[aria-pressed="true"]') || {}).dataset?.pt || 12;
        if (!lines.some(l => l.trim()) && !svgPaths.length) throw new Error('give it some text, or a drawing');
        open(await customJob(plateOf(), lines, size, svgPaths), lines.find(l => l.trim()) || 'Line art');
      } catch (e) { fail(e); } finally { anyBtn.disabled = false; }
    };
  });

  // SVG primitives -> paths, so a logo drawn with <rect>/<circle>/<polygon> engraves too.
  // Same algorithm and constant (0.5522847498, the quarter-circle Bezier ratio) as the
  // companion composer; transforms are not baked - flatten them in the editor first.
  function rewriteShapesToPaths(doc, root) {
    const NS = 'http://www.w3.org/2000/svg';
    const replace = (el, d) => { const p = doc.createElementNS(NS, 'path'); p.setAttribute('d', d); el.parentNode.replaceChild(p, el); };
    for (const r of [...root.querySelectorAll('rect')]) { const x = +r.getAttribute('x') || 0, y = +r.getAttribute('y') || 0, w = +r.getAttribute('width') || 0, h = +r.getAttribute('height') || 0; if (!w || !h) { r.remove(); continue; } replace(r, `M${x} ${y} L${x + w} ${y} L${x + w} ${y + h} L${x} ${y + h} Z`); }
    const ell = (el, cx, cy, rx, ry) => { const kx = 0.5522847498 * rx, ky = 0.5522847498 * ry; replace(el, [`M${cx - rx} ${cy}`, `C${cx - rx} ${cy - ky} ${cx - kx} ${cy - ry} ${cx} ${cy - ry}`, `C${cx + kx} ${cy - ry} ${cx + rx} ${cy - ky} ${cx + rx} ${cy}`, `C${cx + rx} ${cy + ky} ${cx + kx} ${cy + ry} ${cx} ${cy + ry}`, `C${cx - kx} ${cy + ry} ${cx - rx} ${cy + ky} ${cx - rx} ${cy}`, 'Z'].join(' ')); };
    for (const c of [...root.querySelectorAll('circle')]) { const r = +c.getAttribute('r') || 0; if (!r) { c.remove(); continue; } ell(c, +c.getAttribute('cx') || 0, +c.getAttribute('cy') || 0, r, r); }
    for (const e of [...root.querySelectorAll('ellipse')]) { const rx = +e.getAttribute('rx') || 0, ry = +e.getAttribute('ry') || 0; if (!rx || !ry) { e.remove(); continue; } ell(e, +e.getAttribute('cx') || 0, +e.getAttribute('cy') || 0, rx, ry); }
    for (const l of [...root.querySelectorAll('line')]) replace(l, `M${+l.getAttribute('x1') || 0} ${+l.getAttribute('y1') || 0} L${+l.getAttribute('x2') || 0} ${+l.getAttribute('y2') || 0}`);
    for (const poly of [...root.querySelectorAll('polygon, polyline')]) { const pts = (poly.getAttribute('points') || '').split(/[\s,]+/).filter(Boolean).map(Number); if (pts.length < 4) { poly.remove(); continue; } const segs = []; for (let i = 0; i < pts.length; i += 2) segs.push(`${i ? 'L' : 'M'}${pts[i]} ${pts[i + 1]}`); if (poly.tagName.toLowerCase() === 'polygon') segs.push('Z'); replace(poly, segs.join(' ')); }
  }

  window.metal = { open, sharesJobs, customJob };
})();
