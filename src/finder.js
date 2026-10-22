// Any Two Keys — the finder: one search over everything this profile holds, from any screen. ⌘K (Ctrl+K) or the
// magnifier in the top bar opens it; the words typed go to `search_all`, which reads the store, the chain cache and the
// price cache on this computer — no network — and answers with vaults, keys, labels, notes, transactions, coins,
// addresses, the address book, locations and entities, best first. Opening a hit lands on that thing's own page.
(() => {
  const fd = $('#finder'), q = $('#finder-q'), list = $('#finder-list'), read = $('#finder-read'), stat = $('#finder-stat');
  const ICON = { vault: '#i-overview', key: '#i-keys', xkey: '#i-keys', tx: '#i-activity', coin: '#i-coins', addr: '#i-addr', contact: '#i-book', place: '#i-place', entity: '#i-entity' };
  const KIND = { vault: 'Vaults', entity: 'Entities', place: 'Locations', key: 'Keys', xkey: 'Keys', contact: 'Address book', tx: 'Transactions', coin: 'Coins', addr: 'Addresses' };
  const ORDER = ['vault', 'entity', 'place', 'key', 'xkey', 'contact', 'tx', 'coin', 'addr'];
  let hits = [], at = -1, seq = 0, timer = null, lastQ = '';
  const isOpen = () => !fd.hidden;
  const canOpen = () => !!store && $('#shell') && !$('#shell').hidden && $('#lock').hidden;

  function open() {
    if (!canOpen()) return;
    fd.hidden = false; q.value = lastQ; q.focus(); q.select();
    if (lastQ) run(); else idle();
  }
  function close() { fd.hidden = true; clearTimeout(timer); seq++; }
  function idle() {
    hits = []; at = -1; read.textContent = ''; stat.textContent = '';
    list.innerHTML = `<div class="fd-empty">A label or a note you wrote · an address or a few characters of one · a transaction id · an amount like <b>0.25</b>, <b>50,000</b> or <b>$1,200</b> · a day like <b>19 Oct 2026</b>, <b>last month</b> or <b>2025</b> · a vault, a key, a fingerprint, a serial · a location, an entity, someone in the address book.<br>Every vault on this computer is looked through at once.</div>`;
  }
  async function run() {
    const text = q.value.trim(); lastQ = text;
    if (!text) return idle();
    const my = ++seq;
    let r;
    try { r = await invoke('search_all', { q: text, ccy: fiatCcy(), tzMin: new Date().getTimezoneOffset() }); }
    catch (e) { if (my === seq) { list.innerHTML = `<div class="fd-empty">${esc(String(e))}</div>`; stat.textContent = ''; } return; }
    if (my !== seq || !isOpen()) return;
    paint(r);
  }
  const ccyWord = (x) => { try { return money(x, fiatCcy()); } catch { return x.toFixed(2); } };
  function paint(r) {
    hits = r.hits || []; at = hits.length ? 0 : -1;
    const rd = r.reading || {}, ways = [];
    if (rd.sats != null) ways.push(`${sats(rd.sats)} sats${rd.sats >= 1e5 ? ' (' + (rd.sats / 1e8).toLocaleString('en-AU', { maximumFractionDigits: 8 }) + ' BTC)' : ''}`);
    if (rd.fiat != null && store.vaults.some(v => v.network === 'mainnet')) ways.push(ccyWord(rd.fiat));
    if (rd.span) ways.push(`the days of ${rd.span}`);
    read.textContent = ways.length ? `Also read as ${ways.join(', ')} — a transaction or a coin of that amount, or on those days, counts.` : '';
    const n = hits.length, moreN = Object.values(r.more || {}).reduce((a, b) => a + b, 0);
    stat.textContent = n ? `${sats(n)}${moreN ? '+' : ''} found in ${r.ms} ms · ${sats(r.vaults)} vault${r.vaults === 1 ? '' : 's'}, ${sats(r.txs)} transactions, ${sats(r.coins)} coins${r.addresses ? ', ' + sats(r.addresses) + ' addresses' : ''} looked through` : `nothing in ${r.ms} ms · ${sats(r.vaults)} vault${r.vaults === 1 ? '' : 's'}, ${sats(r.txs)} transactions, ${sats(r.coins)} coins${r.addresses ? ', ' + sats(r.addresses) + ' addresses' : ''} looked through`;
    if (!n) { list.innerHTML = `<div class="fd-empty">Nothing matches “${esc(lastQ)}”.<br>Labels, notes, ids, amounts and days all count; a vault that has never synced here has no transactions to look through yet.</div>`; return; }
    // grouped by kind in a fixed order; inside a group, best first as they came
    const groups = ORDER.map(k => [k, hits.map((h, i) => [h, i]).filter(([h]) => h.kind === k)]).filter(g => g[1].length);
    let html = '';
    for (const [k, rows] of groups) {
      html += `<div class="fd-kind">${KIND[k]}</div>`;
      for (const [h, i] of rows) html += rowHtml(h, i);
      if (r.more && r.more[k]) html += `<div class="fd-more">and ${sats(r.more[k])} more ${KIND[k].toLowerCase()} — type more to narrow it</div>`;
    }
    list.innerHTML = html;
    mark(at, false);
  }
  function rowHtml(h, i) {
    // an id stands in for a missing label on a transaction, a coin or an address: drawn as one
    const shortId = h.id.length > 20 ? h.id.slice(0, 10) + '…' + h.id.slice(-6) : h.id;
    const idTitle = ['tx', 'coin', 'addr'].includes(h.kind) && (h.title === h.id || h.title === shortId);
    const amt = h.sats != null && (h.kind === 'tx' || h.kind === 'coin') ? `<div class="fd-amt"><span class="${h.sats > 0 && h.kind === 'tx' ? 'in' : ''}">${h.kind === 'tx' ? (h.sats > 0 ? '+' : h.sats < 0 ? '−' : '') : ''}${sats(Math.abs(h.sats))} sats</span>${h.fiat ? `<small class="muted">${esc(h.fiat)}</small>` : ''}</div>` : '';
    return `<div class="fd-hit" role="option" id="fd-${i}" data-i="${i}" aria-selected="false"><span class="tp-art"><svg><use href="${ICON[h.kind] || '#i-overview'}"/></svg></span><span class="fd-what"><b class="${idTitle ? 'mono' : ''}" title="${esc(h.title)}">${esc(h.title)}</b><small title="${esc(h.sub)}">${esc(h.sub)}</small></span>${amt}</div>`;
  }
  function mark(i, scroll = true) {
    if (!hits.length) return;
    at = (i + hits.length) % hits.length;
    $$('.fd-hit', list).forEach(el => el.setAttribute('aria-selected', String(+el.dataset.i === at)));
    q.setAttribute('aria-activedescendant', 'fd-' + at);
    const el = list.querySelector(`#fd-${at}`); if (el && scroll) el.scrollIntoView({ block: 'nearest' });
  }
  // the vault a hit belongs to becomes the vault on screen, as a click on its tab would
  function pick(name) {
    const v = store.vaults.find(x => x.name === name);
    if (!v) { toast(`${name} is not on this computer any more`); return null; }
    if (!vault || vault.name !== v.name) { vault = v; rememberVault(v); paintNet(); }
    return v;
  }
  const watched = (v) => !!(v.muun || v.spark || v.watch || v.statement || v.node || v.exchange);
  async function go(h) {
    close();
    try {
      switch (h.kind) {
        case 'vault': if (pick(h.id)) show('home'); break;
        case 'entity': openEntity(h.id); break;
        case 'place': openPlace(h.id); break;
        case 'key': if (keyHome(h.id)) openKeyFromApp(h.id); else show('allkeys'); break;
        case 'xkey': if (pick(h.vault)) show('keys'); break;
        case 'tx': await goVaultTx(h.vault, h.id); break;
        case 'contact': {
          const net = (h.sub.match(/· (mainnet|signet) ·/) || [])[1];
          const v = (vault && !watched(vault) && (!net || vault.network === net)) ? vault : store.vaults.find(x => !watched(x) && (!net || x.network === net));
          if (!v) { toast('No vault here can pay that address'); break; }
          pick(v.name); show('send'); fillDestination(h.id); break;
        }
        case 'addr': {
          if (!pick(h.vault)) break;
          show('addresses');
          await renderAddresses();
          if (!(renderAddresses.rows || []).some(r => r.address === h.id)) { addrAhead = Math.min(addrAhead + 50, 200); await renderAddresses(); }
          if ((renderAddresses.rows || []).some(r => r.address === h.id)) recPageOpen('addr', h.id); else toast('That address is further ahead than the list goes');
          break;
        }
        case 'coin': {
          if (!pick(h.vault)) break;
          show('coins');
          if (!vault._coins) await sync();
          await renderAddresses(); renderCoins2();
          recPageOpen('coin', h.id); break;
        }
      }
    } catch (e) { fail(e); }
  }
  // typing: a short pause, then the search; the keys move and open
  q.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(run, 120); });
  q.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); mark(at + 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); mark(at - 1); }
    else if (e.key === 'Enter') { e.preventDefault(); if (at >= 0 && hits[at]) go(hits[at]); }
    else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
  });
  list.addEventListener('click', (e) => { const el = e.target.closest('.fd-hit'); if (el) go(hits[+el.dataset.i]); });
  list.addEventListener('pointermove', (e) => { const el = e.target.closest('.fd-hit'); if (el && +el.dataset.i !== at) mark(+el.dataset.i, false); });
  fd.addEventListener('click', (e) => { if (e.target === fd) close(); });
  $('#finder-close').onclick = close;
  $('#finder-btn').onclick = () => (isOpen() ? close() : open());
  document.addEventListener('keydown', (e) => {
    if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && (e.key === 'k' || e.key === 'K')) { e.preventDefault(); isOpen() ? close() : open(); return; }
    if (e.key === 'Escape' && isOpen()) { e.stopPropagation(); close(); }
  }, true);
  // in a discreet profile the results blur like every other screen, and holding them shows them
  let peekT = null;
  list.addEventListener('pointerdown', () => { if (!document.body.classList.contains('discreet')) return; peekT = setTimeout(() => document.body.classList.add('peek'), 350); });
  for (const ev of ['pointerup', 'pointercancel']) list.addEventListener(ev, () => { clearTimeout(peekT); document.body.classList.remove('peek'); });
})();
