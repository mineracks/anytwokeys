// Spacing audit, run in the page by the UI harnesses: every visible control must have breathing room.
//   1. two blocks stacked in the same container, where either is or holds a control: at least MIN px apart
//   2. two controls side by side: at least MIN px apart
//   3. a control inside a bordered box (card, modal, key row, notice): at least EDGE px from the box's edge
// Deliberate, and allowed: inside a .field its label sits 6px over its control (closer than field to field, so it reads
// as that control's label); table rows touch (their cells pad them); a segmented control's buttons are joined; the
// rename pencil sits inline in a line of text; a full-width row in a list (the address book) sits 6px in from its box.
// Returns a list of violations, each naming the elements so the CSS can be fixed at its cause.
window.__layoutAudit = (MIN = 8, EDGE = 12) => {
  const CONTROL = 'button, .btn, input, textarea, select, .camopt, .devopt, .opt, .camrow, .pick, .cam, video';
  const vis = (e) => { const r = e.getBoundingClientRect(); const cs = getComputedStyle(e); return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none' && cs.position !== 'absolute' && cs.position !== 'fixed'; };
  const INLINE = '.pencil';
  const hasCtl = (e) => (e.matches(CONTROL) && !e.matches(INLINE)) || [...e.querySelectorAll(CONTROL)].some(c => !c.matches(INLINE));
  const skipParent = (e) => e.matches('table, thead, tbody, tr, .seg');
  const name = (e) => { let s = e.tagName.toLowerCase(); if (e.id) s += '#' + e.id; else if (e.className && typeof e.className === 'string') s += '.' + e.className.trim().split(/\s+/).slice(0, 2).join('.'); const t = (e.innerText || e.value || e.placeholder || '').trim().replace(/\s+/g, ' ').slice(0, 28); return t ? `${s} "${t}"` : s; };
  const inView = (e) => { for (let p = e; p; p = p.parentElement) { if (p.hidden) return false; if (p.classList && p.classList.contains('screen') && !p.classList.contains('on')) return false; if (p.classList && p.classList.contains('modal') && !p.classList.contains('on')) return false; } return true; };
  const out = [], seen = new Set();
  const add = (kind, a, b, px) => { const k = kind + name(a) + (b ? name(b) : ''); if (seen.has(k)) return; seen.add(k); out.push({ kind, a: name(a), b: b ? name(b) : '', px: Math.round(px * 10) / 10 }); };
  const root = document.querySelector('.modal.on .box') ? [document.querySelector('.modal.on .box')] : [document.querySelector('.screen.on'), document.querySelector('#staged-bar')].filter(Boolean);
  for (const top of root) {
    if (!inView(top)) continue;
    for (const parent of [top, ...top.querySelectorAll('*')]) {
      if (!inView(parent) || !vis(parent) && parent !== top) continue;
      if (skipParent(parent)) continue;
      const min = parent.matches('.field') ? 6 : MIN;
      const kids = [...parent.children].filter(k => vis(k) && inView(k));
      for (let i = 0; i < kids.length; i++) for (let j = i + 1; j < kids.length; j++) {
        const a = kids[i].getBoundingClientRect(), b = kids[j].getBoundingClientRect();
        const xOverlap = Math.min(a.right, b.right) - Math.max(a.left, b.left), yOverlap = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
        if (!(hasCtl(kids[i]) || hasCtl(kids[j]))) continue;
        if (xOverlap > 4 && (b.top >= a.bottom - 1 || a.top >= b.bottom - 1)) { const gap = b.top >= a.bottom - 1 ? b.top - a.bottom : a.top - b.bottom; if (gap < min && gap > -1) add('stacked too close', kids[i], kids[j], gap); }
        else if (yOverlap > 4 && kids[i].matches(CONTROL) && kids[j].matches(CONTROL)) { const gap = b.left >= a.right - 1 ? b.left - a.right : a.left - b.right; if (gap < min && gap > -1) add('side by side too close', kids[i], kids[j], gap); }
      }
    }
    for (const c of top.querySelectorAll('button, .btn, input, textarea, select')) {
      if (!vis(c) || !inView(c) || c.matches(INLINE) || c.closest('.seg')) continue;
      // the nearest box drawn around it
      let box = c.parentElement; while (box && box !== document.body) { const cs = getComputedStyle(box); if (parseFloat(cs.borderTopWidth) > 0 && cs.borderTopStyle !== 'none' && parseFloat(cs.borderLeftWidth) > 0) break; box = box.parentElement; }
      if (!box || box === document.body || box.matches('.cam, .camopt, .devopt, .opt, .tabs')) continue;
      const r = c.getBoundingClientRect(), bb = box.getBoundingClientRect(), cs = getComputedStyle(box);
      const inner = { l: bb.left + parseFloat(cs.borderLeftWidth), r: bb.right - parseFloat(cs.borderRightWidth), t: bb.top + parseFloat(cs.borderTopWidth), b: bb.bottom - parseFloat(cs.borderBottomWidth) };
      const d = Math.min(r.left - inner.l, inner.r - r.right, r.top - inner.t, inner.b - r.bottom);
      const row = c.parentElement && c.parentElement !== box ? c.parentElement.getBoundingClientRect() : r;
      const listRow = r.width > 0.8 * (inner.r - inner.l) || (box.matches('.book') && row.width > 0.8 * (inner.r - inner.l));
      if (d < (listRow ? 6 : EDGE) && d > -1) add('too close to its box edge', c, box, d);
    }
  }
  return out;
};
