// ---------- market context: where the price sat when coins arrived, left, or are about to ----------
// Worked out in cycle.rs: the USD price that day over its 200-week average, and where that ranked among the weeks
// KNOWN ON THAT DAY since June 2014 — the historical percentile. The percentile leads everywhere (one 0–100 scale),
// the multiple sits beside it. Shown beside the money lines, never instead of them; facts about the price on a day,
// not advice, and the wording keeps to that. Loaded before app.js; everything here is called from the screens there.
const cycleDay = (t) => new Date(t * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
const cycleMonth = (t) => new Date(t * 1000).toLocaleDateString(undefined, { month: 'short', year: 'numeric' });
const cycleX = (m) => (m >= 10 ? m.toFixed(0) : m.toFixed(1)) + '×';
const cycleOrd = (p) => { const n = Math.max(0, Math.min(100, Math.round(p * 100))), r = n % 10, teen = n % 100 >= 11 && n % 100 <= 13; return n + (teen ? 'th' : r === 1 ? 'st' : r === 2 ? 'nd' : r === 3 ? 'rd' : 'th'); };
const cycleQuarter = (p) => Math.min(3, Math.floor(p * 4));
const CYCLE_QUARTERS = ['historically low', 'lower than most weeks', 'higher than most weeks', 'historically high'];
const cycleWord = (p) => CYCLE_QUARTERS[cycleQuarter(p)];
/// Four small segments, one per quarter of the percentile scale, the one the mark falls in lit. The words are on it.
function cycleStrip(mark) {
  const q = cycleQuarter(mark.pct), said = `${cycleOrd(mark.pct)} percentile of the weeks known then; ${cycleX(mark.mult)} the 200-week average`;
  return `<span class="cyc" role="img" aria-label="${said}" title="${said}">${[0, 1, 2, 3].map(i => `<i${i === q ? ' class="on"' : ''}></i>`).join('')}</span>`;
}

// Marks are asked for in one batch a moment after a screen first wants them, kept for the rest of the session, and
// the screen is told to draw again once they are here. A moment with no price or no average is kept as null.
const cycleMemo = new Map();
let cycleAsk = new Set(), cycleThen = new Set(), cycleTimer = null;
const cycleAt = (t) => cycleMemo.get(t) || null;
/// true when every mark for these moments is already here; otherwise asks for the missing ones, calls `then` once
/// they are, and answers false now
function cycleWant(ts, then) {
  const missing = ts.filter(t => t && !cycleMemo.has(t));
  if (!missing.length) return true;
  for (const t of missing) cycleAsk.add(t);
  if (then) cycleThen.add(then);
  if (!cycleTimer) cycleTimer = setTimeout(cycleFlush, 60);
  return false;
}
async function cycleFlush() {
  cycleTimer = null;
  const ts = [...cycleAsk], fns = [...cycleThen]; cycleAsk.clear(); cycleThen.clear();
  if (!ts.length) return;
  let marks;
  try { marks = await invoke('cycle_marks', { times: ts }); } catch { marks = ts.map(() => null); }
  ts.forEach((t, i) => cycleMemo.set(t, marks[i] || null));
  for (const f of fns) { try { f(); } catch {} }
}
/// The sats-weighted percentile and multiple of a set of dated lots: what the coins were received at, taken together.
/// undefined while any mark is still on its way (`then` is called when it has come); null when nothing is dated or
/// no mark exists for any of it.
function cycleOfLots(lots, then) {
  const dated = (lots || []).filter(l => l.t && l.sats > 0);
  if (!dated.length) return null;
  if (!cycleWant(dated.map(l => l.t), then)) return undefined;
  let w = 0, p = 0, s = 0;
  for (const l of dated) { const m = cycleMemo.get(l.t); if (m) { w += l.sats * m.mult; p += l.sats * m.pct; s += l.sats; } }
  return s > 0 ? { mult: w / s, pct: p / s, sats: s } : null;
}
/// Where the price sits now, kept five minutes like the price itself.
let cycleNowMemo = null;
async function cycleNow() {
  if (cycleNowMemo && Date.now() - cycleNowMemo.at < 300000) return cycleNowMemo.mark;
  let mark = null; try { mark = await invoke('cycle_now'); } catch {}
  cycleNowMemo = { at: Date.now(), mark }; return mark;
}
const cycleNowHere = () => cycleNowMemo ? cycleNowMemo.mark : undefined;
const cycleStale = (m) => m && m.t < Date.now() / 1000 - 2 * 86400;
const cycleSpan = (from, to) => !from ? 'day not known here' : from === to ? cycleDay(from) : `${cycleDay(from)} to ${cycleDay(to)}`;
/// The whole curve, once per session: one point a week since 2010, with each week's own percentile.
let cycleCurveP = null;
const cycleCurve = () => cycleCurveP || (cycleCurveP = invoke('cycle_curve').catch(() => { cycleCurveP = null; return null; }));

/// The compact table used on a transaction and on Send: two moments, two rows. `a`/`b` = { title, sub, mark }, where
/// mark is undefined while on its way and null when there is none.
function cycleTable(a, b, note) {
  const cell = (m, f) => m ? f(m) : m === undefined ? '…' : '—';
  return `<table class="mkt"><thead><tr><th></th><th>${esc(a.title)}${a.sub ? `<small>${esc(a.sub)}</small>` : ''}</th><th>${esc(b.title)}${b.sub ? `<small>${esc(b.sub)}</small>` : ''}</th></tr></thead><tbody>
    <tr><td>Historical percentile</td><td>${cell(a.mark, m => cycleOrd(m.pct))}</td><td>${cell(b.mark, m => cycleOrd(m.pct))}</td></tr>
    <tr><td>Price / 200-week average</td><td>${cell(a.mark, m => cycleX(m.mult))}</td><td>${cell(b.mark, m => cycleX(m.mult))}</td></tr></tbody></table>
    <div class="mkt-n">${note || 'The percentile ranks that day’s price, over its 200-week average, against every week known on that day since June 2014. Where the price sat, not what to do.'}</div>`;
}

/// What a steady weekly buy across a span would have sat at, as a percentile: the mean of the weekly percentiles
/// from the week of the first arrival to the week of the last. null for one arrival, or when the curve is not here.
function cycleDca(curve, from, to) {
  if (!curve || !from || !to || to - from < 6 * 86400) return null;
  const w = curve.weeks.filter(x => x.pct !== null && x.t >= from - 7 * 86400 && x.t <= to);
  return w.length ? w.reduce((a, x) => a + x.pct, 0) / w.length : null;
}

const CYCLE_NS = 'http://www.w3.org/2000/svg';
const cycleMk = (tag, a) => { const n = document.createElementNS(CYCLE_NS, tag); for (const k in a) n.setAttribute(k, a[k]); return n; };
/// One selectable dot, with its words on it for hover and for a screen reader; Enter or Space picks it too.
function cycleDot(svg, a, cx, cy, r, cls, selected, pick) {
  if (selected === a) svg.appendChild(cycleMk('circle', { class: 'halo', cx, cy, r: r + 5 }));
  const c = cycleMk('circle', { class: cls + ' pt', cx, cy, r, tabindex: 0, role: 'button' });
  c.setAttribute('aria-label', `${cycleDay(a.t)}: ${sats(a.sats)} sats ${a.way}, ${cycleOrd(a.pct)} percentile`);
  const tt = cycleMk('title'); tt.textContent = `${cycleDay(a.t)} · ${sats(a.sats)} sats ${a.way} · ${cycleOrd(a.pct)} percentile · ${cycleX(a.mult)} the 200-week average`; c.appendChild(tt);
  c.addEventListener('click', () => pick(a)); c.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); pick(a); } });
  svg.appendChild(c);
}
/// Over time: the market's percentile as a subdued line, with each arrival (filled) and departure (hollow) on it,
/// sized by sats. Selecting a dot calls `pick(point)`.
function cycleTimeline(el, { curve, acq, dis, now, from, to, selected, pick }) {
  const W = Math.max(280, el.clientWidth || 560), H = 190, m = { l: 34, r: 14, t: 22, b: 26 };
  const pw = W - m.l - m.r, ph = H - m.t - m.b, span = Math.max(1, to - from);
  const X = (t) => m.l + (Math.min(to, Math.max(from, t)) - from) / span * pw, Y = (p) => m.t + (1 - p) * ph;
  const svg = cycleMk('svg', { viewBox: `0 0 ${W} ${H}`, height: H, role: 'img' });
  const ax = cycleMk('g', { class: 'axis' });
  const yl = cycleMk('text', { x: 0, y: 11, 'text-anchor': 'start', class: 'axlbl' }); yl.textContent = 'Historical percentile'; ax.appendChild(yl);
  for (const p of [0, 0.25, 0.5, 0.75, 1]) { ax.appendChild(cycleMk('line', { class: 'grid', x1: m.l, x2: m.l + pw, y1: Y(p), y2: Y(p) })); const tx = cycleMk('text', { x: m.l - 7, y: Y(p) + 3.5, 'text-anchor': 'end' }); tx.textContent = String(p * 100); ax.appendChild(tx); }
  for (let i = 0; i <= 2; i++) { const t = from + span * i / 2; const tx = cycleMk('text', { x: X(t), y: H - 7, 'text-anchor': i === 0 ? 'start' : i === 2 ? 'end' : 'middle' }); tx.textContent = xLabel(t, span); ax.appendChild(tx); }
  svg.appendChild(ax);
  // the market line: one point a week where the curve has a percentile, then now
  const pts = (curve ? curve.weeks.filter(w => w.pct !== null && w.t >= from - 7 * 86400 && w.t <= to) : []).map(w => [X(w.t), Y(w.pct)]);
  if (now) pts.push([X(Math.min(to, now.t)), Y(now.pct)]);
  if (pts.length > 1) svg.appendChild(cycleMk('path', { class: 'mkt-line', d: pts.map((p, i) => (i ? 'L' : 'M') + p[0].toFixed(1) + ',' + p[1].toFixed(1)).join(' ') }));
  const maxS = Math.max(1, ...acq.map(a => a.sats), ...dis.map(a => a.sats)), R = (s) => 3.5 + 7 * Math.sqrt(s / maxS);
  for (const a of [...acq, ...dis].sort((p, q) => q.sats - p.sats)) cycleDot(svg, a, X(a.t), Y(a.pct), R(a.sats), a.way === 'received' ? 'buy' : 'sell', selected, pick);
  svg.setAttribute('aria-label', `Historical percentile over time, with ${acq.length} arrival${acq.length === 1 ? '' : 's'}${dis.length ? ` and ${dis.length} departure${dis.length === 1 ? '' : 's'}` : ''} marked. Each is selectable; the date buttons below do the same.`);
  el.replaceChildren(svg);
}
/// Distribution: the same points along one 0–100 axis, with the quarters shaded and now marked above the plot.
function cycleDistribution(el, { acq, dis, now, selected, pick }) {
  const W = Math.max(280, el.clientWidth || 560), H = 150, m = { l: 14, r: 14, t: 34, b: 34 };
  const pw = W - m.l - m.r, ph = H - m.t - m.b;
  const X = (p) => m.l + p * pw;
  const svg = cycleMk('svg', { viewBox: `0 0 ${W} ${H}`, height: H, role: 'img' });
  for (let i = 0; i < 4; i++) svg.appendChild(cycleMk('rect', { class: 'band b' + i, x: X(i / 4), y: m.t, width: pw / 4, height: ph }));
  const ax = cycleMk('g', { class: 'axis' });
  for (const p of [0, 0.25, 0.5, 0.75, 1]) { const tx = cycleMk('text', { x: X(p), y: m.t + ph + 14, 'text-anchor': p === 0 ? 'start' : p === 1 ? 'end' : 'middle' }); tx.textContent = String(p * 100); ax.appendChild(tx); }
  const lo = cycleMk('text', { x: X(0), y: H - 4, class: 'axlbl' }); lo.textContent = 'Historically lower'; ax.appendChild(lo);
  const hi = cycleMk('text', { x: X(1), y: H - 4, 'text-anchor': 'end', class: 'axlbl' }); hi.textContent = 'Historically higher'; ax.appendChild(hi);
  svg.appendChild(ax);
  if (now) {   // the now mark lives in its own lane above the plot, so it never collides with a tick
    const x = X(now.pct); svg.appendChild(cycleMk('line', { class: 'hair', x1: x, x2: x, y1: m.t - 4, y2: m.t + ph }));
    const right = now.pct > 0.8, tx = cycleMk('text', { class: 'endlbl', x: x + (right ? -5 : 5), y: m.t - 9, 'text-anchor': right ? 'end' : 'start' }); tx.textContent = `Now · ${cycleOrd(now.pct)}`; svg.appendChild(tx);
  }
  const maxS = Math.max(1, ...acq.map(a => a.sats), ...dis.map(a => a.sats)), R = (s) => 3.5 + 7 * Math.sqrt(s / maxS);
  for (const a of acq.slice().sort((p, q) => q.sats - p.sats)) cycleDot(svg, a, X(a.pct), m.t + ph * 0.34, R(a.sats), 'buy', selected, pick);
  for (const a of dis.slice().sort((p, q) => q.sats - p.sats)) cycleDot(svg, a, X(a.pct), m.t + ph * 0.7, R(a.sats), 'sell', selected, pick);
  svg.setAttribute('aria-label', `Historical percentile of each arrival and departure, from historically lower on the left to higher on the right${now ? `, with now at the ${cycleOrd(now.pct)}` : ''}.`);
  el.replaceChildren(svg);
}

const cycleUsd = (v) => { try { return new Intl.NumberFormat(undefined, { style: 'currency', currency: 'USD', maximumFractionDigits: 0 }).format(v); } catch { return '$' + Math.round(v); } };
/// Price & 200-week average: the USD close each week as a line, the average dashed beneath it, each arrival and
/// departure on the price line, and now as a hollow mark on the average. The same question in the units a person
/// already knows.
function cyclePriceView(el, { curve, acq, dis, now, from, to, selected, pick }) {
  const W = Math.max(280, el.clientWidth || 560), H = 190, m = { l: 40, r: 14, t: 22, b: 26 };
  const pw = W - m.l - m.r, ph = H - m.t - m.b, span = Math.max(1, to - from);
  const weeks = curve ? curve.weeks.filter(w => w.close !== null && w.t >= from - 7 * 86400 && w.t <= to) : [];
  const { top, step } = niceMax(Math.max(1, ...weeks.map(w => w.close), ...[...acq, ...dis].map(a => a.price || 0), now ? now.price : 0));
  const X = (t) => m.l + (Math.min(to, Math.max(from, t)) - from) / span * pw, Y = (v) => m.t + ph - v / top * ph;
  const svg = cycleMk('svg', { viewBox: `0 0 ${W} ${H}`, height: H, role: 'img' });
  const ax = cycleMk('g', { class: 'axis' });
  const yl = cycleMk('text', { x: 0, y: 11, class: 'axlbl' }); yl.textContent = 'BTC price, USD'; ax.appendChild(yl);
  for (let v = 0; v <= top + 1e-9; v += step) { ax.appendChild(cycleMk('line', { class: 'grid', x1: m.l, x2: m.l + pw, y1: Y(v), y2: Y(v) })); const tx = cycleMk('text', { x: m.l - 7, y: Y(v) + 3.5, 'text-anchor': 'end' }); tx.textContent = v >= 1000 ? Math.round(v / 1000) + 'k' : String(v); ax.appendChild(tx); }
  for (let i = 0; i <= 2; i++) { const t = from + span * i / 2; const tx = cycleMk('text', { x: X(t), y: H - 7, 'text-anchor': i === 0 ? 'start' : i === 2 ? 'end' : 'middle' }); tx.textContent = xLabel(t, span); ax.appendChild(tx); }
  svg.appendChild(ax);
  const path = (pts, cls) => { if (pts.length > 1) svg.appendChild(cycleMk('path', { class: cls, d: pts.map((p, i) => (i ? 'L' : 'M') + p[0].toFixed(1) + ',' + p[1].toFixed(1)).join(' ') })); };
  const wma = weeks.filter(w => w.wma !== null).map(w => [X(w.t), Y(w.wma)]); if (now) wma.push([X(Math.min(to, now.t)), Y(now.wma)]);
  const close = weeks.map(w => [X(w.t), Y(w.close)]); if (now) close.push([X(Math.min(to, now.t)), Y(now.price)]);
  path(wma, 'wma-line'); path(close, 'px-line');
  if (now) svg.appendChild(cycleMk('circle', { class: 'nowmark', cx: X(Math.min(to, now.t)), cy: Y(now.wma), r: 3.5 }));
  const maxS = Math.max(1, ...acq.map(a => a.sats), ...dis.map(a => a.sats)), R = (s) => 3.5 + 7 * Math.sqrt(s / maxS);
  for (const a of [...acq, ...dis].filter(a => a.price).sort((p, q) => q.sats - p.sats)) cycleDot(svg, a, X(a.t), Y(a.price), R(a.sats), a.way === 'received' ? 'buy' : 'sell', selected, pick);
  svg.setAttribute('aria-label', `The USD price each week with its 200-week average, and ${acq.length} arrival${acq.length === 1 ? '' : 's'}${dis.length ? ` and ${dis.length} departure${dis.length === 1 ? '' : 's'}` : ''} marked on it. Each is selectable; the date buttons below do the same.`);
  el.replaceChildren(svg);
}

/// The Home card: "Purchase context" — the two percentiles, the picture in either view, the selected point, the
/// date buttons, and how it is worked out. Drawn into `card` once the marks are here; `alive()` says whether this
/// drawing is still the one on screen, so a slower answer never paints over a newer vault.
const cyclePrefs = { view: 'time' };
function cycleSummary(L, card, alive) {
  const acq = [], dis = [];
  for (const e of L.timeline) { const d = e.sats - (e.before ? e.before.sats : 0); if (e.internal || !d) continue; (d > 0 ? acq : dis).push({ t: e.t, sats: Math.abs(d), txid: e.txid, way: d > 0 ? 'received' : 'left' }); }
  const all = [...acq, ...dis];
  card.className = 'pc'; card.hidden = false;
  card.innerHTML = `<div class="pc-eyebrow">Purchase context</div><h3 class="pc-title">Where your buys landed</h3><div class="pc-sub">Market position relative to Bitcoin’s 200-week average, as it stood on each day.</div><div class="pc-stats"><div class="pc-stat"><div class="k">Coins received</div><div class="v">…</div><div class="s">working out where the price sat</div></div><div class="pc-stat"><div class="k">Market now</div><div class="v">…</div><div class="s"></div></div></div><div class="pc-rest"></div>`;
  const stat = (i, v, s) => { const d = card.querySelectorAll('.pc-stat')[i]; d.querySelector('.v').innerHTML = v; d.querySelector('.s').textContent = s; };
  const paint = async () => {
    if (!alive()) return;
    if (!cycleWant(all.map(a => a.t), paint)) return;
    for (const a of all) { const mk = cycleAt(a.t); a.mult = mk ? mk.mult : null; a.pct = mk ? mk.pct : null; a.price = mk ? mk.price : null; a.wma = mk ? mk.wma : null; }
    const A = acq.filter(a => a.mult !== null), D = dis.filter(a => a.mult !== null);
    const tot = A.reduce((a, x) => a + x.sats, 0), avgP = tot ? A.reduce((a, x) => a + x.sats * x.pct, 0) / tot : null, avgM = tot ? A.reduce((a, x) => a + x.sats * x.mult, 0) / tot : null;
    const [curve, now] = await Promise.all([cycleCurve(), cycleNow()]); if (!alive()) return;
    stat(1, now ? `${cycleOrd(now.pct)} <small>percentile</small>` : '—', now ? `${cycleX(now.mult)} the 200-week average${cycleStale(now) ? ` · as of ${cycleDay(now.t)}` : ''}` : 'no price just now');
    const rest = card.querySelector('.pc-rest');
    if (avgP === null) { stat(0, '—', all.length ? 'no price or no average for the days these arrived' : 'nothing has arrived yet'); rest.innerHTML = ''; return; }
    stat(0, `${cycleOrd(avgP)} <small>percentile</small>`, `weighted by sats · ${A.length} arrival${A.length === 1 ? '' : 's'} · ${cycleX(avgM)} the 200-week average`);
    const from = Math.min(...A.map(a => a.t), ...D.map(a => a.t)), to = Math.floor(Date.now() / 1000);
    const dca = A.length > 1 ? cycleDca(curve, Math.min(...A.map(a => a.t)), Math.max(...A.map(a => a.t))) : null;
    rest.innerHTML = `<div class="pc-bar"><div class="seg" role="radiogroup" aria-label="View"><button type="button" role="radio" data-view="time">Over time</button><button type="button" role="radio" data-view="dist">Distribution</button><button type="button" role="radio" data-view="price">Price &amp; 200WMA</button></div><span class="muted pc-range">${esc(cycleMonth(from))} – ${esc(cycleMonth(to))}</span></div>
      <div class="chart pc-chart"></div>
      <div class="legend"><span><i class="dotkey buy"></i>received · size = sats</span>${D.length ? '<span><i class="dotkey sell"></i>left</span>' : ''}<span class="pc-linekey"><i class="linekey"></i>market percentile</span><span class="pc-pricekey"><i class="linekey px"></i>spot price</span><span class="pc-pricekey"><i class="linekey wma"></i>200-week average</span></div>
      <div class="pc-sel"></div><div class="pc-list" role="group" aria-label="Choose an arrival or departure"></div>
      <details class="adv pc-how"><summary>How this is calculated</summary><div class="muted pc-howbody"></div></details>`;
    const points = [...A, ...D].sort((p, q) => p.t - q.t);
    let selected = A.length ? A[A.length - 1] : points[points.length - 1];
    const chart = rest.querySelector('.pc-chart'), sel = rest.querySelector('.pc-sel'), list = rest.querySelector('.pc-list');
    const draw = () => {
      for (const b of rest.querySelectorAll('[data-view]')) b.setAttribute('aria-checked', String(b.dataset.view === cyclePrefs.view));
      rest.querySelector('.pc-linekey').hidden = cyclePrefs.view !== 'time';
      for (const k of rest.querySelectorAll('.pc-pricekey')) k.hidden = cyclePrefs.view !== 'price';
      const args = { curve, acq: A, dis: D, now, from: from - Math.max(7 * 86400, (to - from) * 0.04), to, selected, pick: (a) => { selected = a; draw(); } };
      if (cyclePrefs.view === 'time') cycleTimeline(chart, args); else if (cyclePrefs.view === 'dist') cycleDistribution(chart, args); else cyclePriceView(chart, args);
      const priced = cyclePrefs.view === 'price' && selected && selected.price;
      sel.innerHTML = selected ? `<div><b>${esc(cycleDay(selected.t))} · ${sats(selected.sats)} sats</b><span>${priced ? 'Spot ' + cycleUsd(selected.price) : selected.way === 'received' ? 'Received · counted as a purchase' : 'Left the vault'}</span></div><div><b>${cycleOrd(selected.pct)} percentile</b><span>${cycleX(selected.mult)} the 200-week average${priced ? ' · ' + cycleUsd(selected.wma) : ''} · ${cycleWord(selected.pct)}</span></div>` : '';
      const months = points.map(a => cycleMonth(a.t)), label = new Set(months).size < months.length ? cycleDay : cycleMonth;   // two in one month: say the day
      const at = points.indexOf(selected);
      if (points.length <= 12) {   // few enough: one button each
        list.innerHTML = points.map((a, i) => `<button type="button" class="pc-pick${a.way === 'left' ? ' left' : ''}" data-i="${i}" aria-pressed="${a === selected}">${esc(label(a.t))}${a.way === 'left' ? ' ↑' : ''}</button>`).join('');
        for (const b of list.querySelectorAll('button')) b.onclick = () => { selected = points[+b.dataset.i]; draw(); };
      } else {   // a steady buyer has dozens: step through them, oldest to newest
        list.innerHTML = `<button type="button" class="pc-pick" data-step="-1" ${at <= 0 ? 'disabled' : ''} aria-label="Older">‹ Older</button><span class="muted pc-count">${at + 1} of ${points.length} · ${esc(cycleDay(selected.t))}${selected.way === 'left' ? ' ↑' : ''}</span><button type="button" class="pc-pick" data-step="1" ${at >= points.length - 1 ? 'disabled' : ''} aria-label="Newer">Newer ›</button>`;
        for (const b of list.querySelectorAll('[data-step]')) b.onclick = () => { selected = points[at + (+b.dataset.step)]; draw(); };
      }
    };
    for (const b of rest.querySelectorAll('[data-view]')) b.onclick = () => { cyclePrefs.view = b.dataset.view; draw(); };
    draw();
    const how = [];
    how.push('Each day’s price in USD is divided by the average of the 200 weekly closes before it, so the figure does not depend on your currency.');
    how.push('The historical percentile is where that day’s figure ranked among every week known on that day, from June 2014 when the average first existed. A later rise or fall does not re-rank an earlier day.');
    how.push(`“Coins received” is weighted by sats across the ${A.length} arrival${A.length === 1 ? '' : 's'}; a coin moved in from another of your vaults keeps the day it was first received.`);
    if (dca !== null) how.push(`A steady weekly buy from ${cycleMonth(Math.min(...A.map(a => a.t)))} to ${cycleMonth(Math.max(...A.map(a => a.t)))} would have averaged the ${cycleOrd(dca)} percentile; these arrivals averaged the ${cycleOrd(avgP)}. A lower percentile means the price sat lower against its long average on those days, not that the coins were cheaper or the return better.`);
    how.push('An arrival is counted as a purchase because the chain cannot say otherwise. It says where the price sat, not what to do.');
    if (all.length > A.length + D.length) how.push(`${all.length - A.length - D.length} of these have no price or no average for their day and are left out.`);
    rest.querySelector('.pc-howbody').textContent = how.join(' ');
  };
  paint();
}
