// Any Two Keys — the UI. Every real thing happens in Rust behind `invoke`.
const { invoke } = window.__TAURI__.core;
const $ = (s) => document.querySelector(s);
const $$ = (s, root) => Array.from((root || document).querySelectorAll(s));
const sats = (n) => Number(n).toLocaleString('en-AU');
let otherDevices = [];
// `vault` is the vault on screen. What the window learned about it from the chain (coins, balance, history, the
// recovery line…) lives in fields starting with "_". The store is re-read after most edits, which hands back a FRESH
// vault object — and every such re-read used to drop all of that, so the next screen synced the whole vault again.
// Assigning `vault` now carries those fields to the fresh copy of the SAME vault (same name and descriptor: a vault
// forgotten and made again under the same name starts clean).
const LIVE = {};
const liveKey = (v) => v.name + '|' + (v.ext || v.descriptor || '');
let vaultNow = null;
Object.defineProperty(window, 'vault', { configurable: true, get: () => vaultNow, set: (v) => {
  if (vaultNow && typeof vaultNow === 'object') { const k = LIVE[liveKey(vaultNow)] || (LIVE[liveKey(vaultNow)] = {}); for (const [a, b] of Object.entries(vaultNow)) if (a[0] === '_') k[a] = b; }
  if (v && typeof v === 'object' && v !== vaultNow) { const k = LIVE[liveKey(v)]; if (k) for (const [a, b] of Object.entries(k)) if (!(a in v)) v[a] = b; }
  vaultNow = v;
} });
let store = null, cards = [], cardsErr = '', fees = null, feeLevel = 'medium', built = null, camStream = null, camTimer = null;
let setup = { k: 2, n: 3, keys: [], network: null };

const CHANGE = 0x80000000;
// an index carries a scheme in its top bits (wallet.rs): scheme 7 is a silent payment output, numbered as found
const fmtIndex = (i) => ((i >>> 28) & 7) === 7 ? 'silent payment #' + (i & 0x0fffffff) : (i >= CHANGE ? 'change ' : '') + '#' + (i & 0x0fffffff) + kindTag(i);
// a wallet watched for several kinds of address has a #14 of each kind: the kind is part of the name
const kindTag = (i) => { const k = vault && vault.watch && vault.watch.kinds.length > 1 ? vault.watch.kinds[(i >>> 28) & 7] : null; return k ? ' \u00b7 ' + ((KIND_WORDS[k] || k).split(' \u00b7 ')[1] || k) : ''; };
const PEN = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/></svg>';
// a record lives in the vault on screen — or, for a place, in the app's own records (name '' to the commands)
const recHolder = (key) => key && key.startsWith('place:') ? ((store && store.records) || {}) : (vault || {});
const recName = (key) => key && key.startsWith('place:') ? '' : vault.name;
const labelOf = (key) => ((recHolder(key).labels || {})[key]) || '';
const esc = (t) => String(t).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/"/g, '&quot;');
// an inline label: text plus a hover pencil; click → field; Enter/blur saves through label_set
function lblHtml(key) { return `<span class="lbl" data-lkey="${esc(key)}"><span class="text" title="${esc(labelOf(key))}">${esc(labelOf(key))}</span><button class="pencil" title="Label">${PEN}</button></span>`; }
document.addEventListener('click', (e) => {
  const b = e.target.closest('.lbl .pencil'); if (!b) return; e.preventDefault(); e.stopPropagation();
  const box = b.closest('.lbl'), key = box.dataset.lkey, cur = labelOf(key);
  const inp = document.createElement('input'); inp.className = 'lbl-input'; inp.value = cur; inp.maxLength = 80; inp.placeholder = 'label';
  box.replaceChildren(inp); inp.focus(); inp.select();
  let done = false;
  const finish = async (save) => {
    if (done) return; done = true;
    const v = inp.value.trim();
    if (save && v !== cur) { try { store = await invoke('label_set', { name: vault.name, key, label: v }); labelsSync(); vault = store.vaults.find(x => x.name === vault.name) || vault; } catch (err) { fail(err); } }
    box.innerHTML = `<span class="text">${esc(labelOf(key))}</span><button class="pencil" title="Label">${PEN}</button>`;
  };
  inp.addEventListener('keydown', (ev) => { if (ev.key === 'Enter') { ev.preventDefault(); finish(true); } if (ev.key === 'Escape') finish(false); });
  inp.addEventListener('blur', () => finish(true));
  inp.addEventListener('click', (ev) => ev.stopPropagation());
});

// window.confirm() is inert in the Tauri webview: it shows nothing and answers "no", so a button guarded by it does nothing at all.
// A destructive button asks in the page instead: the first press arms it and says what it will do; a second press within 8 s does it.
function askTwice(btn, question, yesLabel, act) {
  // The question is written into the page right under the button's row, never into the toast: the toast sits at the bottom
  // centre of the window, which is exactly where a bottom-of-page button is, and it covered the button it was asking about.
  const disarm = () => { clearTimeout(btn._t); if (btn._note) { btn._note.remove(); btn._note = null; } if (btn.dataset.armed) { delete btn.dataset.armed; btn.textContent = btn._label; btn.classList.remove('armed'); } };
  btn.onclick = async () => {
    if (btn.dataset.armed) { disarm(); try { await act(); } catch (e) { fail(e); } return; }
    btn._label = btn.textContent; btn.dataset.armed = '1'; btn.textContent = yesLabel; btn.classList.add('armed');
    const note = document.createElement('div'); note.className = 'asknote'; note.setAttribute('role', 'alert');
    note.innerHTML = `<span>${esc(question)} Press “${esc(yesLabel)}” to go ahead.</span><button type="button" class="linkish">Keep it</button>`;
    note.querySelector('button').onclick = disarm;
    (btn.closest('.row, .rowwrap, #staged-bar, .btn-grid') || btn).insertAdjacentElement('afterend', note); btn._note = note;
    note.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    btn._t = setTimeout(disarm, 15000);
  };
}
// where the person goes to let the app use the camera, said for the device in their hand (phone.js sets the class)
const cameraDenied = () => document.body.classList.contains('android') ? 'This phone is not letting Any Two Keys use the camera: allow it in Settings \u203a Apps \u203a Any Two Keys \u203a Permissions \u203a Camera.'
  : document.body.classList.contains('ios') ? 'This iPhone is not letting Any Two Keys use the camera: allow it in Settings \u203a Any Two Keys \u203a Camera.'
  : 'macOS is not letting Any Two Keys use the camera: allow it in System Settings \u203a Privacy & Security \u203a Camera.';
function toast(t, ms = 3500) { const el = $('#toast'); el.textContent = t; el.classList.add('on'); clearTimeout(el._t); el._t = setTimeout(() => el.classList.remove('on'), ms); }
function fail(e) { toast(String(e && e.message || e), 6000); console.error(e); }

// ---------- screens ----------
function show(name) {
  if (name !== 'restore' && $('#s-restore').classList.contains('on')) { seedClear(); if (sparkSelected || sparkBusy) sparkCancel().catch(() => {}); }
  // Restore entered from anywhere else starts clean: what the last restore read is not the next one's problem
  if (name === 'restore' && !$('#s-restore').classList.contains('on')) restoreFresh();
  const first = ['setup', 'restore'].includes(name) && !vault;
  $('#welcome').hidden = true; $('#shell').hidden = false;
  $$('.screen').forEach(s => s.classList.toggle('on', s.id === 's-' + name));
  const navName = name === 'sign' ? (paintCrumbs.via || 'send') : name === 'tx' ? 'activity' : name;   // Sign has no sidebar entry of its own: light the screen it came from
  $$('.nav[data-go]').forEach(n => n.classList.toggle('on', n.dataset.go === navName));
  // the vault you are in stays lit on every screen that is about it, matching the breadcrumb; not while making another, nor in app settings
  // a watched wallet has no keys here: the screens that would sign are not offered at all
  const watched = !!(vault && (vault.muun || vault.spark || vault.watch || vault.statement || vault.node || vault.exchange));
  // a Spark wallet has no addresses and no coins at all — not hidden screens, absent ones. Only ever HIDDEN here:
  // whether the Coins tab is shown at all is the person's own setting, and this must not undo it.
  if (vault && (vault.spark || vault.statement || vault.node || vault.exchange)) for (const go of ['coins', 'addresses']) $$(`.nav[data-go=${go}]`).forEach(n => { n.hidden = true; });
  else { paintCoinsTab(); $$('.nav[data-go=addresses]').forEach(n => { n.hidden = false; }); }
  // only the ways IN to those screens — the sidebar and Home's own actions. A blanket [data-go] would also
  // un-hide buttons that are hidden for their own reasons elsewhere.
  for (const go of ['send', 'messages', 'keys']) $$(`.nav[data-go=${go}], #s-home [data-go=${go}]`).forEach(n => { n.hidden = watched; });
  // Receive is real for a watched wallet of one key — those addresses ARE its own — but means nothing for Muun or Spark
  $$('.nav[data-go=receive], #s-home [data-go=receive]').forEach(n => { n.hidden = !!(vault && (vault.muun || vault.spark || vault.statement || vault.node || vault.exchange)); });
  const inVault = vault && !APP_LEVEL.has(name);
  $$('#vault-tabs .nav').forEach(n => n.classList.toggle('on', !!inVault && n.dataset.vault === vault.name));
  $('#title').textContent = { overview: 'All vaults', home: vault ? vault.name : 'Vault', activity: 'Activity', send: 'Send', receive: 'Receive', coins: 'Coins', addresses: 'Addresses', vault: 'Vault settings', backup: 'Backup', keys: 'Keys', settings: 'App settings', places: 'Locations', entities: 'Entities', allkeys: 'Keys', setup: 'New vault · enrol keys', restore: 'Restore a vault', sign: 'Sign', messages: 'Messages' }[name] || '';
  $('#title-edit').hidden = !(name === 'home' && vault); $('#title-input').hidden = true; $('#title').hidden = false;
  if (name === 'own') $('#title').textContent = ownState.kind === 'split' ? 'Split coin' : 'Combine coins';
  if (name === 'entity') $('#title').textContent = (entityOf(entPage) || {}).name || 'Entity';
  if (name === 'place') $('#title').textContent = ((store.places || []).find(p => p.id === placePage) || {}).name || 'Location';
  paintCrumbs(name); paintStaged(name);
  $('#side-vault').textContent = vault ? vault.name : 'No vault yet'; $('#side-vault').title = vault ? `These screens are about “${vault.name}”` : '';
  if (checkNode.net !== (vault ? vault.network : store.settings.network)) checkNode();
  if (!['tx', 'addresses', 'coins', 'keys', 'place'].includes(name) && $('#tx-notes').parentElement !== recHome) recHome.appendChild($('#tx-notes'));
  if (name !== 'restore') stopCam();
  if (name !== 'keys' && recPage.kind === 'key') closeKeyCard(false);
  if (name !== 'sign') stopQr();
  if (name !== 'keys') hideDesc();
  if (name === 'home') { paintVaultCard(); renderHomeSummary(); syncIfStale(); labelsSync(); paintDrillNudge(); }   // the picture belongs to the vault being shown, not the one left behind
  if (name === 'receive') drawAddress();
  if (name === 'activity') { if (vault && !vault._coins) sync(); else paintActivity(); }
  if (name === 'addresses') { recPageClose('addr'); renderAddresses(); }
  if (name === 'coins') { recPageClose('coin'); if (vault && !vault._coins) sync().then(() => Promise.resolve(renderAddresses()).then(renderCoins2)); else Promise.resolve(renderAddresses()).then(renderCoins2); }
  if (name === 'vault') renderVaultSettings();
  if (name === 'backup') { renderVaultSettings(); renderKeys(); }
  if (name === 'overview') renderOverview();
  if (name === 'send') { closeBook(); paintTo(); loadFees(); if (vault && !vault._coins) sync().then(() => { renderCoins(); sendPreview(); }); else { renderCoins(); sendPreview(); } paintExtSend(); paintSendFrom(); }
  if (name === 'messages') renderMessages();
  if (name === 'keys') renderKeys();
  if (name === 'tx') renderTx();
  if (name === 'setup') renderSetup();
  if (name === 'restore') restoreTab(restoreTab.now || 'keys');
  if (name === 'settings') renderSettings();
  if (name === 'places') renderLocations();
  if (name === 'place') renderPlace();
  if (name === 'entities') renderEntities();
  if (name === 'entity') renderEntity();
  if (name === 'allkeys') renderAllKeys();
  if (name === 'own') renderOwn();
  if (name === 'sign') renderSign();
  void first;
}
document.addEventListener('click', (e) => { const go = e.target.closest('[data-go]'); if (go) { if (go.dataset.k) { setup = { k: +go.dataset.k, n: +go.dataset.n, keys: [], network: store.settings.network }; } show(go.dataset.go); } });
// where you are, and the way back: Vaults › this vault › Send › Sign
const APP_LEVEL = new Set(['setup', 'restore', 'settings', 'overview', 'places', 'place', 'entities', 'entity', 'allkeys']);
let keyFrom = null, entPage = null, placePage = null;
const entityOf = (id) => (store.entities || []).find(e => e.id === id) || null;
function paintCrumbs(name) {
  const trail = [];
  if (['setup', 'restore'].includes(name)) trail.push(['Vaults', store.vaults.length ? 'welcome' : null]);
  else if (name === 'keys' && recPage.kind === 'key' && keyFrom === 'allkeys') trail.push(['Any Two Keys', null], ['Keys', 'allkeys']);
  else if (!APP_LEVEL.has(name) && vault) {
    if (name !== 'home') trail.push([vault.name, 'home']);
    if (name === 'sign') trail.push(paintCrumbs.via === 'messages' ? ['Messages', 'messages'] : paintCrumbs.via === 'own' ? ['Coins', 'coins'] : ['Send', 'send']);
    if (name === 'own') trail.push(['Coins', 'coins']);
    if (name === 'tx') trail.push(['Activity', 'activity']);
    if ((name === 'coins' && recPage.kind === 'coin') || (name === 'addresses' && recPage.kind === 'addr')) trail.push([name === 'coins' ? 'Coins' : 'Addresses', name]);
    if (name === 'keys' && recPage.kind === 'key') trail.push(['Keys', 'keys']);
  }
  if (APP_LEVEL.has(name)) trail.push(['Any Two Keys', null]);
  if (name === 'entity') trail.push(['Entities', 'entities']);
  if (name === 'place') trail.push(['Locations', 'places']);
  const here = { home: '', send: 'Send', receive: 'Receive', backup: 'Backup', keys: recPage.kind === 'key' ? recPage.leaf : 'Keys', settings: 'App settings', places: 'Locations', entities: 'Entities', allkeys: 'Keys', entity: (entityOf(entPage) || {}).name || 'Entity', place: ((store.places || []).find(p => p.id === placePage) || {}).name || 'Location', own: ownState.kind === 'split' ? 'Split coin' : 'Combine coins', setup: 'New vault', restore: 'Restore', sign: 'Sign', messages: 'Messages', tx: name === 'tx' && vault && txView.txid ? txHeadline() : '', coins: recPage.kind === 'coin' ? recPage.leaf : '', addresses: recPage.kind === 'addr' ? recPage.leaf : '' }[name] || '';
  const el = $('#crumbs'); el.hidden = !trail.length;
  const html = trail.map(([t, go]) => (go ? `<a tabindex="0" role="link" data-crumb="${go}">${esc(t)}</a>` : `<span>${esc(t)}</span>`) + '<span class="sep">›</span>').join('') + `<span class="here">${esc(here)}</span>`;
  if (el._html !== html) { el._html = html; el.innerHTML = html; }   // a poll must not take focus off a crumb
}
document.addEventListener('click', (e) => { const c = e.target.closest('[data-crumb]'); if (!c) return; if (c.dataset.crumb === 'welcome') showWelcome(); else show(c.dataset.crumb); });
document.addEventListener('keydown', (e) => { if (e.key === 'Enter' && e.target.matches && e.target.matches('[data-crumb]')) e.target.click(); });
// a spend that is waiting follows you around every screen of its vault until it is sent or discarded
async function paintStaged(name) {
  const bar = $('#staged-bar'); bar.hidden = true;
  if (!vault || ['sign', 'setup', 'restore', 'settings'].includes(name)) return;
  let st; try { st = await invoke('spend_ceremony', { name: vault.name }); } catch { return; }
  if (!st.ours || !$('#s-' + name).classList.contains('on')) return;
  const msg = (await invoke('store_get')).last_message;
  $('#staged-text').textContent = msg ? `A message proof is waiting for signatures: ${st.have} of ${st.need}.` : `A spend is waiting for signatures: ${st.have} of ${st.need}${st.ready ? ', ready to send' : ''}. ${st.pays.filter(p => !p.ours).map(p => sats(p.sats) + ' sats').join(' + ') || 'nothing'} leaving this vault.`;
  paintCrumbs.via = msg ? 'messages' : 'send';
  bar.hidden = false;
}
function showWelcome() { $('#shell').hidden = true; $('#welcome').hidden = false; $('#welcome-back').hidden = !store.vaults.length; paintWelcomePull(); paintCornerChip('welcome-profile'); }
// the quiet fourth path: a vault that already exists, in another profile
function paintWelcomePull() {
  const others = profile && profilesView ? profilesView.profiles.filter(p => p.id !== profile.id) : [];
  $('#welcome-pull').hidden = !others.length; $('#welcome-pull-box').hidden = true;
  $('#welcome-pull-pick').innerHTML = others.map(p => `<div class="opt" data-from="${esc(p.id)}">${esc(p.name)}<small>${PCLASS[p.class][0]}</small></div>`).join('');
}
$('#welcome-pull-open').onclick = () => { $('#welcome-pull-box').hidden = !$('#welcome-pull-box').hidden; };
$('#welcome-pull-pick').addEventListener('click', async (e) => { const o = e.target.closest('.opt'); if (!o || !profile) return;
  try { sessionStorage.setItem('a2k-pull-to', profile.id); } catch {}
  try { await invoke('profile_open', { id: o.dataset.from }); } catch (err) { return fail(err); } location.reload(); });
// in the profile that has the vault: choose it, and it goes back to where the asking came from
function renderPull(dest) {
  $('#shell').hidden = true; $('#welcome').hidden = true; $('#profiles').hidden = true; $('#pull').hidden = false;
  $('#pull-pic').innerHTML = picHtml(dest, 88, true); paintCornerChip('pull-profile');
  $('#pull-h').textContent = `Move a vault to ${dest.name}`;
  $('#pull-say').textContent = `You are in ${profile.name}. Choose the vault to take across; ${dest.name} opens again to receive it.`;
  const kind = (v) => v.exchange ? 'exchange account' : v.node ? 'node' : v.statement ? 'statement' : (v.network === 'mainnet' ? 'mainnet' : esc(v.network)) + (v.keys.length ? ' · ' + v.keys.length + (v.keys.length === 1 ? ' key' : ' keys') : '');
  const how = () => ($('#pull-how [aria-checked="true"]') || {}).dataset?.how || 'copy';
  const picked = new Set();
  const paintBar = () => { const n = picked.size, verb = how() === 'copy' ? 'Copy' : 'Move'; $('#pull-bar').hidden = !n; $('#pull-count').textContent = `${n} vault${n === 1 ? '' : 's'}`; $('#pull-bar-say').textContent = how() === 'copy' ? `${profile.name} keeps ${n === 1 ? 'it' : 'them'}` : `${n === 1 ? 'it leaves' : 'they leave'} ${profile.name}`; $('#pull-go').textContent = `${verb} ${n === 1 ? 'it' : 'them'} to ${dest.name}`; };
  const paintRows = () => { $('#pull-list').innerHTML = store.vaults.map(v => `<div class="pr" role="option" aria-selected="${picked.has(v.name)}" data-pull="${esc(v.name)}" tabindex="0"><span><b>${esc(v.name)}</b><small>${kind(v)}</small></span><span class="tick" aria-hidden="true">✓</span></div>`).join('') || '<div class="muted" style="font-size:13px;padding:10px 14px">This profile holds no vault.</div>';
    const toggle = (r) => { const n = r.dataset.pull; if (picked.has(n)) picked.delete(n); else picked.add(n); r.setAttribute('aria-selected', String(picked.has(n))); paintBar(); };
    $$('#pull-list [data-pull]').forEach(r => { r.onclick = () => toggle(r); r.onkeydown = (e) => { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); toggle(r); } }; });
    paintBar(); };
  $$('#pull-how button').forEach(b => b.onclick = () => { $$('#pull-how button').forEach(x => x.setAttribute('aria-checked', String(x === b))); $('#pull-how-say').textContent = b.dataset.how === 'copy' ? 'A copy leaves this profile exactly as it is; the other gets the same record under its own key.' : `A move takes the vaults out of ${profile.name}. They arrive in ${dest.name} whole.`; paintBar(); });
  $('#pull-all').onclick = () => { store.vaults.forEach(v => picked.add(v.name)); paintRows(); };
  $('#pull-none').onclick = () => { picked.clear(); paintRows(); };
  $('#pull-go').onclick = async () => { if (!picked.size) return; $('#pull-go').disabled = true; let missing = 0;
    try { for (const name of store.vaults.map(v => v.name).filter(n => picked.has(n))) { const st = await invoke('vault_move_start', { name, to: dest.id, keep: how() === 'copy' }); missing += st.missing.length; }
      if (missing) toast(`${missing} attachment${missing === 1 ? ' was' : 's were'} not on this computer and stayed behind`, 8000);
      await invoke('profile_open', { id: dest.id }); location.reload(); }
    catch (e) { fail(e); $('#pull-go').disabled = false; } };
  paintRows();
  $('#pull-cancel').onclick = async () => { try { sessionStorage.removeItem('a2k-pull-to'); } catch {} try { await invoke('profile_open', { id: dest.id }); } catch (e) { return fail(e); } location.reload(); };
}
$('#new-vault-nav').onclick = showWelcome;
$('#welcome-back').onclick = () => { if (vault) show('home'); };

function netName() { return vault ? vault.network : store.settings.network; }
function paintNet() { const p = $('#netpill'); p.classList.toggle('signet', netName() === 'signet'); p.querySelector('span').textContent = netName(); $('#welcome-net').textContent = store.settings.network; renderTabs(); }
function renderTabs() {
  const tabs = $('#vault-tabs'); tabs.innerHTML = '';
  for (const v of store.vaults) {
    const d = document.createElement('div'); d.className = 'nav' + (vault && v.name === vault.name ? ' on' : ''); d.style.fontWeight = '600'; d.dataset.vault = v.name;
    d.innerHTML = `<span style="width:8px;height:8px;border-radius:50%;background:${v.network === 'mainnet' ? 'var(--gold)' : 'var(--signal)'};display:inline-block;flex:0 0 8px"></span><span style="flex-grow:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap" title="${esc(v.name)}">${esc(v.name)}</span><span class="muted" style="font-size:11px;font-weight:400">${v.node ? 'Node' : v.statement ? 'Statement' : v.exchange ? 'Exchange' : v.spark ? (v.spark.authenticated ? 'YubiKey reader' : 'Watched \u00b7 Spark') : v.muun || v.watch ? 'Watched' : v.keys.length === 1 ? '1 key' : v.threshold + ' of ' + v.keys.length}</span>`;
    d.onclick = () => { if (d.dataset.dragged) return; vault = v; rememberVault(v); paintNet(); show('home'); };
    // drag to reorder — by pointer, not HTML5 drag-and-drop: the window's file-drop handling (attachments) takes
    // every drag session on macOS, so a dragover never reaches the page. The drop goes before or after the row
    // it lands on, by which half of it the pointer is in.
    d.addEventListener('pointerdown', (e) => { if (e.button !== 0) return; sidebarDrag = { name: v.name, el: d, x0: e.clientX, y0: e.clientY, id: e.pointerId, moved: false, over: null, before: false }; });
    tabs.appendChild(d);
  }
}
let sidebarDrag = null;
document.addEventListener('pointermove', (e) => {
  const g = sidebarDrag; if (!g) return;
  if (!g.moved) { if (Math.hypot(e.clientX - g.x0, e.clientY - g.y0) < 5) return; g.moved = true; g.el.classList.add('dragging'); try { g.el.setPointerCapture(g.id); } catch {} }
  const rows = $$('#vault-tabs .nav'); rows.forEach(r => r.classList.remove('drop-before', 'drop-after'));
  const over = rows.find(r => { const b = r.getBoundingClientRect(); return e.clientY >= b.top && e.clientY < b.bottom; });
  g.over = over && over !== g.el ? over : null;
  if (g.over) { const b = g.over.getBoundingClientRect(); g.before = e.clientY < b.top + b.height / 2; g.over.classList.add(g.before ? 'drop-before' : 'drop-after'); }
});
const sidebarDragEnd = () => {
  const g = sidebarDrag; if (!g) return; sidebarDrag = null;
  g.el.classList.remove('dragging'); $$('#vault-tabs .nav').forEach(r => r.classList.remove('drop-before', 'drop-after'));
  if (!g.moved) return;
  g.el.dataset.dragged = '1'; setTimeout(() => delete g.el.dataset.dragged, 0);   // the click that follows a drag is not a choice
  if (g.over) reorderVaults(g.name, g.over.dataset.vault, g.before).catch(fail);
};
document.addEventListener('pointerup', sidebarDragEnd); document.addEventListener('pointercancel', sidebarDragEnd);
// The new order is drawn at once and saved behind it; a save that fails redraws the saved order, so the list never
// shows an order the store does not hold.
async function reorderVaults(from, to, before) {
  const names = store.vaults.map(v => v.name).filter(n => n !== from);
  const at = names.indexOf(to) + (before ? 0 : 1); names.splice(at, 0, from);
  const byName = Object.fromEntries(store.vaults.map(v => [v.name, v]));
  store.vaults = names.map(n => byName[n]); renderTabs();
  try { store = await invoke('vault_order', { names }); if (vault) vault = store.vaults.find(x => x.name === vault.name) || vault; }
  catch (e) { try { store = await invoke('store_get'); } catch {} throw e; }
  finally { renderTabs(); }
}

// ---------- locked: everything this app keeps on the computer is encrypted, and something you hold opens it ----------
let lockInfo = null;
const OTP = /^[cbdefghijklnrtuv]{32,}$/;   // a brushed key typing a Yubico one-time code, not a PIN
const lockSaid = (text) => { $('#lock-err').textContent = text || ''; $('#lock-err').hidden = !text; };
function lockScreen() {
  return new Promise((done) => {
    $('#lock').hidden = false;
    let busy = false, sig = null, stop = false;
    const opened = (ls) => { stop = true; lockInfo = ls; $('#lock').hidden = true; $('#lock-keys').innerHTML = ''; done(ls); };
    const withMac = async () => { if (busy) return; busy = true; lockSaid(''); $('#lock-mac').disabled = true;
      try { opened(await invoke('lock_open_mac')); } catch (e) { lockSaid(String(e)); } finally { busy = false; $('#lock-mac').disabled = false; } };
    $('#lock-mac').textContent = lockInfo.touch ? 'Open with Touch ID' : 'Open'; $('#lock-mac').onclick = withMac;
    const keys = async () => { if (stop) return;
      let seen = []; try { seen = await invoke('cards_list'); } catch {}
      const now = seen.map(c => c.reader + c.pubkey).join('|');
      if (now !== sig && !(busy && $('#lock-keys form button:disabled'))) { sig = now;   // never redrawn under a PIN that is on its way to a key
        const rows = seen.filter(c => c.pubkey).map((c, i) => { const slot = lockInfo.cards.find(k => k.pubkey === c.pubkey);
          return slot ? `<form class="lock-key" data-reader="${esc(c.reader)}"><b>${esc(slot.name)}<small>plugged in</small></b><input type="password" autocomplete="off" placeholder="Key PIN" aria-label="PIN of ${esc(slot.name)}" id="lock-pin-${i}"><button class="btn" type="submit">Open</button></form>`
            : `<div class="lock-key off"><b>Key ${esc(String(c.serial || ''))}<small>plugged in, but it has not been added as a way to open this app</small></b></div>`; });
        $('#lock-keys').innerHTML = rows.join('') || `<div class="lock-none">${lockInfo.cards.length ? 'Plug in ' + lockInfo.cards.map(k => esc(k.name)).join(', or ') + '.' : 'No key has been added yet. Once the app is open, add one in App settings › Lock.'}</div>`;
        $$('#lock-keys form').forEach(f => { f.onsubmit = async (ev) => { ev.preventDefault(); if (busy) return; const inp = f.querySelector('input'), pin = inp.value;
          if (OTP.test(pin)) { inp.value = ''; return lockSaid('That was the key typing a one-time code, not a PIN. Nothing was sent to the key and no PIN try was used.'); }
          if (pin.length < 6) return lockSaid('A key PIN has at least 6 characters.');
          busy = true; lockSaid(''); f.querySelector('button').disabled = true;
          try { opened(await invoke('lock_open_card', { reader: f.dataset.reader, pin })); } catch (e) { inp.value = ''; lockSaid(String(e)); } finally { busy = false; if (f.isConnected) f.querySelector('button').disabled = false; } }; });
      }
      setTimeout(keys, 2000); };
    keys(); withMac();   // the system prompt comes up by itself; the keys are there if it is cancelled or cannot answer
  });
}
async function lockNow() { try { await invoke('lock_now'); } catch {} location.reload(); }   // a reload forgets everything the window knew
function idleLock() {
  clearTimeout(idleLock.t); const m = lockInfo && lockInfo.sealed ? lockInfo.idle_minutes : 0; if (!m) return;
  idleLock.t = setTimeout(() => { if (renderSign.signing) return idleLock(); lockNow(); }, m * 60000);
}
for (const ev of ['pointerdown', 'keydown', 'wheel']) document.addEventListener(ev, () => idleLock(), { passive: true, capture: true });
function renderLock() {
  const card = $('#lock-card'); if (!lockInfo || !lockInfo.sealed) { card.hidden = true; return; } card.hidden = false;
  $('#lock-state').textContent = 'Vaults, labels, notes, contacts and attachments are encrypted on this computer. The key is in this Mac\'s Keychain' + (lockInfo.cards.length ? `, and ${lockInfo.cards.length === 1 ? 'one of your keys' : lockInfo.cards.length + ' of your keys'} can open the app without it.` : '. No key of yours can open the app yet: add one, so a lost Keychain is not a lost address book.');
  $('#lock-state').classList.toggle('warn', !lockInfo.cards.length);
  $('#lock-screen').classList.toggle('on', lockInfo.with_screen !== false); $('#lock-screen').setAttribute('aria-checked', lockInfo.with_screen !== false ? 'true' : 'false');
  $('#lock-touch').classList.toggle('on', lockInfo.touch); $('#lock-touch').setAttribute('aria-checked', lockInfo.touch ? 'true' : 'false'); $('#lock-idle').value = String(lockInfo.idle_minutes);
  const plugged = (cards || []).filter(c => c.pubkey && !lockInfo.cards.some(k => k.pubkey === c.pubkey));
  const nameOf = (c) => { for (const v of store.vaults) { const k = v.keys.find(k => k.pubkey === c.pubkey); if (k) return k.name; } return 'key ' + (c.serial || ''); };
  $('#lock-slots').innerHTML = lockInfo.cards.map(k => `<div class="slot"><span><b>${esc(k.name)}</b><small>added ${new Date(k.added * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })}</small></span><button class="btn small" type="button" data-slot-x="${esc(k.pubkey)}">Remove</button></div>`).join('')
    + plugged.map((c, i) => `<form class="slot add" data-reader="${esc(c.reader)}"><span><b>${esc(nameOf(c))}</b><small>plugged in</small></span><span class="row" style="gap:8px"><input type="password" autocomplete="off" placeholder="Key PIN" id="lock-add-pin-${i}" aria-label="PIN of ${esc(nameOf(c))}"><button class="btn small" type="submit">Let it open the app</button></span></form>`).join('')
    + (!lockInfo.cards.length && !plugged.length ? '<div class="muted" style="font-size:13px">Plug in a key to add it.</div>' : '');
  $$('#lock-slots [data-slot-x]').forEach(b => askTwice(b, 'This key will no longer open the app on this computer. It still signs and still restores its vaults.', 'Remove it', async () => { try { lockInfo = await invoke('lock_remove_card', { pubkey: b.dataset.slotX }); renderLock(); } catch (e) { fail(e); } }));
  $$('#lock-slots form').forEach(f => { f.onsubmit = async (ev) => { ev.preventDefault(); const inp = f.querySelector('input'), pin = inp.value;
    if (OTP.test(pin)) { inp.value = ''; return toast('That was the key typing a one-time code, not a PIN. Nothing was sent to the key.', 8000); }
    if (pin.length < 6) return toast('A key PIN has at least 6 characters');
    f.querySelector('button').disabled = true;
    try { lockInfo = await invoke('lock_add_card', { reader: f.dataset.reader, pin }); toast('That key can now open this app'); renderLock.sig = null; renderLock(); } catch (e) { inp.value = ''; fail(e); if (f.isConnected) f.querySelector('button').disabled = false; } }; });
}
const lockSet = async (args) => { try { lockInfo = await invoke('lock_settings', args); idleLock(); } catch (e) { fail(e); } renderLock(); };
$('#lock-touch').onclick = () => lockSet({ touch: !lockInfo.touch }); $('#lock-touch').onkeydown = (e) => { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); lockSet({ touch: !lockInfo.touch }); } };
$('#lock-idle').onchange = () => lockSet({ idleMinutes: +$('#lock-idle').value });
$('#lock-screen').onclick = () => lockSet({ withScreen: !lockInfo.with_screen }); $('#lock-screen').onkeydown = (e) => { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); lockSet({ withScreen: !lockInfo.with_screen }); } };
window.__TAURI__.event.listen('app-locked', () => location.reload());   // the key has already left memory; the window forgets too
$('#lock-now').onclick = lockNow;

function rememberVault(v) { try { const i = store.vaults.findIndex(x => x.name === v.name); if (i >= 0) localStorage.setItem('a2k-vault-n', String(i)); } catch {} }
// ---------- profiles: one app, several sealed trees; which one is this session for? ----------
let profilesView = null, profile = null;
const PCLASS = { private: ['private', 'private', 'ring-private'], work: ['demo', 'demo', 'ring-demo'], fiction: ['demo · fictional', 'demo', 'ring-demo'] };
function picHtml(p, size, ring) {
  const pic = p.picture || '', img = pic.startsWith('img:') ? pic.slice(4) : '', g = pic.startsWith('glyph:') ? pic.slice(6) : (p.class === 'private' ? 'lock' : p.class === 'fiction' ? 'entity' : 'keys');
  return `<span class="ic s${size}${ring ? ' ' + PCLASS[p.class][2] : ''}" aria-hidden="true">${img ? `<img alt="" src="data:image/jpeg;base64,${img}">` : `<svg><use href="${glyphRef(g)}"/></svg>`}</span>`;
}
const ptagHtml = (p) => `<span class="ptag ${PCLASS[p.class][1]}">${PCLASS[p.class][0]}</span>`;
function renderProfiles(view) {
  $('#shell').hidden = true; $('#welcome').hidden = true; $('#lock').hidden = true; $('#profiles').hidden = false;
  const when = (t) => t ? 'opened ' + new Date(t * 1000).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: 'numeric', minute: '2-digit' }) : 'never opened';
  const line = (p) => p.class === 'private' ? 'Your own holdings. Asks for Touch ID or a key every time, and will not draw while another display is attached.'
    : p.class === 'fiction' ? 'A made-up person\'s holdings for showing to a room. Nothing in it is real.' : 'Demo vaults for working on the app. Opens on a click.';
  $('#profiles-grid').innerHTML = view.profiles.map(p => `<div class="card pcard${p.start ? ' hot' : ''}" data-profile="${esc(p.id)}">${picHtml(p, 64, true)}<div class="ctitle"><span class="nm">${esc(p.name)}</span>${ptagHtml(p)}</div><div class="muted grow">${line(p)}<br><small>${when(p.last_opened)}</small></div><div class="acts"><button class="btn${p.start ? ' primary' : ''}" data-open="${esc(p.id)}">${p.class === 'private' ? 'Open…' : 'Open'}</button>${p.start ? '<span class="vchip on" aria-pressed="true">Opens on start</span>' : ''}</div></div>`).join('')
    + `<div class="card pcard ghost"><div class="ctitle" style="font-size:16px">New profile</div><div class="muted">Name it, say who it is for, give it a picture later.</div><button class="btn small" id="pn-start">Start</button></div>`
    + `<div class="card pcard ghost"><div class="ctitle" style="font-size:16px">From another device</div><div class="muted">A profile you sync comes here with one of its keys and that key\u2019s PIN.</div><button class="btn small" id="pb-start">Bring it here</button></div>`;
  if (typeof syncBringWire === 'function') syncBringWire();
  // the names and pictures of synced profiles may have changed on another device: ask once, quietly, and draw again
  // only if something did (the picker is never a device behind, and never waits on the network to appear)
  if (!renderProfiles.asked) { renderProfiles.asked = true;
    invoke('profiles_refresh').then((fresh) => { const sig = (v) => JSON.stringify(v.profiles.map(p => [p.id, p.name, p.picture]));
      if (fresh && sig(fresh) !== sig(view) && !$('#profiles').hidden) { profilesView = fresh; renderProfiles(fresh); } }).catch(() => {}); }
  $$('#profiles-grid [data-open]').forEach(b => b.onclick = () => openProfile(b.dataset.open));
  $('#pn-start').onclick = () => { $('#profile-new').hidden = false; $('#pn-name').focus(); pnNote(); };
}
const pnClass = () => ($('#pn-class .opt.on') || {}).dataset?.class || 'work';
function pnNote() { $('#pn-note').textContent = { work: 'Opens on a click. Touch ID can be asked for in App settings › Lock once it is open.', fiction: 'Opens on a click and carries a DEMO · fictional tag wherever its name is shown. A persona can be generated into it from App settings.',
  private: 'Made only by the person at this Mac: Touch ID first. Always asked when it opens, never opens on start, uncapturable on screen, and closed while another display is attached.' }[pnClass()]; }
$('#pn-class').addEventListener('click', (e) => { const o = e.target.closest('.opt'); if (!o) return; $$('#pn-class .opt').forEach(x => x.classList.toggle('on', x === o)); pnNote(); });
$('#pn-cancel').onclick = () => { $('#profile-new').hidden = true; };
$('#pn-create').onclick = async () => { const name = $('#pn-name').value.trim(); if (!name) return toast('A profile needs a name');
  $('#pn-create').disabled = true;
  try { const p = await invoke('profile_create', { name, class: pnClass(), picture: null }); $('#profile-new').hidden = true; $('#pn-name').value = ''; await openProfile(p.id); } catch (e) { fail(e); } finally { $('#pn-create').disabled = false; } };
async function openProfile(id) {
  try { lockInfo = await invoke('profile_open', { id }); } catch (e) { return fail(e); }
  profilesView = await invoke('profiles_list').catch(() => profilesView);
  profile = (profilesView && profilesView.current) || (profilesView && profilesView.profiles.find(p => p.id === id)) || null;
  $('#profiles').hidden = true;
  await bootProfile();
}
async function switchProfile(id) { try { await invoke('profile_open', { id }); } catch (e) { return fail(e); } location.reload(); }   // the key left memory in profile_open; the window forgets too
async function closeProfile() { try { await invoke('profile_close'); } catch {} location.reload(); }
function paintProfileChip() {
  const chip = $('#profile-chip'); if (!profile) { chip.hidden = true; return; }
  chip.hidden = false; chip.innerHTML = `${picHtml(profile, 22, true)}<span class="nm">${esc(profile.name)}</span>${ptagHtml(profile)}`;
  const dc = $('#discreet-chip'); dc.hidden = profile.class !== 'private';
}
// the same chip on the screens outside the shell (welcome, pull), with the same menu
function paintCornerChip(id) {
  const box = $('#' + id); if (!box) return; if (!profile) { box.innerHTML = ''; return; }
  box.innerHTML = `<button type="button" class="pchip" aria-haspopup="true" aria-expanded="false" title="Profile">${picHtml(profile, 22, true)}<span class="nm">${esc(profile.name)}</span>${ptagHtml(profile)}</button>`;
  box.querySelector('.pchip').onclick = (e) => { e.stopPropagation(); const m = $('#profile-menu'); const open = m.hidden; if (open) menuUnder(e.currentTarget); m.hidden = !open; e.currentTarget.setAttribute('aria-expanded', String(open)); };
}
function paintProfileMenu() {
  const m = $('#profile-menu'), others = (profilesView ? profilesView.profiles : []);
  m.innerHTML = others.map(p => `<button type="button" class="pm" role="menuitem" data-switch="${esc(p.id)}" aria-current="${p.id === profile.id}">${picHtml(p, 22, false)}<span><span class="nm">${esc(p.name)}</span><small>${PCLASS[p.class][0]}${p.id === profile.id ? ' · open now' : ''}</small></span></button>`).join('')
    + `<div class="sep"></div><button type="button" class="pm" role="menuitem" id="pm-all">All profiles…<small>back to the picker; this one locks</small></button>`;
  $$('[data-switch]', m).forEach(b => { if (b.getAttribute('aria-current') !== 'true') b.onclick = () => switchProfile(b.dataset.switch); });
  $('#pm-all').onclick = closeProfile;
}
function menuUnder(anchor) { const m = $('#profile-menu'); paintProfileMenu(); const r = anchor.getBoundingClientRect(); m.style.left = Math.max(12, Math.min(r.right - 280, window.innerWidth - 292)) + 'px'; m.style.top = (r.bottom + 6) + 'px'; }
$('#profile-chip').onclick = (e) => { e.stopPropagation(); const m = $('#profile-menu'); const open = m.hidden; if (open) menuUnder(e.currentTarget); m.hidden = !open; $('#profile-chip').setAttribute('aria-expanded', String(open)); };
document.addEventListener('click', (e) => { const m = $('#profile-menu'); if (!m.hidden && !e.target.closest('#profile-menu')) { m.hidden = true; $('#profile-chip').setAttribute('aria-expanded', 'false'); } });
$('#lock-switch').onclick = closeProfile;

// ---------- the private profile on a screen: uncapturable, curtained while another display is attached, discreet ----------
let curtainUntil = 0;
const isPrivate = () => !!(profile && profile.class === 'private');
function curtainCheck(d) {
  if (!isPrivate()) { $('#curtain').hidden = true; return; }
  const risky = d && (d.count > 1 || d.mirrored);
  const show = risky && Date.now() > curtainUntil;
  $('#curtain').hidden = !show;
  if (show) { $('#curtain-pic').innerHTML = picHtml(profile, 88, true); $('#curtain-say').textContent = (d.mirrored ? 'A mirrored display is attached' : `${d.count} displays are attached`) + ': a projector, a room\'s TV, or a shared screen. This private profile stays closed while it is. Unplug it, or show anyway for ten minutes with Touch ID.'; }
  if (risky && !show) { clearTimeout(curtainCheck.t); curtainCheck.t = setTimeout(() => invoke('guard_displays').then(curtainCheck).catch(() => {}), Math.max(1000, curtainUntil - Date.now() + 50)); }
}
$('#curtain-anyway').onclick = async () => { try { await invoke('guard_override'); curtainUntil = Date.now() + 10 * 60000; curtainCheck(await invoke('guard_displays')); } catch (e) { fail(e); } };
$('#curtain-switch').onclick = closeProfile;
async function applyGuard() {
  try { await invoke('guard_sharing', { none: isPrivate() }); } catch {}
  if (isPrivate()) { try { curtainCheck(await invoke('guard_displays')); } catch {} }
}
window.__TAURI__.event.listen('displays-changed', (ev) => { if (!isPrivate()) return; if (ev.payload && (ev.payload.count > 1 || ev.payload.mirrored)) { curtainUntil = 0; lockNow(); } else curtainCheck(ev.payload); });   // a display appearing re-locks, override or not: the next thing drawn is the lock screen, then the curtain
function setDiscreet(on) {
  document.body.classList.toggle('discreet', !!on); document.body.classList.remove('peek');
  const c = $('#discreet-chip'); c.classList.toggle('on', !!on); c.setAttribute('aria-pressed', on ? 'true' : 'false');
}
$('#discreet-chip').onclick = async () => { const on = !document.body.classList.contains('discreet'); setDiscreet(on); if (profile) { try { profile = await invoke('profile_update', { id: profile.id, discreet: on }); } catch {} } };
// hold the page to read it; let go and it blurs again
let peekT = null;
document.addEventListener('pointerdown', (e) => { if (!document.body.classList.contains('discreet') || !e.target.closest('.content, #vault-tabs, #title')) return; peekT = setTimeout(() => document.body.classList.add('peek'), 350); }, true);
for (const ev of ['pointerup', 'pointercancel']) document.addEventListener(ev, () => { clearTimeout(peekT); document.body.classList.remove('peek'); }, true);
// what a private profile copies does not sit on the clipboard for the rest of the day
{ const wt = navigator.clipboard.writeText.bind(navigator.clipboard);
  navigator.clipboard.writeText = (t) => wt(t).then((r) => { if (isPrivate()) setTimeout(async () => { try { if ((await navigator.clipboard.readText()) === t) await wt(''); } catch {} }, 60000); return r; }); }

async function boot() {
  try { profilesView = await invoke('profiles_list'); } catch (e) { profilesView = null; boot.profilesFailed = String(e); }
  if (profilesView) {
    profile = profilesView.current || null;
    if (!profile) { const auto = profilesView.profiles.find(p => p.start && p.class !== 'private'); if (auto) { try { lockInfo = await invoke('profile_open', { id: auto.id }); profile = auto; } catch (e) { fail(e); } } }
    if (!profile) { renderProfiles(profilesView); return; }
  }
  await bootProfile();
}
async function bootProfile() {
  paintProfileChip(); $('#lock-switch').hidden = !profilesView;
  try { lockInfo = await invoke('lock_status'); } catch { lockInfo = null; }
  if (lockInfo && lockInfo.sealed && !lockInfo.unlocked) await lockScreen();
  else if (lockInfo && !lockInfo.finished) { try { lockInfo = await invoke('lock_start'); } catch (e) { boot.sealFailed = String(e); } }
  store = await invoke('store_get');
  try { osLocale = await invoke('device_locale'); } catch {}   // without it the language list and the clock still answer
  paintNet();
  // which vault was open last is remembered by its POSITION: the window's own storage is not sealed, and a vault's name says things
  let want = -1; try { want = +localStorage.getItem('a2k-vault-n'); localStorage.removeItem('a2k-vault'); } catch {}
  vault = store.vaults[want] || store.vaults[0] || null;
  paintNet();
  let pullTo = null; try { pullTo = sessionStorage.getItem('a2k-pull-to'); } catch {}
  const pullDest = pullTo && profile && profilesView ? profilesView.profiles.find(p => p.id === pullTo) : null;
  if (pullDest && profile && pullDest.id !== profile.id) { renderPull(pullDest); return; }
  if (pullTo) { try { sessionStorage.removeItem('a2k-pull-to'); } catch {} }   // back where the asking came from: the parcel, if any, lands below
  if (vault) show('home'); else showWelcome();
  pollCards(); idleLock(); invoke('location_status').catch(() => {});   // the setting lives in the sealed store, so listening can only start now
  await applyGuard(); setDiscreet(isPrivate() && profile.discreet);
  // a fiction profile made before personas existed, or emptied: fill it the first time it opens
  if (profile && profile.class === 'fiction' && !profile.persona_seed && !store.vaults.length) {
    try { const made = await invoke('persona_generate', { seed: null }); store = await invoke('store_get'); vault = store.vaults[0] || null; profilesView = await invoke('profiles_list').catch(() => profilesView); profile = (profilesView && profilesView.current) || profile; paintNet(); renderTabs(); if (vault) show('home');
      notice({ kind: 'confirmed', title: `${made.name} is here`, body: `A made-up person for this profile: ${made.entities} entities, ${made.vaults.length} vaults and ${sats(made.exchange_rows)} exchange rows. Change the seed in App settings for a different one.` }, 14000); } catch (e) { fail(e); } }
  if (profile) { try { const landed = []; for (let i = 0; i < 64; i++) { const l = await invoke('move_land'); if (!l) break; landed.push(l); }
    if (landed.length) { store = await invoke('store_get'); vault = store.vaults.find(v => v.name === landed[0].vault) || vault; const from = (profilesView.profiles.find(p => p.id === landed[0].from) || {}).name || 'another profile';
      const names = landed.map(l => `“${l.vault}”`).join(', '), back = landed.every(l => l.back);
      notice({ kind: 'confirmed', title: back ? `${names} came back` : landed.length === 1 ? `${names} is now here` : `${landed.length} vaults are now here`, body: back ? `On the way to ${from} when the app quit, so returned to the profile ${landed.length === 1 ? 'it' : 'they'} left.` : `${names} from ${from}, with labels, notes, attachments and links.` }, 14000); renderTabs(); if (vault) show('home'); } } catch (e) { fail(e); } }
  if (profilesView && profilesView.moved && profilesView.moved.moved) notice({ kind: 'confirmed', title: 'Your files now live in the profile “Me”', body: 'Everything this app kept is in a private profile of its own. Demo wallets can move to a profile of their own from Vault settings.' }, 14000);
  const moved = lockInfo && lockInfo.moved;
  if (boot.sealFailed) notice({ kind: 'warn', title: 'This app\'s files are not encrypted yet', body: boot.sealFailed });
  else if (moved && (moved.store || moved.attachments)) notice({ kind: 'confirmed', title: 'This app\'s files are now encrypted', body: `Vaults, labels, notes and contacts${moved.attachments ? `, and ${moved.attachments} attachment${moved.attachments === 1 ? '' : 's'}` : ''}. Add one of your keys in App settings › Lock, so the app can be opened without this Mac's Keychain.` }, 30000);
}

// ---------- fiat: which currency, worked out from the device unless the person chose ----------
const CCYS = { AUD: 'Australian dollar', USD: 'US dollar', EUR: 'Euro', GBP: 'Pound sterling', NZD: 'New Zealand dollar', CAD: 'Canadian dollar', CHF: 'Swiss franc', JPY: 'Japanese yen', SGD: 'Singapore dollar', HKD: 'Hong Kong dollar',
  SEK: 'Swedish krona', NOK: 'Norwegian krone', DKK: 'Danish krone', PLN: 'Polish zloty', CZK: 'Czech koruna', HUF: 'Hungarian forint', RON: 'Romanian leu', ISK: 'Icelandic krona', TRY: 'Turkish lira', ILS: 'Israeli shekel', ZAR: 'South African rand',
  BRL: 'Brazilian real', MXN: 'Mexican peso', ARS: 'Argentine peso', INR: 'Indian rupee', IDR: 'Indonesian rupiah', MYR: 'Malaysian ringgit', PHP: 'Philippine peso', THB: 'Thai baht', KRW: 'South Korean won', TWD: 'New Taiwan dollar', CNY: 'Chinese yuan', MAD: 'Moroccan dirham', DZD: 'Algerian dinar', RUB: 'Russian rouble' };
const EURO = 'AT BE CY DE EE ES FI FR GR HR IE IT LT LU LV MT NL PT SI SK BG AD MC SM VA ME XK'.split(' ');
const REGION_CCY = Object.assign({ AU: 'AUD', US: 'USD', GB: 'GBP', NZ: 'NZD', CA: 'CAD', CH: 'CHF', LI: 'CHF', JP: 'JPY', SG: 'SGD', HK: 'HKD', SE: 'SEK', NO: 'NOK', DK: 'DKK', PL: 'PLN', CZ: 'CZK', HU: 'HUF', RO: 'RON', IS: 'ISK', TR: 'TRY', IL: 'ILS', ZA: 'ZAR',
  BR: 'BRL', MX: 'MXN', AR: 'ARS', IN: 'INR', ID: 'IDR', MY: 'MYR', PH: 'PHP', TH: 'THB', KR: 'KRW', TW: 'TWD', CN: 'CNY', MA: 'MAD', DZ: 'DZD', RU: 'RUB' }, Object.fromEntries(EURO.map(r => [r, 'EUR'])));
// a timezone says where the computer IS; good enough to name a country for the currencies we can price
const ZONE_REGION = [[/^Australia\//, 'AU'], [/^Pacific\/(Auckland|Chatham)$/, 'NZ'], [/^Europe\/(London|Belfast|Jersey|Guernsey|Isle_of_Man)$/, 'GB'], [/^Europe\/Zurich$/, 'CH'], [/^Europe\/Stockholm$/, 'SE'], [/^Europe\/Oslo$/, 'NO'], [/^Europe\/Copenhagen$/, 'DK'],
  [/^Europe\/Warsaw$/, 'PL'], [/^Europe\/Prague$/, 'CZ'], [/^Europe\/Budapest$/, 'HU'], [/^Europe\/Bucharest$/, 'RO'], [/^Europe\/Istanbul$/, 'TR'], [/^Atlantic\/Reykjavik$/, 'IS'], [/^Europe\/(Moscow|Samara|Volgograd|Kaliningrad)$/, 'RU'], [/^Europe\//, 'DE'],
  [/^Asia\/Tokyo$/, 'JP'], [/^Asia\/Seoul$/, 'KR'], [/^Asia\/Hong_Kong$/, 'HK'], [/^Asia\/Taipei$/, 'TW'], [/^Asia\/(Shanghai|Chongqing|Urumqi)$/, 'CN'], [/^Asia\/Singapore$/, 'SG'], [/^Asia\/Bangkok$/, 'TH'], [/^Asia\/Kuala_Lumpur$/, 'MY'], [/^Asia\/Manila$/, 'PH'],
  [/^Asia\/(Jakarta|Makassar|Jayapura)$/, 'ID'], [/^Asia\/(Kolkata|Calcutta)$/, 'IN'], [/^Asia\/(Jerusalem|Tel_Aviv)$/, 'IL'], [/^Africa\/Johannesburg$/, 'ZA'], [/^Africa\/Casablanca$/, 'MA'], [/^Africa\/Algiers$/, 'DZ'], [/^America\/(Sao_Paulo|Manaus|Bahia|Fortaleza|Recife|Belem)$/, 'BR'],
  [/^America\/(Mexico_City|Monterrey|Cancun|Tijuana|Merida)$/, 'MX'], [/^America\/(Argentina\/|Buenos_Aires)/, 'AR'], [/^America\/(Toronto|Vancouver|Edmonton|Winnipeg|Halifax|St_Johns|Regina|Moncton|Whitehorse)$/, 'CA'],
  [/^America\/(New_York|Chicago|Denver|Los_Angeles|Phoenix|Anchorage|Detroit|Boise|Indiana\/|Kentucky\/)|^Pacific\/Honolulu$/, 'US']];
let osLocale = { region: null, currency: null };   // the operating system's Region setting, asked for once at start
function deviceCurrency(os = osLocale, langs = navigator.languages || [navigator.language], zone = (() => { try { return Intl.DateTimeFormat().resolvedOptions().timeZone; } catch { return ''; } })()) {
  // 1. the OS region: deliberate, does not move when the person travels, independent of the language they read
  if (os && os.currency && CCYS[os.currency]) return { ccy: os.currency, why: `this computer's currency is set to ${os.currency}` };
  if (os && os.region) return { ccy: REGION_CCY[os.region] || 'USD', why: REGION_CCY[os.region] ? `this computer's region is ${os.region}` : `there is no price for the currency of ${os.region}, so US dollars` };
  // 2. the language list's region; 3. the clock, which moves with a traveller, so only when nothing else speaks
  let region = ''; for (const l of langs) { try { const r = new Intl.Locale(l).region; if (r) { region = r; break; } } catch {} }
  const zoned = (ZONE_REGION.find(([re]) => re.test(zone || '')) || [])[1] || '';
  // en-US is what a Mac ships with everywhere; a clock set to another country is the better clue
  if (zoned && (!region || (region === 'US' && zoned !== 'US'))) return { ccy: REGION_CCY[zoned] || 'USD', why: `this computer's clock is set to ${zone}` };
  if (region) return { ccy: REGION_CCY[region] || 'USD', why: REGION_CCY[region] ? `this computer's language region is ${region}` : `no price for the currency of ${region}, so US dollars` };
  return { ccy: 'USD', why: 'this computer names no region' };
}
const fiatCcy = () => { const f = (store.settings.fiat ?? 'auto'); return f === 'auto' ? deviceCurrency().ccy : f; };
// a figure the exchange wrote, to the cent: never rounded the way a worth is
const moneyExact = (amount, ccy) => { try { return new Intl.NumberFormat(undefined, { style: 'currency', currency: ccy, minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(amount); } catch { return amount.toFixed(2) + ' ' + ccy; } };
const money = (amount, ccy) => { try { return new Intl.NumberFormat(undefined, { style: 'currency', currency: ccy, maximumFractionDigits: Math.abs(amount) >= 100 ? 0 : 2 }).format(amount); } catch { return amount.toFixed(2) + ' ' + ccy; } };
const worth = (satsN, q, ccy) => money(satsN / 1e8 * q.price, ccy);
const basisWords = (q, ccy) => `1 BTC = ${money(q.price, ccy)} · ${q.basis} · Bitstamp price × ECB reference rate`;

// ---------- cards ----------
async function pollCards() {
  try { cards = await invoke('cards_list'); cardsErr = ''; } catch (e) { cards = []; cardsErr = String(e); }
  try { otherDevices = await invoke('cards_other'); } catch { otherDevices = []; }
  // A key's shape, and its firmware, are remembered the first time they are seen. Firmware in particular can
  // never change on an existing YubiKey, so a version once seen answers for that key even when it is not plugged in.
  for (const c of cards) {
    const formNew = c.form && packedForm(c.pubkey) !== ((c.form & 0x0F) | 0x40 | (c.nfc ? 0x80 : 0));
    const fwNew = c.firmware && ((store.key_fw || {})[c.pubkey] !== c.firmware);
    if (!c.pubkey || !(formNew || fwNew)) continue;
    try { store = await invoke('key_form_seen', { pubkey: c.pubkey, form: c.form || 0, nfc: !!c.nfc, firmware: c.firmware || null }); renderSign.tiles = null; if ($('#s-keys').classList.contains('on') && !keysBusy()) renderKeys(); } catch {}
  }
  const osig = otherDevices.map(d => d.reader).join('|');
  if (osig !== pollCards.osig) { pollCards.osig = osig; if ($('#s-keys').classList.contains('on')) { renderKeysNow(); paintKeyLife(); } }
  if ($('#s-setup').classList.contains('on')) renderSetup();
  if ($('#s-settings').classList.contains('on')) { const sig = cards.map(c => c.reader + c.pubkey).join('|'); if (sig !== renderLock.sig) { renderLock.sig = sig; renderLock(); } }
  if ($('#s-keys').classList.contains('on')) { const sig = cards.map(c => c.serial + ':' + c.pubkey).sort().join(','); if (sig !== pollCards.sig) { if (!keysBusy()) { pollCards.sig = sig; renderKeys(); } } else renderKeysNow(); }
  if (vault) for (const c of cards) { const k = vault.keys.find(k => c.pubkey && k.pubkey === c.pubkey && !k.serial); if (k && c.serial) { invoke('vault_key_seen', { name: vault.name, pubkey: c.pubkey, serial: c.serial }).then(st => { store = st; vault = st.vaults.find(v => v.name === vault.name) || vault; if ($('#s-keys').classList.contains('on')) renderKeys(); }).catch(() => {}); } }
  if ($('#s-restore').classList.contains('on') && restoreTab.now === 'keys') readSeals();
  if ($('#s-sign').classList.contains('on')) renderSign();
  if ($('#s-messages').classList.contains('on')) { const c = cards.find(c => c.pubkey); const was = renderMessages.had; renderMessages.had = !!c; if (was !== !!c) renderMessages(); }
  // awaited: the survey talks to the card for a while, and on macOS a poll closing its own connection in the
  // meantime resets the card under it — the key drops out of the list, the offer blinks, and it starts again
  if (typeof keyOfferTick === 'function') { try { await keyOfferTick(); } catch {} }
  setTimeout(pollCards, 2000);
}
function cardLabel(c) { return `${modelName(c.form, c.nfc)} · serial ${c.serial} · ${c.version}`; }

// ---------- home ----------
// the balance leads with sats or with money, whichever the person last clicked it to; the other sits underneath
let balUnit = 'sats'; try { balUnit = localStorage.getItem('a2k-balance-unit') || 'sats'; } catch {}
function paintBalance() {
  if (!vault || vault._balance === undefined) return;
  const n = vault._now, big = $('#balance'), small = $('#balance-fiat');
  big.classList.toggle('switch', !!n); big.title = n ? 'Click to lead with ' + (balUnit === 'fiat' ? 'sats' : n.ccy) : '';
  if (n && balUnit === 'fiat') { big.innerHTML = `${worth(vault._balance, n.q, n.ccy)} <small>${n.ccy}</small>`; small.textContent = `${sats(vault._balance)} sats`; }
  else { big.innerHTML = `${sats(vault._balance)} <small>sats</small>`; small.textContent = n ? `≈ ${worth(vault._balance, n.q, n.ccy)}` : ''; }
  small.title = n ? basisWords(n.q, n.ccy) : ''; small.hidden = !n;
}
$('#balance').onclick = () => { if (!vault || !vault._now) return; balUnit = balUnit === 'fiat' ? 'sats' : 'fiat'; try { localStorage.setItem('a2k-balance-unit', balUnit); } catch {} paintBalance(); };
$('#balance').addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); $('#balance').click(); } });
// ---------- one transaction ----------
const txFacts = {};                        // vault name → { rows: { txid: {time, net, fee, quote} }, now, ccy }
const txView = { txid: null, height: 0, seq: 0 };
// The list of a vault's transactions, newest first, kept by vault NAME. It used to be read off `vault._history`, and the vault
// object is replaced on every store re-read: the height came back 0 and a transaction confirmed days ago read "in the mempool".
const txList = {};
const txOrder = () => (txList[vault.name] || []);
function openTx(txid) { const h = txOrder().find(x => x.txid === txid); txView.txid = txid; txView.height = h ? h.height : 0; show('tx'); }
function stepTx(by) { const l = txOrder(), i = l.findIndex(x => x.txid === txView.txid); if (i < 0 || !l[i + by]) return; clearTimeout(saveTxField.t); saveTxField('note'); openTx(l[i + by].txid); }
const txHeadline = () => { const f = ((txFacts[vault.name] || {}).rows || {})[txView.txid], lab = labelOf('tx:' + txView.txid);
  if (lab) return lab; if (!f || f.net === null) return txView.txid.slice(0, 10) + '…';
  return (f.time ? new Date(f.time * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short' }) + ' · ' : '') + (f.net > 0 ? '+' : f.net < 0 ? '−' : '') + sats(Math.abs(f.net)) + ' sats'; };
async function renderTx() {
  if (!vault || !txView.txid) return show('home');
  const name = vault.name, txid = txView.txid, mine = ++txView.seq, key = 'tx:' + txid;
  recPoint('tx', key);
  const facts = txFacts[name] || {}, f = (facts.rows || {})[txid];
  paintCrumbs('tx'); $('#title').textContent = txHeadline();
  { const l = txOrder(), i = l.findIndex(x => x.txid === txid); $('#tx-prev').disabled = i <= 0; $('#tx-next').disabled = i < 0 || i >= l.length - 1; $('#tx-pos').textContent = i >= 0 ? `${i + 1} of ${l.length}` : ''; }
  // what the activity list already knows is on screen at once; the decoded transaction fills the rest in
  const amt = $('#tx-amt'); amt.className = 'tx-amt' + (f && f.net > 0 ? ' in' : ''); amt.innerHTML = f && f.net !== null ? `${f.net > 0 ? '+' : f.net < 0 ? '−' : ''}${sats(Math.abs(f.net))} <small>sats</small>` : '…';
  const fiat = $('#tx-fiat');
  fiat.hidden = !(f && f.net !== null && (f.quote || facts.now));
  if (!fiat.hidden) { fiat.textContent = [f.quote ? worth(Math.abs(f.net), f.quote, facts.ccy) + ' then' : '', facts.now ? worth(Math.abs(f.net), facts.now, facts.ccy) + ' now' : ''].filter(Boolean).join(' · '); fiat.title = f.quote ? basisWords(f.quote, facts.ccy) : ''; }
  $('#tx-when').textContent = f && f.time ? new Date(f.time * 1000).toLocaleString(undefined, { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric', hour: 'numeric', minute: '2-digit' }) : (txView.height > 0 ? '' : 'Not in a block yet');
  if (document.activeElement !== $('#tx-label')) $('#tx-label').value = labelOf(key);
  if (document.activeElement !== $('#tx-note')) $('#tx-note').value = (vault.notes || {})[key] || '';
  fitNote(); if (renderTx.last !== name + txid) { renderTx.last = name + txid; recSaid(''); }
  renderFiles(); renderCtx();
  $('#tx-chain').open = (() => { try { return localStorage.getItem('a2k-chain-open') === '1'; } catch { return false; } })();
  const offchain = !!(vault.spark || vault.statement || vault.exchange || (vault.node && !(nodeRow(txid) || {}).txid));
  $('#tx-explorer').hidden = offchain; $('#tx-pdf').hidden = !!(vault.spark || vault.statement || vault.node || vault.exchange); $('#tx-chain').hidden = offchain;
  if (vault.node) {
    const e = nodeRow(txid), st = $('#tx-status'); st.className = 'state ok'; st.querySelector('span').textContent = e ? `${e.layer === 'chain' ? 'on-chain' : e.layer === 'fee' ? 'fee' : 'Lightning'} \u00b7 ${e.kind} \u00b7 from the node` : 'from the node';
    $('#tx-dl').innerHTML = [['Transfer', transferText(facts, txid) || 'Outside the watched vaults'], ['Source', `${esc(vault.node.alias)}\u2019s bookkeeper \u2014 the node\u2019s own ledger`],
      e ? ['Kind', e.kind === 'routed' ? 'routed \u2014 a routing fee earned, income' : e.kind === 'received' ? 'received \u2014 a cost only if it was a purchase' : e.kind] : null,
      e ? ['Amount', `${sats(Math.abs(e.net))} sats${e.fee ? ` (fee ${sats(e.fee)} sats)` : ''}`] : null,
      e && e.txid ? ['Transaction', `<a href="#" class="mono" data-go-tx="${esc(e.txid)}" style="font-size:12px;word-break:break-all">${esc(e.txid)}</a>`] : null,
      e && e.hash ? ['Payment hash', `<span class="mono" style="font-size:12px;word-break:break-all">${esc(e.hash)}</span>`] : null,
      e && e.memo ? ['Memo', esc(e.memo)] : null].filter(Boolean).map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
    $('#tx-proof').textContent = ''; $('#tx-chain-body').innerHTML = ''; return;
  }
  if (vault.exchange) {
    const r = exRow(txid), st = $('#tx-status'), ex = vault.exchange; st.className = 'state ok'; st.querySelector('span').textContent = r ? `${EX_WORDS[r.kind] || r.kind} \u00b7 from the statement` : 'from the statement';
    const price = r ? exRealPrice(r) : null;
    $('#tx-dl').innerHTML = [['Transfer', transferText(facts, txid) || (r && r.kind === 'btc_out' ? 'Not yet linked to a vault it landed in' : 'Within the exchange')], ['Source', `${esc(ex.provider)}\u2019s statement \u2014 the exchange\u2019s word; nothing on the chain to check it against`], r ? ['Kind', esc(exKindWords(r))] : null,
      r && r.sats ? ['Coins', `${sats(Math.abs(r.sats))} sats${r.fee_sats ? ` + ${sats(r.fee_sats)} sats fee` : ''}`] : null,
      r && (r.fiat || r.fee_fiat) ? ['Money', `${moneyExact(Math.abs(r.fiat) / 100, ex.ccy)}${r.fee_fiat ? ` + ${moneyExact(r.fee_fiat / 100, ex.ccy)} fee` : ''}`] : null,
      price ? ['Price', `${moneyExact(price, ex.ccy)} per bitcoin, fee included${r.rate ? ` \u00b7 the exchange quoted ${moneyExact(r.rate / 100, ex.ccy)}` : ''}`] : null,
      r && r.memo ? ['As written', esc(r.memo)] : null].filter(Boolean).map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
    $('#tx-proof').textContent = ''; $('#tx-chain-body').innerHTML = ''; return;
  }
  if (vault.statement) {
    const r = stmtRow(txid), st = $('#tx-status'); st.className = 'state ok'; st.querySelector('span').textContent = r ? `${r.layer === 'chain' ? 'on-chain' : 'Lightning'} \u00b7 from the statement` : 'from the statement';
    const kind = !r ? '' : r.kind === 'income' ? (r.pos ? 'a sale through the point of sale \u2014 income' : 'a reward \u2014 income') : r.kind === 'received' ? 'received \u2014 a cost only if it was a purchase' : 'paid';
    $('#tx-dl').innerHTML = [['Transfer', transferText(facts, txid) || 'Outside the watched vaults'], ['Source', `${esc(vault.statement.source)}\u2019s statement \u2014 the custodian\u2019s word; nothing on the chain to check it against`], ['Kind', kind],
      r ? ['Amount', `${sats(Math.abs(r.net) - r.fee)} sats${r.fee ? ` + ${sats(r.fee)} sats fee` : ''}`] : null,
      r && r.address ? ['Address', `<span class="mono">${esc(r.address)}</span>`] : null, r && r.hash ? ['Payment hash', `<span class="mono" style="font-size:12px;word-break:break-all">${esc(r.hash)}</span>`] : null,
      r && r.memo ? ['Memo', esc(r.memo)] : null].filter(Boolean).map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
    $('#tx-proof').textContent = ''; $('#tx-chain-body').innerHTML = ''; return;
  }
  if (vault.spark) {
    const t = facts.spark?.[txid], st = $('#tx-status'); st.className = 'state ' + (t?.status === 5 ? 'ok' : 'here'); st.querySelector('span').textContent = sparkTransferStatus(t?.status);
    $('#tx-dl').innerHTML = [['Transfer', transferText(facts, txid) || 'Outside the watched vaults'], ['Source', "Spark's word — not verified against the chain"], ['Transfer id', txid], ['From identity', t?.sender || 'unknown'], ['To identity', t?.receiver || 'unknown'], ['Amount', t ? sats(t.sats) + ' sats' : 'unknown'], ['Accounting', t?.status === 5 ? 'Completed transfer included' : 'Not completed; excluded from cost basis']].map(([k,v]) => `<dt>${esc(k)}</dt><dd class="mono">${esc(v)}</dd>`).join('');
    $('#tx-proof').textContent = ''; $('#tx-chain-body').innerHTML = ''; return;
  }
  let d; try { d = await invoke('tx_detail', { name, txid, height: txView.height }); } catch (e) { if (mine === txView.seq) { $('#tx-dl').innerHTML = `<dt>Details</dt><dd class="warn">${esc(e)}</dd>`; $('#tx-proof').textContent = ''; } return; }
  if (mine !== txView.seq || !vault || vault.name !== name) return;   // the person has moved on; this answer is for another screen
  const conf = d.height > 0 ? Math.max(1, d.tip - d.height + 1) : 0, st = $('#tx-status');
  st.className = 'state ' + (conf ? 'ok' : 'here'); st.querySelector('span').textContent = conf ? `confirmed · ${sats(conf)} block${conf === 1 ? '' : 's'} deep` : 'in the mempool';
  if (!f) { amt.className = 'tx-amt' + (d.net > 0 ? ' in' : ''); amt.innerHTML = `${d.net > 0 ? '+' : d.net < 0 ? '−' : ''}${sats(Math.abs(d.net))} <small>sats</small>`; }
  const book = bookOf(), who = (a) => { const c = a && book.find(c => c.address.toLowerCase() === a.toLowerCase()); return c ? `<span class="tag who">${esc(c.name)}</span>` : ''; };
  const pos = (i) => i >= CHANGE ? 'change #' + (i - CHANGE) : 'receive #' + i;
  // this vault's address: a link to its page; an unspent output of ours: a link to the coin; another of this
  // person's vaults: a link to the same transaction there
  const place = (i, a) => a ? `<a href="#" class="tag ours" data-go-addr="${esc(a)}" title="Open this address">${esc(vault.name)} · ${pos(i)}</a>` : `<span class="tag ours">${esc(vault.name)} · ${pos(i)}</span>`;
  const peerTag = (p) => p ? `<a href="#" class="tag peer" data-go-vault-tx="${esc(p[0])}" data-txid="${d.txid}" title="Open this transaction in ${esc(p[0])}">\u21c4 ${esc(p[0])} · ${pos(p[1])}</a>` : '';
  const coinTag = (o) => o.ours !== null && (vault._coins || []).some(c => c.txid === d.txid && c.vout === o.n) ? `<a href="#" class="tag" data-go-coin="${d.txid}:${o.n}" title="Open this coin">coin \u203a</a>` : '';
  const outLine = (o) => `<div><span class="mono">${esc(o.address || o.kind)}</span>${o.ours !== null ? place(o.ours, o.address) + coinTag(o) : o.peer ? peerTag(o.peer) : who(o.address)}<small>${sats(o.sats)} sats</small></div>`;
  const oursIn = d.inputs.filter(i => i.ours !== null), foreignOut = d.outputs.filter(o => o.ours === null), oursOut = d.outputs.filter(o => o.ours !== null);
  const signers = [...new Set(oursIn.flatMap(i => i.signers))];
  const rows = [];
  if (transferText(facts, txid)) rows.push(['Transfer', esc(transferText(facts, txid))]);
  // between two of this person's vaults: said first, with the way to the other end
  if ((d.peers || []).length) rows.push(['Transfer', `${d.net < 0 ? 'to' : d.net > 0 ? 'from' : 'with'} ` + d.peers.map(n => `<a href="#" data-go-vault-tx="${esc(n)}" data-txid="${d.txid}">${esc(n)} \u203a</a>`).join(', ') + `<small>the same transaction, seen from the other vault \u00b7 moving coins between your own vaults is not a disposal${d.fee !== null && d.net < 0 ? `; only the ${sats(d.fee)} sat fee leaves your hands` : ''}</small>`]);
  if (oursIn.length) {            // a spend from this vault
    rows.push(['To', foreignOut.length ? foreignOut.map(outLine).join('') : 'Nowhere outside this vault: the coins moved between its own addresses.']);
    if (oursOut.length) rows.push(['Change', oursOut.map(outLine).join('')]);
    rows.push(['Fee', d.fee !== null ? `${sats(d.fee)} sats<small>${d.fee_rate.toFixed(2)} sat/vB · ${sats(d.vsize)} vB</small>` : 'not known']);
    rows.push(['Signed by', signers.length ? signers.map(esc).join(' and ') + `<small>read from the signatures on the transaction</small>` : 'could not be read from the witness']);
    const rl = realisedRow(name, txid); if (rl) rows.push(['Realised', rl]);
    const mr = marketRow(name, txid); if (mr) rows.push(['Market context', mr]);
  } else {                        // a deposit
    const fromPeer = [...new Set(d.inputs.filter(i => i.peer).map(i => i.peer[0]))];
    rows.push(['From', fromPeer.length ? fromPeer.map(n => `<a href="#" class="tag peer" data-go-vault-tx="${esc(n)}" data-txid="${d.txid}">\u21c4 ${esc(n)}</a>`).join(' ') : `outside this vault · ${d.inputs.length} input${d.inputs.length === 1 ? '' : 's'}`]);
    rows.push(['To', oursOut.map(outLine).join('') || '—']);
    rows.push(['Fee', d.fee !== null ? `${sats(d.fee)} sats, paid by the sender` : 'paid by the sender']);
    const facts = txFacts[name], q = facts && facts.rows[txid] && facts.rows[txid].quote;
    if (q && vault.network === 'mainnet') {
      rows.push(['Cost', `${money(Math.abs(facts.rows[txid].net) / 1e8 * q.price, facts.ccy)}<small>what it was worth as it arrived \u2014 its cost, if it was bought</small>`]);
      // where the price sat the day it arrived, against now: its own row, after the money
      const t = facts.rows[txid].time, cm = t ? (cycleWant([t], renderTx) ? cycleAt(t) : undefined) : null, nm = cycleNowHere();
      if (nm === undefined) cycleNow().then(renderTx);
      rows.push(['Market context', cycleTable({ title: 'When received', sub: t ? cycleDay(t) : 'not in a block yet', mark: cm }, { title: 'Now', sub: cycleStale(nm) ? `as of ${cycleDay(nm.t)}` : '', mark: nm })]); }
  }
  $('#tx-dl').innerHTML = rows.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
  $('#tx-proof').textContent = d.proof === 'proved' ? `the node proved it is in block ${sats(d.height)}` : d.proof === 'unconfirmed' ? 'not in a block yet' : d.proof;
  $('#tx-proof').classList.toggle('warn', d.proof !== 'proved' && d.proof !== 'unconfirmed');
  const kv = [['Transaction id', d.txid], ['Witness id', d.wtxid], ['Version · locktime', `${d.version} · ${d.locktime}`], ['Size', `${sats(d.size)} B · ${sats(d.vsize)} vB · ${sats(d.weight)} WU`], ['Fee', d.fee !== null ? `${sats(d.fee)} sats · ${d.fee_rate.toFixed(2)} sat/vB` : 'unknown (an input could not be fetched)'],
    ['Replaceable (RBF)', d.rbf ? 'signalled' : 'not signalled'], ['Block', d.block ? `${sats(d.height)} · ${d.block}` : '—'], ['Position in block', d.position !== null ? String(d.position) : '—'],
    ['Block time', d.time ? new Date(d.time * 1000).toISOString().replace('.000Z', ' UTC').replace('T', ' ') : '—'], ['Inclusion', d.proof === 'proved' ? 'proved: the server\'s merkle path hashes up to the root in that block\'s header' : d.proof]];
  $('#tx-chain-body').innerHTML = `<dl class="chain-kv">${kv.map(([k, v]) => `<dt>${k}</dt><dd>${esc(v)}</dd>`).join('')}</dl>`
    + `<table class="chain-t"><thead><tr><th>Input</th><th>Address</th><th class="r">Sats</th><th>Sequence</th><th>Witness</th></tr></thead><tbody>${d.inputs.map(i => `<tr><td class="mono">${esc(i.outpoint.slice(0, 12))}…:${esc(i.outpoint.split(':')[1])}</td><td class="mono">${esc(i.address || '—')}${i.ours !== null ? place(i.ours, i.address) : peerTag(i.peer)}</td><td class="r">${i.sats !== null ? sats(i.sats) : '—'}</td><td class="mono">${i.sequence.toString(16)}</td><td>${i.witness} item${i.witness === 1 ? '' : 's'}${i.signers.length ? ' · ' + i.signers.map(esc).join(', ') : ''}</td></tr>`).join('')}</tbody></table>`
    + `<table class="chain-t"><thead><tr><th>Output</th><th>Address</th><th class="r">Sats</th><th>Script</th><th></th></tr></thead><tbody>${d.outputs.map(o => { const unspent = o.ours !== null && (vault._coins || []).some(c => c.txid === d.txid && c.vout === o.n); return `<tr><td>#${o.n}</td><td class="mono">${esc(o.address || '—')}${o.ours !== null ? place(o.ours, o.address) : o.peer ? peerTag(o.peer) : who(o.address)}</td><td class="r">${sats(o.sats)}</td><td>${esc(o.kind)}</td><td class="muted">${o.ours !== null ? (unspent ? 'unspent' : 'spent') : ''}</td></tr>`; }).join('')}</tbody></table>`
    + `<div class="row" style="justify-content:space-between;margin-bottom:var(--s2)"><div class="label">Raw transaction</div><button class="btn small" id="tx-copyhex">Copy hex</button></div><div class="chain-hex" id="tx-hex">${esc(d.hex)}</div>`;
  $('#tx-copyhex').onclick = () => navigator.clipboard.writeText(d.hex).then(() => toast('Raw transaction copied'));
}
// ---------- attachments on the transaction ----------
// ---------- the record card travels ----------
// Label, note and attachments are keyed by a plain string the backend does not interpret, and a record carries the
// same three things whatever it is about. So there is ONE record card: it is moved into whichever screen is asking,
// and everything hung off it — the note that saves itself, drag and drop, the camera, the photo cleaning — goes on
// working because only the key underneath it changes.
let recView = { kind: 'tx', key: '' };
const recKey = () => recView.key;
const recHome = $('#tx-notes').parentElement;
const REC_WORDS = {
  tx:   ['Anything worth keeping with this transaction: what it paid for, an invoice number, who to ask.', 'A few words: who, or what for'],
  addr: ['Anything worth keeping with this address: who you gave it to, what it is for, when you handed it over.', 'A few words: who you gave it to'],
  coin: ['Anything worth keeping with this coin: where it came from, what it is being kept for, why it is not to be spent yet.', 'A few words: where it came from'],
  key:  ['Anything worth keeping with this key: where it is kept, who knows, what its PIN hint is, when it was last checked. Photos of it and its sheet can go in the files.', 'A few words: where it lives'],
  place: ['Anything worth keeping with this place: who has access, how it is secured, what else is there. Photos of it can go in the files.', 'A few words: whose it is, what kind of place'],
};
function recPoint(kind, key, slot) {
  recView = { kind, key };
  const card = $('#tx-notes'), want = slot || recHome;
  if (card.parentElement !== want) want.appendChild(card);
  const [note, label] = REC_WORDS[kind] || REC_WORDS.tx;
  $('#tx-note').placeholder = note; $('#tx-label').placeholder = label;
  if (document.activeElement !== $('#tx-label')) $('#tx-label').value = labelOf(key);
  if (document.activeElement !== $('#tx-note')) $('#tx-note').value = (recHolder(key).notes || {})[key] || '';
  fitNote(); recSaid(''); renderFiles();
}
const thumbCache = new Map();
const sizeWords = (n) => n >= 1048576 ? (n / 1048576).toFixed(1) + ' MB' : n >= 1024 ? Math.round(n / 1024) + ' KB' : n + ' B';
async function renderFiles() {
  if (!recKey()) return;
  const key = recKey(), name = recName(key), list = ((recHolder(key).files || {})[key] || []).slice();
  let here = []; try { here = await invoke('attach_have', { hashes: list.map(e => e.sha256) }); } catch {}
  if (key !== recKey()) return;
  const kind = (e) => (e.mime || '').startsWith('image/') ? 'IMG' : e.mime === 'application/pdf' ? 'PDF' : e.mime === 'text/plain' ? 'TXT' : 'FILE';
  $('#tx-files').innerHTML = list.map((e, i) => `<div class="att${here[i] ? '' : ' away'}" role="button" tabindex="0" data-att="${i}" title="${esc(e.name)}${here[i] ? '' : ' — on another computer; click to fetch it from your backup'}"><div class="pic" data-thumb="${esc(e.thumb || '')}"><span class="doc">${kind(e)}</span></div><div class="nm">${nameParts(e.name)}</div><div class="sz">${here[i] ? sizeWords(e.size) + (e.meta === 'original' && kind(e) === 'IMG' ? ' · as taken' : '') : 'click to fetch'}</div><button type="button" class="att-x" data-att-x="${i}" aria-label="Remove ${esc(e.name)}">×</button></div>`).join('');
  const total = list.reduce((n, e) => n + (e.size || 0), 0);
  $('#tx-files-sum').textContent = list.length ? `${list.length} file${list.length === 1 ? '' : 's'} · ${sizeWords(total)}` : '';
  paintZone(false);
  $$('#tx-files [data-thumb]').forEach(async (pic) => { const h = pic.dataset.thumb; if (!h) return;
    try { if (!thumbCache.has(h)) thumbCache.set(h, await invoke('attach_thumb', { sha256: h })); const img = new Image(); img.alt = ''; img.src = thumbCache.get(h); pic.replaceChildren(img); } catch {} });
  $$('#tx-files [data-att]').forEach(t => { const e = list[+t.dataset.att];
    const openIt = async () => {
      if (t.classList.contains('away')) {   // added on another computer: bring it here from the encrypted backup, prove it, then open it
        if (t.dataset.fetching) return; t.dataset.fetching = '1'; t.querySelector('.sz').textContent = 'fetching from your backup…';
        try { await invoke('attach_fetch', { name, sha256: e.sha256, size: e.size || 1 }); if (e.thumb) { try { await invoke('attach_fetch', { name, sha256: e.thumb, size: 1 }); } catch {} } await renderFiles(); toast(`${e.name} is on this computer now`); }
        catch (err) { delete t.dataset.fetching; t.querySelector('.sz').textContent = 'not on this computer yet'; fail(err); }
        return; }
      // a phone has no viewer to hand a file to: a picture is shown in the window itself (phone.js)
      if (typeof phoneView === 'function' && document.body.matches('.ios, .android')) { try { await phoneView(e); } catch (err) { fail(err); } return; }
      try { toast(`${e.name}: ${await invoke('attach_open', { sha256: e.sha256, name: e.name, mime: e.mime })}`); } catch (err) { fail(err); } };
    t.onclick = (ev) => { if (ev.target.closest('.att-x')) return; openIt(); };
    t.onkeydown = (ev) => { if (ev.key === 'Enter' && !ev.target.closest('.att-x')) openIt(); }; });
  $$('#tx-files [data-att-x]').forEach(b => { const e = list[+b.dataset.attX];
    b.onclick = async (ev) => { ev.stopPropagation();
      if (!b.classList.contains('armed')) { b.classList.add('armed'); b.textContent = 'Remove?'; setTimeout(() => { if (b.isConnected) { b.classList.remove('armed'); b.textContent = '×'; } }, 5000); return; }
      try { store = await invoke('attach_remove', { name, key, sha256: e.sha256 }); vault = store.vaults.find(v => v.name === name) || vault; renderFiles(); } catch (err) { fail(err); } }; });
}
// ---------- where and on what: written once when this computer makes or first sees the transaction ----------
function renderCtx() {
  if (!vault || !txView.txid) return;
  const ctx = (((vault.more || {})['tx:' + txView.txid]) || {}).ctx, gone = !ctx || ctx.removed, dev = (ctx && ctx.device) || null;
  const how = { made: 'Made', seen: 'First seen', noted: 'Noted' }[ctx && ctx.how] || 'Recorded';
  $('#tx-ctx-h').textContent = gone ? 'Where and on what' : ctx.how === 'made' ? 'Where it was made, and on what' : ctx.how === 'seen' ? 'Where this computer first saw it' : 'Where and on what';
  $('#tx-ctx-when').textContent = !gone && ctx.t ? new Date(ctx.t * 1000).toLocaleString(undefined, { day: 'numeric', month: 'short', year: 'numeric', hour: 'numeric', minute: '2-digit' }) : '';
  const rows = [];
  if (gone) rows.push(['Place', ctx && ctx.removed ? 'Removed from this record.' : 'Nothing was recorded: this transaction was already here before the app began keeping this.']);
  else {
    rows.push(['Place', ctx.place ? esc(ctx.place) + (ctx.picked ? `<small>as you chose it from the search${ctx.geo ? ` · ${ctx.geo.lat.toFixed(3)}, ${ctx.geo.lon.toFixed(3)}` : ''}</small>` : ctx.named ? '<small>as you named it</small>' : ctx.geo ? `<small>${ctx.geo.lat.toFixed(3)}, ${ctx.geo.lon.toFixed(3)} · to about ${sats(Math.round(ctx.geo.acc))} m</small>` : '') : (ctx.geo ? `${ctx.geo.lat.toFixed(3)}, ${ctx.geo.lon.toFixed(3)}` : `not kept<small>${store.settings.location ? 'this computer did not know where it was at that moment' : 'places are off in App settings'}</small>`)]);
    if (dev) rows.push([how + ' on', `${esc(dev.name || 'this computer')}<small>${[dev.model, dev.os, dev.app ? 'Any Two Keys ' + dev.app : ''].filter(Boolean).map(esc).join(' · ')}</small>`]);
    if (ctx.how === 'seen') rows.push(['', '<small>A deposit happens wherever the sender was. This is only where this computer was when it noticed.</small>']);
  }
  $('#tx-ctx-dl').innerHTML = rows.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
  $('#tx-ctx-remove').hidden = gone || !(ctx.place || ctx.geo);
  if (document.activeElement !== $('#tx-place') || (ctx && ctx.picked)) $('#tx-place').value = ctx && ctx.named ? ctx.place : '';
  paintMap(gone ? null : ctx);
}
// What a spend realised: what the coins that left had cost, what they were worth as they left, and how long they were
// held — measured by the vault's method, and saying which. Facts an accountant can use, not a tax position.
/// "bought 3 Sep 2026 \u00b7 held 20 days" — one wording, used before a spend (Send) and after it (Activity).
function heldFor(from, to, at) {
  at = at || Math.floor(Date.now() / 1000);
  if (!from) return 'bought on a day not known here';
  const day = (t) => new Date(t * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
  const days = (t) => sats(Math.max(0, Math.floor((at - t) / 86400)));
  return (from === to ? `bought ${day(from)}` : `bought between ${day(from)} and ${day(to)}`) + ` \u00b7 held ${days(to)}${from !== to ? '\u2013' + days(from) : ''} days`;
}
function realisedRow(name, txid) {
  const L = vaultLedger(name), r = L && L.realised[txid], ccy = (txFacts[name] || {}).ccy;
  if (!r || !ccy) return '';
  const held = heldFor(r.from, r.to, r.t);
  if (r.moved) return `<span class="muted">not a disposal</span><small>you said these ${sats(Math.round(r.sats))} sats went to a wallet of your own \u00b7 their cost of ${r.cost === null ? 'an unknown amount' : money(r.cost, ccy)} left the books with them \u00b7 ${held}</small>`;
  if (r.gain === null) return `unknown<small>${r.cost === null ? 'some of the coins that left have no known cost' : 'no price for the day they left'} \u00b7 ${held} \u00b7 by ${esc(COST_METHODS[r.method].short)}</small>`;
  return `<span class="${r.gain >= 0 ? 'ok' : 'warn'}">${r.gain >= 0 ? '\u25b2 +' : '\u25bc \u2212'}${money(Math.abs(r.gain), ccy)}</span><small>worth ${money(r.proceeds, ccy)} as ${sats(Math.round(r.sats))} sats left \u00b7 they had cost ${money(r.cost, ccy)} \u00b7 ${held} \u00b7 by ${esc(COST_METHODS[r.method].short)}</small>`;
}
/// Where the price sat when the coins that left were received, against the day they left (cycle.js). Its own row,
/// after the money: market context is kept apart from what was realised.
function marketRow(name, txid) {
  const L = vaultLedger(name), r = L && L.realised[txid], ccy = (txFacts[name] || {}).ccy;
  if (!r || !ccy) return '';
  const sm = r.t ? (cycleWant([r.t], renderTx) ? cycleAt(r.t) : undefined) : null, bm = cycleOfLots(r.lots, renderTx);
  return cycleTable({ title: 'When received', sub: cycleSpan(r.from, r.to), mark: bm }, { title: 'When it left', sub: cycleDay(r.t), mark: sm });
}
// the picture is drawn in Rust from tiles on our own host; the window only adds the dot and how sure it is
async function paintMap(ctx) {
  const box = $('#tx-map'), want = ctx && ctx.geo ? `${ctx.geo.lat},${ctx.geo.lon},${matchMedia('(prefers-color-scheme: dark)').matches}` : '';
  // a place with a name but no position: no map can be drawn, so offer to look the name up rather than show nothing
  if (!want) {
    box.dataset.for = '';
    if (ctx && ctx.place && !ctx.removed) {
      box.innerHTML = `<div class="nomap"><b>No position for this place</b><span>A place you typed carries no coordinates.</span><button type="button" class="btn small" id="ctx-find">Find it on the map</button></div>`;
      box.hidden = false;
      $('#ctx-find').onclick = async () => { const b = $('#ctx-find'), place = ctx.place; b.disabled = true; b.textContent = 'Looking…';
        try { const hits = await invoke('place_search', { q: place });
          if (!hits.length) { b.textContent = 'Find it on the map'; b.disabled = false; return toast(`Nothing found for “${place}”. Retype it and pick from the list.`, 8000); }
          const h = hits[0];
          await ctxSave({ place: [h.label, h.detail.replace(/ · /g, ', ')].filter(Boolean).join(', ').slice(0, 160), lat: h.lat, lon: h.lon });
          toast(hits.length > 1 ? `Found ${h.label}. If that is the wrong one, retype the place and pick from the list.` : `Found ${h.label}.`, 8000);
        } catch (e) { b.textContent = 'Find it on the map'; b.disabled = false; fail(e); } };
    } else box.hidden = true;
    return;
  }
  if (box.dataset.for === want) return; box.dataset.for = want; box.hidden = true;
  let pic = null; try { pic = await invoke('tx_map', { lat: ctx.geo.lat, lon: ctx.geo.lon, dark: want.endsWith('true') }); } catch {}
  if (box.dataset.for !== want) return;
  // a position with no picture still shows that it HAS a position: an empty corner reads as 'the address was not understood'
  if (!pic) { box.innerHTML = `<div class="nomap"><b>No map just now</b><span>${ctx.geo.lat.toFixed(3)}, ${ctx.geo.lon.toFixed(3)}</span><span>The position is kept. The map host did not answer.</span></div>`; box.hidden = false; box.dataset.for = ''; return; }
  const ring = Math.max(14, 2 * (ctx.geo.acc || 0) / pic.metres_per_px * (320 / pic.width));   // diameter, in the 320-px-wide box
  box.innerHTML = `<img alt="Map of the area around ${esc(ctx.place || 'the place this was recorded')}" src="${pic.png}"><span class="zone" style="width:${ring}px;height:${ring}px"></span><span class="pin"></span><small>© OpenStreetMap</small>`;
  box.hidden = false;
}
const ctxSave = async (args) => { const name = vault.name; try { store = await invoke('ctx_set', { name, txid: txView.txid, remove: false, place: null, lat: null, lon: null, ...args }); vault = store.vaults.find(v => v.name === name) || vault; renderCtx(); } catch (e) { fail(e); } };
// Search as you type — but not as you TYPE: the question goes (from Rust, to our own server only) once the typing has
// stopped for a moment and there are four characters to ask about. Choosing an answer keeps its position too, so the
// record gets a map; pressing Save on words of your own keeps exactly those words and no position.
const placeUI = { seq: 0, hits: [], at: -1, t: null };
function placeList(hits, note) {
  const ul = $('#tx-place-list'); placeUI.hits = hits; placeUI.at = -1;
  ul.innerHTML = hits.map((h, i) => `<li role="option" id="tx-place-opt-${i}" data-i="${i}" aria-selected="false"><b>${esc(h.label)}</b><small>${esc(h.detail)}</small></li>`).join('') + (hits.length || note ? `<li class="foot" aria-hidden="true">${esc(note || 'Asked of our own server only · © OpenStreetMap')}</li>` : '');
  ul.hidden = !(hits.length || note); $('#tx-place').setAttribute('aria-expanded', ul.hidden ? 'false' : 'true');
  // it opens where there is room to read it: below the field, or above it near the bottom of the window
  if (!ul.hidden) { ul.classList.remove('up'); const r = $('#tx-place').getBoundingClientRect(); if (innerHeight - r.bottom < Math.min(300, ul.scrollHeight) + 12 && r.top > innerHeight - r.bottom) ul.classList.add('up'); }
}
const placeMark = (i) => { placeUI.at = i; $$('#tx-place-list li[role=option]').forEach((li, k) => { li.setAttribute('aria-selected', k === i ? 'true' : 'false'); if (k === i) li.scrollIntoView({ block: 'nearest' }); }); $('#tx-place').setAttribute('aria-activedescendant', i >= 0 ? 'tx-place-opt-' + i : ''); };
// the crosshair in the place box: where this Mac is now, named from the built-in list of towns, kept with its position
$('#tx-place-here').onclick = async () => {
  const b = $('#tx-place-here'); if (b.getAttribute('aria-busy') === 'true') return;
  b.setAttribute('aria-busy', 'true'); b.title = 'Finding where this Mac is…';
  try {
    const r = await invoke('location_now');
    const name = r.name || `${r.lat.toFixed(4)}, ${r.lon.toFixed(4)}`;
    placeList([]); $('#tx-place').value = name;
    await ctxSave({ place: name, lat: r.lat, lon: r.lon });
  } catch (e) { fail(e); }
  b.removeAttribute('aria-busy'); b.title = 'Where this Mac is now';
};
const placeChoose = (h) => { clearTimeout(placeUI.t); placeUI.seq++; placeList([]); const full = [h.label, h.detail.replace(/ · /g, ', ')].filter(Boolean).join(', ').slice(0, 160); $('#tx-place').value = full; ctxSave({ place: full, lat: h.lat, lon: h.lon }); };   // the whole address goes in the box and the record, not just its first line
$('#tx-place').addEventListener('input', () => {
  clearTimeout(placeUI.t); const q = $('#tx-place').value.trim(), mine = ++placeUI.seq;
  if (q.length < 4) return placeList([]);
  placeUI.t = setTimeout(async () => { let hits = [], note = ''; try { hits = await invoke('place_search', { q }); if (!hits.length) note = 'Nothing found by that. Save keeps your own words.'; } catch (e) { note = String(e) + '. Save keeps your own words.'; }
    if (mine === placeUI.seq && document.activeElement === $('#tx-place')) placeList(hits, note); }, 350);
});
$('#tx-place').addEventListener('keydown', (e) => { const n = placeUI.hits.length; if ($('#tx-place-list').hidden) return;
  if (e.key === 'ArrowDown' && n) { e.preventDefault(); placeMark((placeUI.at + 1) % n); } else if (e.key === 'ArrowUp' && n) { e.preventDefault(); placeMark((placeUI.at - 1 + n) % n); }
  else if (e.key === 'Enter' && placeUI.at >= 0) { e.preventDefault(); placeChoose(placeUI.hits[placeUI.at]); } else if (e.key === 'Escape') { e.stopPropagation(); placeList([]); } });
$('#tx-place-list').addEventListener('mousedown', (e) => { const li = e.target.closest('li[role=option]'); if (li) { e.preventDefault(); placeChoose(placeUI.hits[+li.dataset.i]); } });
$('#tx-place').addEventListener('blur', () => setTimeout(() => placeList([]), 120));
// Save, on words that are already the place on this record, must KEEP the position those words came with: choosing an
// answer from the search has saved it already, and Save used to re-save the words alone — which drops the coordinates
// by design (below), so the map vanished and the screen fell back to "as you named it". Only words that have actually
// been changed are the person's own, and only those give up the position.
$('#tx-place-form').onsubmit = (e) => {
  e.preventDefault(); clearTimeout(placeUI.t); placeUI.seq++; placeList([]);
  const p = $('#tx-place').value.trim(); if (!p) return;
  const ctx = (((vault.more || {})['tx:' + txView.txid]) || {}).ctx;
  const same = ctx && !ctx.removed && ctx.geo && ctx.place === [...p].slice(0, 160).join('');   // Rust keeps 160 characters
  ctxSave(same ? { place: p, lat: ctx.geo.lat, lon: ctx.geo.lon } : { place: p });
};
askTwice($('#tx-ctx-remove'), 'The place comes off this record, here and in your backup. It cannot be worked out again later.', 'Remove it', () => ctxSave({ remove: true }));

// ---------- App settings › Places ----------
async function renderPlaces() {
  const on = !!store.settings.location;
  $('#loc-on').classList.toggle('on', on); $('#loc-on').setAttribute('aria-checked', on ? 'true' : 'false');
  $('#loc-exact').classList.toggle('on', !!store.settings.location_exact); $('#loc-exact-row').style.opacity = on ? 1 : .5;
  let st = 'off'; try { st = await invoke('location_status'); } catch {}
  let near = null; if (st === 'allowed') { try { near = await invoke('location_preview'); } catch {} }
  $('#loc-state').textContent = !on ? '' : st === 'allowed' ? (near ? `Working: this computer is ${near.startsWith('near ') ? near : 'in ' + near}.` : 'Allowed. Waiting for this computer to work out where it is.') : st === 'asking' ? 'macOS is asking whether Any Two Keys may use your location. Answer it, then come back here.' : st === 'denied' ? 'macOS is refusing: allow Any Two Keys in System Settings › Privacy & Security › Location Services.' : st;
  $('#loc-state').classList.toggle('warn', on && st === 'denied');
  if (on && st !== 'allowed' && st !== 'denied' && $('#s-settings').classList.contains('on')) { clearTimeout(renderPlaces.t); renderPlaces.t = setTimeout(renderPlaces, 3000); }
}
const placeSet = async (args) => { try { store = await invoke('settings_set', { network: netName(), custom: store.settings.custom, electrum: store.settings.electrum, explorer: store.settings.explorer, ...args }); } catch (e) { fail(e); } renderPlaces(); };
for (const [id, key] of [['#loc-on', 'location'], ['#loc-exact', 'locationExact']]) { const flip = () => placeSet({ [key]: !$(id).classList.contains('on') }); $(id).onclick = flip; $(id).onkeydown = (e) => { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); flip(); } }; }

// ---------- a picture from any camera the computer can see (built in, plugged in, or a phone nearby) ----------
const shot = { stream: null, taken: false };
// What KIND of camera this is, from the name macOS gives it — so the person picks a picture of the thing on their desk
// rather than reading a list. ORDER MATTERS, and the traps are real ones, not hypothetical: a Desk View camera is also
// an iPhone; IPEVO's VZ-R has HDMI in its name but is a document camera, not a capture stick; Canon's bridge has the
// word webcam in its name but is an SLR; and the one Insta360 that sits on a desk is a webcam while the rest of that
// brand are action cameras. Each of these is checked before the rule that would otherwise swallow it.
const CAM_KINDS = [
  ['desk',    /desk ?view/],
  ['display', /display/],
  ['ipad',    /ipad/],
  ['iphone',  /iphone|continuity/],
  ['mac',     /facetime|macbook|built-?in|imac/],
  ['doc',     /ipevo|vz-?[xr]\b|ziggi|do-?cam|point 2 view|avervision|visuali[sz]er|document|\belmo\b|okiocam|hue hd/],
  ['ptz',     /rally|meetup|conferencecam|panacast|meeting owl|ptzoptics|poly studio|huddly|cam\d\d\d|group camera/],
  ['slr',     /\beos\b|imaging edge|\bilce|lumix|nikon|fujifilm|\bx webcam\b|mirrorless|\bdslr\b/],
  ['webcam',  /insta360 link/],
  ['action',  /gopro|insta360|osmo action|hero ?\d/],
  ['capture', /cam ?link|capture|hdmi|av\.io|magewell|ninja/],
  ['virtual', /virtual|\bobs\b|camo|snap camera/],
];
function camKind(label) {
  const l = (label || '').toLowerCase();
  for (const [kind, re] of CAM_KINDS) if (re.test(l)) return kind;
  return 'webcam';   // a plain USB camera with a no-name label is far likelier to be a webcam than anything else
}
// its name without the words nearly every one of them ends in — but only as whole trailing words, so IPEVO's DO-CAM
// keeps its name
const camName = (label, i) => ((label || '').replace(/\s+(usb\s+)?(document\s+|video\s+|web\s*)?camera$/i, '').trim()) || ('Camera ' + (i + 1));
/// Draw the chooser into `row` and hide it when there is nothing to choose between. `pick(deviceId)` restarts whatever
/// is using the camera. Names arrive only after the person has allowed one, so this is called once a stream is up.
async function camChooser(row, stream, pick) {
  if (!row) return;
  const had = row.contains(document.activeElement);
  let cams = [];
  try { cams = (await navigator.mediaDevices.enumerateDevices()).filter(d => d.kind === 'videoinput'); } catch {}
  // macOS lists the camera used most recently FIRST, so choosing one moved it to the front and every tile jumped.
  // The row keeps the order each camera was first seen in; a new one joins at the end.
  let order = []; try { order = JSON.parse(localStorage.getItem('a2k-camera-order') || '[]'); } catch {}
  for (const d of cams) if (!order.includes(d.deviceId)) order.push(d.deviceId);
  try { localStorage.setItem('a2k-camera-order', JSON.stringify(order.slice(-24))); } catch {}
  cams.sort((x, y) => order.indexOf(x.deviceId) - order.indexOf(y.deviceId));
  const live = stream && stream.getVideoTracks ? stream.getVideoTracks()[0] : null;
  const using = live && live.getSettings ? live.getSettings().deviceId : '';
  row.innerHTML = cams.map((d, i) => { const k = camKind(d.label), on = d.deviceId === using;
    return `<button type="button" class="camopt" role="radio" data-id="${esc(d.deviceId)}" data-kind="${k}" title="${esc(d.label || '')}" aria-checked="${on ? 'true' : 'false'}" tabindex="${on ? '0' : '-1'}">${camArt(k)}<span>${esc(camName(d.label, i))}</span></button>`; }).join('');
  // WebKit does not give a clicked button the focus; this one takes it, so the arrow keys carry on from the camera chosen
  $$('.camopt', row).forEach(b => { b.onclick = () => { b.focus(); try { localStorage.setItem('a2k-camera', b.dataset.id); } catch {} pick(b.dataset.id); }; });
  // the row is drawn again on every switch, so the button that is now chosen takes the focus the old one had
  if (had) { const now = $('.camopt[aria-checked="true"]', row); if (now) now.focus(); }
  row.onkeydown = (e) => {
    if (!/^Arrow(Left|Right|Up|Down)$/.test(e.key)) return;
    const all = $$('.camopt', row); if (all.length < 2) return;
    e.preventDefault();
    const at = all.findIndex(b => b.getAttribute('aria-checked') === 'true');
    all[(at + (/Left|Up/.test(e.key) ? all.length - 1 : 1)) % all.length].click();
  };
  const holder = row.closest('.field') || row;
  holder.hidden = cams.length < 2;
}
/// The camera the person last chose, if it is still there; otherwise whatever the computer offers.
async function camWanted() {
  let want = ''; try { want = localStorage.getItem('a2k-camera') || ''; } catch {}
  if (!want) return null;
  try { if (!(await navigator.mediaDevices.enumerateDevices()).some(d => d.kind === 'videoinput' && d.deviceId === want)) return null; } catch {}
  return want;
}
/// Ask for a camera: the one named, else the one last chosen, else any. Never throws for a device that has gone.
async function camOpen(deviceId) {
  const want = deviceId || await camWanted();
  const plain = { width: { ideal: 1920 }, height: { ideal: 1080 } };
  if (want) { try { return await navigator.mediaDevices.getUserMedia({ video: Object.assign({ deviceId: { exact: want } }, plain) }); } catch (e) { if (/NotAllowed|denied/i.test(String(e))) throw e; } }
  return await navigator.mediaDevices.getUserMedia({ video: Object.assign({ facingMode: 'environment' }, plain) });
}
function camArt(kind) {
  const lens = (cx, cy, r) => `<circle class="c-glass" cx="${cx}" cy="${cy}" r="${r}"/><circle class="c-fill" cx="${cx}" cy="${cy}" r="${r / 2.6}"/>`;
  switch (kind) {
    case 'mac':      // a laptop, lens over the screen
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><rect class="c-body" x="12" y="6" width="48" height="32" rx="3"/>${lens(36, 11, 2.6)}<path class="c-line" d="M4 46h64"/><path class="c-body" d="M12 38h48l6 8H6z"/></svg>`;
    case 'iphone':   // a phone stood on the desk edge, looking at you
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><rect class="c-body" x="24" y="2" width="24" height="42" rx="5"/><rect class="c-fill" x="31" y="6" width="10" height="2.6" rx="1.3"/>${lens(36, 23, 5)}<path class="c-line" d="M8 50h56"/><path class="c-line" d="M27 50v-6h18v6"/></svg>`;
    case 'desk':     // the same phone, tipped over the top of the screen, looking DOWN at the desk
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><g transform="rotate(-28 27 18)"><rect class="c-body" x="17" y="3" width="20" height="32" rx="4.5"/>${lens(27, 18, 4)}</g><path class="c-beam" d="M31 33l-7 13M41 31l9 15"/><path class="c-line" d="M6 48h60"/><ellipse class="c-body" cx="38" cy="45" rx="12" ry="3.4"/></svg>`;
    case 'display':  // a big screen on a stand, lens centred at the top
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><rect class="c-body" x="6" y="6" width="60" height="36" rx="3"/>${lens(36, 11.5, 2.8)}<path class="c-line" d="M30 42v6h12v-6M22 50h28"/></svg>`;
    case 'ipad':
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><rect class="c-body" x="20" y="4" width="32" height="44" rx="4"/>${lens(36, 11, 3)}<path class="c-line" d="M30 42h12"/></svg>`;
    case 'virtual':  // not a real camera: a screen with a lens drawn ON it
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><rect class="c-body" x="8" y="8" width="56" height="34" rx="4"/>${lens(36, 25, 7)}<path class="c-beam" d="M14 14l44 24"/></svg>`;
    case 'doc':      // a document camera: a weighted base, an arm over the desk, the head looking DOWN at a page
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><path class="c-line" d="M4 50h64"/><path class="c-body" d="M30 43h26l5 7H35z"/><rect class="c-body" x="6" y="44" width="20" height="5" rx="2"/><path class="c-line" d="M16 44V12a4 4 0 0 1 4-4h16a4 4 0 0 1 4 4v4"/><g transform="rotate(14 42 20)"><rect class="c-body" x="33" y="13" width="18" height="11" rx="3"/></g>${lens(43, 26, 3.6)}<path class="c-beam" d="M39 31l-5 10M48 31l5 10"/></svg>`;
    case 'ptz':      // a conference bar: one lens in the middle, microphones either side, on a short stand
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><rect class="c-body" x="6" y="14" width="60" height="20" rx="10"/>${lens(36, 24, 6)}<circle class="c-fill" cx="17" cy="24" r="1.8"/><circle class="c-fill" cx="55" cy="24" r="1.8"/><path class="c-line" d="M31 34v8h10v-8M24 46h24"/><path class="c-line" d="M6 50h60"/></svg>`;
    case 'slr':      // a camera body with a proper lens on the front, and the hump over the viewfinder
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><rect class="c-body" x="8" y="14" width="46" height="28" rx="4"/><path class="c-body" d="M24 14v-5h14v5"/><circle class="c-body" cx="31" cy="28" r="11"/>${lens(31, 28, 7)}<rect class="c-fill" x="44" y="19" width="6" height="3" rx="1.5"/><path class="c-line" d="M6 48h60"/></svg>`;
    case 'action':   // a small square body, all lens, on its mount
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><rect class="c-body" x="18" y="12" width="36" height="28" rx="5"/><circle class="c-body" cx="32" cy="26" r="9"/>${lens(32, 26, 5.5)}<rect class="c-fill" x="44" y="18" width="5" height="5" rx="1"/><path class="c-line" d="M28 40v5M44 40v5"/><path class="c-line" d="M6 48h60"/></svg>`;
    case 'capture':  // not a camera at all: a stick that takes in whatever is plugged into it
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><rect class="c-body" x="22" y="18" width="30" height="16" rx="3"/><path class="c-body" d="M10 21h12v10H10z"/><path class="c-line" d="M52 26h10"/><rect class="c-fill" x="28" y="24" width="5" height="4" rx="1.5"/><path class="c-beam" d="M16 14v-6M16 44v-6"/><path class="c-line" d="M6 50h60"/></svg>`;
    default:         // a webcam on a clip
      return `<svg viewBox="0 0 72 56" role="img" aria-hidden="true"><rect class="c-body" x="18" y="8" width="36" height="24" rx="11"/>${lens(36, 20, 6)}<path class="c-body" d="M24 32h24l4 10H20z"/><path class="c-line" d="M10 46h52"/></svg>`;
  }
}
const shotStop = () => { if (shot.stream) { shot.stream.getTracks().forEach(t => t.stop()); shot.stream = null; } };
function shotClose() { shotStop(); $('#shot-modal').classList.remove('on'); }
async function shotStart(deviceId) {
  shotStop(); shot.taken = false; $('#shot-still').hidden = true; $('#shot-video').hidden = false; $('#shot-again').hidden = true; $('#shot-take').textContent = 'Take the picture'; $('#shot-take').disabled = true;
  try {
    shot.stream = await camOpen(deviceId);
    $('#shot-video').srcObject = shot.stream; $('#shot-take').disabled = false;
    await camChooser($('#shot-pick'), shot.stream, shotStart);
    $('#shot-note').classList.remove('warn'); $('#shot-note').textContent = 'A receipt, a label on a parcel, the thing you bought. It is kept with this transaction, encrypted like everything else.';
  } catch (e) { $('#shot-note').classList.add('warn'); $('#shot-note').textContent = /NotAllowed|denied/i.test(String(e)) ? cameraDenied() : /NotFound|Overconstrained/i.test(String(e)) ? 'No camera was found.' : 'The camera could not be started: ' + String(e.message || e); }
}
$('#tx-camera').onclick = () => { if (!onRec()) return; $('#shot-modal').classList.add('on'); shotStart(''); };

$('#shot-close').onclick = shotClose; $('#shot-again').onclick = () => shotStart('');
$('#shot-take').onclick = async () => {
  const v = $('#shot-video'), cv = $('#shot-still');
  if (!shot.taken) {   // first press: hold the frame so it can be looked at before it is kept
    if (!v.videoWidth) { $('#shot-note').textContent = 'The camera has not produced a picture yet. Give it a moment.'; return; }
    cv.width = v.videoWidth; cv.height = v.videoHeight; cv.getContext('2d').drawImage(v, 0, 0); shotStop();
    v.hidden = true; cv.hidden = false; shot.taken = true; $('#shot-again').hidden = false; $('#shot-take').textContent = 'Keep it with this transaction'; return; }
  const name = recName(recKey()), key = recKey(); $('#shot-take').disabled = true;
  try { const blob = await new Promise(res => cv.toBlob(res, 'image/jpeg', 0.92)); const b64 = await new Promise((res, rej) => { const r = new FileReader(); r.onload = () => res(String(r.result).split(',')[1]); r.onerror = rej; r.readAsDataURL(blob); });
    const r = await invoke('attach_bytes', { name, key, filename: 'photo ' + new Date().toISOString().slice(0, 16).replace('T', ' ').replace(':', '.') + '.jpg', base64: b64, keepOriginal: false }); shotClose(); attachDone(r, name); }
  catch (e) { $('#shot-take').disabled = false; fail(e); }
};
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && $('#shot-modal').classList.contains('on')) { e.stopPropagation(); shotClose(); } }, true);

// a file name keeps its ending when it has to be shortened: "a very long invoi….pdf"
const nameParts = (n) => { const i = n.lastIndexOf('.'); return i > 0 && n.length - i <= 8 ? `<span class="stem">${esc(n.slice(0, i))}</span><span class="ext">${esc(n.slice(i))}</span>` : `<span class="stem">${esc(n)}</span>`; };
const stripOn = () => $('#tx-strip').classList.contains('on');
function paintStrip() { const on = stripOn(); $('#tx-strip').setAttribute('aria-checked', on ? 'true' : 'false');
  $('#tx-strip-note').textContent = on ? 'Location and camera details are removed before a photo is kept.' : 'Off: photos are kept exactly as taken, location included.'; $('#tx-strip-note').classList.toggle('warn', !on); }
// the drop zone says what will happen: tall and inviting on an empty record, one line once there are files, lit while something is held over the window
function paintZone(over, busy) { const z = $('#tx-addfiles'), has = !!$('#tx-files').children.length;
  z.classList.toggle('slim', has && !over); z.classList.toggle('over', !!over); z.classList.toggle('busy', !!busy); z.disabled = !!busy;
  $('#tx-dz-main').textContent = busy ? 'Adding…' : over ? 'Let go to attach' : has ? 'Drop more files' : 'Drop files here';
  $('#tx-dz-sub').innerHTML = busy ? '' : over ? 'to this transaction' : has ? 'or <u>choose files</u>' : 'or <u>choose files</u> · paste a picture with ⌘V'; }
const toggleStrip = () => { $('#tx-strip').classList.toggle('on'); paintStrip(); };
$('#tx-strip').onclick = toggleStrip; $('#tx-strip').onkeydown = (e) => { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); toggleStrip(); } };
paintStrip();
function attachDone(r, name) {
  store = r.store; vault = store.vaults.find(v => v.name === name) || vault; renderFiles();
  const said = [r.added ? `${r.added} file${r.added === 1 ? '' : 's'} attached` : '', ...r.notes, ...r.errors].filter(Boolean).join(' · ');
  if (said) toast(said, r.errors.length || r.notes.length ? 9000 : 3500);
}
const onRec = () => !!recKey() && ($('#s-tx').classList.contains('on') || $('#addr-detail').contains($('#tx-notes')) || $('#coins-detail').contains($('#tx-notes')));
const onTx = () => $('#s-tx').classList.contains('on') && vault && txView.txid;
$('#tx-addfiles').onclick = async () => { const name = recName(recKey()); paintZone(false, true); try { attachDone(await invoke('attach_pick', { name, key: recKey(), keepOriginal: !stripOn() }), name); } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } finally { paintZone(false); } };
// an OS drop reaches a Tauri window as PATHS in an event, never as File objects in the page
for (const [ev, on] of [['tauri://drag-enter', true], ['tauri://drag-over', true], ['tauri://drag-leave', false]]) window.__TAURI__.event.listen(ev, () => { const lit = on && onRec(); if (lit && !$('#tx-addfiles').classList.contains('over')) $('#tx-addfiles').scrollIntoView({ block: 'nearest', behavior: 'smooth' }); paintZone(lit); });
window.__TAURI__.event.listen('tauri://drag-drop', async (e) => {
  paintZone(false);
  const paths = (e.payload && e.payload.paths) || []; if (!paths.length) return;
  if (!onRec()) return toast('Open a transaction, an address or a coin first: files attach to the record on screen');
  const name = recName(recKey()); paintZone(false, true); try { attachDone(await invoke('attach_paths', { name, key: recKey(), paths, keepOriginal: !stripOn() }), name); } catch (err) { fail(err); } finally { paintZone(false); }
});
document.addEventListener('paste', async (e) => {
  if (!onRec()) return; const item = [...(e.clipboardData ? e.clipboardData.items : [])].find(i => i.kind === 'file' && i.type.startsWith('image/')); if (!item) return;   // text pastes into the note as usual
  e.preventDefault(); const f = item.getAsFile(), name = vault.name;
  const b64 = await new Promise((res, rej) => { const r = new FileReader(); r.onload = () => res(String(r.result).split(',')[1]); r.onerror = rej; r.readAsDataURL(f); });
  try { attachDone(await invoke('attach_bytes', { name, key: recKey(), filename: 'pasted ' + new Date().toISOString().slice(0, 16).replace('T', ' ').replace(':', '.') + '.png', base64: b64, keepOriginal: false }), name); } catch (err) { fail(err); }
});

$('#tx-chain').addEventListener('toggle', () => { try { localStorage.setItem('a2k-chain-open', $('#tx-chain').open ? '1' : '0'); } catch {} });
$('#tx-explorer').onclick = () => { const a = document.createElement('a'); a.dataset.tx = txView.txid; document.body.appendChild(a); a.click(); a.remove(); };
$('#tx-copyid').onclick = () => navigator.clipboard.writeText(txView.txid).then(() => toast('Transaction id copied'));
// everything on this screen, printable: the facts, your record, the place with its map, the pictures themselves
$('#tx-pdf').onclick = async () => {
  const b = $('#tx-pdf'), name = vault.name, was = b.textContent;
  b.disabled = true; b.textContent = 'Building…';
  try { toast('Saved ' + await invoke('tx_pdf', { name, txid: txView.txid, height: txView.height, ccy: vault.network === 'mainnet' ? fiatCcy() : '' }), 7000); }
  catch (e) { if (!/cancelled/.test(String(e))) fail(e); }
  finally { b.disabled = false; b.textContent = was; }
};
// the record saves itself: the label when you leave the field or press Enter, the note a moment after you stop typing
async function saveTxField(which) {
  if (!recKey()) return;
  const key = recKey(), name = recName(key), val = which === 'label' ? $('#tx-label').value.trim() : $('#tx-note').value;
  const cur = which === 'label' ? labelOf(key) : ((recHolder(key).notes || {})[key] || '');
  if (val.trimEnd() === cur) return;
  recSaid('Saving…');
  try { store = await invoke(which === 'label' ? 'label_set' : 'note_set', which === 'label' ? { name, key, label: val } : { name, key, note: val }); vault = store.vaults.find(v => v.name === name) || vault;
    const sealed = name && vault && vault.seal && vault.seal.shares && vault.seal.shares.length;
    recSaid('Saved ' + new Date().toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' }) + (sealed ? ' · backed up' : ' · on this computer'), true, sealed ? 'Encrypted and backed up with this vault\'s labels; any two keys bring it back' : 'Kept on this computer only. Seal the keys to back it up.');
    if (which === 'label') { if (recView.kind === 'tx') { paintCrumbs('tx'); $('#title').textContent = txHeadline(); } else renderRecHead(); } }
  catch (e) { recSaid(''); fail(e); }
}
$('#tx-label').addEventListener('blur', () => saveTxField('label'));
$('#tx-label').addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); $('#tx-label').blur(); } });
function recSaid(text, ok, why) { const el = $('#tx-saved'); el.textContent = text; el.classList.toggle('ok', !!ok); el.title = why || ''; }
// the note is as tall as what is in it (to a limit), and only mentions its ceiling when it is close
function fitNote() { const n = $('#tx-note'); n.style.height = 'auto'; n.style.height = (n.scrollHeight + 2) + 'px';
  const left = 20000 - n.value.length, c = $('#tx-count'); c.hidden = left > 2000; c.classList.toggle('near', left < 200); c.textContent = `${sats(left)} characters left`; }
$('#tx-note').addEventListener('input', () => { fitNote(); clearTimeout(saveTxField.t); recSaid('Not saved yet…'); saveTxField.t = setTimeout(() => saveTxField('note'), 1200); });
$('#tx-note').addEventListener('blur', () => { clearTimeout(saveTxField.t); saveTxField('note'); });
$('#tx-prev').onclick = () => stepTx(-1); $('#tx-next').onclick = () => stepTx(1);
// Newer / Older on the arrow keys — but not while something is in front of the screen, or while a control that uses
// the arrows itself has the focus: a camera being chosen must not step to another transaction underneath it.
const overTx = () => $('.modal.on') || !$('#lock').hidden || /INPUT|TEXTAREA|SELECT/.test(document.activeElement.tagName) || document.activeElement.closest('[role="radiogroup"], [role="listbox"], details[open] > summary');
document.addEventListener('keydown', (e) => { if (!$('#s-tx').classList.contains('on') || overTx() || e.metaKey || e.ctrlKey || e.altKey) return; if (e.key === 'ArrowLeft') stepTx(-1); if (e.key === 'ArrowRight') stepTx(1); });
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && $('#s-tx').classList.contains('on') && !/INPUT|TEXTAREA/.test(document.activeElement.tagName)) show('home'); });

let transferView = { links: {}, proposals: [] };
let checkingTransfers = false;
function linkedNames(start) {
  const names=[...start];
  for (let i=0;i<names.length;i++) for (const peer of Object.values(txFacts[names[i]]?.transfers || {}).flat())
    if (!names.includes(peer) && txFacts[peer] && store.vaults.some(v=>v.name===peer && v.network===store.vaults.find(x=>x.name===names[i])?.network)) names.push(peer);
  return names;
}
async function loadTransferPeers(start) {
  const seen=new Set(), todo=[...start], failed=[];
  while (todo.length) {
    const name=todo.shift(); if (seen.has(name)) continue; seen.add(name);
    try { await sumInput(name); } catch {failed.push(name);continue;}
    for (const peer of Object.values(txFacts[name]?.transfers || {}).flat()) if (!seen.has(peer) && store.vaults.some(v=>v.name===peer)) todo.push(peer);
  }
  return failed;
}
function paintTransferProposals() {
  const box=$('#transfer-proposals'); if (!box || !vault) return;
  const proposals=(transferView.proposals || []).filter(p=>p.pair.from===vault.name || p.pair.to===vault.name);
  // nothing to review: the box steps back to one quiet line, so the list is not led by an empty card
  if (!proposals.length) {
    box.className = 'muted'; box.style.cssText = 'font-size:13px;margin:0 0 4px 4px';
    box.innerHTML = '<span>No possible transfers to review.</span> <button type="button" class="linkish" id="transfer-check">Check other vaults</button> <span id="transfer-check-note"></span>';
  } else {
    box.className = 'card'; box.style.cssText = '';
    box.innerHTML='<div class="label">Possible transfers</div><p class="muted">Amounts and dates can suggest a move between your wallets. Your figures change only after you confirm it. Split payments are not matched.</p><button class="btn" id="transfer-check">Check other vaults</button><p id="transfer-check-note" class="muted"></p>';
  }
  // proposals that share a leg (the same payment out, or the same arrival in) are one question: which of these, if
  // any, is real. They are drawn together, and a Yes on one settles the rest.
  const legs = (p) => [p.pair.from + '\u0000' + p.pair.from_id, p.pair.to + '\u0000' + p.pair.to_id];
  const groups = [], byLeg = new Map();
  for (const p of proposals) {
    let g = null; for (const l of legs(p)) { const h = byLeg.get(l); if (h && h !== g) { if (!g) g = h; else { g.push(...h); groups.splice(groups.indexOf(h), 1); for (const q of h) for (const m of legs(q)) byLeg.set(m, g); } } }
    if (!g) { g = []; groups.push(g); } g.push(p); for (const l of legs(p)) byLeg.set(l, g);
  }
  const answerBtn = (p, yes, text, small) => { const b=document.createElement('button'); b.type='button'; b.className='btn' + (small ? ' small' : ''); b.textContent=text; b.dataset.transferAnswer=p.key; b.dataset.yes=String(yes);
    b.onclick=async()=>{b.disabled=true;try {store=await invoke('transfer_answer',{key:p.key,yes}); await refreshTransferLinks(); if(vault){vault=store.vaults.find(v=>v.name===vault.name)||vault;repaintFacts(txFacts[vault.name]);renderHomeSummary();}} catch(e){fail(e);} finally {b.disabled=false;}}; return b; };
  const sameDay = (a, b) => new Date(a*1000).toDateString() === new Date(b*1000).toDateString();
  const whenOpts = { day: 'numeric', month: 'short', year: 'numeric', hour: 'numeric', minute: '2-digit' }, timeOpts = { hour: 'numeric', minute: '2-digit' };
  const shortId = (s) => s.length > 20 ? s.slice(0, 10) + '\u2026' + s.slice(-8) : s;
  const row = (p) => { const c=p.pair, d=document.createElement('div'); d.className='tp';
    const amount = c.exact ? `<b>exactly ${sats(c.incoming)} sats</b> to one output of a ${sats(c.out)} sat payment; the rest is change and fee` : c.out === c.incoming ? `<b>${sats(c.out)} sats</b> out and in` : `${sats(c.out)} sats out \u00b7 ${sats(c.incoming)} sats in \u00b7 ${sats(c.out-c.incoming)} sats apart`;
    d.innerHTML=`<div class="tp-what"><div class="tp-head"><b>${esc(c.from)}</b> \u2192 <b>${esc(c.to)}</b> <span class="muted">\u00b7 ${new Date(c.at*1000).toLocaleString(undefined, whenOpts)} \u2192 ${new Date(c.arrived*1000).toLocaleString(undefined, sameDay(c.at, c.arrived) ? timeOpts : whenOpts)}</span></div><div>${amount}</div><div class="tp-ids mono muted" title="${esc(c.from_id)} \u2192 ${esc(c.to_id)}">${esc(shortId(c.from_id))} \u2192 ${esc(shortId(c.to_id))}</div></div>`;
    const act=document.createElement('div'); act.className='tp-act'; d.appendChild(act); return { d, act }; };
  const list=document.createElement('div'); list.className='tp-list'; box.appendChild(list);
  for (const g of groups) {
    if (g.length === 1 && g[0].unique) { const { d, act } = row(g[0]); act.append(answerBtn(g[0], true, 'Yes, that\u2019s mine', true), answerBtn(g[0], false, 'No', true)); list.appendChild(d); continue; }
    const gd=document.createElement('div'); gd.className='tp-group';
    const outs = new Set(g.map(p => p.pair.from + '\u0000' + p.pair.from_id)).size, ins = new Set(g.map(p => p.pair.to + '\u0000' + p.pair.to_id)).size;
    const what = g.length === 1 ? 'Another match for the same payment sits in a vault not shown here, so this one is not assumed.' : outs === 1 ? `One payment out, ${g.length} possible arrivals. At most one is the real one.` : ins === 1 ? `One arrival, ${g.length} possible payments for it. At most one is the real one.` : `${g.length} matches share payments between them. At most one per payment is real.`;
    gd.innerHTML = `<div class="tp-group-head">${what} Pick the one that is right, or none.</div>`;
    for (const p of g) { const { d, act } = row(p); act.append(answerBtn(p, true, 'This one', true)); gd.appendChild(d); }
    const none=document.createElement('button'); none.type='button'; none.className='btn small'; none.style.marginTop='8px'; none.textContent='None of these'; none.dataset.transferNone='1';
    none.onclick=async()=>{none.disabled=true;try {for (const p of g) store=await invoke('transfer_answer',{key:p.key,yes:false}); await refreshTransferLinks(); if(vault){vault=store.vaults.find(v=>v.name===vault.name)||vault;repaintFacts(txFacts[vault.name]);renderHomeSummary();}} catch(e){fail(e);} finally {none.disabled=false;}};
    gd.appendChild(none); list.appendChild(gd);
  }
  $('#transfer-check').disabled=checkingTransfers;
  $('#transfer-check').onclick=async()=>{
    if(checkingTransfers)return;checkingTransfers=true;
    const b=$('#transfer-check');b.disabled=true;const failures=[];
    const net=vault.network;
    for (const v of store.vaults.filter(v=>v.network===net)) {
      $('#transfer-check-note').textContent='Checking '+v.name+'…';
      try {await sumInput(v.name);} catch {failures.push(v.name);}
    }
    checkingTransfers=false;await refreshTransferLinks();
    $('#transfer-check-note').textContent=failures.length ? 'Unlock and refresh these vaults, then check again: '+failures.join(', ') : 'Checked every available history.';
    if(vault) {repaintFacts(txFacts[vault.name]);renderHomeSummary();}
  };
}
async function refreshTransferLinks() {
  try { transferView = await invoke('transfer_links'); } catch { return; }
  for (const [name, f] of Object.entries(txFacts)) {
    f.links = transferView.links?.[name] || {};
    f.transfers = { ...(f.chainTransfers || {}) };
    for (const [id,l] of Object.entries(f.links)) if (!l.forgotten && !l.own) f.transfers[id] = [l.peer];
  }
  vaultLedger.memo = {};
  paintTransferProposals();
}
const factOwner = v => v ? JSON.stringify([v.network,v.spark?.identity,v.ext || v.descriptor,v.watch?.key,v.muun?.user,v.muun?.cosign]) : '';
function keepActivity(name, a) {
  vaultLedger.memo = {};
  return txFacts[name] = { owner:factOwner(store.vaults.find(v=>v.name===name)), rows: Object.fromEntries(a.rows.map(r => [r.txid, r])), now: a.now, ccy: a.ccy,
    transfers: a.transfers || {}, chainTransfers: (a.spark || []).length ? {} : (a.transfers || {}), links: transferView.links?.[name] || {},
    spark: Object.fromEntries((a.spark || []).map(t => [t.id, t])), pending: pricesPending(a.note) };
}
function transferText(f, id) {
  const l = f.links?.[id];
  if (!l) return '';
  if (l.own) return 'Between this wallet’s own addresses; no disposal';
  if (l.forgotten) return `Was a transfer ${f.rows[id]?.net < 0 ? 'to' : 'from'} ${l.peer}, no longer watched`;
  return `Transfer ${f.rows[id]?.net < 0 ? 'to' : 'from'} ${l.peer}`;
}
async function fillActivity(name, s) {
  const ccy = s.servers && vault.network === 'mainnet' ? fiatCcy() : '';
  let a; try { a = await invoke('vault_activity', { name, history: s.history.map(h => [h.txid, h.height]), ccy }); }
  catch (e) { clearTimeout(fillLater.t); if (vault && vault.name === name) { $$('#activity .when, #activity .amt, #act-all .when, #act-all .amt').forEach(td => { if (td.textContent === '…') td.textContent = '—'; }); $('#activity-note').textContent = 'Dates and amounts could not be fetched: ' + e; $('#activity-note').hidden = false; } return; }
  if (!vault || vault.name !== name) return;
  keepActivity(name, a); txFacts[name].history=s.history.map(h=>[h.txid,h.height]); await refreshTransferLinks();
  if (!vault || vault.name !== name) return;
  if (vault.spark) { txList[name] = (a.spark || []).map(t => ({ txid: t.id, height: 0, index: 0 })); paintActivity(); }
  if ($('#s-tx').classList.contains('on')) renderTx();
  repaintFacts(txFacts[name]);
  vault._now = a.now ? { q: a.now, ccy: a.ccy } : null; paintBalance();
  renderHomeSummary();
  if (a.note && vault.network === 'mainnet') { $('#activity-note').textContent = a.note; $('#activity-note').hidden = false; }
  // a transfer's other end, fetched once, so this vault's figures treat the move as a move and not a sale
  const wanted = [...new Set(Object.values(txFacts[name].transfers || {}).flat())].filter(n => (!txFacts[n] || txFacts[n].stale) && store.vaults.some(v => v.name === n));
  if (wanted.length && !fillActivity.fetching) {
    fillActivity.fetching = true;
    loadTransferPeers(wanted).then(failed => {
      if(failed.length && vault) {$('#activity-note').textContent='Transfer cost history is incomplete. Unlock and refresh: '+failed.join(', ');$('#activity-note').hidden=false;}
      fillActivity.fetching = false; vaultLedger.memo = {};
      if (vault && txFacts[vault.name]) { repaintFacts(txFacts[vault.name]); renderHomeSummary(); if ($('#s-tx').classList.contains('on')) renderTx(); }
    });
  }
}
// A sync younger than this is shown as it is; the background watcher checks every vault each minute and marks a vault
// stale the moment it sees a change, so a quick return to Home costs nothing and still never shows old news.
const FRESH_MS = 30000;
/// Home: what is known, at once; the chain is asked only when that is stale.
function syncIfStale() {
  if (!vault) return;
  // a snapshot is used only for the vault it was taken of — never because it happens to sit on this object
  if (!(vault._snap && vault._snapFor === liveKey(vault) && !vault._stale && Date.now() - (vault._syncedAt || 0) < FRESH_MS)) return sync();
  sync.shown = vault.name; paintSnap(vault._snap);
  // the money column follows the currency setting: a change there is new facts to fetch, not old ones to show
  const f = txFacts[vault.name], want = vault.network === 'mainnet' ? fiatCcy() : '';
  if (!f || (f.ccy || '') !== want) return fillActivity(vault.name, vault._snap);
  repaintFacts(f); vault._now = f.now ? { q: f.now, ccy: f.ccy } : null; paintBalance(); renderHomeSummary();
}
async function sync() {
  if (!vault) return;
  paintVaultCard();
  $('#balance-sub').textContent = 'syncing\u2026'; { const b = $('#scanbar'); if (b) b.hidden = false; }
  // Nothing of another vault may sit under this vault's name, not even for the seconds a sync takes:
  // a vault that has not been drawn yet starts from a blank screen, and an answer that comes back after the person has
  // moved to another vault is for that OTHER vault and is dropped here.
  const me = vault.name;
  if (sync.shown !== me && vault._snap && vault._snapFor === liveKey(vault)) { sync.shown = me; paintSnap(vault._snap); const f = txFacts[me]; if (f) { repaintFacts(f); vault._now = f.now ? { q: f.now, ccy: f.ccy } : null; paintBalance(); } }   // last known, at once, while the chain is asked
  if (sync.shown !== me) { sync.shown = me; vault._balance = undefined; vault._now = null; $('#balance').innerHTML = '— <small>sats</small>'; $('#balance-fiat').hidden = true; $('#activity').innerHTML = '<tr><td class="muted" colspan="5">syncing…</td></tr>'; $('#activity-note').hidden = true; $('#conn').textContent = ''; }
  try {
    if (vault.spark?.authenticated && !(await ensureSparkSession(vault))) { $('#scanbar').hidden = true; $('#balance-sub').textContent = 'reader locked — refresh to open'; return; }
    const s = await invoke('vault_sync', { name: me });
    if (!vault || vault.name !== me) return;
    vault._snap = s; vault._snapFor = liveKey(vault); vault._syncedAt = Date.now(); vault._stale = false;
    paintSnap(s);
    fillActivity(vault.name, s);   // dates, amounts and values arrive after the balance; the balance never waits for them
  } catch (e) { { const b = $('#scanbar'); if (b) b.hidden = true; } if (!vault || vault.name !== me) return; checkNode(); $('#balance-sub').textContent = String(e).startsWith('SPARK_UNLOCK:') ? 'reader needs unlocking — refresh to retry' : 'could not reach the server'; $('#conn').textContent = String(e).replace(/^SPARK_UNLOCK: /, ''); }
}
/// Draw one vault_sync answer: balance, coins, history, where it came from.
// Which keys sign this vault, on the card. Drawn wherever the vault is drawn — a snapshot shown without a fresh
// sync used to leave the LAST vault's keys under this vault's name.
function paintVaultCard() {
  if (!vault) return;
  // a watched wallet has no keys HERE, so the card must not read as though it had ("2 of 0 keys sign")
  $('#vault-kind').textContent = vault.node ? `Node \u00b7 ${vault.node.kind === 'cln' ? 'Core Lightning' : 'LND'}` : vault.exchange ? `Exchange \u00b7 ${vault.exchange.provider}` : vault.statement ? 'Closed wallet \u00b7 from its statement' : vault.spark ? (vault.spark.authenticated ? 'Watched \u00b7 YubiKey reader' : 'Watched \u00b7 Spark') : vault.muun ? 'Watched' : vault.watch ? 'Watched \u00b7 one key' : vault.keys.length === 1 ? 'Single key' : `${vault.threshold} of ${vault.keys.length} keys sign`;
  $('#vault-keys').textContent = vault.node ? `${esc(vault.node.alias)} \u00b7 on-chain ${sats(vault.node.onchain)} \u00b7 in channels ${sats(vault.node.channels)}${vault.node.pending ? ' \u00b7 pending ' + sats(vault.node.pending) : ''} \u00b7 read only` : vault.exchange ? `Read from ${vault.exchange.provider}\u2019s statements \u00b7 ${sats(vault.exchange.rows.length)} rows \u00b7 ${moneyExact(vault.exchange.fiat_now / 100, vault.exchange.ccy)} in cash there \u00b7 nothing here can be spent` : vault.statement ? `Read from the statement ${vault.statement.source} gave \u00b7 ${sats(vault.statement.rows.length)} rows \u00b7 nothing here can be spent` : vault.watch ? `From one exported key \u00b7 ${vault.watch.kinds.length > 1 ? 'every kind of address watched' : vault.watch.kinds[0]}` : vault.spark ? 'Told to us by Spark \u2014 these figures are not checked against the chain' : vault.muun ? 'Its keys stay in Muun; this vault only reads the chain' : vault.keys.map(k => k.name).join(' · ');
}
function paintSnap(s) {
  { const b = $('#scanbar'); if (b) b.hidden = true; }
  {
    vault._coins = s.coins; vault._receive = s.receive; vault._receiveIndex = s.receive_index; vault._rotating = s.rotating; vault._height = s.height; vault._history = s.history; vault._nextChange = s.next_change; vault._gap = s.gap;
    if ($('#s-send').classList.contains('on')) renderCoins();
    if ($('#s-receive').classList.contains('on')) renderAddresses();
    vault._balance = s.balance; vault._now = null; paintBalance();
    // A watched wallet that comes back with nothing at all is far more often the wrong sort of key than an empty
    // wallet, and "0 sats" tells the person neither. Say what was looked at and what else it could be.
    if (vault.watch && !s.coins.length && !(s.history || []).length) {
      const many = vault.watch.kinds.length > 1;
      $('#balance-sub').textContent = `nothing found in the first ${sats((vault._gap || 20))} addresses of ${many ? 'any kind' : vault.watch.kinds[0]}, on both chains`;
      const note = $('#watch-empty') || (() => { const d = document.createElement('div'); d.id = 'watch-empty'; d.className = 'muted'; d.style.cssText = 'font-size:13px;margin-top:10px'; $('#balance-sub').parentElement.appendChild(d); return d; })();
      note.innerHTML = 'If this wallet is a <b>multisig</b>, one key cannot find its addresses \u2014 every cosigner goes into the address, so paste the wallet\u2019s <b>descriptor</b> instead. Otherwise check the key is the one your wallet exports for watching, and that the account is the right one.';
    } else { const n = $('#watch-empty'); if (n) n.remove(); }
    const unconf = s.coins.filter(c => c.height === 0).reduce((a, c) => a + c.value, 0);
    $('#balance-sub').textContent = `${s.coins.length} coin${s.coins.length === 1 ? '' : 's'}` + (unconf ? ` · ${sats(unconf)} unconfirmed` : ' · all confirmed');
    $('#conn').textContent = `Connected to ${s.servers.electrum.replace(/^\w+:\/\//, '')} · block ${sats(s.height)}`;
    if (vault.node) { $('#balance-sub').textContent = `on-chain ${sats(vault.node.onchain)} + in channels ${sats(vault.node.channels)}${vault.node.pending ? ` \u00b7 ${sats(vault.node.pending)} pending` : ''} \u2014 as the node reports them`; $('#conn').textContent = `Read from ${esc(vault.node.alias)} with a read-only credential`; }
    if (vault.exchange) { const ex = vault.exchange; $('#balance-sub').textContent = `held at ${ex.provider}, ${ex.stated ? 'as its newest statement states' : 'as the rows add up'} \u2014 with ${moneyExact(ex.fiat_now / 100, ex.ccy)} in cash beside it${ex.agrees === false ? ' \u00b7 the running balance did not chain in every file' : ''}`; $('#conn').textContent = 'Off-chain \u00b7 the exchange\u2019s own statements, nothing on the chain to check them against'; }
    if (vault.statement) { $('#balance-sub').textContent = `closed wallet \u2014 the statement leaves ${sats(vault.statement.closes_at)} sats; its history is kept for the coins that left it`; $('#conn').textContent = 'Off-chain \u00b7 figures from the custodian\u2019s statement, nothing on the chain to check them against'; }
    if (vault.spark) { $('#balance-sub').textContent = vault.spark.authenticated ? 'Spark reports this balance · YubiKey reader' : 'Spark reports this balance · anonymous read'; $('#conn').textContent = 'Off-chain · figures supplied by Spark, not verified against the chain'; }
    $('#activity-note').hidden = true;
    txList[vault.name] = s.history.slice().reverse();
    paintActivity();
    $('#send-have').textContent = `${sats(s.balance)} sats`;
  }
}
document.addEventListener('click', (e) => { const a = e.target.closest('[data-tx]'); if (a) { e.preventDefault(); const base = ($('#conn').textContent.includes('signet') ? 'https://signet.mineracks.com' : (store.settings.custom && store.settings.explorer) || (netName() === 'mainnet' ? 'https://mempool.mineracks.com' : 'https://signet.mineracks.com')); invoke('open_url', { url: `${base}/tx/${a.dataset.tx}` }).catch(fail); } });

// ---------- activity: Home keeps a recent few, the Activity screen holds the lot ----------
// One row builder for both, so a transaction reads the same wherever it is seen, and fillActivity fills both tables.
const HOME_ACT = 10, ACT_PAGE = 60;
function actRowHtml(h) {
  if (vault?.node) {
    const e = nodeRow(h.txid), how = e ? `${e.layer === 'chain' ? 'on-chain' : e.layer === 'fee' ? 'fee' : 'Lightning'} \u00b7 ${e.kind}` : 'from the node';
    return `<td class="when muted">…</td><td class="mono">${esc(h.txid.slice(0, 10))}…</td><td>${lblHtml('tx:' + h.txid)}<span class="xfer" data-xfer="${esc(h.txid)}"></span></td><td class="r amt muted">…</td><td class="r">${esc(how)}</td>`;
  }
  if (vault?.exchange) {
    const r = exRow(h.txid), how = r ? `${EX_WORDS[r.kind] || r.kind}${r.fiat || r.fee_fiat ? ' \u00b7 ' + moneyExact((Math.abs(r.fiat) + (r.kind === 'buy' ? r.fee_fiat : 0)) / 100, vault.exchange.ccy) : ''}` : 'from the statement';
    return `<td class="when muted">…</td><td class="mono">${esc(h.txid.slice(0, 10))}…</td><td>${lblHtml('tx:' + h.txid)}<span class="xfer" data-xfer="${esc(h.txid)}"></span></td><td class="r amt muted">…</td><td class="r">${esc(how)}</td>`;
  }
  if (vault?.statement) {
    const r = stmtRow(h.txid), how = r ? `${r.layer === 'chain' ? 'on-chain' : 'Lightning'} \u00b7 ${r.kind === 'income' ? (r.pos ? 'sale' : 'reward') : r.kind}` : 'from the statement';
    return `<td class="when muted">…</td><td class="mono">${esc(h.txid.slice(0, 10))}…</td><td>${lblHtml('tx:' + h.txid)}<span class="xfer" data-xfer="${esc(h.txid)}"></span></td><td class="r amt muted">…</td><td class="r">${esc(how)}</td>`;
  }
  if (vault?.spark) {
    const t = txFacts[vault.name]?.spark?.[h.txid];
    return `<td class="when muted">…</td><td class="mono">${esc(h.txid.slice(0, 10))}…</td><td>${lblHtml('tx:' + h.txid)}<span class="xfer" data-xfer="${esc(h.txid)}"></span></td><td class="r amt muted">…</td><td class="r">${esc(sparkTransferStatus(t?.status))}</td>`;
  }
  return `<td class="when muted">${h.height > 0 ? '…' : 'pending'}</td><td><span class="mono">${h.txid.slice(0, 10)}…${h.txid.slice(-6)}</span>${vault._rotating ? ' <span class="muted">· ' + fmtIndex(h.index) + '</span>' : ''}</td><td>${lblHtml('tx:' + h.txid)}<span class="xfer" data-xfer="${h.txid}"></span></td><td class="r amt muted">…</td><td class="r ${h.height > 0 ? 'ok' : 'muted'}">${h.height > 0 ? 'confirmed · ' + sats(h.height) : 'unconfirmed'}</td>`;
}
function actFill(tb, list, key) {
  tb.innerHTML = '';
  if (!list.length) { tb.innerHTML = '<tr><td class="muted" colspan="5">nothing yet</td></tr>'; return; }
  const rows = txFacts[vault.name]?.rows || {};
  let i = 0;
  while (i < list.length) {
    if (key) { const g = key(list[i]); let j = i, net = 0, known = true; while (j < list.length && key(list[j]) === g) { const r = rows[list[j].txid]; if (r && r.net != null) net += r.net; else known = false; j++; } tb.appendChild(groupRow(g, j - i, net, known, 5)); for (; i < j; i++) { const tr = document.createElement('tr'); tr.dataset.txid = list[i].txid; tr.innerHTML = actRowHtml(list[i]); tb.appendChild(tr); } }
    else { const tr = document.createElement('tr'); tr.dataset.txid = list[i].txid; tr.innerHTML = actRowHtml(list[i]); tb.appendChild(tr); i++; }
  }
  tb.onclick = (e) => { if (e.target.closest('.lbl, a, button, input')) return; const tr = e.target.closest('tr[data-txid]'); if (tr) openTx(tr.dataset.txid); };
}
function paintActivity() {
  if (!vault) return;
  const all = txList[vault.name] || [];
  actFill($('#activity'), all.slice(0, HOME_ACT));
  $('#activity-more-row').hidden = all.length <= HOME_ACT;
  $('#activity-more').textContent = `See all ${sats(all.length)} transactions`;
  renderActScreen();
}
let actShown = ACT_PAGE;
// ---------- list controls: which rows, in what order, gathered how — the same on every list ----------
const listPref = (list, k, d) => { try { return localStorage.getItem(`a2k-list-${list}-${k}`) || d; } catch { return d; } };
const listSave = (list, k, v) => { try { localStorage.setItem(`a2k-list-${list}-${k}`, v); } catch {} };
// spec: [{ key, label, def, options: [[value, words], …] }]; a choice is kept per list, not per vault, so every
// vault's list answers the same way once the person has said how they like it
function listControls(host, list, spec, onchange) {
  if (!host) return;
  host.innerHTML = ''; host.className = 'list-ctl';
  for (const s of spec) {
    const cur = listPref(list, s.key, s.def), item = document.createElement('div'); item.className = 'list-ctl-item';
    const name = document.createElement('span'); name.className = 'list-ctl-name'; name.textContent = s.label; item.appendChild(name);
    const seg = document.createElement('div'); seg.className = 'seg'; seg.setAttribute('role', 'radiogroup'); seg.setAttribute('aria-label', s.label);
    for (const [k, n] of s.options) { const b = document.createElement('button'); b.type = 'button'; b.setAttribute('role', 'radio'); b.setAttribute('aria-checked', k === cur ? 'true' : 'false'); b.dataset.ctl = s.key; b.dataset.val = k; b.textContent = n; b.onclick = () => { listSave(list, s.key, k); onchange(); }; seg.appendChild(b); }
    item.appendChild(seg); host.appendChild(item);
  }
}
// a group's header row: its name, how many, and what they add up to when that is known
function groupRow(label, items, net, known, cols) {
  const tr = document.createElement('tr'); tr.className = 'grp';
  const sum = known ? `${sats(items)} \u00b7 ${net > 0 ? '+' : net < 0 ? '\u2212' : ''}${sats(Math.abs(net))} sats` : sats(items);
  tr.innerHTML = `<td colspan="${cols}">${esc(label)}<span class="muted">${sum}</span></td>`; return tr;
}
const ACT_CTL = [
  { key: 'show', label: 'Show', def: 'all', options: [['all', 'All'], ['in', 'In'], ['out', 'Out'], ['xfer', 'Transfers']] },
  { key: 'sort', label: 'Sort', def: 'new', options: [['new', 'Newest'], ['old', 'Oldest'], ['big', 'Largest'], ['small', 'Smallest']] },
  { key: 'group', label: 'Group', def: 'none', options: [['none', 'None'], ['month', 'Month'], ['year', 'Year'], ['kind', 'Kind']] },
];
const actNeedsFacts = () => listPref('act', 'show', 'all') !== 'all' || listPref('act', 'sort', 'new') !== 'new' || listPref('act', 'group', 'none') !== 'none';
// what kind of row this is, in the words its Status column uses
function actKind(h, f) {
  if (vault?.node) { const e = nodeRow(h.txid); return e ? `${e.layer === 'chain' ? 'On-chain' : e.layer === 'fee' ? 'Fee' : 'Lightning'} \u00b7 ${e.kind}` : 'From the node'; }
  if (vault?.exchange) { const r = exRow(h.txid); return r ? (EX_WORDS[r.kind] || r.kind).replace(/^./, c => c.toUpperCase()) : 'From the statement'; }
  if (vault?.statement) { const r = stmtRow(h.txid); return r ? `${r.layer === 'chain' ? 'On-chain' : 'Lightning'} \u00b7 ${r.kind === 'income' ? (r.pos ? 'sale' : 'reward') : r.kind}` : 'From the statement'; }
  if (vault?.spark) { const t = f?.spark?.[h.txid]; return sparkTransferStatus(t?.status) || 'Spark'; }
  const r = f?.rows?.[h.txid];
  if (f?.transfers?.[h.txid]) return 'Transfers between your vaults';
  if (!r || r.net === null) return h.height > 0 ? 'Confirmed' : 'Unconfirmed';
  return r.net > 0 ? 'Deposits' : r.net < 0 ? 'Withdrawals' : 'No change';
}
// the rows to draw: filtered, sorted, and cut into groups when asked — from the list and whatever facts have arrived
function actView(all, q) {
  const f = txFacts[vault.name] || {}, rows = f.rows || {}, xf = f.transfers || {};
  const show = listPref('act', 'show', 'all'), sort = listPref('act', 'sort', 'new'), group = listPref('act', 'group', 'none');
  const xs = all.map((h, i) => ({ h, i, r: rows[h.txid] })).filter(x => actMatch(x.h, q))
    .filter(x => show === 'all' || (show === 'in' && x.r?.net > 0) || (show === 'out' && x.r?.net < 0) || (show === 'xfer' && xf[x.h.txid]));
  const tk = (x) => x.r?.time ?? (x.h.height > 0 ? 0 : 9e12), big = (x) => x.r?.net == null ? -1 : Math.abs(x.r.net), small = (x) => x.r?.net == null ? Infinity : Math.abs(x.r.net);
  xs.sort({ new: (a, b) => (tk(b) - tk(a)) || (a.i - b.i), old: (a, b) => (tk(a) - tk(b)) || (b.i - a.i), big: (a, b) => (big(b) - big(a)) || (a.i - b.i), small: (a, b) => (small(a) - small(b)) || (a.i - b.i) }[sort]);
  const key = group === 'none' ? null : group === 'kind' ? (x) => actKind(x.h, f)
    : (x) => x.r?.time ? new Date(x.r.time * 1000).toLocaleDateString(undefined, group === 'month' ? { month: 'long', year: 'numeric' } : { year: 'numeric' }) : (x.h.height > 0 ? 'Not dated yet' : 'Pending');
  return { hits: xs.map(x => x.h), key: key ? (h) => key({ h, r: rows[h.txid] }) : null };
}
function actMatch(h, q) {
  if (!q) return true;
  const k = 'tx:' + h.txid;
  return (h.txid + ' ' + labelOf(k) + ' ' + ((vault.notes || {})[k] || '')).toLowerCase().includes(q);
}
function renderActScreen() {
  if (!vault) return;
  paintTransferProposals();
  const all = txList[vault.name] || [], q = ($('#act-find').value || '').trim().toLowerCase();
  listControls($('#act-ctl'), 'act', ACT_CTL, () => { actShown = ACT_PAGE; renderActScreen(); });
  const { hits, key } = actView(all, q), narrowed = q || listPref('act', 'show', 'all') !== 'all';
  actFill($('#act-all'), hits.slice(0, actShown), key);
  $('#act-count').textContent = narrowed ? `${sats(hits.length)} of ${sats(all.length)}` : `${sats(all.length)} transaction${all.length === 1 ? '' : 's'}`;
  $('#act-more').hidden = hits.length <= actShown;
  $('#act-more').textContent = `Show ${sats(Math.min(ACT_PAGE, hits.length - actShown))} more`;
  $('#act-foot').textContent = narrowed && !hits.length ? (q ? 'Nothing here matches that. A label or a note you have written counts, as well as the id.' : 'Nothing of that kind here.')
    : `Showing ${sats(Math.min(actShown, hits.length))}. Amounts and dates arrive from the server a moment after the list.`;
  if (vault.name) fillLater();
}
// the cells are filled a tick later, so a fetch that fails in between can cancel it and show dashes instead of stale figures
const fillLater = () => { clearTimeout(fillLater.t); fillLater.t = setTimeout(() => { const f = txFacts[vault.name]; if (f) paintFactCells(f); }, 0); };
// facts arrived or changed: a list ordered or gathered by them is drawn again; otherwise its cells are filled in place
function repaintFacts(facts) {
  if (!facts) return;
  if (vault && $('#s-activity').classList.contains('on') && actNeedsFacts()) { renderActScreen(); return; }
  paintFactCells(facts);
}
// the amounts already fetched are put back into whichever rows have just been drawn
function paintFactCells(facts) {
  if (!facts) return;
  for (const el of $$('[data-xfer]')) {
    const peers = (facts.transfers || {})[el.dataset.xfer], r = (facts.rows || {})[el.dataset.xfer];
    const link = facts.links?.[el.dataset.xfer];
    if (link?.forgotten || link?.own) { el.textContent = ' · ' + transferText(facts, el.dataset.xfer); continue; }
    el.innerHTML = peers ? peers.map(n => ` <a href="#" class="tag peer" data-go-vault-tx="${esc(n)}" data-txid="${esc(facts.links?.[el.dataset.xfer]?.peer_id || el.dataset.xfer)}" title="A transfer: open it in ${esc(n)}">${r && r.net < 0 ? '\u2192' : r && r.net > 0 ? '\u2190' : '\u21c4'} ${esc(n)}</a>`).join('') : '';
  }
  for (const [txid, r] of Object.entries(facts.rows || {})) paintFactRow(txid, r, facts);
}
function paintFactRow(txid, r, facts) {
  for (const tr of $$(`#activity tr[data-txid="${txid}"], #act-all tr[data-txid="${txid}"]`)) {
    const when = tr.querySelector('.when'), amt = tr.querySelector('.amt');
    if (!when || !amt) continue;
    if (r.time) { const d = new Date(r.time * 1000); when.classList.remove('muted'); when.innerHTML = `${d.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })}<small>${d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}</small>`; when.title = d.toISOString().replace('.000Z', ' UTC').replace('T', ' ') + (vault?.spark ? ' · Spark transfer creation time' : ' · the time in the block header'); }
    else when.textContent = 'pending';
    if (r.net === null) { amt.textContent = '—'; continue; }
    const q = r.quote || (!r.time ? facts.now : null);
    amt.classList.remove('muted'); amt.classList.toggle('in', r.net > 0);
    const vn = (tr.closest('#act-all, #activity') && vault) ? vault.name : null, L = vn ? vaultLedger(vn) : null, rz = L && L.realised[txid];
    const rtext = rz && rz.gain !== null ? ` \u00b7 realised ${rz.gain >= 0 ? '+' : '\u2212'}${money(Math.abs(rz.gain), facts.ccy)}` : '';
    amt.innerHTML = `${r.net > 0 ? '+' : r.net < 0 ? '−' : ''}${sats(Math.abs(r.net))} sats<small>${q ? worth(Math.abs(r.net), q, facts.ccy) + (r.time ? ' then' : ' now') : ''}${r.fee !== null && r.net < 0 ? (q ? ' · ' : '') + 'fee ' + sats(r.fee) : ''}${rtext}</small>`;
    if (q) amt.title = basisWords(q, facts.ccy);
  }
}
$('#act-find').addEventListener('input', () => { actShown = ACT_PAGE; renderActScreen(); });
$('#act-more').onclick = () => { actShown += ACT_PAGE; renderActScreen(); };

// ---------- coins: one unspent output each, and a record of its own ----------
function coinKey(c) { return `output:${c.txid}:${c.vout}`; }
function coinAddr(c) { const r = (renderAddresses.rows || []).find(r => r.index === c.index); return r ? r.address : ''; }
function renderCoins2() {
  if (!vault) return;
  listControls($('#coin-ctl'), 'coin', [
    { key: 'sort', label: 'Sort', def: 'big', options: [['big', 'Largest'], ['small', 'Smallest'], ['new', 'Newest'], ['old', 'Oldest']] },
    { key: 'group', label: 'Group', def: 'none', options: [['none', 'None'], ['addr', 'Address'], ['status', 'Status']] }], renderCoins2);
  const sort = listPref('coin', 'sort', 'big'), group = listPref('coin', 'group', 'none');
  const hk = (c) => c.height > 0 ? c.height : 9e9;   // unconfirmed is the newest there is
  const all = (vault._coins || []).slice().sort({ big: (a, b) => b.value - a.value, small: (a, b) => a.value - b.value, new: (a, b) => hk(b) - hk(a) || b.value - a.value, old: (a, b) => hk(a) - hk(b) || b.value - a.value }[sort]);
  const q = ($('#coin-find').value || '').trim().toLowerCase();
  const hits = all.filter(c => !q || (coinKey(c) + ' ' + coinAddr(c) + ' ' + labelOf(coinKey(c)) + ' ' + ((vault.notes || {})[coinKey(c)] || '')).toLowerCase().includes(q));
  const gkey = group === 'addr' ? (c) => { const a = coinAddr(c); return a ? (labelOf('addr:' + a) ? labelOf('addr:' + a) + ' \u00b7 ' + shortAddr(a) : shortAddr(a)) : 'Address unknown'; } : group === 'status' ? (c) => c.height > 0 ? 'Confirmed' : 'Unconfirmed' : null;
  if (gkey) hits.sort((a, b) => gkey(a).localeCompare(gkey(b)) || all.indexOf(a) - all.indexOf(b));
  const tb = $('#coin-rows'); tb.innerHTML = '';
  if (!hits.length) tb.innerHTML = `<tr><td class="muted" colspan="5">${all.length ? 'nothing here matches that' : 'no coins yet: nothing has arrived, or nothing is left unspent'}</td></tr>`;
  let lastG = null;
  for (const c of hits) {
    if (gkey) { const g = gkey(c); if (g !== lastG) { const mine = hits.filter(x => gkey(x) === g); tb.appendChild(groupRow(g, mine.length, mine.reduce((a, x) => a + x.value, 0), true, 6)); lastG = g; } }
    const k = coinKey(c), addr = coinAddr(c), tip = vault._height || 0, deep = c.height > 0 && tip ? tip - c.height + 1 : 0, op = c.txid + ':' + c.vout;
    const tr = document.createElement('tr'); tr.className = 'rowpick' + (coinSel.has(op) ? ' picked' : ''); tr.dataset.coin = k; tr.dataset.op = op;
    tr.innerHTML = `<td class="tick"><input type="checkbox" data-sel="${op}" ${coinSel.has(op) ? 'checked' : ''} aria-label="Tick this coin"></td><td>${lblHtml(k)}</td><td class="r" style="font-variant-numeric:tabular-nums">${sats(c.value)} sats${(() => { const f = txFacts[vault.name]; return f && f.now && f.ccy ? `<small class="muted" style="display:block;font-size:11.5px">${worth(c.value, f.now, f.ccy)} now</small>` : ''; })()}</td><td><span class="mono">${c.txid.slice(0, 8)}…:${c.vout}</span>${vault._rotating && c.index !== undefined ? ' <span class="muted">· ' + fmtIndex(c.index) + '</span>' : ''}</td><td class="mono" title="${esc(addr)}">${addr ? esc(addr.slice(0, 18)) + '…' : '—'}</td><td class="r ${c.height > 0 ? 'ok' : 'muted'}">${c.height > 0 ? (deep ? sats(deep) + ' deep' : 'confirmed') : 'unconfirmed'}</td>`;
    tr.addEventListener('click', (e) => { if (e.target.closest('.lbl, input')) return; openCoin(c); });
    tb.appendChild(tr);
  }
  renderCoins2.hits = hits;   // the order ‹ › walks on a coin's page: as listed, filter and all
  const tot = hits.reduce((a, c) => a + c.value, 0);
  $('#coin-count').textContent = `${sats(hits.length)} coin${hits.length === 1 ? '' : 's'} · ${sats(tot)} sats`;
  const live = new Set(all.map(c => c.txid + ':' + c.vout)); for (const op of [...coinSel]) if (!live.has(op)) coinSel.delete(op);
  $('#coin-tick-all').checked = hits.length > 0 && hits.every(c => coinSel.has(c.txid + ':' + c.vout));
  paintCoinBar();
}
// ---------- coins ticked on Coins: what they add up to, and what can be done with them ----------
const coinSel = new Set();
let sendFrom = false;   // Send was reached from Coins with these coins: say so at the top of the form
function coinSelList() { return ((vault && vault._coins) || []).filter(c => coinSel.has(c.txid + ':' + c.vout)); }
function paintCoinBar() {
  const bar = $('#coin-bar'); if (!bar) return;
  const cs = coinSelList(), n = cs.length, tot = cs.reduce((a, c) => a + c.value, 0);
  bar.hidden = !n; if (!n) return;
  const f = txFacts[vault.name] || {};
  $('#coin-bar-sum').textContent = `${n} coin${n === 1 ? '' : 's'} · ${sats(tot)} sats`;
  const labels = cs.map(c => labelOf(coinKey(c))).filter(Boolean);
  $('#coin-bar-sub').textContent = [f.now && f.ccy ? worth(tot, f.now, f.ccy) + ' now' : '', labels.length ? [...new Set(labels)].slice(0, 3).join(', ') + (labels.length < n ? ` and ${n - labels.length} without a label` : '') : ''].filter(Boolean).join(' · ');
  $('#coin-combine').disabled = n < 2; $('#coin-combine').title = n < 2 ? 'Tick two or more coins to combine them' : '';
  $('#coin-split').disabled = n !== 1; $('#coin-split-why').textContent = n === 1 ? '' : 'one coin at a time';
  $$('#coin-rows tr[data-op]').forEach(tr => tr.classList.toggle('picked', coinSel.has(tr.dataset.op)));
}
$('#coin-rows').addEventListener('change', (e) => { const i = e.target.closest('input[data-sel]'); if (!i) return; if (i.checked) coinSel.add(i.dataset.sel); else coinSel.delete(i.dataset.sel); $('#coin-tick-all').checked = (renderCoins2.hits || []).every(c => coinSel.has(c.txid + ':' + c.vout)); paintCoinBar(); });
$('#coin-tick-all').addEventListener('change', () => { for (const c of renderCoins2.hits || []) { if ($('#coin-tick-all').checked) coinSel.add(c.txid + ':' + c.vout); else coinSel.delete(c.txid + ':' + c.vout); } renderCoins2(); });
$('#coin-sel-all').onclick = () => { for (const c of renderCoins2.hits || []) coinSel.add(c.txid + ':' + c.vout); renderCoins2(); };
$('#coin-sel-none').onclick = () => { coinSel.clear(); renderCoins2(); };
$('#coin-send').onclick = () => { coinPick.clear(); for (const op of coinSel) coinPick.add(op); sendFrom = true; show('send'); };
$('#coin-combine').onclick = () => openOwn('combine');
$('#coin-split').onclick = () => openOwn('split');
// Send, reached from Coins: the coins are ticked in Advanced, Send all is on, and a line at the top says so
function paintSendFrom() {
  const box = $('#send-from'); if (!box) return;
  const cs = ((vault && vault._coins) || []).filter(c => coinPick.has(c.txid + ':' + c.vout));
  box.hidden = !(sendFrom && cs.length); if (box.hidden) return;
  const tot = cs.reduce((a, c) => a + c.value, 0), labels = [...new Set(cs.map(c => labelOf(coinKey(c))).filter(Boolean))];
  $('#send-from-text').innerHTML = `<b>From these ${cs.length} coin${cs.length === 1 ? '' : 's'}</b> · ${sats(tot)} sats${labels.length ? ' · ' + esc(labels.slice(0, 3).join(', ')) : ''}`;
  $('#coins-adv').open = true; $('#send-all').checked = true; $('#send-amount').disabled = true;
}
$('#send-from-change').onclick = () => { for (const op of coinPick) coinSel.add(op); show('coins'); };
// ---------- combine coins, or split one: back into this vault ----------
let ownState = { kind: 'combine', count: 4, equal: true, amounts: [], plan: null, err: '', seq: 0 }, ownDone = null;
function openOwn(kind) {
  ownState = { kind, count: kind === 'split' ? 4 : 1, equal: true, amounts: [], plan: null, err: '', seq: 0 };
  show('own');
}
const ownCoins = () => coinSelList();
function ownCountFor() { return ownState.kind === 'split' ? ownState.count : 1; }
async function renderOwn() {
  const card = $('#own-card'); if (!vault) return;
  const cs = ownCoins(), split = ownState.kind === 'split';
  if (!cs.length || (split && cs.length !== 1) || (!split && cs.length < 2)) { card.innerHTML = `<div class="label">${split ? 'Split coin' : 'Combine coins'}</div><div class="muted">${split ? 'Tick one coin on Coins to split it.' : 'Tick two or more coins on Coins to combine them.'}</div><div class="row"><button class="btn" type="button" data-go="coins">Back to Coins</button></div>`; return; }
  if (!fees) { try { await loadFees(); } catch {} }
  const tot = cs.reduce((a, c) => a + c.value, 0);
  const inRows = cs.slice().sort((a, b) => b.value - a.value).map(c => `<div>${labelOf(coinKey(c)) ? `<b style="font-family:var(--display);font-weight:600">${esc(labelOf(coinKey(c)))}</b>` : '<span class="muted">no label</span>'}<span class="muted"><span class="mono">${c.txid.slice(0, 10)}…:${c.vout}</span>${vault._rotating && c.index !== undefined ? ' · ' + fmtIndex(c.index) : ''}</span><span class="amt">${sats(c.value)} sats</span></div>`).join('');
  const feeSeg = `<div class="seg" role="radiogroup" aria-label="Fee" id="own-fee">${['slow', 'medium', 'fast', 'custom'].map(l => `<button type="button" role="radio" aria-checked="${feeLevel === l ? 'true' : 'false'}" data-fee-own="${l}">${l === 'custom' ? 'Custom' : l[0].toUpperCase() + l.slice(1)}${fees && l !== 'custom' ? ` · ${fees[l]}` : ''}</button>`).join('')}</div><input id="own-rate" type="text" inputmode="decimal" placeholder="sat/vB" style="width:90px;padding:6px 10px;border:1px solid var(--line);border-radius:6px;background:var(--panel);font:inherit" ${feeLevel === 'custom' ? '' : 'hidden'} value="${esc($('#fee-custom').value || '')}">`;
  card.innerHTML = `<div class="strip" id="own-strip"><div><span class="k">Selected amount</span><div class="v">${sats(tot)} sats<small>${cs.length} coin${cs.length === 1 ? '' : 's'}</small></div></div><div><span class="k">Remaining in this vault</span><div class="v" id="own-back">…</div></div><div><span class="k">Network fee</span><div class="v" id="own-fee-v">…</div></div></div>
    <dl class="tx-dl" style="margin:8px 0 0">
      <dt>Going in</dt><dd><div class="inlist">${inRows}</div></dd>
      ${split ? `<dt>Into</dt><dd><div class="row" style="gap:14px"><div class="seg" role="radiogroup" aria-label="How many coins" id="own-count">${[2, 3, 4, 5].map(n => `<button type="button" role="radio" aria-checked="${ownState.count === n && !ownState.custom ? 'true' : 'false'}" data-count="${n}">${n}</button>`).join('')}<button type="button" role="radio" aria-checked="${ownState.custom ? 'true' : 'false'}" data-count="custom">Custom</button></div><input id="own-count-n" type="number" min="2" max="10" value="${ownState.count}" style="width:70px;padding:6px 10px;border:1px solid var(--line);border-radius:6px;background:var(--panel);font:inherit" ${ownState.custom ? '' : 'hidden'} aria-label="Number of coins"><div class="seg" role="radiogroup" aria-label="Amounts" id="own-mode"><button type="button" role="radio" aria-checked="${ownState.equal ? 'true' : 'false'}" data-mode="equal">Equal after the fee</button><button type="button" role="radio" aria-checked="${ownState.equal ? 'false' : 'true'}" data-mode="mine">Amounts I choose</button></div></div><div class="own-amts" id="own-amts" style="margin-top:8px"></div></dd>` : ''}
      <dt>Coming out</dt><dd><div class="outs" id="own-outs"></div><div class="muted" id="own-outs-note" style="font-size:12.5px;margin-top:6px"></div></dd>
      <dt>Fee</dt><dd><div class="row" style="gap:10px;align-items:center">${feeSeg}</div></dd>
    </dl>
    <div class="warn" id="own-err" hidden></div>
    <details class="adv" open id="own-dest-box"><summary>Destination addresses</summary><div class="dest" id="own-dest" style="margin-top:10px"></div><div class="muted" style="font-size:12.5px;margin-top:8px">${vault._rotating ? 'Each is a fresh receive address of this vault, the next ones Receive would hand out.' : 'This vault has one fixed address, so every coin goes back to it.'}</div></details>
    <div style="font-size:15px;max-width:720px" id="own-say"></div>
    <div class="muted" style="font-size:13px;max-width:720px">The amount returned to this vault keeps its cost basis${split ? ', shared across the coins by their amounts' : ''}. The network fee is recorded separately. ${vault.ext ? 'A signer with a screen will show every output as this vault\u2019s own.' : 'A security key has no screen of its own, so this card and the review that follows are where you check the destinations and amounts.'}</div>
    <div style="display:flex;flex-direction:column;gap:6px;align-items:flex-start;margin-top:4px"><div class="row" style="gap:10px"><button class="btn primary" type="button" id="own-build">Build transaction</button><button class="btn" type="button" data-go="coins">Back to Coins</button></div><span class="muted" style="font-size:13px">Next: review, sign with ${vault.keys.length === 1 ? 'the key' : `${vault.threshold} of ${vault.keys.length} keys`}, then broadcast.</span></div>`;
  $$('#own-fee [data-fee-own]').forEach(b => b.onclick = () => { feeLevel = b.dataset.feeOwn; $$('#fee-pick .opt').forEach(x => x.classList.toggle('on', x.dataset.fee === feeLevel)); $$('#own-fee [data-fee-own]').forEach(x => x.setAttribute('aria-checked', x === b ? 'true' : 'false')); $('#own-rate').hidden = feeLevel !== 'custom'; if (feeLevel === 'custom') $('#own-rate').focus(); ownPreview(); });
  $('#own-rate').addEventListener('input', () => { $('#fee-custom').value = $('#own-rate').value; paintCustomFee(); ownPreviewSoon(); });
  if (split) {
    $$('#own-count [data-count]').forEach(b => b.onclick = () => { if (b.dataset.count === 'custom') { ownState.custom = true; $('#own-count-n').hidden = false; $('#own-count-n').focus(); } else { ownState.custom = false; ownState.count = +b.dataset.count; $('#own-count-n').hidden = true; $('#own-count-n').value = ownState.count; } $$('#own-count [data-count]').forEach(x => x.setAttribute('aria-checked', x === b ? 'true' : 'false')); paintOwnAmounts(); ownPreview(); });
    $('#own-count-n').addEventListener('input', () => { const n = Math.min(10, Math.max(2, +$('#own-count-n').value || 2)); ownState.count = n; paintOwnAmounts(); ownPreviewSoon(); });
    $$('#own-mode [data-mode]').forEach(b => b.onclick = () => { ownState.equal = b.dataset.mode === 'equal'; $$('#own-mode [data-mode]').forEach(x => x.setAttribute('aria-checked', x === b ? 'true' : 'false')); paintOwnAmounts(); ownPreview(); });
    paintOwnAmounts();
  }
  $('#own-build').onclick = ownBuild;
  ownPreview();
}
// the amounts of the person's choosing: one field for every coin but the last, which takes the remainder
function paintOwnAmounts() {
  const box = $('#own-amts'); if (!box) return;
  if (ownState.equal) { box.innerHTML = ''; return; }
  const n = ownCountFor();
  ownState.amounts = ownState.amounts.slice(0, n - 1); while (ownState.amounts.length < n - 1) ownState.amounts.push(0);
  box.innerHTML = ownState.amounts.map((a, i) => `<input type="text" inputmode="numeric" data-amt="${i}" value="${a ? sats(a) : ''}" placeholder="coin ${i + 1}, sats" aria-label="Amount of coin ${i + 1}">`).join('') + `<span class="muted" style="font-size:13px">the last coin takes the remainder</span>`;
  $$('#own-amts [data-amt]').forEach(i => i.addEventListener('input', () => { ownState.amounts[+i.dataset.amt] = +(i.value.replace(/[^0-9]/g, '') || 0); ownPreviewSoon(); }));
}
const ownPreviewSoon = () => { clearTimeout(ownPreviewSoon.t); ownPreviewSoon.t = setTimeout(ownPreview, 350); };
function ownArgs() { return { name: vault.name, coins: [...coinSel], count: ownCountFor(), amounts: ownState.kind === 'split' && !ownState.equal ? ownState.amounts : null, rate: feeRate() }; }
async function ownPreview() {
  if (!vault || !$('#s-own').classList.contains('on')) return;
  const me = ++ownState.seq, a = ownArgs();
  if (!a.rate) { paintOwnPlan(null, 'Give a fee rate in sats per virtual byte, or choose Slow, Medium or Fast'); return; }
  if (a.amounts && a.amounts.some(x => !x)) { paintOwnPlan(null, 'Give an amount for every coin but the last'); return; }
  try { const p = await invoke('spend_preview_own', a); if (me !== ownState.seq) return; paintOwnPlan(p, ''); }
  catch (e) { if (me !== ownState.seq) return; paintOwnPlan(null, String(e && e.message ? e.message : e)); }
}
function paintOwnPlan(p, err) {
  ownState.plan = p; ownState.err = err;
  const f = txFacts[vault.name] || {}, split = ownState.kind === 'split';
  $('#own-err').hidden = !err; $('#own-err').textContent = err;
  $('#own-build').disabled = !p;
  if (!p) { $('#own-back').textContent = '—'; $('#own-fee-v').textContent = '—'; $('#own-outs').innerHTML = ''; $('#own-outs-note').textContent = ''; $('#own-dest').innerHTML = ''; $('#own-say').textContent = ''; return; }
  $('#own-back').innerHTML = `${sats(p.back)} sats<small>as ${p.outs.length === 1 ? 'one coin' : p.outs.length + ' coins'}</small>`;
  $('#own-fee-v').innerHTML = `${sats(p.fee)} sats<small>${feeRate()} sat/vB · ${sats(p.vsize)} vB${f.now && f.ccy ? ' · about ' + worth(p.fee, f.now, f.ccy) : ''}</small>`;
  $('#own-outs').innerHTML = p.outs.map((o, i) => `<div class="outc">${sats(o.sats)} sats<small>${vault._rotating ? '#' + o.index : 'the vault\u2019s address'}${!ownState.equal && i === p.outs.length - 1 && split ? ' · the remainder' : ''}</small></div>`).join('');
  const lo = Math.min(...p.outs.map(o => o.sats)), hi = Math.max(...p.outs.map(o => o.sats));
  $('#own-outs-note').textContent = split ? (ownState.equal ? (hi > lo ? 'The amount did not divide evenly, so they differ by one sat.' : 'Equal, after the fee.') : 'The last coin takes what is left after the fee.') : 'Everything, less the fee, as one coin.';
  $('#own-dest').innerHTML = p.outs.map(o => `<span class="muted">#${o.index}</span><span class="mono">${esc(o.address)}</span><span style="font-family:var(--display);font-weight:600;font-variant-numeric:tabular-nums">${sats(o.sats)} sats</span><button class="btn small" type="button" data-copy="${encodeURIComponent(o.address)}">Copy</button>`).join('');
  const cs = ownCoins(), labels = [...new Set(cs.map(c => labelOf(coinKey(c))).filter(Boolean))], change = cs.filter(c => c.index !== undefined && c.index >= CHANGE).length;
  $('#own-say').textContent = split ? 'Smaller coins give you more control over how much you spend later. They will still share a visible history: anyone who sees one can see the split it came from.'
    : `Spending these coins together publicly links them, and can show that they are controlled together.${labels.length > 1 ? ` They carry different labels — ${labels.slice(0, 3).join(', ')}.` : ''}${change ? ` ${change === 1 ? 'One came' : change + ' came'} back as change from another payment.` : ''}`;
}
async function ownBuild() {
  if (!ownState.plan) return;
  $('#own-build').disabled = true;
  try {
    built = await invoke('spend_build_own', ownArgs());
    ownDone = { kind: ownState.kind, n: ownCoins().length, count: built ? ownState.plan.outs.length : 0, spending: ownState.plan.spending, txid: built.txid, vault: vault.name };
    paintCrumbs.via = 'own'; show('sign');
  } catch (e) { fail(e); $('#own-build').disabled = false; }
}
// after a combine or a split is broadcast, its coins and its row say what they are
async function ownLabels(txid) {
  if (!ownDone || ownDone.txid !== txid || !vault || vault.name !== ownDone.vault) return;
  const d = ownDone; ownDone = null; coinSel.clear();
  const tx = d.kind === 'split' ? `Split one coin into ${d.count}` : `Combined ${d.n} coins into one`;
  const out = d.kind === 'split' ? `Split from one coin of ${sats(d.spending)} sats` : `Combined from ${d.n} coins`;
  try { if (!labelOf('tx:' + txid)) store = await invoke('label_set', { name: d.vault, key: 'tx:' + txid, label: tx }); for (let k = 0; k < d.count; k++) store = await invoke('label_set', { name: d.vault, key: 'output:' + txid + ':' + k, label: out }); labelsSync(); vault = store.vaults.find(x => x.name === vault.name) || vault; } catch {}
}
$('#coin-find').addEventListener('input', renderCoins2);
// ---------- a report of the whole vault, to print or keep ----------
// The same figures as the screen: its ledger, the Home summary's series, its coins, every transaction. The recovery
// line is left off on purpose — that is the recovery sheet, kept apart; a report is shared more freely.
async function vaultReport() {
  if (!vault) return;
  const name = vault.name;
  try {
    if (!vault._snap) await sync();
    if (!txFacts[name]) await fillActivity(name, vault._snap);
    const f = txFacts[name] || { rows: {}, ccy: '' }, L = vaultLedger(name), m = methodFor(vault), ccy = f.ccy, now = f.now;
    const priced = vault.network === 'mainnet' && ccy && now && L;
    const dateOf = (t) => new Date(t * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
    const last = L && L.timeline.length ? L.timeline[L.timeline.length - 1] : { sats: vault._balance || 0, basis: 0 };
    const held = vault._balance !== undefined ? vault._balance : last.sats, value = priced ? held / 1e8 * now.price : null;
    const facts = [['Vault', `${vault.threshold} of ${vault.keys.length} · ${vault.network}${vault.ext ? ' · keys held on signing devices' : ''}`],
      ['Created', vault.created ? dateOf(vault.created) : '—'], ['Holding', `${sats(held)} sats in ${sats((vault._coins || []).length)} coin${(vault._coins || []).length === 1 ? '' : 's'}`]];
    if (priced) {
      facts.push(['Worth now', `${money(value, ccy)}  (1 BTC = ${money(now.price, ccy)})`], ['Cost basis', `${money(last.basis, ccy)}  (${COST_METHODS[m].name})`]);
      const g = value - last.basis; facts.push(['Unrealised', `${g >= 0 ? '+' : '−'}${money(Math.abs(g), ccy)}${last.basis > 0 ? `  (${(g >= 0 ? '+' : '−') + Math.abs(g / last.basis * 100).toFixed(1)}% on cost)` : ''}`]);
      const rz = Object.values(L.realised), known = rz.filter(x => x.gain !== null), rg = known.reduce((a, x) => a + x.gain, 0);
      facts.push(['Realised', rz.length ? `${rg >= 0 ? '+' : '−'}${money(Math.abs(rg), ccy)} over ${rz.length} disposal${rz.length === 1 ? '' : 's'}${known.length < rz.length ? ` (${rz.length - known.length} of unknown cost)` : ''}` : 'nothing yet']);
    }
    const blocks = [{ kind: 'facts', rows: facts }];
    // the two charts from Home, over the vault's whole life
    if (L && L.timeline.length) {
      const first = L.timeline[0].t, to = Math.floor(Date.now() / 1000), from = Math.max(0, first - 3600);
      const run = priced ? await pricesRange(ccy, first, to) : { prices: {} };
      const S = sumSeries(L, from, to, run.prices || {}, now);
      const lbl = (t) => new Date(t * 1000).toLocaleDateString();
      const hiB = Math.max(1, ...S.bal.map(p => p[1])) * 1.05;
      blocks.push({ kind: 'chart', chart: { title: 'Balance, in sats', lo: 0, hi: hiB, lo_label: '0 sats', hi_label: sats(Math.round(hiB)) + ' sats', from_label: lbl(from), to_label: lbl(to), series: [{ name: 'Balance', color: 'value', pts: S.bal, step: true }] } });
      if (priced && S.val.length) {
        const all = [...S.val, ...S.cost].map(p => p[1]), hi = Math.max(1, ...all) * 1.05;
        blocks.push({ kind: 'chart', chart: { title: `Worth and cost basis, in ${ccy}`, lo: 0, hi, lo_label: money(0, ccy), hi_label: money(hi, ccy), from_label: lbl(from), to_label: lbl(to), series: [{ name: 'Worth', color: 'value', pts: S.val }, { name: 'Cost basis', color: 'cost', pts: S.cost, step: true }] } });
      }
    }
    blocks.push({ kind: 'heading', text: 'Keys' });
    blocks.push({ kind: 'facts', rows: vault.keys.map(k => { const x = /^\[([0-9a-f]{8})\/([^\]]+)\]/.exec(k.pubkey); const d = k.device && typeof deviceOf === 'function' && k.device !== 'card' ? deviceOf(k.device).name : 'YubiKey';
      return [k.name, x ? `${d} · fingerprint ${x[1]} · m/${x[2]}` : `${d}${k.serial ? ' · serial ' + k.serial : ''}${vault.ext ? ' · a fixed key' : ''}`]; }) });
    const coins = (vault._coins || []).slice().sort((a, b) => b.value - a.value);
    if (coins.length) {
      blocks.push({ kind: 'heading', text: `Coins held (${coins.length})` });
      blocks.push({ kind: 'facts', rows: coins.map(c => { const k = 'output:' + c.txid + ':' + c.vout, lab = labelOf(k);
        return [`${c.txid.slice(0, 8)}…:${c.vout}`, `${sats(c.value)} sats${priced ? ' · ' + money(c.value / 1e8 * now.price, ccy) : ''} · ${c.height > 0 ? 'block ' + sats(c.height) : 'unconfirmed'}${lab ? ' · ' + lab : ''}`]; }) });
    }
    const rows = Object.values(f.rows).sort((a, b) => (b.time || 9e9) - (a.time || 9e9));
    if (rows.length) {
      blocks.push({ kind: 'heading', text: `Transactions (${rows.length})` });
      blocks.push({ kind: 'facts', rows: rows.map(r => { const lab = labelOf('tx:' + r.txid), rz = L && L.realised[r.txid], xf = (f.transfers || {})[r.txid];
        const worth = r.quote && ccy ? ' · ' + money(Math.abs(r.net) / 1e8 * r.quote.price, ccy) + (r.time ? ' then' : ' now') : '';
        return [r.time ? dateOf(r.time) : 'unconfirmed', `${r.net > 0 ? '+' : r.net < 0 ? '−' : ''}${sats(Math.abs(r.net || 0))} sats${worth}${rz && rz.gain !== null ? ' · realised ' + (rz.gain >= 0 ? '+' : '−') + money(Math.abs(rz.gain), ccy) : ''}${xf ? ' · transfer ' + (r.net < 0 ? 'to ' : 'from ') + xf.join(', ') : ''}${lab ? ' · ' + lab : ''} · ${r.txid.slice(0, 12)}…`]; }) });
    }
    blocks.push({ kind: 'para', text: `The recovery line is not in this report: it is on the recovery sheet (Vault settings › Save the recovery sheet), which should be kept apart from anything shared. ${priced ? 'Figures in ' + ccy + ' use ' + COST_METHODS[m].name.toLowerCase() + ' for cost; they are facts about the coins, not a tax position.' : ''}` });
    const path = await invoke('report_pdf', { title: name, subtitle: `Vault report · ${vault.network} · printed ${new Date().toLocaleDateString()}`, blocks, file: `${name}-report` });
    toast('Saved ' + path, 5000);
  } catch (e) { if (!/cancelled/.test(String(e))) fail(e); }
}
document.addEventListener('click', (e) => { if (e.target.closest('[data-vault-report]')) vaultReport(); });

// ---------- one coin, or one address, as a page: vault › Coins › this coin ----------
// The list steps aside for the page; ‹ › and the arrow keys walk the list in its order; the strip at the top is what
// it cost against what it has been worth since it arrived; the record (label, note, files) sits below; and the whole
// of it saves as a PDF.
const recPage = { kind: null, id: null, leaf: '' };
function coinOf(id) { return (vault && vault._coins || []).find(c => c.txid + ':' + c.vout === id); }
function openCoin(c) { recPageOpen('coin', c.txid + ':' + c.vout); }
function openAddress(r) { recPageOpen('addr', r.address); }
function recPageList(kind) { return kind === 'coin' ? (renderCoins2.hits || []).map(c => c.txid + ':' + c.vout) : (renderAddresses.order || []).map(r => r.address); }
function recPageClose(kind) {
  const box = kind === 'coin' ? $('#coins-detail') : $('#addr-detail'); box.hidden = true;
  const list = kind === 'coin' ? $('#coin-rows').closest('.card') : $('#addr-card'); if (list) list.hidden = kind === 'addr' && !vault._rotating;
  if (recPage.kind === kind) { recPage.kind = null; recPage.id = null; recPage.leaf = ''; recPoint('tx', recKey(), recHome); }
}
function recPageStep(by) { const l = recPageList(recPage.kind), i = l.indexOf(recPage.id); if (i < 0 || !l[i + by]) return; recPageOpen(recPage.kind, l[i + by]); }
/// The coins held at one address, or the one coin, with what each cost (its own lots, whatever the vault's method).
function recHoldings(kind, id) {
  const coins = kind === 'coin' ? [coinOf(id)].filter(Boolean) : (vault._coins || []).filter(c => { const r = (renderAddresses.rows || []).find(r => r.address === id); return r && c.index === r.index; });
  const f = txFacts[vault.name], L = f ? buildLedger([{ name: vault.name, rows: Object.values(f.rows), now: f.now }], 'utxo') : null;
  return coins.map(c => { const op = c.txid + ':' + c.vout, lots = L ? L.coins[op] || null : null, row = f && f.rows[c.txid];
    return { c, op, lots, cost: lots && !lots.some(l => l.cost === null) ? lots.reduce((a, l) => a + l.cost, 0) : null, born: row && row.time || null,
      // held since the oldest of its sats arrived; with no dated lots (a price was missing), since the coin itself arrived
      since: (() => { const t = lots ? Math.min(...lots.map(l => l.t || Infinity)) : Infinity; return isFinite(t) ? t : (row && row.time) || null; })() }; });
}
function recPageOpen(kind, id) {
  const coin = kind === 'coin' ? coinOf(id) : null, row = kind === 'addr' ? (renderAddresses.rows || []).find(r => r.address === id) : null;
  if (kind === 'coin' && !coin) return toast('That coin is not in this vault any more: it has been spent');
  if (kind === 'addr' && !row) return;
  recPage.kind = kind; recPage.id = id;
  const box = kind === 'coin' ? $('#coins-detail') : $('#addr-detail'), list = kind === 'coin' ? $('#coin-rows').closest('.card') : $('#addr-card');
  if (list) list.hidden = true; if (kind === 'addr') $('#addr-fixed').hidden = true;
  // the record card (label, note, files) is ONE element that moves from page to page: rebuilding a page while it is
  // inside would destroy it — as stepping ‹ › from one coin to the next did — so it goes home first
  if (box.contains($('#tx-notes'))) recHome.appendChild($('#tx-notes'));
  // one page at a time: the other kind's page is emptied, so its buttons and charts (same ids) cannot be the ones
  // this page's handlers find — an address's "Save as PDF" was wiring up the hidden coin page's button
  const other = kind === 'coin' ? $('#addr-detail') : $('#coins-detail');
  if (other.contains($('#tx-notes'))) recHome.appendChild($('#tx-notes'));
  other.innerHTML = ''; other.hidden = true;
  box.hidden = false;
  const l = recPageList(kind), i = l.indexOf(id), words = kind === 'coin' ? ['‹ Larger', 'Smaller ›', 'coin'] : ['‹ Previous', 'Next ›', 'address'];
  const H = recHoldings(kind, id), held = H.reduce((a, h) => a + h.c.value, 0), f = txFacts[vault.name], ccy = f && f.ccy, now = f && f.now;
  const known = H.length && H.every(h => h.cost !== null), cost = known ? H.reduce((a, h) => a + h.cost, 0) : null, value = now && held ? held / 1e8 * now.price : null;
  const since = H.length ? Math.min(...H.map(h => h.since || Infinity)) : null, tip = vault._height || 0, m = methodFor(vault);
  const dateOf = (t) => new Date(t * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
  recPage.leaf = kind === 'coin' ? `${id.slice(0, 8)}…:${id.split(':')[1]} \u00b7 ${sats(coin.value)} sats` : `${fmtIndex(row.index)} \u00b7 ${id.slice(0, 12)}…`;
  const money_ = [];
  if (held && ccy && vault.network === 'mainnet') {
    money_.push(['Cost basis', cost === null ? 'not known \u2014 a price is missing for part of it' : money(cost, ccy) + `<small class="muted"> \u00b7 ${m === 'utxo' ? 'what these coins cost as they arrived' : 'this coin\u2019s own history; this vault counts ' + COST_METHODS[m].short + ', which does not give one coin its own cost'}</small>`]);
    money_.push(['Worth now', value === null ? '\u2014' : money(value, ccy) + `<small class="muted"> \u00b7 1 BTC = ${money(now.price, ccy)}</small>`]);
    if (cost !== null && value !== null) { const g = value - cost; money_.push(['Unrealised', `<span class="${g >= 0 ? 'up' : 'down'}">${g >= 0 ? '\u25b2 +' : '\u25bc \u2212'}${money(Math.abs(g), ccy)}</span>${cost > 0 ? `<small class="muted"> \u00b7 ${(g >= 0 ? '+' : '\u2212') + Math.abs(g / cost * 100).toFixed(1)}% on cost</small>` : ''}`]); }
  }
  if (since && isFinite(since)) money_.push(['Held since', dateOf(since) + `<small class="muted"> \u00b7 ${Math.max(0, Math.floor((Date.now() / 1000 - since) / 86400))} days</small>`]);
  const chain = kind === 'coin'
    ? [['Amount', sats(coin.value) + ' sats'], ['Output', `<a href="#" class="mono" data-go-tx="${coin.txid}">${coin.txid}</a>:${coin.vout}`], ['Address', (() => { const a = coinAddr(coin); return a ? `<a href="#" class="mono" data-go-addr="${esc(a)}">${esc(a)}</a>` : 'not known until the next sync'; })()],
       ['Status', coin.height > 0 ? `in block ${sats(coin.height)}${tip ? ' \u00b7 ' + sats(tip - coin.height + 1) + ' deep' : ''}` : 'not in a block yet']]
    : [['Address', `<span class="mono" style="word-break:break-all;-webkit-user-select:text;user-select:text">${esc(id)}</span>`], ['Chain', `${row.chain === 'change' ? 'change \u2014 the app sends what is left over here' : 'receive \u2014 one to hand out'}${vault._rotating ? ' \u00b7 ' + fmtIndex(row.index) : ''}`],
       ['Holding', held ? `${sats(held)} sats in ${sats(H.length)} coin${H.length === 1 ? '' : 's'} \u2014 listed below` : ((vault._history || []).some(x => x.index === row.index) ? 'nothing now \u2014 it has been used and spent from' : 'nothing yet \u2014 this address has never been used')]];
  const txs = kind === 'addr' ? [...new Set((vault._history || []).filter(x => x.index === row.index).map(x => x.txid))] : [];
  if (txs.length) chain.push(['Transactions', txs.map(t => `<a href="#" class="mono" data-go-tx="${t}">${t.slice(0, 10)}\u2026</a>`).join(', ')]);
  box.innerHTML = `<div class="txnav"><button class="btn small" type="button" data-rec-step="-1"${i > 0 ? '' : ' disabled'}>${words[0]}</button><span class="muted">${i >= 0 ? `${i + 1} of ${l.length}` : ''}</span><button class="btn small" type="button" data-rec-step="1"${i >= 0 && i < l.length - 1 ? '' : ' disabled'}>${words[1]}</button></div>
    <div class="chart rec-strip" id="rec-chart"></div>${kind === 'addr' ? '<div class="chart rec-strip" id="rec-bal"></div>' : ''}
    <dl class="tx-dl">${[...money_, ...chain].map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('')}</dl>
    ${kind === 'addr' && H.length ? `<div class="label" style="margin-top:6px">Coins here now</div><table class="tbl" id="addr-coins"><thead><tr><th>Coin</th><th class="r">Amount</th><th>Arrived</th><th class="r">Depth</th><th class="r">Cost</th></tr></thead><tbody>${H.slice().sort((a, b) => b.c.value - a.c.value).map(h => `<tr class="rowpick" data-go-coin="${esc(h.op)}"><td><a href="#" class="mono" data-go-coin="${esc(h.op)}">${esc(h.op.slice(0, 10))}\u2026:${esc(h.op.split(':')[1])}</a></td><td class="r" style="font-variant-numeric:tabular-nums">${sats(h.c.value)} sats</td><td>${h.since ? dateOf(h.since) : '\u2014'}</td><td class="r muted">${h.c.height > 0 ? (tip ? sats(tip - h.c.height + 1) + ' deep' : 'in block ' + sats(h.c.height)) : 'pending'}</td><td class="r muted">${h.cost === null || h.cost === undefined ? '\u2014' : ccy ? money(h.cost, ccy) : '\u2014'}</td></tr>`).join('')}</tbody></table>` : ''}
    <div class="row">${kind === 'coin' ? '<button class="btn small" type="button" id="coin-spend">Spend this coin</button>' : '<button class="btn small" type="button" id="addr-show">Show it on Receive</button><button class="btn small" type="button" id="addr-copy">Copy it</button>'}<button class="btn small" type="button" id="rec-pdf">Save as PDF</button></div>
    ${kind === 'addr' ? `<div class="muted" style="font-size:13px">${(vault._history || []).some(x => x.index === row.index) ? 'An address that has been used is public: whoever you gave it to can see everything it has ever held.' : 'Labelling an address before you hand it out is the point: when a deposit arrives you already know who it was from.'}</div>` : ''}
    <div id="rec-slot"></div>`;
  $$('[data-rec-step]', box).forEach(b => b.onclick = () => recPageStep(+b.dataset.recStep));
  if (kind === 'coin') $('#coin-spend').onclick = () => { coinPick.clear(); coinPick.add(id); show('send'); setTimeout(() => { const d = $('#coins-adv'); if (d) d.open = true; renderCoins(); coinSum(); }, 60); };
  else { $('#addr-show').onclick = () => { drawAddress(row); show('receive'); }; $('#addr-copy').onclick = () => navigator.clipboard.writeText(id).then(() => toast('Address copied')); }
  recPoint(kind, kind === 'coin' ? 'output:' + id : 'addr:' + id, $('#rec-slot'));
  paintCrumbs(kind === 'coin' ? 'coins' : 'addresses');
  const series = recSeries(H, ccy, now);
  recPage.pdf = { held, cost, value, since, ccy, series: null, facts: [...money_, ...chain] };
  series.then(S => { if (recPage.id !== id) return; recPage.pdf.series = S; recChart($('#rec-chart'), S, ccy, kind); });
  if (kind === 'addr') addrBalance(row, id);
  $('#rec-pdf').onclick = () => recPdf(kind, id);
  box.scrollIntoView({ block: 'nearest' });
}
/// An address's balance in sats, from the first time it received to now: each transaction that touched it steps the
/// line up (paid to it) or down (spent from it). Worked out in Rust from the cached transactions, not fetched again.
async function addrBalance(row, id) {
  const el = $('#rec-bal'); if (!el) return;
  const hist = [...new Map((vault._history || []).filter(x => x.index === row.index).map(x => [x.txid, [x.txid, x.height]])).values()];
  if (!hist.length) { el.innerHTML = '<div class="sum-empty">Nothing has ever arrived here, so there is no balance to draw.</div>'; recPage.pdf.balance = null; return; }
  let flow; try { flow = await invoke('address_flow', { name: vault.name, index: row.index, history: hist }); } catch (e) { el.innerHTML = `<div class="sum-empty">The balance could not be worked out just now (${esc(String(e))}).</div>`; return; }
  if (recPage.id !== id) return;
  const nowT = Math.floor(Date.now() / 1000), ev = flow.map(f => [f.time || nowT, f.delta]).sort((a, b) => a[0] - b[0]);
  if (!ev.length) { el.innerHTML = '<div class="sum-empty">Nothing has ever arrived here, so there is no balance to draw.</div>'; return; }
  let bal = 0; const pts = [[ev[0][0] - 1, 0]]; for (const [t, d] of ev) { bal += d; pts.push([t, bal]); } pts.push([nowT, bal]);
  const from = pts[0][0], to = Math.max(nowT, from + 3600);
  recPage.pdf.balance = { pts, from, to };
  lineChart(el, { title: 'Balance, in sats', sub: `It changes only when something arrives or leaves: ${ev.length} transaction${ev.length === 1 ? '' : 's'} here.`, from, to,
    series: [{ name: 'Balance', color: 'var(--series-value)', pts, step: true, area: true }], fmt: (v) => sats(Math.round(v)) + ' sats', fmtAxis: (v) => csats(v) });
}
/// Worth and cost of what is held, day by day since the first of it arrived: worth counts each coin from the day it
/// came (so an address's line steps up as its coins arrive), cost likewise.
async function recSeries(H, ccy, now) {
  const born = H.filter(h => h.born || h.c.height === 0).map(h => ({ ...h, born: h.born || Math.floor(Date.now() / 1000) }));
  if (!born.length || !ccy || !now || vault.network !== 'mainnet') return null;
  // a coin that came today would otherwise be drawn across a few hours, every label the same day and hour: a week of
  // context at least, the line starting where the coin does
  const to = Math.floor(Date.now() / 1000), from = Math.min(...born.map(h => h.born), to - 7 * 86400);
  const run = await pricesRange(ccy, from, to);
  const days = Object.keys(run.prices || {}).sort().map(d => [Math.floor(Date.parse(d + 'T12:00:00Z') / 1000), run.prices[d]]).filter(([t, q]) => q && t >= from - 86400);
  const at = (t) => born.filter(h => h.born <= t);
  const val = days.map(([t, q]) => [Math.max(t, from), at(Math.max(t, from)).reduce((a, h) => a + h.c.value, 0) / 1e8 * q.price]);
  val.push([to, born.reduce((a, h) => a + h.c.value, 0) / 1e8 * now.price]);
  const known = born.every(h => h.cost !== null);
  const cost = known ? [...new Set([from, ...born.map(h => h.born), to])].sort((a, b) => a - b).map(t => [t, at(t).reduce((a, h) => a + h.cost, 0)]) : null;
  return { from, to, val, cost, every: run.every, ok: run.ok };
}
function recChart(el, S, ccy, kind) {
  if (!el) return;
  if (!S) { el.innerHTML = `<div class="sum-empty">${vault.network !== 'mainnet' ? 'Test coins have no price, so there is nothing to draw.' : !ccy ? 'No currency is chosen in App settings, so there is nothing to draw.' : 'Nothing held here now, so there is nothing to draw.'}</div>`; return; }
  const series = [{ name: 'Worth', color: 'var(--series-value)', pts: S.val, step: false }];
  if (S.cost) series.push({ name: 'Cost basis', color: 'var(--series-cost)', pts: S.cost, step: true });
  lineChart(el, { title: `What ${kind === 'coin' ? 'this coin is' : 'this address holds is'} worth, and what it cost, in ${ccy}`, sub: S.cost ? 'The gap between the two lines is the gain or loss not yet realised.' : 'Its cost is not known: a price is missing for part of it.', from: S.from, to: S.to, series, fmt: (v) => money(v, ccy), fmtAxis: (v) => cmoney(v, ccy) });
}
async function recPdf(kind, id) {
  const P = recPage.pdf, strip = (h) => { const d = document.createElement('div'); d.innerHTML = h; return d.textContent.trim(); };
  const charts = [];
  if (P.balance) {
    const hi = Math.max(...P.balance.pts.map(p => p[1])) * 1.05 || 1;
    charts.push({ title: 'Balance, in sats', lo: 0, hi, lo_label: '0 sats', hi_label: sats(Math.round(hi)) + ' sats', from_label: new Date(P.balance.from * 1000).toLocaleDateString(), to_label: new Date(P.balance.to * 1000).toLocaleDateString(),
      series: [{ name: 'Balance', color: 'value', pts: P.balance.pts, step: true }] });
  }
  let chart = null;
  if (P.series && P.ccy) {
    const all = [...P.series.val.map(p => p[1]), ...(P.series.cost || []).map(p => p[1])], lo = Math.min(0, ...all), hi = Math.max(...all) * 1.05 || 1;
    chart = { title: `Worth and cost basis, in ${P.ccy}`, lo, hi, lo_label: money(lo, P.ccy), hi_label: money(hi, P.ccy),
      from_label: new Date(P.series.from * 1000).toLocaleDateString(), to_label: new Date(P.series.to * 1000).toLocaleDateString(),
      series: [{ name: 'Worth', color: 'value', pts: P.series.val }, ...(P.series.cost ? [{ name: 'Cost basis', color: 'cost', pts: P.series.cost, step: true }] : [])] };
  }
  const H = recHoldings(kind, id), lots = H.flatMap(h => (h.lots || []).map(l => [new Date((l.t || 0) * 1000).toLocaleDateString(), `${sats(Math.round(l.sats))} sats \u00b7 ${l.cost === null ? 'cost not known' : money(l.cost, P.ccy || 'USD')}`]));
  const label = labelOf(kind === 'coin' ? 'output:' + id : 'addr:' + id);
  try {
    const path = await invoke('record_pdf', { name: vault.name, key: kind === 'coin' ? 'output:' + id : 'addr:' + id,
      title: label || (kind === 'coin' ? 'Coin ' + recPage.leaf : 'Address ' + id), subtitle: `${vault.name} \u00b7 ${vault.network} \u00b7 printed ${new Date().toLocaleDateString()}`,
      facts: P.facts.map(([k, v]) => [k, strip(v)]), sections: P.ccy ? [['Where its cost comes from', lots]] : [], charts: [...(chart ? [chart] : []), ...charts], file: `${vault.name}-${kind}-${id.slice(0, 12)}` });
    toast('Saved ' + path, 5000);
  } catch (e) { if (!/cancelled/.test(String(e))) fail(e); }
}
async function goVaultTx(name, txid) {
  const v = store.vaults.find(x => x.name === name); if (!v) return toast(`${name} is not on this computer any more`);
  vault = v; rememberVault(v); paintNet();
  if (!vault._snap) { try { await sync(); } catch {} }
  if (!txOrder().some(x => x.txid === txid)) { await sync(); }
  openTx(txid);
}
document.addEventListener('click', (e) => { const g = e.target.closest('[data-go-vault-tx]'); if (!g) return; e.preventDefault(); e.stopPropagation(); goVaultTx(g.dataset.goVaultTx, g.dataset.txid); }, true);
// cross-links on a page: a transaction, an address, a coin — each opens its own page
document.addEventListener('click', (e) => {
  const t = e.target.closest('[data-go-tx]'), a = e.target.closest('[data-go-addr]'), c = e.target.closest('[data-go-coin]');
  if (!t && !a && !c) return; e.preventDefault();
  if (t) return openTx(t.dataset.goTx);
  if (a) { show('addresses'); Promise.resolve(renderAddresses()).then(() => recPageOpen('addr', a.dataset.goAddr)); return; }
  if (c) { show('coins'); Promise.resolve(renderAddresses()).then(() => { renderCoins2(); recPageOpen('coin', c.dataset.goCoin); }); }
});
document.addEventListener('keydown', (e) => {
  if (!recPage.kind || !($('#s-coins').classList.contains('on') || $('#s-addresses').classList.contains('on')) || overTx() || e.metaKey || e.ctrlKey || e.altKey) return;
  if (e.key === 'ArrowLeft') recPageStep(-1); if (e.key === 'ArrowRight') recPageStep(1);
});
function renderRecHead() { if (recView.kind === 'coin') renderCoins2(); if (recView.kind === 'addr') renderAddresses(); }


// ---------- a vault over time: balance, value, and what it cost ----------
// The ledger is where correctness lives, so it is a plain function of the transactions and nothing else.
//
// Cost basis moves with the coins under each of the three methods. A deposit from outside costs what it was worth when it
// arrived. A spend takes its share of the basis with it. A transfer between two vaults in scope is not a sale and a
// purchase: the coins' cost goes across with them and only the fee's share is spent — otherwise moving coins between
// your own vaults would quietly reset their cost to that day's price. Shared transaction ids and the evidence index join the two legs; amount/time suggestions
// are joined only after confirmation. Spark has no coin lineage, so its account entries carry dated lots.
//
// It is what each deposit was worth on arrival, which is a cost only if the deposit was a purchase. A deposit from a
// wallet this app does not hold (an exchange, Sparrow) is counted at that day's price because nothing better is known.
function buildLedger(inputs, method, kinds, scope) {
  const inScope = name => !scope || scope.includes(name);
  method = method || 'average';
  kinds = kinds || {};
  // ---- events: one per transaction, with a leg per vault it touched and the coins it spent and made ----
  const ev = new Map();
  const inputByName = new Map(inputs.map(v => [v.name,v]));
  const eventKey = (v,r) => {
    const l=v.links?.[r.txid], peer=l && inputByName.get(l.peer), pr=peer?.rows.find(x=>x.txid===l.peer_id);
    if (l && !l.forgotten && !l.own && peer?.network === v.network && peer.links?.[l.peer_id]?.event === l.event && pr && r.net*pr.net<0 && (r.net<0 ? -r.net>=pr.net : -pr.net>=r.net)) return 'link:'+l.event;
    return (v.network || '') + ':' + r.txid;
  };
  for (const v of inputs) for (const r of v.rows) {
    if (r.net === null || r.net === undefined) continue;
    const t = r.time || Math.floor(Date.now() / 1000);   // not in a block yet: it is happening now
    const k = eventKey(v,r);
    const e = ev.get(k) || { txid: r.txid, t, legs: [], price: null };
    e.t = Math.min(e.t, t);
    e.legs.push({ vault: v.name, id:r.txid, t, price:r.quote?.price || (!r.time ? v.now?.price : null), net: v.links?.[r.txid]?.own ? 0 : r.net, ins: (r.ins || []).map(x => [...x]), outs: (r.outs || []).map(([n, sats]) => [r.txid + ':' + n, sats]) });
    const p = r.quote ? r.quote.price : (!r.time && v.now ? v.now.price : null);
    if (p && !e.price) e.price = p;
    ev.set(k, e);
  }
  for (const e of ev.values()) {
    const outgoing=e.legs.find(l=>l.net<0);
    if (outgoing) {e.txid=outgoing.id; e.t=outgoing.t; if(outgoing.price)e.price=outgoing.price;}
  }
  // ---- order: by time; inside one instant, a transaction that spends a coin another one made comes after it (the
  // chain's own order); anything still undecided takes covered spends first, then arrivals, then the rest ----
  const byT = new Map(); for (const e of ev.values()) { if (!byT.has(e.t)) byT.set(e.t, []); byT.get(e.t).push(e); }
  const netOf = (e) => e.legs.reduce((a, l) => a + l.net, 0);
  const events = []; let held = 0;
  for (const t of [...byT.keys()].sort((a, b) => a - b)) {
    let group = byT.get(t).sort((a, b) => (a.txid < b.txid ? -1 : 1));
    while (group.length) {
      // ready = spends no coin that another transaction still waiting in this instant makes
      const madeBy = new Map(); for (const e of group) for (const l of e.legs) for (const [op] of l.outs) madeBy.set(op, e);
      const ready = group.filter(e => !e.legs.some(l => l.ins.some(([op]) => { const m = madeBy.get(op); return m && m !== e; })));
      const pick = ready.length ? ready : group;
      const covered = pick.filter(e => netOf(e) < 0 && -netOf(e) <= held), arr = pick.filter(e => netOf(e) >= 0), rest = pick.filter(e => netOf(e) < 0 && -netOf(e) > held);
      const next = (covered[0] || arr[0] || rest[0]);
      events.push(next); held += netOf(next); group = group.filter(e => e !== next);
    }
  }
  // ---- the three ways of finding what a disposal cost ----
  const scale = (lots, f) => lots.map(l => ({ sats: l.sats * f, cost: l.cost === null ? null : l.cost * f, t: l.t }));
  const sumCost = (lots) => lots.some(l => l.cost === null) ? null : lots.reduce((a, l) => a + l.cost, 0);
  const coins = new Map();        // utxo: "txid:vout" → { vault, lots }
  const queues = {};              // fifo: vault → lots, oldest first
  const pools = {};               // average: vault → { sats, basis }
  const realised = {}, sats = {}, timeline = [];
  let internal = 0, unpriced = 0;
  const heldLots = () => method === 'utxo' ? [...coins.values()].filter(c => inScope(c.vault)).flatMap(c => c.lots) : method === 'fifo' ? Object.entries(queues).filter(([n]) => inScope(n)).flatMap(([,q]) => q) : null;
  const basisNow = () => method === 'average' ? Object.entries(pools).filter(([n]) => inScope(n)).reduce((a, [,p]) => a + p.basis, 0) : (heldLots().reduce((a, l) => a + (l.cost || 0), 0));
  const popFifo = (vault, n) => { const q = queues[vault] || (queues[vault] = []), out = []; let need = n;
    while (need > 1e-9 && q.length) { const l = q[0]; if (l.sats <= need + 1e-9) { out.push(q.shift()); need -= l.sats; } else { const f = need / l.sats; out.push({ sats: need, cost: l.cost === null ? null : l.cost * f, t: l.t }); l.sats -= need; if (l.cost !== null) l.cost *= (1 - f); need = 0; } }
    if (need > 1e-9) out.push({ sats: need, cost: null, t: null });   // more left than was known to be held: its cost is unknown, never zero
    return out; };
  for (const e of events) {
    const before = { sats: Object.entries(sats).filter(([n]) => inScope(n)).reduce((a, [,x]) => a + x, 0), basis: basisNow() };
    if (method === 'utxo') for (const l of e.legs) {
      if (l.ins.length || l.outs.length || !l.net) continue;
      if (l.net > 0) l.outs = [[l.id + ':account:' + l.vault, l.net]];
      else {
        let need = -l.net, used = 0;
        // Spark exposes amounts, not coin lineage: take its account lots oldest first.
        for (const [op,c] of coins) if (c.vault === l.vault && need > 0) {
          const n=c.lots.reduce((a,x)=>a+x.sats,0), take=Math.min(n,need);
          if(take<n) {
            const picked=l.id+':slice:'+op;
            coins.set(picked,{vault:l.vault,lots:scale(c.lots,take/n)});
            c.lots=scale(c.lots,1-take/n);l.ins.push([picked,take]);
          } else l.ins.push([op,take]);
          used+=take;need-=take;
        }
        if (need > 0) { l.ins.push([l.id+':unknown:'+l.vault,need]); used+=need; }
        if (used > -l.net) l.outs.push([l.id+':change:'+l.vault,used+l.net]);
      }
    }
    const insAll = e.legs.flatMap(l => l.ins.map(([op, n]) => ({ op, sats: n, vault: l.vault })));
    const outsAll = e.legs.flatMap(l => l.outs.map(([op, n]) => ({ op, sats: n, vault: l.vault })));
    const inTotal = insAll.reduce((a, x) => a + x.sats, 0), outTotal = outsAll.reduce((a, x) => a + x.sats, 0);
    const srcVaults = new Set(e.legs.filter(l => l.net < 0).map(l => l.vault)), dstVaults = new Set(e.legs.filter(l => l.net > 0).map(l => l.vault));
    const isInternal = srcVaults.size > 0 && dstVaults.size > 0 && [...dstVaults].some(v => !srcVaults.has(v));
    if (isInternal) internal++;
    let pool = null, left = 0;
    if (method === 'utxo' && (insAll.length || !e.legs.some(l => l.net < 0))) {
      pool = insAll.flatMap(i => { const c = coins.get(i.op); coins.delete(i.op); return c ? c.lots : [{ sats: i.sats, cost: null, t: null }]; });
      if (!insAll.length) {
        for (const o of outsAll) { const cost = e.price ? o.sats / 1e8 * e.price : null; if (cost === null) unpriced++; coins.set(o.op, { vault: o.vault, lots: [{ sats: o.sats, cost, t: e.t }] }); }
      } else {
        const carry = Math.min(1, outTotal / inTotal), extra = Math.max(0, outTotal - inTotal);
        // each coin that stays with us (change, or a move to another vault in scope) carries its share of every
        // spent coin's lots, WITH their dates; coins in beyond ours (a payjoin) are bought at that moment
        for (const o of outsAll) {
          const share = outTotal > 0 ? o.sats / outTotal : 0;
          const carried = scale(pool, share * carry);
          if (extra > 0) carried.push({ sats: extra * share, cost: e.price ? extra * share / 1e8 * e.price : null, t: e.t });
          coins.set(o.op, { vault: o.vault, lots: carried });
        }
        left = Math.max(0, inTotal - outTotal);
        if (left > 0) pool = scale(pool, left / inTotal); else pool = null;
      }
    } else if (method === 'fifo') {
      const leaving = [...srcVaults].map(v => { const l = e.legs.find(x => x.vault === v); return [v, -l.net]; });
      const popped = leaving.flatMap(([v, n]) => popFifo(v, n));
      const outflow = leaving.reduce((a, x) => a + x[1], 0), toOthersInScope = e.legs.filter(l => l.net > 0 && isInternal).reduce((a, l) => a + l.net, 0);
      for (const l of e.legs.filter(x => x.net > 0)) {
        const q = queues[l.vault] || (queues[l.vault] = []);
        if (isInternal && outflow > 0) q.push(...scale(popped, l.net / outflow));   // a move keeps the coins' dates and cost
        else { const cost = e.price ? l.net / 1e8 * e.price : null; if (cost === null) unpriced++; q.push({ sats: l.net, cost, t: e.t }); }
        q.sort((a, b) => (a.t || 0) - (b.t || 0));
      }
      left = outflow - toOthersInScope;
      pool = left > 0 && outflow > 0 ? scale(popped, left / outflow) : null;
    } else {   // average: per vault, one pool
      let leaving = 0;
      for (const l of e.legs.filter(x => x.net < 0)) { const p = pools[l.vault] || (pools[l.vault] = { sats: 0, basis: 0 }); const out = -l.net;
        const share = p.sats > 0 ? p.basis * Math.min(1, out / p.sats) : 0; p.basis -= share; p.sats -= out; leaving += share; }
      const outflow = e.legs.filter(x => x.net < 0).reduce((a, l) => a - l.net, 0), inflow = e.legs.filter(x => x.net > 0).reduce((a, l) => a + l.net, 0);
      for (const l of e.legs.filter(x => x.net > 0)) { const p = pools[l.vault] || (pools[l.vault] = { sats: 0, basis: 0 });
        if (isInternal) p.basis += (outflow > 0 ? leaving * Math.min(1, inflow / outflow) : 0) * (l.net / inflow); else if (e.price) p.basis += l.net / 1e8 * e.price; else unpriced++;
        p.sats += l.net; }
      left = Math.max(0, outflow - (isInternal ? inflow : 0));
      pool = left > 0 ? [{ sats: left, cost: outflow > 0 ? leaving * left / outflow : 0, t: null }] : null;
    }
    for (const l of e.legs) sats[l.vault] = (sats[l.vault] || 0) + l.net;
    if (pool && left > 0 && e.legs.some(l => l.net < 0 && inScope(l.vault))) {
      const cost = sumCost(pool), proceeds = e.price ? left / 1e8 * e.price : null;
      const dated = pool.filter(l => l.t !== null);
      // What the person said this spend WAS. Only "my own wallet elsewhere" changes the arithmetic: coins moved
      // between wallets of the same owner are not disposed of, so nothing is realised and the cost leaves the books
      // with them. The app cannot tell that from the chain — an address it does not hold looks like anyone's.
      realised[e.txid] = { sats: left, cost, proceeds, gain: cost !== null && proceeds !== null ? proceeds - cost : null, t: e.t,
        from: dated.length ? Math.min(...dated.map(l => l.t)) : null, to: dated.length ? Math.max(...dated.map(l => l.t)) : null, lots: pool, method, kind: kinds[e.txid] || '' };
      if (kinds[e.txid] === 'self') { realised[e.txid].moved = true; realised[e.txid].gain = null; realised[e.txid].proceeds = null; }
    }
    if (realised[e.txid]) for (const l of e.legs) if (l.net < 0) realised[l.id] = realised[e.txid];
    timeline.push({ t: e.t, txid: e.txid, price: e.price, before, sats: Object.entries(sats).filter(([n]) => inScope(n)).reduce((a, [,x]) => a + x, 0), basis: basisNow(), internal: isInternal });
  }
  // each coin still held, with the lots its sats came from (meaningful under 'utxo', where coins keep their own history)
  return { coins: Object.fromEntries([...coins].map(([op, c]) => [op, c.lots])), queues, pools, timeline, internal, unpriced, realised, method, perVault: { sats, basis: method === 'average' ? Object.fromEntries(Object.entries(pools).map(([k, p]) => [k, p.basis])) : Object.fromEntries(Object.keys(sats).map(v => [v, (method === 'utxo' ? [...coins.values()].filter(c => c.vault === v).flatMap(c => c.lots) : (queues[v] || [])).reduce((a, l) => a + (l.cost || 0), 0)])) } };
}
// Which coins a disposal is taken to be — the taxpayer's rule, not the app's. Sources in roadmap/anytwokeys/ACCOUNTING.md.
const COST_METHODS = {
  utxo:    { name: 'The coins actually spent', short: 'the coins this transaction actually spent', note: 'Specific identification, with the chain as the record: a transaction names exactly which coins it spends. Accepted in Australia (the ATO accepts your identification of the units disposed of, with records) and in the United States.' },
  fifo:    { name: 'First in, first out', short: 'first in, first out', note: 'The oldest coins are taken to leave first, whichever the transaction spent. The fallback in Australia and the default in the United States when units are not identified; counted vault by vault, as US rules have required since 2025.' },
  average: { name: 'Average cost', short: 'average cost', note: 'Every coin costs the same average. NOT generally accepted for bitcoin: the ATO accepts it only for units acquired on the same day, and the US not at all. The UK pools instead, with its own same-day and 30-day rules, which this app does not compute. Kept for reference.' },
};
const methodFor = (v) => (v && v.cost_method) || (store && store.settings && store.settings.cost_method) || 'utxo';
// A vault's ledger is built from EVERY vault whose figures are loaded, not this one alone: moving coins to another
// of your own wallets is not a disposal, and its cost goes with them — which the ledger can only see if it is
// shown both ends. (Loaded elsewhere: fillActivity fetches a transfer's other vault when it sees one.)
function vaultLedger(name) {
  const f = txFacts[name], v = store && store.vaults.find(x => x.name === name);
  if (!f || !v) return null;
  // only the vaults this one has actually moved coins with (its transfers), not every vault that happens to be
  // loaded: another vault's history says nothing about this one unless they share a transaction
  const names = [name];
  for (let i=0; i<names.length; i++) for (const peer of Object.values(txFacts[names[i]]?.transfers || {}).flat())
    if (!names.includes(peer) && txFacts[peer] && store.vaults.some(x => x.name === peer && x.network === v.network)) names.push(peer);
  const inputs = names.map(n => ({ name: n, network: v.network, links: txFacts[n].links, rows: Object.values(txFacts[n].rows), now: txFacts[n].now })).filter(i => i.rows.length || i.name === name);
  // what the person said each spend out of these vaults was (Send asks; Activity can change it)
  const kinds = {};
  for (const n of names) for (const [k, rec] of Object.entries((store.vaults.find(x => x.name === n) || {}).more || {}))
    if (k.startsWith('tx:') && rec.disposal && rec.disposal.kind) kinds[k.slice(3)] = rec.disposal.kind;
  const m = methodFor(v), key = m + ':' + inputs.map(i => i.name + ':' + i.rows.length).join(',') + ':' + (f.now ? f.now.price : 0) + ':' + JSON.stringify(kinds);
  if (!vaultLedger.memo[name] || vaultLedger.memo[name].key !== key) vaultLedger.memo[name] = { key, L: buildLedger(inputs, m, kinds, [name]) };
  return vaultLedger.memo[name].L;
}
vaultLedger.memo = {};
const ledgerAt = (L, t) => { let s = { sats: 0, basis: 0 }; for (const e of L.timeline) { if (e.t > t) break; s = e; } return s; };
// What a spend WOULD realise, before it is made: the cost of the sats that actually leave (the payment and the fee,
// never the change), under the method this taxpayer uses. Only 'utxo' answers from the coins picked — under FIFO the
// oldest lots leave whichever coins the transaction spends, and under average cost every sat costs the same.
function costOfLeaving(L, vaultName, chosen, spending, leaving) {
  if (!L || !leaving) return null;
  const scale = (lots, f) => lots.map(l => ({ sats: l.sats * f, cost: l.cost === null ? null : l.cost * f, t: l.t }));
  let lots = null;
  if (L.method === 'utxo') {
    const have = chosen.map(op => L.coins[op]);
    if (have.some(x => !x)) return { sats: leaving, cost: null, from: null, to: null, why: 'one of these coins has no history here yet' };
    // change carries its share back, so only leaving/spending of every lot is realised — as the ledger itself does
    lots = scale(have.flat(), spending > 0 ? leaving / spending : 0);
  } else if (L.method === 'fifo') {
    const q = (L.queues && L.queues[vaultName]) || []; lots = []; let need = leaving;
    for (const l of q) { if (need <= 1e-9) break; const take = Math.min(l.sats, need); lots.push({ sats: take, cost: l.cost === null ? null : l.cost * (take / l.sats), t: l.t }); need -= take; }
    if (need > 1e-9) lots.push({ sats: need, cost: null, t: null });
  } else {
    const pool = (L.pools && L.pools[vaultName]) || { sats: 0, basis: 0 };
    lots = [{ sats: leaving, cost: pool.sats > 0 ? pool.basis * Math.min(1, leaving / pool.sats) : null, t: null }];
  }
  const dated = lots.filter(l => l.t !== null);
  return { sats: leaving, cost: lots.some(l => l.cost === null) ? null : lots.reduce((a, l) => a + l.cost, 0),
    from: dated.length ? Math.min(...dated.map(l => l.t)) : null, to: dated.length ? Math.max(...dated.map(l => l.t)) : null, lots: dated,
    why: lots.some(l => l.cost === null) ? 'part of what leaves has no known cost' : '' };
}

// The three series for a range. Balance and basis only change at transactions, so they are steps; value moves with
// the price, so it is sampled — and it steps at each transaction too, because holdings did.
function sumSeries(L, from, to, prices, now) {
  // one point per instant: several transactions in one block are drawn as the state after all of them, because the
  // states in between existed only in the order this app chose to apply them (a spike to 39,010 sats, a dip to 0)
  const raw = L.timeline.filter(e => e.t > from && e.t <= to), inRange = [];
  for (const e of raw) { const last = inRange[inRange.length - 1];
    if (last && last.t === e.t) inRange[inRange.length - 1] = Object.assign({}, e, { before: last.before, price: last.price || e.price });
    else inRange.push(e); }
  const start = ledgerAt(L, from);
  const bal = [[from, start.sats]], cost = [[from, start.basis]], val = [];
  for (const e of inRange) { bal.push([e.t, e.sats]); cost.push([e.t, e.basis]); }
  bal.push([to, (inRange.length ? inRange[inRange.length - 1] : start).sats]);
  cost.push([to, (inRange.length ? inRange[inRange.length - 1] : start).basis]);
  const px = (day) => { const q = prices[day]; return q ? q.price : null; };
  for (const [day] of Object.entries(prices)) {
    const t = Date.parse(day + 'T23:59:59Z') / 1000; if (t < from || t > to) continue;
    const p = px(day); if (p === null) continue;
    val.push([t, ledgerAt(L, t).sats / 1e8 * p]);
  }
  const dayPrices = Object.entries(prices).filter(([, q]) => q).map(([d, q]) => [Date.parse(d + 'T12:00:00Z') / 1000, q.price]);
  const nearPrice = (t) => { let best = null; for (const [x, p] of dayPrices) if (!best || Math.abs(x - t) < Math.abs(best[0] - t)) best = [x, p]; return best ? best[1] : (now ? now.price : null); };
  for (const e of inRange) { const p = e.price || nearPrice(e.t); if (p === null) continue; val.push([e.t, e.before.sats / 1e8 * p]); val.push([e.t, e.sats / 1e8 * p]); }
  if (now) val.push([to, (inRange.length ? inRange[inRange.length - 1] : start).sats / 1e8 * now.price]);
  val.sort((a, b) => a[0] - b[0]);
  return { bal, cost, val };
}

// ---------- a line chart: one axis, thin marks, a crosshair, and a table behind it ----------
const niceMax = (m) => { if (!(m > 0)) return { top: 1, step: 1 }; const raw = m / 4, mag = 10 ** Math.floor(Math.log10(raw)), f = raw / mag; const step = (f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10) * mag; return { top: Math.ceil(m * 1.06 / step) * step, step }; };
const csats = (n) => n >= 1e8 ? +(n / 1e8).toFixed(2) + ' BTC' : n >= 1e6 ? +(n / 1e6).toFixed(1) + 'M' : n >= 1e4 ? Math.round(n / 1e3) + 'k' : n >= 1e3 ? +(n / 1e3).toFixed(1) + 'k' : String(Math.round(n));
const cmoney = (n, ccy) => { try { return new Intl.NumberFormat(undefined, { style: 'currency', currency: ccy, notation: 'compact', maximumFractionDigits: 1 }).format(n); } catch { return money(n, ccy); } };
function xLabel(t, span) {
  const d = new Date(t * 1000);
  if (span <= 3 * 86400) return d.toLocaleString(undefined, { day: 'numeric', month: 'short', hour: 'numeric' });
  if (span <= 200 * 86400) return d.toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
  return d.toLocaleDateString(undefined, { month: 'short', year: 'numeric' });   // "Dec 22" read as the 22nd of December
}
// The value a series has AT time t: a step series holds its last value; a sampled one reads its nearest point.
function seriesAt(s, t) {
  if (!s.pts.length) return null;
  if (s.step) { let v = s.pts[0][1]; for (const [x, y] of s.pts) { if (x > t) break; v = y; } return v; }
  let i = -1; for (let k = 0; k < s.pts.length; k++) if (s.pts[k][0] <= t) i = k;
  if (i < 0) return s.pts[0][1];
  if (i === s.pts.length - 1) return s.pts[i][1];
  const [x0, y0] = s.pts[i], [x1, y1] = s.pts[i + 1];
  return x1 === x0 ? y1 : y0 + (y1 - y0) * (t - x0) / (x1 - x0);
}
function lineChart(el, { title, sub, series, fmt, fmtAxis, from, to }) {
  el.innerHTML = '';
  const h4 = document.createElement('h4'); h4.textContent = title; el.appendChild(h4);
  if (sub) { const d = document.createElement('div'); d.className = 'sub'; d.textContent = sub; el.appendChild(d); }
  if (series.length > 1) {   // two or more: a legend, always, with line keys because these are lines
    const lg = document.createElement('div'); lg.className = 'legend';
    for (const s of series) { const sp = document.createElement('span'); const i = document.createElement('i'); i.style.background = s.color; sp.append(i, document.createTextNode(s.name)); lg.appendChild(sp); }
    el.appendChild(lg);
  }
  const W = Math.max(280, el.clientWidth || 560), H = 168, m = { l: 58, r: 84, t: 8, b: 22 };
  const pw = W - m.l - m.r, ph = H - m.t - m.b;
  const maxY = Math.max(0, ...series.flatMap(s => s.pts.map(p => p[1])));
  const { top, step } = niceMax(maxY);
  const span = Math.max(1, to - from);
  const X = (t) => m.l + (t - from) / span * pw, Y = (v) => m.t + ph - (v / top) * ph;
  const NS = 'http://www.w3.org/2000/svg', mk = (tag, a) => { const n = document.createElementNS(NS, tag); for (const k in a) n.setAttribute(k, a[k]); return n; };
  const svg = mk('svg', { viewBox: `0 0 ${W} ${H}`, height: H, role: 'img' });
  svg.setAttribute('aria-label', `${title}. ${series.map(s => `${s.name}: ${fmt(seriesAt(s, to) || 0)} now`).join('; ')}. A table of the same figures is below.`);
  const ax = mk('g', { class: 'axis' });
  for (let v = 0; v <= top + 1e-9; v += step) {   // recessive: one-step-off-surface hairlines, solid
    ax.appendChild(mk('line', { class: 'grid', x1: m.l, x2: m.l + pw, y1: Y(v), y2: Y(v) }));
    const tx = mk('text', { x: m.l - 8, y: Y(v) + 3.5, 'text-anchor': 'end' }); tx.textContent = fmtAxis(v); ax.appendChild(tx);
  }
  for (let i = 0; i <= 3; i++) { const t = from + span * i / 3; const tx = mk('text', { x: X(t), y: H - 6, 'text-anchor': i === 0 ? 'start' : i === 3 ? 'end' : 'middle' }); tx.textContent = xLabel(t, span); ax.appendChild(tx); }
  svg.appendChild(ax);
  const ends = [];
  for (const s of series) {
    if (!s.pts.length) continue;
    let d = '';
    s.pts.forEach(([t, v], i) => { const x = X(t), y = Y(v); d += i === 0 ? `M${x},${y}` : s.step ? `H${x}V${y}` : `L${x},${y}`; });
    if (s.area && series.length === 1) { const a = mk('path', { class: 'wash', d: `${d}V${Y(0)}H${X(s.pts[0][0])}Z` }); a.style.fill = s.color; svg.appendChild(a); }
    const ln = mk('path', { class: 'ln', d }); ln.style.stroke = s.color; svg.appendChild(ln);
    const [lt, lv] = s.pts[s.pts.length - 1];
    const dot = mk('circle', { class: 'dot', cx: X(lt), cy: Y(lv), r: 4 }); dot.style.fill = s.color; svg.appendChild(dot);
    ends.push({ y: Y(lv), x: X(lt), text: fmt(lv) });
  }
  ends.sort((a, b) => a.y - b.y);   // end labels never sit on each other
  for (let i = 1; i < ends.length; i++) if (ends[i].y - ends[i - 1].y < 14) ends[i].y = ends[i - 1].y + 14;
  for (const e of ends) { const tx = mk('text', { class: 'endlbl', x: e.x + 9, y: e.y + 4 }); tx.textContent = e.text; svg.appendChild(tx); }
  // the crosshair finds the X: a hairline that snaps to the nearest moment anything changed, one readout for all
  const xs = [...new Set(series.flatMap(s => s.pts.map(p => p[0])))].sort((a, b) => a - b);
  const hair = mk('line', { class: 'hair', y1: m.t, y2: m.t + ph, visibility: 'hidden' }); svg.appendChild(hair);
  const hit = mk('rect', { class: 'hit plotframe', x: m.l, y: m.t, width: pw, height: ph, tabindex: 0 });
  hit.setAttribute('aria-label', `${title}: use the arrow keys to move through the dates`);
  svg.appendChild(hit); el.appendChild(svg);
  const tip = document.createElement('div'); tip.className = 'ctip'; tip.hidden = true; el.appendChild(tip);
  let at = xs.length - 1;
  const show = (i, clientX) => {
    if (!xs.length) return; at = Math.max(0, Math.min(xs.length - 1, i)); const t = xs[at], x = X(t);
    hair.setAttribute('x1', x); hair.setAttribute('x2', x); hair.setAttribute('visibility', 'visible');
    tip.innerHTML = ''; const dd = document.createElement('div'); dd.className = 'd';
    dd.textContent = new Date(t * 1000).toLocaleString(undefined, { day: 'numeric', month: 'short', year: 'numeric', hour: 'numeric', minute: '2-digit' }); tip.appendChild(dd);
    for (const s of series) { const r = document.createElement('div'); r.className = 'r'; const i2 = document.createElement('i'); i2.style.background = s.color; const b = document.createElement('b'); b.textContent = fmt(seriesAt(s, t) || 0); const n = document.createElement('span'); n.textContent = s.name; r.append(i2, b, n); tip.appendChild(r); }
    tip.hidden = false;
    const px = x / W * el.clientWidth, right = px > el.clientWidth - 190;
    tip.style.left = (right ? px - tip.offsetWidth - 12 : px + 12) + 'px'; tip.style.top = (m.t + 30) + 'px';
  };
  const hide = () => { hair.setAttribute('visibility', 'hidden'); tip.hidden = true; };
  const nearest = (clientX) => { const r = svg.getBoundingClientRect(), t = from + ((clientX - r.left) / r.width * W - m.l) / pw * span; let bi = 0; xs.forEach((x, i) => { if (Math.abs(x - t) < Math.abs(xs[bi] - t)) bi = i; }); return bi; };
  hit.addEventListener('pointermove', (e) => show(nearest(e.clientX)));
  hit.addEventListener('pointerleave', hide);
  // the readout on focus is for the keyboard; focus that lands here by itself (a re-render taking the focused
  // element away) must not pop a reading of its own
  hit.addEventListener('focus', () => { if (hit.matches(':focus-visible')) show(at); }); hit.addEventListener('blur', hide);
  hit.addEventListener('keydown', (e) => { if (e.key === 'ArrowLeft') { e.preventDefault(); show(at - 1); } if (e.key === 'ArrowRight') { e.preventDefault(); show(at + 1); } if (e.key === 'Home') { e.preventDefault(); show(0); } if (e.key === 'End') { e.preventDefault(); show(xs.length - 1); } });
  return { xs };
}

window.addEventListener('resize', () => { clearTimeout(window.__sumResize); window.__sumResize = setTimeout(() => { if ($('#s-home').classList.contains('on')) renderHomeSummary(); if ($('#s-overview').classList.contains('on')) renderOverview(); }, 180); });
// ---------- the summary card, for one vault or for all of them ----------
const SUM_RANGES = [['30', '30 days', 30], ['90', '90 days', 90], ['365', '1 year', 365], ['all', 'All', 0]];
const sumPref = (k, d) => { try { return localStorage.getItem('a2k-sum-' + k) || d; } catch { return d; } };
const sumSave = (k, v) => { try { localStorage.setItem('a2k-sum-' + k, v); } catch {} };
// One request for the whole range: the price service's series question answers every day (or every Nth, for a span
// longer than a year) exactly as it answers each day on its own. Rust keeps closed days in the sealed cache for good.
const sumRuns = {};   // "CCY:from:to" → { every, prices }, for this session
const isoDay = (t) => new Date(t * 1000).toISOString().slice(0, 10);
async function pricesRange(ccy, fromT, toT) {
  const from = isoDay(fromT), to = isoDay(toT), key = `${ccy}:${from}:${to}`;
  if (!sumRuns[key]) {
    try { const r = await invoke('prices_series', { ccy, from, to }); sumRuns[key] = { every: r.every, prices: Object.fromEntries(r.points.map(([d, q]) => [d, q])), ok: true }; }
    catch (e) { return { every: 1, prices: {}, ok: false, why: String(e) }; }   // not kept: a failure is asked again next time
  }
  return sumRuns[key];
}
// What the ledger needs for one vault: its transactions with dates, amounts and prices. The screen usually has it.
// how many past prices the backend is still fetching for this vault — its own count, from its own note
const pricesPending = note => { const m = /^Fetching (\d+) past price/.exec(note || ''); return m ? Number(m[1]) : 0; };
async function sumInput(name) {
  const v = store.vaults.find(x => x.name === name); if (!v) return null;
  const ccy = v.network === 'mainnet' ? fiatCcy() : '';
  let f = txFacts[name];
  if (!f || (f.stale && name !== vault?.name) || (f.owner && f.owner !== factOwner(v)) || (f.ccy || '') !== ccy) {
    const s = await invoke('vault_sync', { name });
    const a = await invoke('vault_activity', { name, history: s.history.map(h => [h.txid, h.height]), ccy });
    f = keepActivity(name, a); f.history=s.history.map(h=>[h.txid,h.height]); await refreshTransferLinks();
  }
  return { name, network: v.network, links: f.links, rows: Object.values(f.rows), now: f.now, ccy: f.ccy, pending: f.pending || 0 };
}
async function renderSummary(box, names, opts = {}) {
  const me = (renderSummary.seq = (renderSummary.seq || 0) + 1);
  // Keeping the last drawing, dimmed, is right for a REFRESH of the same wallet and wrong for a different one: it
  // would leave another vault's line on screen, and a stray focus on its plot would read that vault's figures out.
  const who = names.join('\u0000');
  if (box.dataset.names !== undefined && box.dataset.names !== who) box.innerHTML = '';
  box.dataset.names = who;
  box.classList.add('sum-loading');   // refetch keeps the frame: the last drawing stays, dimmed, until the new one is ready
  // one vault that cannot be read (its reader locked, its node away) must not blank the others: draw what came,
  // say what did not, and mark it on its chip
  const settled = await Promise.all(names.map(n => sumInput(n).then(v => ({ v }), e => ({ e, n }))));
  if (me !== renderSummary.seq) return;
  const inputs = settled.filter(r => r.v).map(r => r.v);
  const locked = settled.filter(r => r.e).map(r => ({ name: r.n, why: lockedWhy(r.e) }));
  if (opts.chips) for (const b of opts.chips.querySelectorAll('.vchip')) { const l = locked.find(x => x.name === b.dataset.vault); b.classList.toggle('locked', !!l); b.title = l ? l.why : ''; const old = b.querySelector('svg'); if (old && !l) old.remove(); if (l && !old) b.insertAdjacentHTML('afterbegin', '<svg aria-hidden="true"><use href="#i-lock"/></svg>'); }
  const lockedLine = () => { if (!locked.length) return null; const d = document.createElement('div'); d.className = 'sum-locked'; d.innerHTML = `<svg aria-hidden="true"><use href="#i-lock"/></svg><span>${locked.map(l => `<b>${esc(l.name)}</b> is not counted: ${esc(l.why)}.`).join(' ')} ${locked.length === 1 ? 'The figures are' : 'The figures are'} incomplete until ${locked.length === 1 ? 'it is' : 'they are'} unlocked and refreshed.</span>`; return d; };
  if (!inputs.length && locked.length) { box.classList.remove('sum-loading'); box.innerHTML = ''; const lbl0 = document.createElement('div'); lbl0.className = 'label'; lbl0.textContent = opts.title || 'Over time'; box.append(lbl0); if (opts.chips) box.append(opts.chips); box.append(lockedLine()); return; }
  const priced = inputs.length && inputs.every(i => i.network === 'mainnet' && i.ccy);
  const ccy = priced ? inputs[0].ccy : '';
  const now = priced ? (inputs.find(i => i.now) || {}).now : null;
  const methods = [...new Set(inputs.map(i => methodFor(store.vaults.find(v => v.name === i.name))))];
  const method = methods.length === 1 ? methods[0] : methodFor(null);
  const ledgerInputs = linkedNames(names).map(n => ({name:n, network:store.vaults.find(v=>v.name===n)?.network, rows:Object.values(txFacts[n]?.rows || {}), now:txFacts[n]?.now, links:txFacts[n]?.links}));
  const L = buildLedger(ledgerInputs, method, {}, names);
  const rangeKey = sumPref('range', 'all'), R = SUM_RANGES.find(r => r[0] === rangeKey) || SUM_RANGES[3];
  const to = Math.floor(Date.now() / 1000), first = L.timeline.length ? L.timeline[0].t : to;
  const from = R[2] ? Math.min(to - R[2] * 86400, to - 3600) : Math.max(0, first - Math.max(3600, (to - first) * 0.03));
  const run = priced && L.timeline.length ? await pricesRange(ccy, Math.max(from, first), to) : { every: 1, prices: {}, ok: true };
  const prices = run.prices;
  if (me !== renderSummary.seq) return;
  const S = sumSeries(L, from, to, prices, now);
  const nowState = L.timeline.length ? L.timeline[L.timeline.length - 1] : { sats: 0, basis: 0 };
  const value = now ? nowState.sats / 1e8 * now.price : null, gain = value !== null ? value - nowState.basis : null;
  // past prices still on their way: a basis drawn without them is not a small basis, it is an unfinished one
  const loading = linkedNames(names).map(n=>txFacts[n] || {}).reduce((n, i) => n + (i.pending || 0), 0), loadingS = loading === 1 ? '' : 's';
  box.classList.remove('sum-loading');
  box.innerHTML = '';
  const topRow = document.createElement('div'); topRow.className = 'sum-top';
  const lbl = document.createElement('div'); lbl.className = 'label'; lbl.textContent = opts.title || 'Over time';
  const filters = document.createElement('div'); filters.className = 'sum-filters';
  if (opts.chips) filters.appendChild(opts.chips);
  const seg = document.createElement('div'); seg.className = 'seg'; seg.setAttribute('role', 'radiogroup'); seg.setAttribute('aria-label', 'Time range');
  for (const [k, n] of SUM_RANGES) { const b = document.createElement('button'); b.type = 'button'; b.setAttribute('role', 'radio'); b.setAttribute('aria-checked', k === R[0] ? 'true' : 'false'); b.dataset.range = k; b.textContent = n; b.onclick = () => { sumSave('range', k); renderSummary(box, names, opts); }; seg.appendChild(b); }
  filters.appendChild(seg); topRow.append(lbl, filters); box.appendChild(topRow);
  { const ll = lockedLine(); if (ll) box.appendChild(ll); }
  if (!L.timeline.length) { const e = document.createElement('div'); e.className = 'sum-empty'; e.textContent = opts.empty || 'Nothing has arrived yet, so there is nothing to draw.'; box.appendChild(e); return; }
  // stat tiles first: the numbers a reader came for, before any picture
  const k = document.createElement('div'); k.className = 'kpis';
  const tile = (key, v, sub, cls) => { const d = document.createElement('div'); d.className = 'kpi'; d.innerHTML = `<div class="k"></div><div class="v"></div><div class="s"></div>`; d.querySelector('.k').textContent = key; d.querySelector('.v').textContent = v; const sEl = d.querySelector('.s'); sEl.textContent = sub || ''; if (cls) sEl.classList.add(cls); return d; };
  k.appendChild(tile('Holding', sats(nowState.sats) + ' sats', inputs.length > 1 ? `across ${inputs.length} vaults` : ''));
  if (priced && value !== null) {
    k.appendChild(tile('Worth now', money(value, ccy), '1 BTC = ' + money(now.price, ccy)));
    k.appendChild(tile('Cost basis', money(nowState.basis, ccy), loading ? `incomplete \u00b7 ${loading} past price${loadingS} still loading` : 'what it was worth as it arrived'));
    const pct = nowState.basis > 0 ? gain / nowState.basis * 100 : null;
    // a status colour, so it goes with a sign and an arrow and never by colour alone
    if (loading) k.appendChild(tile('Unrealised', '\u2026', `settles when ${loading === 1 ? 'that price arrives' : 'those prices arrive'}`));
    else k.appendChild(tile('Unrealised', (gain >= 0 ? '▲ +' : '▼ −') + money(Math.abs(gain), ccy), pct === null ? '' : (gain >= 0 ? '+' : '−') + Math.abs(pct).toFixed(1) + '% on cost', gain >= 0 ? 'up' : 'down'));
    const inR = Object.values(L.realised).filter(x => x.t > from && x.t <= to), known = inR.filter(x => x.gain !== null), rg = known.reduce((a, x) => a + x.gain, 0);
    k.appendChild(tile('Realised', inR.length ? (rg >= 0 ? '▲ +' : '▼ −') + money(Math.abs(rg), ccy) : 'nothing yet', inR.length ? `${inR.length} disposal${inR.length === 1 ? '' : 's'} in this range${known.length < inR.length ? ` \u00b7 ${inR.length - known.length} of unknown cost` : ''}` : 'nothing has left in this range', inR.length ? (rg >= 0 ? 'up' : 'down') : ''));
  }
  box.appendChild(k);
  const c1 = document.createElement('div'); c1.className = 'chart'; box.appendChild(c1);
  lineChart(c1, { title: 'Balance, in sats', sub: 'It changes only when something arrives or leaves.', from, to,
    series: [{ name: 'Balance', color: 'var(--series-value)', pts: S.bal, step: true, area: true }],
    fmt: (v) => sats(Math.round(v)) + ' sats', fmtAxis: (v) => csats(v) });
  if (priced) {
    const c2 = document.createElement('div'); c2.className = 'chart'; box.appendChild(c2);
    lineChart(c2, { title: `What it is worth, and what it cost, in ${ccy}`, sub: 'The gap between the two lines is the gain or loss not yet realised.', from, to,
      series: [{ name: 'Worth', color: 'var(--series-value)', pts: S.val, step: false }, { name: 'Cost basis', color: 'var(--series-cost)', pts: S.cost, step: true }],
      fmt: (v) => money(v, ccy), fmtAxis: (v) => cmoney(v, ccy) });
  }
  // where the price sat on the long cycle as coins arrived and left: drawn when its marks are here, into its own slot
  if (priced) { const card = document.createElement('div'); box.appendChild(card); cycleSummary(L, card, () => me === renderSummary.seq); }
  // what the figures rest on, said once and plainly
  const notes = [];
  if (!priced) notes.push(inputs.some(i => i.network !== 'mainnet') ? 'Test coins have no price, so only the balance is drawn.' : 'No currency is chosen in App settings, so only the balance is drawn.');
  if (priced && run.ok) notes.push(run.every === 1 ? 'Worth is drawn from every day\u2019s closing price, and at each transaction.' : `Worth is drawn from the closing price on every ${run.every}th day across this range, and at each transaction, so a sharp move between two of those days does not show.`);
  if (priced && !run.ok) notes.push(`Daily prices could not be fetched just now (${run.why}), so worth is drawn only at each transaction.`);
  const holes = Object.values(prices).filter(q => q === null).length;
  if (priced && holes) notes.push(`${holes} day${holes === 1 ? ' has' : 's have'} no price at the source and ${holes === 1 ? 'is' : 'are'} left out rather than filled from ${holes === 1 ? 'its' : 'their'} neighbours.`);
  if (priced) notes.push(`Cost is by ${COST_METHODS[method].short}: what each coin was worth when it arrived \u2014 a true cost only if it was a purchase.`);
  if (methods.length > 1) notes.push('These vaults do not all follow the same method, so they are counted here by the app\u2019s; each vault\u2019s own Home shows it by its own.');
  if (L.internal) notes.push(`${L.internal} transfer${L.internal === 1 ? '' : 's'} between these vaults ${L.internal === 1 ? 'was' : 'were'} recognised and counted as a move, not a sale and a purchase: the coins kept their cost.`);
  if (method === 'utxo' && linkedNames(names).some(n=>store.vaults.find(v=>v.name===n)?.spark)) notes.push('Spark supplies no coin lineage. Its account entries are used oldest first; the chain’s actual inputs are still followed.');
  if (loading) notes.push(`${loading} past price${loadingS} ${loading === 1 ? 'is' : 'are'} still being fetched; cost basis and gains settle as they arrive.`);
  else if (L.unpriced) notes.push(`${L.unpriced} deposit${L.unpriced === 1 ? ' has' : 's have'} no price, so ${L.unpriced === 1 ? 'its' : 'their'} cost is left out rather than guessed.`);
  if (opts.extra) notes.push(opts.extra);
  const nt = document.createElement('div'); nt.className = 'muted'; nt.style.cssText = 'font-size:12.5px;margin-top:12px'; nt.textContent = notes.join(' '); box.appendChild(nt);
  // a table of the same figures: nothing is reachable only by hovering
  const tg = document.createElement('button'); tg.type = 'button'; tg.className = 'linkish'; tg.style.marginTop = '8px'; tg.textContent = 'Show the figures as a table';
  const tw = document.createElement('div'); tw.className = 'sum-table'; tw.hidden = true;
  tg.onclick = () => {
    tw.hidden = !tw.hidden; tg.textContent = tw.hidden ? 'Show the figures as a table' : 'Hide the table'; if (tw.hidden) return;
    const cols = [{ n: 'Balance', s: { pts: S.bal, step: true }, f: (v) => sats(Math.round(v)) + ' sats' }].concat(priced ? [{ n: 'Worth', s: { pts: S.val }, f: (v) => money(v, ccy) }, { n: 'Cost basis', s: { pts: S.cost, step: true }, f: (v) => money(v, ccy) }] : []);
    const xs = [...new Set([...S.bal, ...S.val, ...S.cost].map(p => p[0]))].sort((a, b) => b - a);
    const t = document.createElement('table'), hr = t.insertRow(); for (const h of ['When'].concat(cols.map(c => c.n))) { const th = document.createElement('th'); th.textContent = h; hr.appendChild(th); }
    for (const x of xs) { const r = t.insertRow(); r.insertCell().textContent = new Date(x * 1000).toLocaleString(undefined, { day: 'numeric', month: 'short', year: 'numeric', hour: 'numeric', minute: '2-digit' }); for (const c of cols) r.insertCell().textContent = c.f(seriesAt(c.s, x) || 0); }
    tw.innerHTML = ''; tw.appendChild(t);
  };
  box.append(tg, tw);
}
function renderHomeSummary() { if (vault && $('#s-home').classList.contains('on')) renderSummary($('#home-summary'), [vault.name]); }

// ---------- all vaults: the same view, of everything that has a price ----------
// why a vault's history could not be read just now, in the words the person needs
function lockedWhy(e) { const s = String(e); if (s.startsWith('SPARK_UNLOCK:')) return 'its YubiKey reader is locked — open the vault, plug the key in and unlock it'; if (/timed out|timeout|connect|refused|unreachable|dns/i.test(s)) return 'it could not be reached just now'; return s.replace(/^[A-Z_]+:\s*/, '').replace(/\.$/, ''); }
function renderOverview() {
  // in a fiction profile the made-up person's world is the real one: its signet vaults count as their holdings
  const fiction = !!(profile && profile.class === 'fiction');
  const main = store.vaults.filter(v => fiction || v.network === 'mainnet'), test = fiction ? [] : store.vaults.filter(v => v.network !== 'mainnet');
  let off = []; try { off = JSON.parse(sumPref('off', '[]')); } catch {}
  const names = main.map(v => v.name).filter(n => !off.includes(n));
  const chips = document.createElement('div'); chips.style.cssText = 'display:flex;gap:var(--s2) 14px;flex-wrap:wrap;align-items:center';
  const ents = (store.entities || []).slice().sort((a, b) => a.name.localeCompare(b.name)), groups = ents.length ? ents.map(e => [e.name, main.filter(v => v.entity === e.id)]).concat([['Unassigned', main.filter(v => !v.entity || !entityOf(v.entity))]]).filter(g => g[1].length) : [['', main]];
  for (const [gname, vs] of groups) {
    if (gname) { const l = document.createElement('span'); l.className = 'ent-kind'; l.style.cssText = 'margin-right:-6px;display:inline-flex;align-items:center;gap:6px'; const e = ents.find(x => x.name === gname); l.innerHTML = (e ? iconHtml('entity', e.id, 24, entGlyph(e)) : '') + esc(gname); chips.appendChild(l); }
    for (const v of vs) { const b = document.createElement('button'); b.type = 'button'; b.className = 'vchip'; b.dataset.vault = v.name; b.setAttribute('aria-pressed', off.includes(v.name) ? 'false' : 'true'); b.textContent = v.name;
      b.onclick = () => { const set = new Set(off); set.has(v.name) ? set.delete(v.name) : set.add(v.name); sumSave('off', JSON.stringify([...set])); renderOverview(); }; chips.appendChild(b); }
  }
  $('#ov-note').textContent = test.length ? `Not counted: ${test.map(v => '“' + v.name + '”').join(', ')} — on a test network, whose coins are worth nothing and must never be added to real ones.` : '';
  if (!main.length) { $('#ov-summary').innerHTML = '<div class="label">All vaults</div><div class="sum-empty">No vault on mainnet yet. Test-network vaults are never added into a total of real coins.</div>'; return; }
  paintIcons(chips);
  renderSummary($('#ov-summary'), names, { title: 'All vaults', chips: main.length > 1 ? chips : null,
    empty: names.length ? 'Nothing has arrived in these vaults yet.' : 'Every vault is switched off above. Switch one on to see it.',
    extra: main.length > 1 ? 'Switch a vault off above to leave it out; a transfer from a vault that is switched off then counts as coming from outside.' : '' });
}

// ---------- a Lightning node the person runs ----------
function nodeRow(id) { if (!vault || !vault.node) return null; if (!vault._node) vault._node = Object.fromEntries((vault.node.events || []).map(e => [e.id, e])); return vault._node[id] || null; }
let nodeChecked = null;
const nodeKind = () => ($('#node-kind [aria-checked=true]') || {}).dataset?.kind || 'cln';
$('#node-kind').onclick = (e) => { const b = e.target.closest('[data-kind]'); if (!b || b.disabled) return; $$('#node-kind [data-kind]').forEach(x => x.setAttribute('aria-checked', x === b ? 'true' : 'false')); const lnd = b.dataset.kind === 'lnd';
  $('#node-cred-label').textContent = lnd ? 'Read-only macaroon, or an lndconnect link' : 'Read-only rune'; $('#node-cred').placeholder = lnd ? 'paste the macaroon (hex) or an lndconnect:// link' : 'paste the rune'; $('#node-url').placeholder = lnd ? 'https://your-node:8080' : 'https://your-node:3010  (or http:// on your own network)';
  $('#node-help-cln').hidden = lnd; $('#node-help-lnd').hidden = !lnd; };
$('#node-check').onclick = async () => {
  const kind = nodeKind(), url = $('#node-url').value.trim(), cred = $('#node-cred').value.trim(); nodeChecked = null; $('#node-preview').hidden = true;
  if (!url && !cred.startsWith('lndconnect://')) return toast('Where is the node? Its address first.'); if (!cred) return toast(kind === 'lnd' ? 'Paste the read-only macaroon' : 'Paste the read-only rune');
  $('#node-check').disabled = true; $('#node-note').textContent = 'Asking the node… first whether this credential could spend, then who it is and what it holds.';
  try {
    const c = await invoke('node_check', { kind, url, cred, pin: '' });
    nodeChecked = { kind, url, cred, pin: c.pin };
    $('#node-note').textContent = '';
    $('#node-facts').innerHTML = [['Node', `${esc(c.alias)} · ${c.kind === 'cln' ? 'Core Lightning' : 'LND'} ${esc(c.version)}`], ['Identity', `<span class="mono" style="font-size:12px;word-break:break-all">${esc(c.pubkey)}</span>`], ['Height', sats(c.height)],
      ['On-chain', `${sats(c.onchain)} sats`], ['In channels', `${sats(c.channels)} sats across ${sats(c.channel_count)} channel${c.channel_count === 1 ? '' : 's'}`], ['Pending', c.pending ? `${sats(c.pending)} sats` : 'nothing'], ['Ledger', `${sats(c.events)} events from the bookkeeper`]].map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
    $('#node-proof').innerHTML = `<b>✓ This credential cannot spend.</b> The node was asked whether it would allow ${sats(c.refused.length)} ways of moving money — ${esc(c.refused.slice(0, 4).join(', '))} and the rest — and refused every one. Nothing was executed; asking is a read.`;
    $('#node-proof').style.borderLeftColor = 'var(--signal)';
    if (c.pin) { $('#node-pin-row').hidden = false; $('#node-pin-row').innerHTML = `<b>Its certificate: </b><span class="mono" style="font-size:12px;word-break:break-all">${esc(c.pin.replace(/(..)(?=.)/g, '$1:').toUpperCase())}</span><br><span class="muted">This is remembered. If the node ever answers with a different certificate, the app stops rather than trusting whoever is answering. Compare it with your node’s if you can.</span>`; }
    else { $('#node-pin-row').hidden = true; }
    if (!$('#node-name').value) $('#node-name').value = c.alias || 'My node';
    $('#node-preview').hidden = false;
  } catch (e) { $('#node-note').textContent = String(e); }
  finally { $('#node-check').disabled = false; }
};
$('#node-add').onclick = async () => {
  if (!nodeChecked) return toast('Check the node first');
  const name = $('#node-name').value.trim() || 'My node';
  $('#node-add').disabled = true;
  try {
    const v = await invoke('node_add', { name, kind: nodeChecked.kind, url: nodeChecked.url, cred: nodeChecked.cred, pin: nodeChecked.pin });
    nodeChecked = null; $('#node-cred').value = ''; $('#node-preview').hidden = true;
    store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v; rememberVault(vault);
    toast('Watching the node. Its credential is sealed here and can only read.', 8000);
    renderTabs(); paintNet(); show('home');
  } catch (e) { fail(e); } finally { $('#node-add').disabled = false; }
};

// ---------- a closed custodial wallet, from its statement ----------
// The rows live in the vault (store_get hands them over); one map per vault object, built once, never on draw.
// ---------- an exchange account: its rows, its words, and the import ----------
const EX_WORDS = { buy: 'bought', sell: 'sold', fiat_in: 'money in', fiat_out: 'money out', btc_out: 'withdrawn', btc_in: 'received', other: 'other' };
function exRow(id) { if (!vault || !vault.exchange) return null; if (!vault._ex) vault._ex = Object.fromEntries((vault.exchange.rows || []).map(r => [r.id, r])); return vault._ex[id] || null; }
const exRealPrice = (r) => { if (!r.sats) return null; const paid = r.kind === 'buy' ? Math.abs(r.fiat) + r.fee_fiat : r.kind === 'sell' ? Math.max(0, Math.abs(r.fiat) - r.fee_fiat) : 0; return paid ? paid / 100 / (Math.abs(r.sats) / 1e8) : null; };
const exKindWords = (r) => ({ buy: 'a purchase — its cost is what was paid, fee included', sell: 'a sale — a disposal at what was received', fiat_in: 'money deposited at the exchange', fiat_out: 'money withdrawn from the exchange', btc_out: 'coins withdrawn to a wallet', btc_in: 'coins received at the exchange', other: 'a row this app did not recognise, kept as written' })[r.kind] || r.kind;
let exEntity = null;
// what an export says, before anything is kept — the same facts for a new account and for a later export
function exFactsHtml(p) {
  const day = t => new Date(t * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' }), m = (c) => moneyExact(c / 100, p.ccy);
  return [
    ['Statements', `${esc(p.provider)} · ${sats(p.rows)} rows · ${p.files.length === 1 ? esc(p.files[0]) : sats(p.files.length) + ' files'}`],
    ['Covers', p.rows ? `${day(p.first)} to ${day(p.last)}` : '—'],
    p.buys ? ['Buys', `${sats(p.buys)} · ${sats(p.bought_sats)} sats for ${m(p.spent_fiat)}${p.buy_fees_fiat ? ` including ${m(p.buy_fees_fiat)} in fees` : ''}`] : null,
    p.sells ? ['Sells', `${sats(p.sells)} · ${sats(p.sold_sats)} sats for ${m(p.got_fiat)}`] : null,
    p.withdrawals ? ['Withdrawals', `${sats(p.withdrawals)} · ${sats(p.withdrawn_sats)} sats to wallets${p.wd_fee_sats ? ` (fees ${sats(p.wd_fee_sats)} sats)` : ''}`] : null,
    p.rewards_sats ? ['Received', `${sats(p.rewards_sats)} sats in rewards or deposits`] : null,
    p.deposits || p.fiat_out ? ['Money', `${p.deposits ? `${sats(p.deposits)} deposit${p.deposits === 1 ? '' : 's'} of ${m(p.deposited_fiat)}` : ''}${p.deposits && p.fiat_out ? ' · ' : ''}${p.fiat_out ? `${m(p.fiat_out)} withdrawn` : ''}`] : null,
    ['Held now', `${sats(Math.max(0, p.btc_now))} sats · ${m(p.fiat_now)} — ${p.stated ? 'as the newest statement states' : 'as the rows add up'}`],
    p.unknown.length ? ['Not understood', `${p.unknown.map(esc).join(', ')} — kept as written, with the amount the statement gives`] : null,
    p.new !== null && p.new !== undefined ? ['New here', `${sats(p.new)} row${p.new === 1 ? '' : 's'} this account does not have yet`] : null,
  ].filter(Boolean).map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
}
function exReconHtml(p) {
  if (!p.stated) return ['<b>No balance is stated in this export</b>, so what is held is the sum of its rows. If the exchange shows a different figure, a row is missing from the export.', 'var(--muted)'];
  return p.agrees ? ['<b>✓ The running balances chain</b>, row by row, through every file: each balance is the one before plus its row — to the sat for coins, to the cent for money, since the exchange rounds its fees.', 'var(--signal)'] : ['<b>The running balance does not chain somewhere:</b> a file is partial, or a row was read differently from how the exchange wrote it. It can still be imported; the figure stays visible on the account.', 'var(--no)'];
}
$('#ex-pick').onclick = async () => {
  $('#ex-note').textContent = ''; $('#ex-preview').hidden = true;
  let p; try { p = await invoke('exchange_pick_preview', { existing: null }); } catch (e) { if (String(e) !== 'cancelled') $('#ex-note').textContent = /not an export this app knows/.test(String(e)) ? String(e) + ' \u2014 for another exchange, use \u201cAnother exchange\u2019s CSV\u201d and say which column is what.' : String(e); return; }
  $('#ex-map').hidden = true; paintExPreview(p);
};
function paintExPreview(p) {
  $('#ex-facts').innerHTML = exFactsHtml(p);
  const [words, colour] = exReconHtml(p); $('#ex-recon').innerHTML = words; $('#ex-recon').style.borderLeftColor = colour;
  lineChart($('#ex-chart'), { title: 'Coins held at the exchange, in sats', sub: 'Row by row: bought, then withdrawn. If this is the shape you remember, it is your account.', from: p.first, to: Math.max(p.last, p.first + 86400),
    series: [{ name: 'Held', color: 'var(--series-value)', pts: p.series, step: true, area: true }], fmt: v => sats(Math.round(v)) + ' sats', fmtAxis: v => csats(v) });
  if (!$('#ex-name').value) $('#ex-name').value = p.provider;
  const box = $('#ex-owner'), paintOwner = (id) => { exEntity = id; box.replaceChildren(entityChips(id, $('#ex-owner-form'), paintOwner)); }; paintOwner(null);
  $('#ex-preview').hidden = false;
}
$('#ex-import').onclick = async () => {
  const name = $('#ex-name').value.trim(); if (!name) return toast('Give the account a name');
  $('#ex-import').disabled = true;
  try {
    const [v, n] = await invoke('exchange_import', { name, entity: exEntity });
    $('#ex-preview').hidden = true; $('#ex-name').value = '';
    store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v; rememberVault(vault);
    toast(`${sats(n)} row${n === 1 ? '' : 's'} kept. Nothing here can be spent; every buy carries what it really cost.`, 8000);
    renderTabs(); paintNet(); show('home');
  } catch (e) { fail(e); } finally { $('#ex-import').disabled = false; }
};
// another exchange's CSV: the header comes back and the person says which column is what, in the house segments
const MAP_ROLES = [['', '—'], ['date', 'Date'], ['kind', 'Kind'], ['sats', 'Coins'], ['fiat', 'Money'], ['fee_fiat', 'Fee, money'], ['fee_sats', 'Fee, coins'], ['id', 'Id']];
// a first guess from the column's name; the person corrects it
function mapGuess(h) {
  const n = h.toLowerCase();
  if (/date|time|when|created/.test(n)) return 'date';
  if (/^id$|ref|txid|identifier|order/.test(n)) return 'id';
  if (/fee/.test(n)) return /btc|coin|sat/.test(n) ? 'fee_sats' : 'fee_fiat';
  if (/type|kind|product|action|side|description/.test(n)) return 'kind';
  if (/btc|bitcoin|coin|sats|crypto|quantity|qty/.test(n)) return 'sats';
  if (/aud|usd|eur|gbp|nzd|fiat|price|total|money|cash|amount/.test(n)) return 'fiat';
  return '';
}
function paintMapping(raw) {
  const box = $('#ex-map'); box.hidden = false;
  const roles = raw.header.map(mapGuess); const seenRole = new Set();
  for (let i = 0; i < roles.length; i++) { if (roles[i] && seenRole.has(roles[i])) roles[i] = ''; else if (roles[i]) seenRole.add(roles[i]); }
  const tz = -new Date().getTimezoneOffset() / 60;
  box.innerHTML = `<div class="sk-step"><div class="sk-num">1b</div><div class="sk-lead"><b>Say which column is what.</b> <span class="muted" style="font-weight:400">${sats(raw.rows)} rows in ${raw.files.length === 1 ? esc(raw.files[0]) : sats(raw.files.length) + ' files'}. A column left at — is not read. Coins and money are read to the sat and the cent; the kind is read from its words, or from the signs when there is no kind column.</span></div></div>
    <div id="ex-map-cols">${raw.header.map((h, i) => `<div class="mapcol"><div class="col"><b>${esc(h || '(unnamed)')}</b><small>${esc(raw.sample.slice(0, 3).map(r => r[i] || '').filter(Boolean).join(' · ') || 'empty in the first rows')}</small></div><div class="seg" role="radiogroup" aria-label="What column ${esc(h)} is" data-col="${i}">${MAP_ROLES.map(([r, w]) => `<button type="button" role="radio" aria-checked="${roles[i] === r ? 'true' : 'false'}" data-role="${r}">${w}</button>`).join('')}</div></div>`).join('')}</div>
    <div class="row" style="gap:18px;align-items:flex-end;flex-wrap:wrap;margin-top:6px">
      <div class="field" style="max-width:220px"><div class="label">Exchange</div><input id="ex-map-provider" maxlength="40" placeholder="its name"></div>
      <div class="field" style="max-width:110px"><div class="label">Currency</div><input id="ex-map-ccy" maxlength="3" value="${esc((store.settings.fiat && store.settings.fiat !== 'auto' ? store.settings.fiat : '') || 'AUD')}"></div>
      <div class="field"><div class="label">Times are</div><div class="seg" role="radiogroup" id="ex-map-tz"><button type="button" role="radio" aria-checked="true" data-tz="0">UTC</button><button type="button" role="radio" aria-checked="false" data-tz="${tz}">Local (${tz >= 0 ? '+' : ''}${tz})</button></div></div>
      <div class="field"><div class="label">Coins written as</div><div class="seg" role="radiogroup" id="ex-map-unit"><button type="button" role="radio" aria-checked="true" data-unit="btc">BTC</button><button type="button" role="radio" aria-checked="false" data-unit="sats">sats</button></div></div>
    </div>
    <div class="row" style="gap:10px;align-items:center"><button class="btn primary" type="button" id="ex-map-read">Read it</button><span class="muted" id="ex-map-note" aria-live="polite"></span></div>`;
  $$('#ex-map-cols .seg', box).forEach(seg => $$('[data-role]', seg).forEach(b => b.onclick = () => {
    const role = b.dataset.role;
    // one column per role: a role taken here is given up wherever else it was
    if (role) $$('#ex-map-cols .seg', box).forEach(o => { if (o !== seg) $$('[data-role]', o).forEach(x => { if (x.dataset.role === role && x.getAttribute('aria-checked') === 'true') { x.setAttribute('aria-checked', 'false'); o.querySelector('[data-role=""]').setAttribute('aria-checked', 'true'); } }); });
    $$('[data-role]', seg).forEach(x => x.setAttribute('aria-checked', x === b ? 'true' : 'false'));
  }));
  for (const id of ['#ex-map-tz', '#ex-map-unit']) $$(id + ' [role=radio]').forEach(b => b.onclick = () => $$(id + ' [role=radio]').forEach(x => x.setAttribute('aria-checked', x === b ? 'true' : 'false')));
  $('#ex-map-provider').focus();
  $('#ex-map-read').onclick = async () => {
    const col = (role) => { const seg = $$('#ex-map-cols .seg').find(s => $$('[data-role]', s).some(x => x.dataset.role === role && x.getAttribute('aria-checked') === 'true')); return seg ? +seg.dataset.col : null; };
    const provider = $('#ex-map-provider').value.trim(); if (!provider) { $('#ex-map-note').textContent = 'Name the exchange.'; return $('#ex-map-provider').focus(); }
    if (col('date') === null) { $('#ex-map-note').textContent = 'Which column is the date?'; return; }
    if (col('sats') === null && col('fiat') === null) { $('#ex-map-note').textContent = 'Which column holds the coins, or the money?'; return; }
    const mapping = { provider, date: col('date'), tz_hours: +($('#ex-map-tz [aria-checked=true]').dataset.tz || 0), kind: col('kind'), sats: col('sats'), sats_unit: $('#ex-map-unit [aria-checked=true]').dataset.unit, fiat: col('fiat'), fee_fiat: col('fee_fiat'), fee_sats: col('fee_sats'), id: col('id'), ccy: $('#ex-map-ccy').value.trim().toUpperCase() || 'AUD' };
    $('#ex-map-note').textContent = 'Reading…';
    let p; try { p = await invoke('exchange_map_preview', { mapping, existing: null }); } catch (e) { $('#ex-map-note').textContent = String(e); return; }
    $('#ex-map-note').textContent = ''; paintExPreview(p);
  };
}
$('#ex-map-pick').onclick = async () => {
  $('#ex-note').textContent = ''; $('#ex-preview').hidden = true; $('#ex-map').hidden = true;
  let raw; try { raw = await invoke('exchange_pick_raw'); } catch (e) { if (String(e) !== 'cancelled') $('#ex-note').textContent = String(e); return; }
  paintMapping(raw);
};
// a later export, from the account's Vault settings: the same preview, then only the new rows are added
function paintExchangeSettings() {
  const box = $('#vs-exchange'); if (!box) return;
  if (!vault || !vault.exchange) { box.hidden = true; box.innerHTML = ''; return; }
  const ex = vault.exchange, last = ex.imports[ex.imports.length - 1];
  box.hidden = false;
  box.innerHTML = `<div class="label" style="margin-top:6px">Statements</div><div class="muted" style="font-size:13px">${sats(ex.imports.length)} import${ex.imports.length === 1 ? '' : 's'}${last && last.at ? ' · the last on ' + new Date(last.at * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' }) : ''} · ${sats(ex.rows.length)} rows · ${ex.stated ? (ex.agrees ? 'balances chain' : 'a balance did not chain') : 'no balance stated'}</div>
    <div class="row" style="gap:8px;margin-top:6px"><button class="btn small" type="button" id="vs-ex-pick">Add a newer export…</button><span class="muted" id="vs-ex-note" style="font-size:13px" aria-live="polite"></span></div><div id="vs-ex-preview" hidden style="margin-top:8px"></div>`;
  $('#vs-ex-pick').onclick = async () => {
    $('#vs-ex-note').textContent = ''; const pv = $('#vs-ex-preview'); pv.hidden = true;
    let p; try { p = await invoke('exchange_pick_preview', { existing: vault.name }); } catch (e) { if (String(e) !== 'cancelled') $('#vs-ex-note').textContent = String(e); return; }
    if (p.provider !== ex.provider) { $('#vs-ex-note').textContent = `That export is from ${p.provider}; this account is ${ex.provider}'s.`; return; }
    pv.hidden = false; pv.innerHTML = `<dl class="tx-dl">${exFactsHtml(p)}</dl><div class="row" style="gap:8px;margin-top:8px"><button class="btn small primary" type="button" id="vs-ex-add">${p.new ? `Add the ${sats(p.new)} new row${p.new === 1 ? '' : 's'}` : 'Nothing new — keep as is'}</button><button class="btn small" type="button" id="vs-ex-cancel">Cancel</button></div>`;
    $('#vs-ex-cancel').onclick = () => { pv.hidden = true; };
    $('#vs-ex-add').onclick = async () => { try { const [v, n] = await invoke('exchange_import', { name: vault.name, entity: null }); store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v; vault._ex = null; vault._stale = true; toast(n ? `${sats(n)} row${n === 1 ? '' : 's'} added.` : 'Already here: that export added nothing new.'); paintExchangeSettings(); } catch (e) { fail(e); } };
  };
}
function stmtRow(id) { if (!vault || !vault.statement) return null; if (!vault._stmt) vault._stmt = Object.fromEntries((vault.statement.rows || []).map(r => [r.id, r])); return vault._stmt[id] || null; }
let stmtText = '';
$('#stmt-pick').onclick = async () => {
  $('#stmt-note').textContent = ''; $('#stmt-preview').hidden = true;
  let text; try { text = await invoke('statement_pick'); } catch (e) { if (String(e) !== 'cancelled') $('#stmt-note').textContent = String(e); return; }
  let p; try { p = await invoke('statement_preview', { text }); } catch (e) { $('#stmt-note').textContent = String(e); return; }
  stmtText = text;
  const day = t => new Date(t * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
  $('#stmt-facts').innerHTML = [
    ['Statement', `${esc(p.source)} · ${sats(p.rows)} rows`],
    ['Covers', `${day(p.first)} to ${day(p.last)}`],
    ['Of which', `${sats(p.lightning)} Lightning · ${sats(p.chain)} on-chain${p.pos ? ` · ${sats(p.pos)} through the point of sale` : ''}${p.rewards ? ` · ${sats(p.rewards)} reward${p.rewards === 1 ? '' : 's'}` : ''}`],
    ['In', `${sats(p.credits)} sats`],
    ['Out', `${sats(p.debits)} sats, plus ${sats(p.fees)} sats in fees`],
  ].map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
  const dust = Math.abs(p.closes_at) <= 100;
  $('#stmt-recon').innerHTML = dust
    ? `<b>✓ It reconciles.</b> In, less out, less the fees on what went out, leaves ${sats(p.closes_at)} sats — a closed wallet, read right.`
    : `<b>It does not close at zero:</b> the rows leave ${sats(p.closes_at)} sats. Either the export is partial or its fees read differently from Wallet of Satoshi’s. It can still be imported; the figure stays visible on the vault.`;
  $('#stmt-recon').style.borderLeftColor = dust ? 'var(--signal)' : 'var(--no)';
  lineChart($('#stmt-chart'), { title: 'Balance, in sats', sub: 'What the wallet held, row by row. If this is the shape you remember, it is your wallet.', from: p.first, to: Math.max(p.last, p.first + 86400),
    series: [{ name: 'Balance', color: 'var(--series-value)', pts: p.series, step: true, area: true }], fmt: v => sats(Math.round(v)) + ' sats', fmtAxis: v => csats(v) });
  $('#stmt-columns').textContent = 'utcDate → when (UTC) · type → in or out · currency → Lightning or on-chain · amount, fees → sats, read exactly from the decimal · address → the invoice (its payment hash is kept) or the bitcoin address · description → the row’s label · pointOfSale → a sale, counted as income. A payment’s fee was charged on top of it; a receipt’s fee is what the sender paid and was not deducted.';
  if (!$('#stmt-name').value) $('#stmt-name').value = 'Wallet of Satoshi · custodial, closed';
  $('#stmt-preview').hidden = false;
};
$('#stmt-import').onclick = async () => {
  if (!stmtText) return toast('Choose the statement first');
  const name = $('#stmt-name').value.trim() || 'Wallet of Satoshi · custodial, closed';
  $('#stmt-import').disabled = true;
  try {
    const [v, n] = await invoke('statement_import', { name, text: stmtText });
    stmtText = ''; $('#stmt-preview').hidden = true;
    store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v; rememberVault(vault);
    toast(n ? `${sats(n)} row${n === 1 ? '' : 's'} kept. Nothing in this wallet can be spent; its history is here for the coins that left it.` : 'Already here: that statement added nothing new.', 8000);
    renderTabs(); paintNet(); show('home');
  } catch (e) { fail(e); } finally { $('#stmt-import').disabled = false; }
};

// ---------- receive ----------
// A wallet watched from a plain xpub is watched for every kind of address, and only the wallet that holds the keys
// knows which it hands out: so Receive asks, defaulting to the kind that has history. A wallet that also receives
// silent payments has that as one more choice — a different address entirely, found by the scan key, not the xpub.
const KIND_WORDS = { pkh: 'Legacy · 1…', shwpkh: 'Nested segwit · 3…', wpkh: 'Native segwit · bc1q…', tr: 'Taproot · bc1p…', sp: 'Silent payment · sp1…' };
const receiveMode = {};   // vault name → kind, or 'sp'
function receiveChoices(v) {
  if (!v.watch) return [];
  const used = new Set((v._history || []).map(h => (h.index >>> 28) & 7).filter(s => s < v.watch.kinds.length).map(s => v.watch.kinds[s]));
  const kinds = v.watch.kinds.length > 1 ? v.watch.kinds : [];
  const list = kinds.concat(v.watch.sp ? ['sp'] : []);
  if (list.length < 2 && !v.watch.sp) return [];
  const fallback = used.size === 1 ? [...used][0] : v.watch.kinds.includes('wpkh') ? 'wpkh' : v.watch.kinds[0];
  return { list: kinds.length ? list : [v.watch.kinds[0]].concat(['sp']), pick: receiveMode[v.name] || fallback };
}
async function drawAddress(chosen) {
  if (!vault) return;
  if (vault._receive === undefined) await sync();
  const seg = $('#addr-seg'), choices = chosen ? [] : receiveChoices(vault);
  seg.hidden = !choices.list; seg.innerHTML = '';
  let mode = null;
  if (choices.list) {
    mode = choices.pick;
    for (const k of choices.list) { const b = document.createElement('button'); b.type = 'button'; b.setAttribute('role', 'radio'); b.setAttribute('aria-checked', k === mode ? 'true' : 'false'); b.dataset.kind = k; b.textContent = KIND_WORDS[k] || k; b.onclick = () => { receiveMode[vault.name] = k; drawAddress(); }; seg.appendChild(b); }
  }
  const me = vault.name;
  let addr = chosen ? chosen.address : (vault._receive || vault.address);
  if (mode === 'sp') { try { addr = await invoke('sp_address', { name: me }); } catch (e) { addr = ''; fail(e); } }
  else if (mode && mode !== vault.watch.kinds[0]) { try { addr = await invoke('vault_address_kind', { name: me, kind: mode }); } catch (e) { fail(e); } }
  if (!vault || vault.name !== me) return;
  $('#addr-text').textContent = addr;
  if (mode === 'sp') {
    $('#addr-note').innerHTML = `<b style="font-family:var(--display);font-weight:600">This wallet's silent payment address.</b> Give it when the sender's wallet can pay silent payments: the coins arrive as taproot outputs only this wallet's scan key can find, so nothing on the chain links them to this wallet or to each other, and the same address serves every payment. They show here once the read has reached the block they are in.`;
    const qr = qrcode(0, 'M'); qr.addData(addr); qr.make(); $('#addr-qr').innerHTML = qr.createSvgTag({ cellSize: 3, margin: 2, scalable: true });
    return;
  }
  $('#addr-note').innerHTML = chosen && !chosen.current
    ? `<b style="font-family:var(--display);font-weight:600">${chosen.chain === 'change' ? 'Change' : 'Receive'} address ${fmtIndex(chosen.index).replace('change ', '')}${chosen.future ? ', ahead of the next unused one' : ''}.</b> ${chosen.future ? 'Fine to hand out now; the scan will find it as long as it is within the gap limit of the last one used.' : 'Already used. Paying it again works, but links the payments together on the chain.'} <a href="#" id="addr-back">Back to the next unused</a>`
    : vault._rotating && vault.watch
    ? `<b style="font-family:var(--display);font-weight:600">Fresh address, #${vault._receiveIndex}${mode ? ', ' + (KIND_WORDS[mode] || mode).split(' \u00b7')[0].toLowerCase() : ''}.</b> Every deposit gets its own. The wallet that holds the keys derives the same one, so it will see what arrives here${vault.watch.kinds.length > 1 ? ' \u2014 if it uses this kind of address; a wallet watched from a plain xpub is watched for every kind, and only that wallet knows which it hands out' : ''}.`
    : vault._rotating
    ? `<b style="font-family:var(--display);font-weight:600">Fresh address, #${vault._receiveIndex}.</b> Every deposit gets its own. Any two cards can still spend all of them: the app derives each address from the card keys and the vault's chain code, and the recovery line carries both.`
    : `<b style="font-family:var(--display);font-weight:600">This vault has one address.</b> It was made before rotating addresses existed, so every deposit lands here and anyone who knows it sees its whole history.`;
  const qr = qrcode(0, 'M'); qr.addData('bitcoin:' + addr); qr.make();
  $('#addr-qr').innerHTML = qr.createSvgTag({ cellSize: 4, margin: 2, scalable: true });
  const back = $('#addr-back'); if (back) back.onclick = (e) => { e.preventDefault(); drawAddress(); };
}
$('#copy-addr').onclick = () => { navigator.clipboard.writeText($('#addr-text').textContent).then(() => toast('Address copied')); };

// ---------- send ----------
async function loadFees() {
  if (!vault) return;
  if (!vault._coins) await sync();
  try {
    fees = await invoke('fees_get', { network: vault.network });
    const vb = vault.keys.length === 1 && !vault.ext ? 141 : (11 + inVb(vault.threshold, vault.keys.length, scriptOf(vault)) + 31 + 43);
    loadFees.vb = vb;
    for (const lvl of ['slow', 'medium', 'fast']) { const r = fees[lvl]; $(`[data-fee="${lvl}"] small`).textContent = `${r} sat/vB · ≈${sats(Math.ceil(r * vb))} sats`; }
    const shape = `A ${vault.keys.length === 1 ? 'single-key' : vault.threshold + '-of-' + vault.keys.length} spend is about ${vb} vB per input set.`;
    // when the three are the same number they are not three choices, and saying so beats three buttons that pretend
    $('#fee-note').textContent = fees.slow === fees.fast
      ? `The explorer is quoting the same rate for all three just now, so Slow, Medium and Fast come to the same fee. Choose Custom to set your own. ${shape}`
      : `Rates from the explorer just now. ${shape}`;
    paintCustomFee();
  } catch (e) { $('#fee-note').textContent = 'Could not read fee rates: ' + e; }
}
// the script a vault of extended keys pays to, off its descriptor; a card vault is wsh
const scriptOf = (v) => { const e = (v && v.ext || '').trimStart(); return e.startsWith('sh(wsh(') ? 'sh-wsh' : e.startsWith('sh(wpkh(') ? 'sh-wpkh' : e.startsWith('sh(') ? 'sh' : e.startsWith('wpkh(') ? 'wpkh' : e.startsWith('pkh(') ? 'pkh' : e.startsWith('tr(') ? 'tr' : 'wsh'; };
function inVb(k, n, script) {
  if (script === 'wpkh' || (!script && n === 1)) return 68; if (script === 'sh-wpkh') return 91; if (script === 'pkh') return 148; if (script === 'tr') return 58;
  const sl = 1 + 34 * n + 2; if (script === 'sh') return 41 + 1 + k * 73 + sl + 2;
  const w = 41 + Math.ceil((1 + 1 + k * 73 + (sl < 76 ? 1 : 2) + sl) / 4); return script === 'sh-wsh' ? w + 36 : w;
}
// paste and scan into the To field
// ---------- the address book ----------
const bookOf = () => (store.contacts || []).filter(c => c.network === (vault ? vault.network : store.settings.network));
const shortAddr = (a) => a.length > 26 ? a.slice(0, 12) + '…' + a.slice(-8) : a;
const chunked = (a) => a.replace(/(.{4})/g, '$1 ').trim();
function closeBook() { $('#book').hidden = true; $('#book-to').setAttribute('aria-expanded', 'false'); }
function renderBook() {
  const own = store.vaults.filter(v => vault && v.network === vault.network && v.name !== vault.name), saved = bookOf().slice().sort((a, b) => (b.last_used || b.added) - (a.last_used || a.added));
  $('#book').innerHTML =
    (own.length ? '<h4>Your own vaults · a fresh address every time</h4>' + own.map(v => `<button type="button" class="entry" data-own="${esc(v.name)}"><span class="nm">${esc(v.name)}</span><span class="ad">${v.chain_code ? 'next unused address' : 'its one fixed address'}</span><span class="ct">${v.threshold} of ${v.keys.length}</span></button>`).join('') : '')
    + '<h4>Saved addresses</h4>' + (saved.length ? saved.map(c => `<div class="rowwrap"><button type="button" class="entry" data-addr="${esc(c.address)}"><span class="nm">${esc(c.name)}</span><span class="ad" title="${esc(c.address)}">${esc(shortAddr(c.address))}</span><span class="ct">${c.used ? 'paid ' + c.used + (c.used === 1 ? ' time' : ' times') : 'not yet paid'}</span></button><button type="button" class="del" data-del="${esc(c.address)}" title="Remove from the address book">Remove</button></div>`).join('')
      : '<div class="empty">None yet. Paste or scan an address, then “Save this address”.</div>');
  $$('#book [data-del]').forEach(b => askTwice(b, 'Remove this address from the book? Payments already made are not affected.', 'Yes, remove', async () => { store = await invoke('contact_delete', { address: b.dataset.del, network: vault.network }); renderBook(); paintTo(); }));
}
$('#book').addEventListener('click', async (e) => {
  const own = e.target.closest('[data-own]'), c = e.target.closest('[data-addr]');
  if (own) { try { $('#send-to').value = await invoke('vault_receive_address', { name: own.dataset.own }); paintTo.own = { name: own.dataset.own, address: $('#send-to').value }; } catch (err) { return fail(err); } }
  else if (c) { $('#send-to').value = c.dataset.addr; paintTo.own = null; }
  else return;
  closeBook(); paintTo(); $('#send-amount').focus();
});
$('#book-to').onclick = () => { const open = $('#book').hidden; if (open) renderBook(); $('#book').hidden = !open; $('#book-to').setAttribute('aria-expanded', String(open)); };
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && !$('#book').hidden) { closeBook(); $('#book-to').focus(); } });
// under the To field: who this address is, how often it has been paid, or the offer to save it
function paintTo() {
  const who = $('#to-who'), a = $('#send-to').value.trim(); who.hidden = true; who.innerHTML = '';
  if (!a || !vault) return;
  const own = paintTo.own && paintTo.own.address === a ? paintTo.own : null, c = bookOf().find(c => c.address.toLowerCase() === a.toLowerCase());
  if (own) who.innerHTML = `<span>To your own vault <b>${esc(own.name)}</b>, at an address it has not used before.</span>`;
  else if (c) who.innerHTML = `<span>To <b>${esc(c.name)}</b> · <span class="mono" style="font-size:12.5px">${esc(chunked(c.address))}</span></span><span>${c.used ? `Paid ${c.used} time${c.used === 1 ? '' : 's'} before: anyone watching that address can link these payments.` : 'Saved, not yet paid.'}</span>`;
  else if (/^(bc1|tb1|[13mn2])[a-zA-Z0-9]{20,}$/.test(a)) who.innerHTML = `<button type="button" class="linkish" id="save-addr">Save this address…</button>`;
  else return;
  who.hidden = false;
  if ($('#save-addr')) $('#save-addr').onclick = () => {
    who.innerHTML = `<div class="savebar"><input id="save-addr-name" placeholder="A name for it, e.g. exchange deposit" maxlength="60"><button type="button" class="btn small primary" id="save-addr-go">Save</button><button type="button" class="linkish" id="save-addr-no">Cancel</button></div>`;
    const go = async () => { try { store = await invoke('contact_save', { name: $('#save-addr-name').value, address: a, network: vault.network }); toast('Saved to the address book'); paintTo(); } catch (e) { fail(e); } };
    $('#save-addr-go').onclick = go; $('#save-addr-no').onclick = paintTo; $('#save-addr-name').focus();
    $('#save-addr-name').addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); go(); } if (e.key === 'Escape') paintTo(); });
  };
}
$('#send-to').addEventListener('input', () => { paintTo.own = null; paintTo(); });
$('#paste-to').onclick = async () => {
  try { const t = (await navigator.clipboard.readText()).trim(); if (!t) return toast('Clipboard is empty'); fillDestination(t); }
  catch { toast('Could not read the clipboard'); }
};
function fillDestination(text) {
  let t = text.trim();
  if (/^bitcoin:/i.test(t)) {
    const u = t.slice(8); const [addr, q] = u.split('?');
    t = addr;
    if (q) { const amt = new URLSearchParams(q).get('amount'); if (amt) { $('#send-amount').value = String(Math.round(parseFloat(amt) * 1e8)); } }
  }
  $('#send-to').value = t; paintTo.own = null; paintTo();
  toast('Destination filled');
}
let cam2Stream = null, cam2Timer = null;
// One camera modal, two jobs: a destination for Send, or whatever a caller asks for. `onRead` gets each decoded
// code and says whether it was the thing wanted; the default is Send's address.
const openScan = async (deviceId, onRead) => {
  if (onRead) openScan.onRead = onRead; else if (!deviceId) openScan.onRead = null;   // a camera switch keeps the job
  const want = openScan.onRead || ((data) => { if (/^(bitcoin:|bc1|tb1|[13mn2])/i.test(data.trim())) { fillDestination(data); return true; } return false; });
  if (cam2Timer) { clearInterval(cam2Timer); cam2Timer = null; }
  if (cam2Stream) { cam2Stream.getTracks().forEach(t => t.stop()); cam2Stream = null; }
  $('#scan-modal').classList.add('on');
  try {
    cam2Stream = await camOpen(deviceId);
    $('#cam2').srcObject = cam2Stream;
    camChooser($('#scan-pick'), cam2Stream, openScan);
    const reader = makeQrReader($('#cam2-note'));
    cam2Timer = setInterval(() => { const v = $('#cam2'); if (!v.videoWidth) return; const data = reader.read(v); if (data && want(data)) closeScan(); }, 350);
  } catch { $('#cam2-note').textContent = 'No camera available on this computer. Paste the address instead.'; }
};
$('#scan-to').onclick = () => openScan('');
// A Spark wallet's Receive screen IS the import: any code with a spark1… in it — address, invoice, a URI carrying
// either — holds the wallet's identity key, and that is all the watching needs.
$('#spark-scan') && ($('#spark-scan').onclick = () => openScan('', (data) => {
  const m = String(data).match(/spark[a-z0-9]*1[a-z0-9]{18,}/i); if (!m) return false;
  $('#spark-addr').value = m[0]; toast('Spark address read'); $('#spark-read').click(); return true;
}));
function closeScan() { if (cam2Timer) clearInterval(cam2Timer); cam2Timer = null; if (cam2Stream) { cam2Stream.getTracks().forEach(t => t.stop()); cam2Stream = null; } $('#scan-modal').classList.remove('on'); }
$('#scan-close').onclick = closeScan;

// ---------- the fee, including one of your own ----------
// A rate typed here goes to the builder exactly as it goes to the buttons: one function answers what the rate is, so
// the review, the estimate under the button and the transaction cannot disagree.
const customRate = () => { const v = parseFloat(($('#fee-custom').value || '').replace(/[^0-9.]/g, '')); return Number.isFinite(v) && v > 0 ? v : null; };
const feeRate = () => feeLevel === 'custom' ? customRate() : (fees ? fees[feeLevel] : 1);
function paintCustomFee() {
  const r = customRate(), vb = loadFees.vb || 0, raw = ($('#fee-custom').value || '').trim();
  $('#fee-custom-row').hidden = feeLevel !== 'custom';
  $('[data-fee="custom"] small').textContent = r ? `${r} sat/vB · ≈${sats(Math.ceil(r * vb))} sats` : 'your rate';
  const out = $('#fee-custom-out');
  out.classList.toggle('warn', !!raw && !r);
  out.textContent = !raw ? 'Anything above 1 sat/vB. Below 1 and most nodes will not pass it on.'
    : !r ? 'That is not a rate. Give a number of sats per virtual byte, like 2.5.'
    : r < 1 ? `≈${sats(Math.ceil(r * vb))} sats — but under 1 sat/vB most nodes will not relay it at all.`
    : `≈${sats(Math.ceil(r * vb))} sats for this spend${fees && r > fees.fast * 5 ? `, which is more than five times the Fast rate of ${fees.fast} sat/vB` : ''}.`;
}
$('#fee-custom').addEventListener('input', paintCustomFee);
$('#fee-pick').addEventListener('click', (e) => { const o = e.target.closest('.opt'); if (!o) return; feeLevel = o.dataset.fee; $$('#fee-pick .opt').forEach(x => x.classList.toggle('on', x === o)); paintCustomFee(); if (feeLevel === 'custom') $('#fee-custom').focus(); });
$('#send-all').onchange = () => { $('#send-amount').disabled = $('#send-all').checked; if (!$('#send-all').checked) $('#send-amount').value = ''; };
// ---------- what this spend would do, worked out before anything is built ----------
// The same choice `build` makes (wallet::select answers both), so what is shown here and what gets signed cannot
// drift apart. Two questions matter and only one of them is on the chain: where the money goes, and what it cost.
let sendPrev = null, sendPrevErr = '', disposalKind = '', sendSeq = 0;
const DISPOSALS = [['sold', 'Sold or converted'], ['spent', 'Spent on something'], ['gift', 'Gift'], ['self', 'My own wallet elsewhere'], ['', 'Not decided yet']];

function sendFields() {
  const to = $('#send-to').value.trim(), all = $('#send-all').checked;
  const amount = +($('#send-amount').value.replace(/[^0-9]/g, '') || 0);
  return { to, all, amount, rate: feeRate() };
}
async function sendPreview() {
  const f = sendFields(), me = ++sendSeq;
  if (!vault || !f.to || (!f.all && !f.amount) || !f.rate) { sendPrev = null; sendPrevErr = ''; return paintReview(); }
  try {
    const p = await invoke('spend_preview', { name: vault.name, to: f.to, amount: f.amount, rate: f.rate, sendAll: f.all, coins: [...coinPick] });
    if (me !== sendSeq) return;   // a later edit has already asked
    sendPrev = p; sendPrevErr = '';
  } catch (e) { if (me !== sendSeq) return; sendPrev = null; sendPrevErr = String(e && e.message ? e.message : e); }
  paintReview();
}
const sendPreviewSoon = () => { clearTimeout(sendPreviewSoon.t); sendPreviewSoon.t = setTimeout(sendPreview, 350); };

/// The cost of what would leave, and what it is worth now — null when there is no price or no history to answer from.
function leavingCost() {
  if (!sendPrev || !vault) return null;
  const L = vaultLedger(vault.name); if (!L) return null;
  const c = costOfLeaving(L, vault.name, sendPrev.choice.chosen, sendPrev.choice.spending, sendPrev.choice.leaving);
  if (!c) return null;
  const f = txFacts[vault.name] || {}, now = f.now, ccy = f.ccy;
  const value = now && ccy ? c.sats / 1e8 * now.price : null;
  return { ...c, ccy, value, gain: c.cost !== null && value !== null ? value - c.cost : null, method: L.method };
}

function paintReview() {
  const box = $('#review'); if (!box) return;
  const f = sendFields();
  if (sendPrevErr) { box.innerHTML = `<div class="warn" style="font-size:14px">${esc(sendPrevErr)}</div>`; return; }
  if (!sendPrev) { box.innerHTML = '<div class="muted">Fill in the destination and amount to see what this spend would do.</div>'; return; }
  const ch = sendPrev.choice, ours = sendPrev.to_ours, k = leavingCost();
  if (f.all && $('#send-amount').disabled) $('#send-amount').value = sats(ch.send);   // what the recipient gets, read only while Send all is on
  const money_ = (v) => k && k.ccy && v !== null && v !== undefined ? money(v, k.ccy) : null;
  const row = (a, b) => `<div class="row" style="justify-content:space-between;gap:12px"><span class="muted">${a}</span><span style="text-align:right">${b}</span></div>`;
  let out = `<div style="display:flex;flex-direction:column;gap:8px;font-size:14px">
    ${row('To', `<span class="mono">${esc(f.to.slice(0, 10))}…${esc(f.to.slice(-6))}</span>`)}
    ${row('Amount', f.all ? sats(ch.send) + ' sats — everything, less the fee' : sats(ch.send) + ' sats')}
    ${row('Coins', `${ch.chosen.length}${coinPick.size ? ' chosen by you' : ' picked by the app'} · ${sats(ch.spending)} sats`)}
    ${row('Fee', `${sats(ch.fee)} sats · ${ch.vsize} vB`)}
    ${row('Change back to vault', sats(ch.change) + ' sats')}`;
  if (ours) {
    out += `<div class="ownmove" style="border-top:1px solid var(--line);padding-top:10px">
      <div><span class="tag ours">stays with you</span> ${ours.this_vault ? 'this vault\u2019s own' : esc(ours.vault)} ${esc(ours.chain)} #${ours.index}</div>
      <div class="muted" style="font-size:13px;margin-top:4px">Moving coins between your own vaults is not a disposal, so no gain is realised. Only the ${sats(ch.fee)} sat fee leaves your hands.</div></div>`;
  } else {
    const held = k && k.from ? heldFor(k.from, k.to) : '';
    out += `<div style="border-top:1px solid var(--line);padding-top:10px;display:flex;flex-direction:column;gap:6px">
      <div><span class="tag out">leaves your vaults</span> ${sats(ch.leaving)} sats — ${sats(ch.send)} paid and ${sats(ch.fee)} in fee</div>`;
    if (!k || !k.ccy) out += `<div class="muted" style="font-size:13px">No price is loaded for this vault, so what it cost and what it is worth cannot be shown here.</div>`;
    else {
      out += `${row('They cost', k.cost === null ? `<span class="muted">not known — ${esc(k.why)}</span>` : money_(k.cost))}
      ${row('Worth now', money_(k.value) || '—')}
      ${row('Gain if this is a disposal', k.gain === null ? '<span class="muted">unknown</span>' : `<span class="${k.gain >= 0 ? 'ok' : 'warn'}">${k.gain >= 0 ? '\u25b2 +' : '\u25bc \u2212'}${money_(Math.abs(k.gain))}</span>`)}
      <div class="muted" style="font-size:13px">${held ? held + ' · ' : ''}by ${esc(COST_METHODS[k.method].short)}${k.method === 'utxo' ? '' : ' — which coins you pick does not change this'}.</div>`;
      // where the price sat when the coins leaving were received, against now: after the money, at the moment of deciding
      const nm = cycleNowHere(); if (nm === undefined) cycleNow().then(() => paintReview());
      const bm = cycleOfLots(k.lots, paintReview);
      out += cycleTable({ title: 'When received', sub: k.from ? cycleSpan(k.from, k.to) : 'day not known here', mark: bm }, { title: 'Now', sub: cycleStale(nm) ? `as of ${cycleDay(nm.t)}` : '', mark: nm });
    }
    out += `<div class="field" style="margin-top:4px"><div class="label">What is this?</div><div class="pick tight" id="disposal-pick">
      ${DISPOSALS.map(([v, n]) => `<div class="opt${v === disposalKind ? ' on' : ''}" data-kind="${v}">${n}</div>`).join('')}</div>
      <div class="muted" style="font-size:13px;margin-top:6px" id="disposal-note"></div></div>`;
    out += '</div>';
  }
  out += `<div class="muted" style="border-top:1px solid var(--line);padding-top:10px;font-size:13px">${built ? `Next: sign with ${vault.keys.length === 1 ? 'the key' : 'any ' + vault.threshold + ' of your ' + vault.keys.length + ' keys'}. Nothing is sent until you confirm after the last signature.` : 'Nothing is built yet: this is what it would do.'}</div></div>`;
  box.innerHTML = out;
  const note = $('#disposal-note');
  if (note) note.textContent = disposalKind === 'self' ? 'Not a disposal: no gain is realised and the cost goes with the coins. Their holding period is not carried, because this app cannot see the other wallet.'
    : disposalKind ? 'A disposal. The gain above is realised when this confirms; the reason is kept with the transaction.'
    : 'Kept with the transaction for your records. You can set or change it later on the transaction itself.';
}
$('#review') && ['#send-to', '#send-amount'].forEach(id => $(id).addEventListener('input', sendPreviewSoon));
$('#send-all').addEventListener('change', sendPreviewSoon);
$('#fee-pick').addEventListener('click', sendPreviewSoon);
$('#fee-custom').addEventListener('input', sendPreviewSoon);
document.addEventListener('click', (e) => {
  const o = e.target.closest('#disposal-pick .opt'); if (!o) return;
  disposalKind = o.dataset.kind;
  $$('#disposal-pick .opt').forEach(x => x.classList.toggle('on', x === o));
  paintReview();
  if (built && built.txid) invoke('disposal_set', { name: vault.name, txid: built.txid, kind: disposalKind }).then(st => { store = st; }).catch(fail);
});

$('#build-btn').onclick = async () => {
  const to = $('#send-to').value.trim(), all = $('#send-all').checked, amount = +($('#send-amount').value.replace(/[^0-9]/g, '') || 0);
  if (!to) return toast('Enter a destination address');
  if (!all && !amount) return toast('Enter an amount in sats');
  const rate = feeRate();
  if (!rate) return toast('Give a fee rate in sats per virtual byte, or choose Slow, Medium or Fast');
  try {
    built = await invoke('spend_build', { name: vault.name, to, amount, rate, sendAll: all, coins: [...coinPick] });
    // the review is already showing this very spend (same selection, from wallet::select); keep it and say it is real now
    await sendPreview();
    // a segwit transaction's id cannot change when it is signed, so what the person said this spend is can be
    // recorded against it now, before a single signature
    if (disposalKind) { try { store = await invoke('disposal_set', { name: vault.name, txid: built.txid, kind: disposalKind }); } catch (e) { fail(e); } }
    $('#build-btn').textContent = 'Sign with keys'; $('#build-btn').onclick = () => show('sign');
  } catch (e) { fail(e); }
};

// ---------- sign ----------
const packedForm = (pub) => (store.forms || {})[pub] || 0;        // low 4 bits: form factor; top bit: NFC
const isNano = (pub) => [2, 4].includes(packedForm(pub) & 0x0F);
// packed byte: low 4 bits form factor · 0x80 has NFC · 0x40 NFC was actually asked (older builds stored the form alone)
// form 14 is a Nitrokey 3 (cards.rs FORM_NITROKEY3): its maker is read from the card's AID, never from the reader's name
const FORM_NITROKEY3 = 14;
const isNitro = (form) => (form & 0x0F) === FORM_NITROKEY3;
const modelName = (form, nfc, asked = true) => ({ 1: nfc ? 'YubiKey 5 NFC' : asked ? 'YubiKey 5 (USB-A)' : 'YubiKey 5, USB-A', 2: 'YubiKey 5 Nano', 3: nfc ? 'YubiKey 5C NFC' : asked ? 'YubiKey 5C' : 'YubiKey 5, USB-C', 4: 'YubiKey 5C Nano', 5: 'YubiKey 5Ci', [FORM_NITROKEY3]: 'Nitrokey 3' }[form & 0x0F] || 'YubiKey 5');
// where a key is touched, in words: a YubiKey on its gold contact (a Nano: the strip along its edge), a Nitrokey 3 on its top
const touchWhere = (form) => isNitro(form) ? 'top of the key' : [2, 4].includes(form & 0x0F) ? 'gold strip' : 'gold contact';
// our own drawings of each shape (not Yubico's artwork): body, gold touch contact, the plug that tells A from C, the keyring hole, an NFC mark
function keyArt(form, nfc, h = 44) {
  const f = form & 0x0F, usbC = [3, 4, 5].includes(f), nano = [2, 4].includes(f);
  // a Nitrokey 3 is the silhouette the device chooser draws, at this height
  if (f === FORM_NITROKEY3) { const w = Math.round(h * 72 / 56); return deviceOf('nitrokey3').svg.replace('<svg ', `<svg style="width:${w}px;height:${h}px;flex:0 0 auto;color:var(--ink)" `); }
  const plug = usbC ? '<rect x="15" y="2" width="14" height="9" rx="4.5" fill="none" stroke="var(--muted)" stroke-width="1.8"/>' : '<rect x="12" y="2" width="20" height="12" rx="1.5" fill="none" stroke="var(--muted)" stroke-width="1.8"/><rect x="16" y="5" width="4" height="4" fill="var(--muted)"/><rect x="24" y="5" width="4" height="4" fill="var(--muted)"/>';
  const top = usbC ? 10 : 13;
  const body = nano ? `<rect x="9" y="${top}" width="26" height="22" rx="5" fill="var(--ink)"/><circle cx="22" cy="${top + 11}" r="6" fill="none" stroke="var(--gold)" stroke-width="3"/><circle cx="22" cy="${top + 11}" r="1.8" fill="var(--signal)"/>`
    : `<rect x="7" y="${top}" width="30" height="${f === 5 ? 50 : 58}" rx="7" fill="var(--ink)"/><circle cx="22" cy="${top + 22}" r="8" fill="none" stroke="var(--gold)" stroke-width="3.5"/><circle cx="22" cy="${top + 22}" r="2.2" fill="var(--signal)"/>`
      + (f === 5 ? `<rect x="17" y="${top + 50}" width="10" height="9" rx="2" fill="none" stroke="var(--muted)" stroke-width="1.8"/>` : `<circle cx="22" cy="${top + 47}" r="4" fill="var(--panel)"/>`)
      + (nfc ? `<path d="M15 ${top + 8}a10 10 0 0 1 14 0M18 ${top + 11}a6 6 0 0 1 8 0" fill="none" stroke="var(--gold)" stroke-width="1.4" stroke-linecap="round" opacity=".8"/>` : '');
  const H = nano ? top + 24 : top + 62;
  const unit = h / 75;   // one scale for every model, so a Nano is drawn as small as it is
  return `<svg viewBox="0 0 44 ${H}" height="${Math.round(H * unit)}" width="${Math.round(44 * unit)}" aria-hidden="true" style="flex:0 0 auto">${plug}${body}</svg>`;
}
const keyGlyph = (pub) => isNano(pub)
  ? '<svg viewBox="0 0 44 84" width="34" height="65" aria-hidden="true"><circle class="khalo" cx="22" cy="65" r="15"/><rect class="kplug" x="13" y="38" width="18" height="14" rx="2"/><rect class="kbody" x="7" y="50" width="30" height="30" rx="7"/><circle class="kring" cx="22" cy="65" r="7"/><circle class="kdot" cx="22" cy="65" r="2"/></svg>'
  : '<svg viewBox="0 0 44 84" width="34" height="65" aria-hidden="true"><circle class="khalo" cx="22" cy="40" r="17"/><rect class="kplug" x="13" y="2" width="18" height="14" rx="2"/><rect class="kbody" x="5" y="14" width="34" height="66" rx="8"/><circle class="kring" cx="22" cy="40" r="9"/><circle class="kdot" cx="22" cy="40" r="2.5"/></svg>';
// the ring: one arc per signature the vault needs, filled as they arrive; the centre turns green when it can be sent
/// The ceremony's ring: `need` segments round a centre, `have` of them filled. Signing draws it, and so does every
/// scanner (plates, shares, animated codes) — one picture for "this many of that many are in". Past 24 segments it
/// is one arc that fills, since a long animated code has too many pieces to show each.
function ringMarkup(have, need, centre) {
  const R = 44, C = 2 * Math.PI * R;
  let g = `<circle class="halo" cx="60" cy="60" r="56"/>`;
  if (need > 24) {
    const f = Math.max(0, Math.min(1, have / need)) * C;
    g += `<circle class="track" cx="60" cy="60" r="${R}"/><circle class="arc" cx="60" cy="60" r="${R}" stroke-dasharray="${f} ${C - f}" transform="rotate(-90 60 60)"/>`;
  } else {
    const gap = need > 1 ? 7 : 0, seg = C / need - gap;
    for (let i = 0; i < need; i++) {
      const rot = -90 + i * 360 / need + (gap / 2) / C * 360;
      g += `<circle class="track" cx="60" cy="60" r="${R}" stroke-dasharray="${seg} ${C - seg}" transform="rotate(${rot} 60 60)"/>`;
      g += `<circle class="arc" cx="60" cy="60" r="${R}" stroke-dasharray="${seg} ${C - seg}" stroke-dashoffset="${i < have ? 0 : seg}" stroke-linecap="butt" transform="rotate(${rot} 60 60)" style="${i < have ? '' : 'opacity:0'}"/>`;
    }
  }
  return g + (centre || `<circle class="dot" cx="60" cy="60" r="11"/>`);
}
const TICK = '<path class="tick" d="M42 61l12 12 25-27"/>';
function drawRing(have, need, ready) {
  const g = ringMarkup(have, need);
  const el = $('#cer-ring'), sig = `${have}/${need}/${ready}`;
  if (drawRing.sig === sig && el.firstChild) return;   // the 2 s card poll must not restart the drawing
  const was = drawRing.need === need ? (drawRing.have || 0) : have; drawRing.sig = sig; drawRing.have = have; drawRing.need = need;
  el.innerHTML = g; el.classList.toggle('ready', ready);
  // arcs that are new since the last look sweep in
  [...el.querySelectorAll('.arc')].forEach((a, i) => { if (i >= was && i < have) { const seg = a.getAttribute('stroke-dasharray').split(' ')[0]; a.style.opacity = 1; a.setAttribute('stroke-dashoffset', seg); requestAnimationFrame(() => requestAnimationFrame(() => a.setAttribute('stroke-dashoffset', 0))); } });
  el.setAttribute('aria-label', `${have} of ${need} signatures collected`);
}
const canSign0 = (st, k) => !st.ready && k.signed < st.inputs && cards.some(c => c.pubkey === k.pubkey);
// One drawn scene for where the person is: key and arrow at the port, key seated, finger on the contact.
// `form` picks the key: a Nano sits nearly flush and is touched on the gold strip along its top edge.
function guideArt(step, form) {
  const f = form & 0x0F, nano = [2, 4].includes(f), usbA = [1, 2].includes(f), seated = step !== 'plug';
  const caps = [0, 1, 2].map(i => `<rect class="g-cap" x="${180 + i * 30}" y="28" width="24" height="22" rx="4"/><rect class="g-cap" x="${180 + i * 30}" y="56" width="24" height="22" rx="4"/><rect class="g-cap" x="${180 + i * 30}" y="84" width="24" height="22" rx="4"/>`).join('');
  const lap = `<rect class="g-lap" x="158" y="14" width="120" height="104" rx="12"/>${caps}<rect class="g-port" x="158" y="${usbA ? 55 : 58}" width="7" height="${usbA ? 22 : 16}" rx="${usbA ? 1.5 : 3.5}"/>`;
  let key, touchAt;
  if (isNitro(form)) {
    // a Nitrokey 3: the flat wedge, widest at the plug, the whole top face its touch; a finger mark and a lanyard slot
    const x = seated ? 60 : 26;
    key = `<g${seated ? '' : ' class="g-nudge"'}><rect class="g-line" x="${x + 84}" y="58" width="20" height="16" rx="6"/><path class="g-ink" d="M${x + 8} 55L${x + 84} 49q5 -.5 5 5v24q0 5.5 -5 5L${x + 8} 77q-8 0 -8 -8v-6q0 -8 8 -8z"/><rect class="g-hole" x="${x + 10}" y="62" width="5" height="8" rx="1.5"/><circle class="g-ring" cx="${x + 48}" cy="66" r="7.5"/><path class="g-ring" d="M${x + 59} 59l2 -2.6M${x + 61} 66h3.5M${x + 59} 73l2 2.6"/><circle class="g-led${step === 'touch' ? ' g-blink' : ''}" cx="${x + 76}" cy="57" r="2.2"/></g>`;
    touchAt = [x + 48, 66];
  } else if (nano) {
    const x = seated ? 136 : 82;   // only the head shows once it is in
    key = `<g${seated ? '' : ' class="g-nudge"'}><rect class="g-line" x="${x + 22}" y="${usbA ? 57 : 60}" width="18" height="${usbA ? 18 : 12}" rx="${usbA ? 1.5 : 5}"/><rect class="g-ink" x="${x}" y="52" width="24" height="28" rx="5"/><rect class="g-strip" x="${x + 3}" y="52" width="18" height="4" rx="2"/>${step === 'touch' ? `<circle class="g-led g-blink" cx="${x + 12}" cy="68" r="2.4"/>` : ''}</g>`;
    touchAt = [x + 12, 54];
  } else {
    const x = seated ? 60 : 26;
    key = `<g${seated ? '' : ' class="g-nudge"'}><rect class="g-line" x="${x + 84}" y="${usbA ? 55 : 58}" width="${usbA ? 24 : 20}" height="${usbA ? 22 : 16}" rx="${usbA ? 1.5 : 6}"/><rect class="g-ink" x="${x}" y="46" width="88" height="40" rx="9"/><circle class="g-hole" cx="${x + 14}" cy="66" r="4.5"/><circle class="g-gold" cx="${x + 52}" cy="66" r="10"/><circle class="g-led${step === 'touch' ? ' g-blink' : ''}" cx="${x + 52}" cy="66" r="2.6"/></g>`;
    touchAt = [x + 52, 66];
  }
  const arrow = seated ? '' : `<path class="g-arrow g-nudge" d="M${nano ? 128 : 134} ${usbA ? 66 : 66}h16m-6 -6l6 6l-6 6"/>`;
  const [tx, ty] = touchAt;
  // the fingertip hovers just above the touch point and three ticks point at it: the contact itself stays in view
  const finger = step !== 'touch' ? '' : `<circle class="g-pulse" cx="${tx}" cy="${ty}" r="${nano ? 9 : 15}"/><g transform="translate(${tx} ${ty - (nano ? 9 : 17)}) rotate(-38)"><rect class="g-finger" x="-13" y="-120" width="26" height="120" rx="13"/><path class="g-nail" d="M-7 -20 v-9 a7 7 0 0 1 14 0 v9"/></g><path class="g-tick" d="M${tx + (nano ? 13 : 20)} ${ty - 12}l5 -6M${tx + (nano ? 17 : 25)} ${ty - 2}l8 -2M${tx - (nano ? 13 : 20)} ${ty + 8}l-6 5"/>`;
  // at the touch step the picture moves in on the key, so where to touch is the biggest thing in it
  const box = step !== 'touch' ? '0 0 250 132' : nano ? `${tx - 58} ${ty - 40} 125 66` : `${tx - 92} ${ty - 58} 170 90`;
  // the laptop is drawn over the plug, so a seated key's plug disappears into the port
  return `<svg viewBox="${box}" role="img">${seated ? key + lap : lap + key}${arrow}${finger}</svg>`;
}
let guideOn = true; try { guideOn = localStorage.getItem('a2k-guide') !== 'off'; } catch {}
function paintGuide(step, form) {
  $$('#gsteps li').forEach(li => { const order = ['plug', 'pin', 'touch'], i = order.indexOf(li.dataset.step), n = order.indexOf(step); li.classList.toggle('now', i === n); li.classList.toggle('done', i < n); });
  $('#guide-toggle').textContent = guideOn ? 'Hide the pictures' : 'Show me how';
  const g = $('#guide'), sig = guideOn ? step + ':' + (form & 0x0F) : 'off';
  g.hidden = !guideOn; if (g._sig === sig) return; g._sig = sig;   // the 2 s card poll must not restart the animation
  g.innerHTML = guideOn ? guideArt(step, form) : '';
}
$('#pin').addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); if (!$('#sign-btn').disabled) $('#sign-btn').click(); } });
$('#guide-toggle').onclick = () => { guideOn = !guideOn; try { localStorage.setItem('a2k-guide', guideOn ? 'on' : 'off'); } catch {} $('#guide')._sig = null; renderSign(); };
async function renderSign() {
  if (!vault) return;
  let st = null; try { st = await invoke('spend_ceremony', { name: vault.name }); } catch {}
  if (!st || !st.ours) {
    for (const id of ['#ceremony', '#export-card', '#broadcast-card', '#message-card']) $(id).hidden = true; stopQr();
    $('#nospend').hidden = false; $('#nospend-title').textContent = st ? 'That spend belongs to another vault' : 'No spend in progress';
    $('#nospend-text').textContent = st ? 'The transaction waiting for signatures spends a different vault\'s coins. Switch to that vault to carry on.' : 'Build one on the Send screen, or load a PSBT there.';
    return;
  }
  $('#nospend').hidden = true;
  $('#ceremony').hidden = false; $('#ceremony').classList.toggle('can-sign', st.keys.some(k => canSign0(st, k)));
  const have = st.have, need = st.need, full = k => k.signed >= st.inputs;
  const canSign = k => !st.ready && !full(k) && cards.some(c => c.pubkey === k.pubkey);
  drawRing(have, need, st.ready);
  $('#cer-have').textContent = have; $('#cer-need').textContent = need; $('#cer-word').textContent = need === 1 ? 'signature' : 'signatures';
  const signed = st.keys.filter(full).map(k => k.name), open = st.keys.filter(k => !full(k)).map(k => k.name), left = need - have;
  $('#sign-progress').textContent = st.ready ? `${signed.join(' and ')} signed. Nothing has been sent yet.`
    : (signed.length ? `${signed.join(' and ')} ${signed.length === 1 ? 'has' : 'have'} signed. ` : '') + `${left === 1 ? 'One more key' : left + ' more keys'}${open.length > left ? ', any of ' : ': '}${open.join(', ')}.` + (st.inputs > 1 ? ` Each key signs all ${st.inputs} inputs, one touch each.` : '');
  const out = st.pays.filter(p => !p.ours), back = st.pays.filter(p => p.ours).reduce((a, p) => a + p.sats, 0);
  $('#sign-summary').textContent = (out.length ? 'Pays ' + out.map(p => `${sats(p.sats)} sats to ${p.address.slice(0, 12)}…${p.address.slice(-6)}`).join(', ') : 'Pays nothing outside this vault') + (back ? ` · ${sats(back)} back to this vault` : '') + ` · fee ${sats(st.fee)} sats`;
  const tiles = st.keys.map(k => {
    const waiting = renderSign.signing === k.pubkey;
    const cls = full(k) ? 'signed' : waiting ? 'touch' : k.signed ? 'part' : canSign(k) ? 'here' : st.ready ? 'spare' : '';
    const label = full(k) ? '✓ signed' : waiting ? 'waiting for touch' : k.signed ? `${k.signed} of ${st.inputs} inputs` : canSign(k) ? 'plugged in · enter PIN' : st.ready ? 'not needed' : 'not yet';
    const vk = vault.keys.find(x => x.pubkey === k.pubkey), glyph = vk && vk.device && k.pubkey.startsWith('[') ? `<span class="devart">${deviceOf(vk.device).svg}</span>` : keyGlyph(k.pubkey);
    const pickable = !full(k) && !st.ready, picked = renderSign.chosen === k.pubkey;
    return `<div class="ckey ${cls}${isNano(k.pubkey) ? ' nano' : ''}${picked ? ' chosen' : ''}"${pickable ? ` role="button" tabindex="0" data-pick-key="${esc(k.pubkey)}" title="Sign with ${esc(k.name)}"` : ''}>${glyph}<span class="kname" title="${esc(k.name)}">${esc(k.name)}</span><span class="kstate">${label}</span></div>`;
  }).join('');
  if (renderSign.tiles !== tiles) { renderSign.tiles = tiles; $('#cer-keys').innerHTML = tiles; }   // unchanged tiles keep their pulse running
  const staged = (await invoke('store_get')).last_message;
  paintCrumbs.via = staged ? 'messages' : 'send'; paintCrumbs('sign');
  if (staged) { $('#sign-summary').innerHTML = `Message for <span class="mono">${staged.address.slice(0, 14)}…${staged.address.slice(-6)}</span>: “${staged.message.length > 60 ? staged.message.slice(0, 60) + '…' : staged.message}”`; $('#message-summary').textContent = `The proof for ${staged.address} is fully signed. Nothing is broadcast; the signature is text you can hand to whoever asked.`; }
  $('#message-card').hidden = !(st.ready && staged);
  $('#broadcast-card').hidden = !st.ready || !!staged;
  $('#export-card').hidden = !!staged; $('#export-tx').hidden = !st.ready;
  if (st.ready) { $('#sign-waiting').hidden = true; $('#act-row').hidden = false; $('#gsteps').hidden = true; $('#guide').hidden = true; $('#guide-toggle').textContent = ''; $('#sign-title').textContent = 'All signatures in'; $('#sign-text').textContent = staged ? 'Produce the signature below; nothing is broadcast.' : 'Nothing has been sent. Review the summary, then send it.'; $('#pin-field').hidden = true; $('#sign-warn').hidden = true; $('#sign-btn').disabled = true; $('#broadcast-text').textContent = `Broadcast to ${netName()} through the Electrum server.`; return; }
  // while a key is signing, the 2 s poll redraws the ring and tiles above but must not touch this part:
  // it used to flip "TOUCH THE KEY NOW" back to "plugged in" and re-arm the Sign button mid-signature
  if (renderSign.signing) return;
  const usable = cards.filter(c => c.pubkey && st.keys.some(k => k.pubkey === c.pubkey && !full(k)));
  $('#sign-choose').hidden = st.ready || st.keys.filter(k => !full(k)).length < 2;
  const q = (n) => `‘${n}’`, list = (ks) => ks.map(k => q(k.name)).join(ks.length === 2 ? ' or ' : ', ');
  $('#gsteps').hidden = false; $('#act-row').hidden = false; $('#sign-waiting').hidden = true; $('#sign-cancel').hidden = false; $('#dev-sign-row').hidden = true;
  if (!$('#dev-cer').hidden) { $('#dev-cer').hidden = true; stopDevQr(); devCer.sig = ''; }
  let openDev = vault.ext ? st.keys.filter(k => !full(k) && k.pubkey.startsWith('[')) : [];
  const chosen = st.keys.find(k => k.pubkey === renderSign.chosen && !full(k));
  if (renderSign.chosen && !chosen) renderSign.chosen = null;   // it signed, or is not this vault's: back to the default
  let chosenCardAway = null;
  if (chosen && chosen.pubkey.startsWith('[')) { openDev = [chosen]; usable.length = 0; }            // a device, even with a card plugged in
  else if (chosen) { const c = usable.find(c => c.pubkey === chosen.pubkey); usable.length = 0; if (c) usable.push(c); else chosenCardAway = chosen; }
  if (chosenCardAway) {
    $('#sign-title').textContent = `Plug in ${q(chosenCardAway.name)}`; $('#sign-text').textContent = 'Or choose another key above.';
    $('#pin-field').hidden = true; $('#sign-warn').hidden = true; $('#sign-btn').disabled = true; $('#dev-sign-row').hidden = true;
    renderSign.focusedFor = null; paintGuide('plug', packedForm(chosenCardAway.pubkey) || 3); return;
  }
  if (!usable.length && openDev.length) {
    // signing on a device: the PSBT goes out as a code or a file and comes back signed; nothing to plug in here
    const devs = openDev.map(k => ({ k, d: deviceOf((vault.keys.find(x => x.pubkey === k.pubkey) || {}).device) }));
    const { k, d } = devs[0], cardsOpen = st.keys.filter(x => !full(x) && !x.pubkey.startsWith('['));
    const qr = d.offers.includes('qr') || d.id === 'other', sd = d.offers.includes('sd');
    $('#sign-title').textContent = devs.length > 1 ? `Sign on ${list(devs.map(x => x.k))}` : `Sign on ${q(k.name)}`;
    $('#sign-text').textContent = (qr ? `Show ${devs.length > 1 ? 'the device' : 'the ' + d.name} the code: scan it there, check what it pays \u2014 the change should show as change, back to this vault \u2014 and approve. Then scan the signed code it shows.`
      : sd ? `Save the PSBT to its SD card, sign on the ${d.name}, and open the signed file it writes.`
      : `It signs over USB, which this app does not do yet: save the PSBT, sign it in the ${d.name}\u2019s own app or in Sparrow, and open the signed file here.`)
      + (cardsOpen.length ? ` Or plug in ${list(cardsOpen)}.` : '');
    $('#sign-warn').hidden = true;
    renderSign.devFormat = d.id === 'coldcard-q' ? 'bbqr' : 'ur';
    if (qr) { devCeremony(st, k); return; }   // a device that reads codes: the guided ceremony
    $('#act-row').hidden = true; $('#gsteps').hidden = true; $('#dev-cer').hidden = true; $('#dev-sign-row').hidden = false;
    $('#dev-sign-show').hidden = true; $('#dev-sign-scan').hidden = true; $('#dev-sign-save').classList.toggle('primary', true);
    const g = $('#guide'), sig = 'dev:' + d.id; g.hidden = false; if (g._sig !== sig) { g._sig = sig; g.innerHTML = `<div class="devguide">${d.svg}</div>`; }
    renderSign.focusedFor = null; return;
  }
  if (!usable.length) {
    const open = st.keys.filter(k => !full(k)), done = st.keys.filter(full), next = open[0];
    const inNow = cards.map(c => st.keys.find(k => k.pubkey === c.pubkey)), signedIn = inNow.filter(k => k && full(k)), stranger = cards.length && inNow.some(k => !k);
    // three different situations, three different sentences: a success must not read like a fault
    if (signedIn.length) { $('#sign-title').textContent = `${q(signedIn[0].name)} signed. Plug in another key`; $('#sign-text').textContent = `${have} of ${need} signatures are on. Use ${list(open)} next; ${q(signedIn[0].name)} can come out or stay in.`; }
    else if (stranger) { $('#sign-title').textContent = 'That key is not one of this vault’s'; $('#sign-text').textContent = `Swap it for ${list(open)}.`; }
    else { $('#sign-title').textContent = open.length === 1 ? `Plug in ${q(open[0].name)}` : 'Plug in a key'; $('#sign-text').textContent = done.length ? `${have} of ${need} signatures are on. Use ${list(open)} next.` : `Any of ${list(open)}.`; }
    $('#pin-field').hidden = true; $('#sign-warn').hidden = true; $('#sign-btn').disabled = true;
    renderSign.focusedFor = null; paintGuide('plug', next ? (packedForm(next.pubkey) || 3) : 3); return;
  }
  const c = usable[0]; const k = vault.keys.find(k => k.pubkey === c.pubkey), nano = [2, 4].includes(c.form & 0x0F), where = touchWhere(c.form);
  $('#sign-title').textContent = `Enter the PIN for ${q(k.name)}`; $('#sign-text').textContent = `${modelName(c.form, c.nfc)} · serial ${c.serial}. Press Enter, then touch the key when its light flashes.`;
  $('#pin-field').hidden = false; $('#sign-warn').hidden = false; $('#sign-btn').disabled = false;
  paintGuide('pin', c.form || 3);
  // hands stay on the keyboard: the PIN field takes focus once when a key that can sign appears, and Enter signs
  if (renderSign.focusedFor !== c.pubkey) { renderSign.focusedFor = c.pubkey; if (!document.activeElement || document.activeElement === document.body) $('#pin').focus(); }
  $('#sign-btn').onclick = async () => {
    const pin = $('#pin').value; if (!pin) return toast('Enter the key’s PIN');
    // A brushed key with Yubico OTP on types 44 letters of "modhex" and presses Enter. Sent to the card as a PIN that would cost
    // one of its three tries, so it is stopped here: nothing a person chooses as a PIN is 32+ characters from these sixteen letters.
    if (/^[cbdefghijklnrtuv]{32,}$/.test(pin)) { $('#pin').value = ''; return toast('That was the key typing a one-time code, not a PIN. Nothing was sent to the card and no PIN try was used. Turn off Yubico OTP on this key to stop it happening.', 9000); }
    // the next action is physical: the software steps back — no buttons, one status line, the picture moves in on the key
    renderSign.signing = c.pubkey; renderSign.tiles = null; $('#act-row').hidden = true; $('#sign-cancel').hidden = true; $('#sign-warn').hidden = true; $('#sign-waiting').hidden = false;
    $('#sign-title').textContent = `Touch the ${where} now`;
    $('#sign-text').textContent = `${nano ? 'It runs along the key’s top edge. Touch it' : isNitro(c.form) ? 'Press it' : 'Touch it'} when its light flashes${st.inputs > 1 ? `, once for each of the ${st.inputs} inputs` : ''}. Keep the key plugged in.`;
    paintGuide('touch', c.form || 3); renderSign();
    try { const r = await invoke('spend_sign', { name: vault.name, reader: c.reader, pin }); $('#pin').value = ''; toast(`${r.signer} signed ${r.signed_inputs.length} input${r.signed_inputs.length === 1 ? '' : 's'}`); }
    catch (e) { fail(e); }
    renderSign.signing = false; renderSign.tiles = null; renderSign();
  };
}
document.addEventListener('click', (e) => { const t = e.target.closest('[data-pick-key]'); if (!t || renderSign.signing) return; renderSign.chosen = renderSign.chosen === t.dataset.pickKey ? null : t.dataset.pickKey; renderSign.tiles = null; renderSign(); });
document.addEventListener('keydown', (e) => { const t = e.target.closest && e.target.closest('[data-pick-key]'); if (t && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); t.click(); } });
$('#export-psbt').onclick = async () => { try { const p = await invoke('spend_export', { name: vault.name, what: 'psbt' }); $('#export-note').textContent = 'Saved ' + p; } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
$('#export-tx').onclick = async () => { try { const p = await invoke('spend_export', { name: vault.name, what: 'tx' }); $('#export-note').textContent = 'Saved ' + p + ' — raw hex, broadcastable from any wallet or explorer.'; } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
try { $('#export-card').open = localStorage.getItem('a2k-export-open') === '1'; } catch {}
$('#export-card').addEventListener('toggle', () => { try { localStorage.setItem('a2k-export-open', $('#export-card').open ? '1' : '0'); } catch {} if (!$('#export-card').open) stopQr(); });
$('#export-pdf').onclick = async () => { try { $('#export-note').textContent = 'Saved ' + await invoke('spend_pdf', { name: vault.name }) + ' — the codes and the text on it are the same PSBT.'; } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
// QR: UR crypto-psbt. A looping set of parts (plain ones, then fountain mixtures so a late camera still completes), or one still code when it fits.
const qrShow = { timer: null, parts: [], single: null, still: false, i: 0, bbqr: [], fmt: 'ur' };
function drawQr(text, canvas) {
  const alnum = /^[0-9A-Z $%*+\-./:]*$/.test(text);
  const qr = qrcode(0, 'L'); qr.addData(text, alnum ? 'Alphanumeric' : 'Byte'); qr.make();
  const n = qr.getModuleCount(), cv = canvas || $('#qr-canvas'), ctx = cv.getContext('2d'), m = Math.floor(cv.width / (n + 8)), off = Math.floor((cv.width - m * n) / 2);
  ctx.fillStyle = '#fff'; ctx.fillRect(0, 0, cv.width, cv.height); ctx.fillStyle = '#000';
  for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) if (qr.isDark(r, c)) ctx.fillRect(off + c * m, off + r * m, m, m);
}
function stopQr() { clearInterval(qrShow.timer); qrShow.timer = null; $('#qr-panel').hidden = true; }
function playQr() {
  clearInterval(qrShow.timer); qrShow.timer = null;
  $('#qr-format').textContent = qrShow.fmt === 'bbqr' ? 'As UR (most signers)' : 'As BBQr (Coldcard Q)'; $('#qr-format').hidden = !qrShow.bbqr.length;
  if (qrShow.fmt === 'bbqr' && qrShow.bbqr.length) {
    // BBQr parts are all needed, in any order: loop them
    $('#qr-mode').hidden = true;
    const tick = () => { const i = qrShow.i++ % qrShow.bbqr.length; drawQr(qrShow.bbqr[i]); $('#qr-title').textContent = qrShow.bbqr.length > 1 ? `Frame ${i + 1} of ${qrShow.bbqr.length}` : 'One code'; };
    $('#qr-note').textContent = `BBQr, the format the Coldcard Q scans.${qrShow.bbqr.length > 1 ? ' It needs every frame, in any order; keep the camera on it until it completes.' : ''}`;
    tick(); if (qrShow.bbqr.length > 1) qrShow.timer = setInterval(tick, 400); return;
  }
  const still = qrShow.still && qrShow.single, plain = qrShow.parts.length / 2;
  $('#qr-mode').hidden = !qrShow.single || qrShow.parts.length < 2; $('#qr-mode').textContent = still ? 'Show the animated code' : 'Show one still code';
  if (still || qrShow.parts.length < 2) { drawQr(qrShow.single || qrShow.parts[0]); $('#qr-title').textContent = 'One code'; $('#qr-note').textContent = 'UR crypto-psbt, the format Sparrow, Keystone, Passport, Jade and SeedSigner scan. Dense: hold the camera steady, or use the animated code.'; return; }
  const tick = () => { const i = qrShow.i++ % qrShow.parts.length; drawQr(qrShow.parts[i]); $('#qr-title').textContent = `Frame ${i + 1} of ${qrShow.parts.length}`; };
  $('#qr-note').textContent = `Point the other wallet's camera at it and wait. Frames 1 to ${plain} carry the transaction; the rest are spare frames, each a mix of several parts, so a camera that starts late or misses one still finishes. UR crypto-psbt, as Sparrow, Keystone, Passport, Jade and SeedSigner scan.`;
  tick(); qrShow.timer = setInterval(tick, 260);
}
async function startQr(fmt) { try { const r = await invoke('spend_qr'); qrShow.fmt = fmt || 'ur'; qrShow.bbqr = r.bbqr || []; qrShow.parts = r.parts; qrShow.single = r.single; qrShow.still = !!r.single && r.parts.length < 2; qrShow.i = 0; $('#qr-panel').hidden = false; playQr(); } catch (e) { fail(e); } }
$('#qr-format').onclick = () => { qrShow.fmt = qrShow.fmt === 'bbqr' ? 'ur' : 'bbqr'; qrShow.i = 0; playQr(); };
$('#export-qr').onclick = async () => { if (!$('#qr-panel').hidden) return stopQr(); try { const r = await invoke('spend_qr'); qrShow.fmt = 'ur'; qrShow.bbqr = r.bbqr || []; qrShow.parts = r.parts; qrShow.single = r.single; qrShow.still = !!r.single && r.parts.length < 2; qrShow.i = 0; $('#qr-panel').hidden = false; playQr(); } catch (e) { fail(e); } };
$('#dev-sign-show').onclick = () => { const c = $('#export-card'); c.hidden = false; c.open = true; startQr(renderSign.devFormat); c.scrollIntoView({ behavior: 'smooth', block: 'center' }); };
$('#dev-sign-save').onclick = () => $('#export-psbt').click();
$('#dev-sign-file').onclick = () => loadPsbt(null);
const pscan = { stream: null, timer: null, parts: new Set() };
function pscanStop() { if (pscan.timer) clearInterval(pscan.timer); pscan.timer = null; if (pscan.stream) pscan.stream.getTracks().forEach(t => t.stop()); pscan.stream = null; }
function pscanClose() { pscanStop(); pscan.parts = new Set(); $('#pscan-modal').classList.remove('on'); }
async function pscanOpen(deviceId) {
  pscanStop(); stopQr(); $('#pscan-modal').classList.add('on');
  const note = $('#pscan-note');
  try { pscan.stream = await camOpen(deviceId); $('#cam4').srcObject = pscan.stream; camChooser($('#pscan-pick'), pscan.stream, pscanOpen); }
  catch { note.textContent = 'No camera available on this computer. Save the PSBT and open the signed file instead.'; return; }
  const reader = makeQrReader(note);
  let busy = false;
  pscan.timer = setInterval(async () => {
    const v = $('#cam4'); if (busy || !v || !v.videoWidth) return;
    const data = reader.read(v); if (!data) return;
    const t = data.trim(), multi = /^ur:/i.test(t) || t.startsWith('B$');
    if (multi) { const n = pscan.parts.size; pscan.parts.add(/^ur:/i.test(t) ? t.toLowerCase() : t); if (pscan.parts.size === n) return; }
    busy = true;
    try {
      const c = await invoke('spend_import', { name: vault.name, text: multi ? [...pscan.parts].join('\n') : t });
      scanBar($('#pscan-note'), null); pscanClose(); toast(`Signed code read: ${c.have} of ${c.need} signatures on it, every one checked`, 5000); renderSign.tiles = null; renderSign();
    } catch (e) {
      if (!scanShow(note, e)) { note.textContent = String(e && e.message || e); scanBar(note, null); pscan.parts = new Set(); }
    }
    busy = false;
  }, 250);
}
$('#dev-sign-scan').onclick = () => pscanOpen();
$('#pscan-close').onclick = pscanClose;
$('#qr-mode').onclick = () => { qrShow.still = !qrShow.still; playQr(); };
$('#qr-close').onclick = stopQr;
// a PSBT made or part-signed elsewhere
async function loadPsbt(text) {
  try { const c = await invoke('spend_import', { name: vault.name, text }); built = null; $('#psbt-text').value = ''; $('#psbt-paste').hidden = true; toast(`PSBT loaded: ${c.have} of ${c.need} signatures on it, every one checked`, 5000); show('sign'); }
  catch (e) { if (!/cancelled/.test(String(e))) fail(e); }
}
$('#psbt-open').onclick = () => loadPsbt(null);
$('#psbt-paste-toggle').onclick = () => { $('#psbt-paste').hidden = !$('#psbt-paste').hidden; if (!$('#psbt-paste').hidden) $('#psbt-text').focus(); };
$('#psbt-load').onclick = () => { const t = $('#psbt-text').value.trim(); if (!t) return toast('Paste the PSBT first'); loadPsbt(t); };
$('#copy-psbt').onclick = async () => { try { const st = await invoke('store_get'); if (!st.last_psbt) return toast('No spend in progress'); await navigator.clipboard.writeText(st.last_psbt); toast('PSBT copied (base64)'); } catch (e) { fail(e); } };
askTwice($('#staged-discard'), 'Discard the transaction that is waiting for signatures? Nothing has been sent; any signatures on it are lost.', 'Yes, discard it', async () => { await invoke('spend_discard'); built = null; location.reload(); });
$('#sign-cancel').onclick = async () => { await invoke('spend_discard'); built = null; ownDone = null; coinPick.clear(); $('#build-btn').textContent = 'Build transaction'; $('#build-btn').onclick = null; show('send'); location.reload(); };
$('#broadcast-btn').onclick = async () => {
  $('#broadcast-btn').disabled = true;
  try { // who is being paid is read from the transaction itself, not from what the Send form remembered: a spend loaded from a file, or finished after a relaunch, counts too
    let dests = []; try { dests = (await invoke('spend_ceremony', { name: vault.name })).pays.filter(p => !p.ours).map(p => p.address.toLowerCase()); } catch {}
    const r = await invoke('spend_broadcast', { name: vault.name }); vault._stale = true; await ownLabels(r.txid); const paid = bookOf().find(c => dests.includes(c.address.toLowerCase()));
    if (paid) { try { store = await invoke('contact_used', { address: paid.address, network: vault.network }); if (!labelOf('tx:' + r.txid)) await invoke('label_set', { name: vault.name, key: 'tx:' + r.txid, label: 'to ' + paid.name }); } catch {} }
    built = null; coinPick.clear(); toast('Sent. ' + r.txid.slice(0, 16) + '…', 8000); $('#broadcast-text').innerHTML = `Broadcast. <a href="#" data-tx="${r.txid}">${r.txid}</a>`; setTimeout(() => show('home'), 2500); }
  catch (e) { fail(e); $('#broadcast-btn').disabled = false; }
};

$('#message-finish').onclick = async () => {
  $('#message-finish').disabled = true;
  try { const r = await invoke('message_finish', { name: vault.name }); built = null; lastMessageOut = r; show('messages'); toast('Message signed'); }
  catch (e) { fail(e); }
  $('#message-finish').disabled = false;
};

// ---------- coin control ----------
const coinPick = new Set();
function renderCoins() {
  const list = $('#coin-list'); if (!list) return;
  const coins = (vault && vault._coins) || [];
  const live = new Set(coins.map(c => c.txid + ':' + c.vout)); for (const op of [...coinPick]) if (!live.has(op)) coinPick.delete(op);
  if (!coins.length) { list.innerHTML = '<div class="muted" style="font-size:13px">the vault holds no coins</div>'; $('#coin-sum').textContent = ''; return; }
  const h = vault._height || 0;
  const L = vaultLedger(vault.name), f = txFacts[vault.name] || {}, price = f.now && f.ccy ? f.now.price : null;
  list.innerHTML = coins.slice().sort((a, b) => b.value - a.value).map(c => { const op = c.txid + ':' + c.vout; const conf = c.height ? h - c.height + 1 : 0;
    return `<label class="coin"><input type="checkbox" data-op="${op}" ${coinPick.has(op) ? 'checked' : ''}><span class="mono">${c.txid.slice(0, 10)}…${c.txid.slice(-4)}:${c.vout}${vault._rotating ? ' <span class="muted">· ' + fmtIndex(c.index) + '</span>' : ''}</span>${lblHtml('out:' + op)}<span style="font-variant-numeric:tabular-nums">${sats(c.value)} sats</span><span class="muted">${conf ? conf + ' conf' : 'unconfirmed'}</span>${coinTax(L, c, price, f.ccy)}</label>`; }).join('');
  // Under first-in-first-out or average cost the pick does not decide the cost, so the screen must not imply it does
  const note = $('#coin-method');
  if (note) note.textContent = !L ? '' : L.method === 'utxo'
    ? 'Your method counts the coins a transaction actually spends, so what you tick here is what you dispose of.'
    : `Your method is ${COST_METHODS[L.method].short}, so which coins you tick does not change what the spend costs — only the fee and what it links together.`;
  coinSum();
}
/// One coin's own history: what it cost as it arrived and what it is worth now. Only meaningful under 'utxo' —
/// under the other methods a coin has no cost of its own, so the line says the part that is still true (how long
/// it has been here), and nothing that isn't.
function coinTax(L, c, price, ccy) {
  const op = c.txid + ':' + c.vout, lots = L && L.coins[op];
  const value = price !== null ? c.value / 1e8 * price : null;
  const bits = [];
  if (L && L.method === 'utxo' && lots && lots.length) {
    const cost = lots.some(l => l.cost === null) ? null : lots.reduce((a, l) => a + l.cost, 0);
    const dated = lots.filter(l => l.t !== null), from = dated.length ? Math.min(...dated.map(l => l.t)) : null;
    if (ccy && cost !== null) {
      bits.push(`cost ${money(cost, ccy)}`);
      if (value !== null) { const g = value - cost; bits.push(`now ${money(value, ccy)}`, `<span class="${g >= 0 ? 'ok' : 'warn'}">${g >= 0 ? '+' : '\u2212'}${money(Math.abs(g), ccy)} if spent</span>`); }
    } else if (ccy) bits.push('cost not known here');
    if (from) bits.push(heldFor(from, from).split(' \u00b7 ')[1]);
  } else if (L && lots && lots.length) {
    const dated = lots.filter(l => l.t !== null), from = dated.length ? Math.min(...dated.map(l => l.t)) : null;
    if (from) bits.push(heldFor(from, from).split(' \u00b7 ')[1]);
  }
  // where the price sat when it was bought: by its lots when it has them, else by the day the coin itself arrived
  if (vault.network === 'mainnet') {
    const dated = lots && lots.length ? lots.filter(l => l.t !== null) : [], row = ((txFacts[vault.name] || {}).rows || {})[c.txid];
    const cm = dated.length ? cycleOfLots(dated, renderCoins) : row && row.time && cycleWant([row.time], renderCoins) ? cycleAt(row.time) : null;
    if (cm) bits.push(`received at the ${cycleOrd(cm.pct)} percentile ${cycleStrip(cm)}`);
  }
  return bits.length ? `<span class="coin-tax">${bits.join(' \u00b7 ')}</span>` : '';
}

/// Ways to pick coins, each for a reason a person actually has. The app's own default is "fewest coins", which is
/// the cheapest in fees; the others trade fee for something else and say so.
const COIN_PLANS = {
  fewest:  { name: 'Fewest coins', why: 'largest first: the smallest fee, and what the app does on its own', cmp: (a, b) => b.value - a.value },
  gain:    { name: 'Least gain', why: 'the coins standing at the smallest gain (or a loss) go first', cmp: null },
  oldest:  { name: 'Longest held', why: 'the coins that have been here longest go first', cmp: null },
  private: { name: 'Keep addresses apart', why: 'as few of your addresses joined in one transaction as possible', cmp: null },
  dear:    { name: 'Received highest', why: 'the coins received at the highest historical percentile go first', cmp: null },
};
function planPick(plan) {
  const coins = ((vault && vault._coins) || []).slice(); if (!coins.length) return toast('the vault holds no coins');
  const f = sendFields();
  if (f.all) return toast('This spend takes everything, so there is nothing to choose');
  if (!f.amount) return toast('Enter an amount first, so the coins can be chosen to cover it');
  const L = vaultLedger(vault.name), fx = txFacts[vault.name] || {}, price = fx.now && fx.ccy ? fx.now.price : null;
  const lotsOf = (c) => (L && L.coins[c.txid + ':' + c.vout]) || [];
  const gainOf = (c) => { const lots = lotsOf(c); if (!lots.length || price === null || lots.some(l => l.cost === null)) return Infinity;
    return c.value / 1e8 * price - lots.reduce((a, l) => a + l.cost, 0); };
  const ageOf = (c) => { const d = lotsOf(c).filter(l => l.t !== null); return d.length ? Math.min(...d.map(l => l.t)) : Infinity; };
  if (plan === 'gain') coins.sort((a, b) => (gainOf(a) - gainOf(b)) || (b.value - a.value));
  else if (plan === 'oldest') coins.sort((a, b) => (ageOf(a) - ageOf(b)) || (b.value - a.value));
  else if (plan === 'dear') {
    if (!cycleWant(coins.flatMap(c => lotsOf(c).filter(l => l.t !== null).map(l => l.t)), null)) return toast('Still working out where the price sat when these coins arrived; try again in a moment');
    const multOf = (c) => { const m = cycleOfLots(lotsOf(c), null); return m ? m.pct : -Infinity; };
    coins.sort((a, b) => (multOf(b) - multOf(a)) || (b.value - a.value));
  }
  else if (plan === 'private') coins.sort((a, b) => (b.value >= f.amount) - (a.value >= f.amount) || a.value - b.value || 0);
  else coins.sort(COIN_PLANS.fewest.cmp);
  // a coin costs roughly one input in fee; the exact figure comes back from the preview, which is the real arithmetic
  const inVb = (sendPrev && sendPrev.choice.chosen.length ? (sendPrev.choice.vsize - 95) / sendPrev.choice.chosen.length : 0) || 110;
  const rate = f.rate || 1;
  coinPick.clear(); let have = 0, n = 0;
  for (const c of coins) {
    if (have >= f.amount + Math.ceil(rate * (11 + inVb * n + 90))) break;
    coinPick.add(c.txid + ':' + c.vout); have += c.value; n++;
  }
  if (have < f.amount) { coinPick.clear(); return toast('These coins do not cover that amount'); }
  renderCoins(); sendPreview();
  toast(`${n} coin${n === 1 ? '' : 's'} ticked \u00b7 ${COIN_PLANS[plan].why}`, 5000);
}

function coinSum() { const n = coinPick.size; const tot = ((vault && vault._coins) || []).filter(c => coinPick.has(c.txid + ':' + c.vout)).reduce((a, c) => a + c.value, 0); $('#coin-sum').textContent = n ? `${n} coin${n === 1 ? '' : 's'} ticked · ${sats(tot)} sats` : 'none ticked: the app picks'; }
$('#coin-list').addEventListener('change', (e) => { const i = e.target.closest('[data-op]'); if (!i) return; if (i.checked) coinPick.add(i.dataset.op); else coinPick.delete(i.dataset.op); coinSum(); sendPreview(); });
$('#coins-all').onclick = () => { ((vault && vault._coins) || []).forEach(c => coinPick.add(c.txid + ':' + c.vout)); renderCoins(); sendPreview(); };
$('#coins-none').onclick = () => { coinPick.clear(); sendFrom = false; paintSendFrom(); renderCoins(); sendPreview(); };
$('#coin-plans') && $('#coin-plans').addEventListener('click', (e) => { const b = e.target.closest('[data-plan]'); if (b) planPick(b.dataset.plan); });
$('#consolidate').onclick = () => {
  if (!coinPick.size) ((vault && vault._coins) || []).forEach(c => coinPick.add(c.txid + ':' + c.vout));
  renderCoins();
  if (coinPick.size < 2) return toast('Tick at least two coins to consolidate');
  $('#send-to').value = vault._receive || vault.address; $('#send-all').checked = true; $('#send-amount').disabled = true;
  toast('The ticked coins will be joined into one at the vault\'s own address. Build to see the fee.', 6000);
};

// ---------- addresses ----------
let addrAhead = 0;
async function renderAddresses() {
  const card = $('#addr-card'); $('#addr-fixed').hidden = true;
  // a vault with one fixed address has no list to show, but the screen must still say so rather than sit empty
  if (!vault || !vault._rotating) { card.hidden = true; $('#addr-fixed').hidden = !vault; return; }
  try {
    const all = await invoke('vault_addresses', { name: vault.name, ahead: addrAhead });
    renderAddresses.rows = all;
    const coins = vault._coins || [], hist = vault._history || [];
    // a wallet watched for several kinds, or one with silent payments, lists one kind at a time — the one with
    // history first, since only the wallet that holds the keys knows which kind it hands out
    const kinds = [...new Set(all.map(r => r.kind))].filter(Boolean);
    let seg = card.querySelector('#addr-kind-seg');
    if (kinds.length > 1) {
      const used = new Set(hist.map(h => (h.index >>> 28) & 7).map(s => s === 7 ? 'sp' : (vault.watch ? vault.watch.kinds[s] : null)).filter(Boolean));
      const pick = renderAddresses.kind && kinds.includes(renderAddresses.kind) ? renderAddresses.kind : (kinds.find(k => used.has(k) && k !== 'sp') || (kinds.includes('wpkh') ? 'wpkh' : kinds[0]));
      renderAddresses.kind = pick;
      if (!seg) { seg = document.createElement('div'); seg.id = 'addr-kind-seg'; seg.className = 'seg'; seg.setAttribute('role', 'radiogroup'); seg.setAttribute('aria-label', 'Kind of address'); seg.style.marginBottom = '12px'; card.prepend(seg); }
      seg.innerHTML = ''; seg.hidden = false;
      for (const k of kinds) { const b = document.createElement('button'); b.type = 'button'; b.setAttribute('role', 'radio'); b.setAttribute('aria-checked', k === pick ? 'true' : 'false'); b.textContent = (KIND_WORDS[k] || k) + (used.has(k) ? '' : ' \u00b7 unused'); b.onclick = () => { renderAddresses.kind = k; renderAddresses(); }; seg.appendChild(b); }
    } else { renderAddresses.kind = null; if (seg) seg.hidden = true; }
    const rows = renderAddresses.kind ? all.filter(r => r.kind === renderAddresses.kind) : all;
    listControls($('#addr-ctl'), 'addr', [
      { key: 'sort', label: 'Sort', def: 'new', options: [['new', 'Newest'], ['old', 'Oldest'], ['bal', 'Balance'], ['coins', 'Coins']] },
      { key: 'show', label: 'Show', def: 'all', options: [['all', 'All'], ['funded', 'Holding coins'], ['used', 'Used'], ['unused', 'Unused']] }], renderAddresses);
    const sort = listPref('addr', 'sort', 'new'), show = listPref('addr', 'show', 'all');
    const balOf = (r) => coins.filter(c => c.index === r.index).reduce((a, c) => a + c.value, 0), nOf = (r) => coins.filter(c => c.index === r.index).length, usedOf = (r) => hist.some(x => x.index === r.index);
    const ordered = (ch) => { const xs = rows.filter(r => r.chain === ch).filter(r => show === 'all' || (show === 'funded' && nOf(r) > 0) || (show === 'used' && usedOf(r)) || (show === 'unused' && !usedOf(r)));
      return sort === 'old' ? xs : sort === 'bal' ? xs.slice().sort((a, b) => balOf(b) - balOf(a) || b.position - a.position) : sort === 'coins' ? xs.slice().sort((a, b) => nOf(b) - nOf(a) || b.position - a.position) : xs.slice().reverse(); };
    // the order ‹ › walks on an address's page: as listed — receive, then change
    renderAddresses.order = ['receive', 'change'].flatMap(ordered);
    for (const [chain, sel] of [['receive', '#addr-recv tbody'], ['change', '#addr-chg tbody']]) {
      const tb = $(sel); tb.innerHTML = '';
      const list = ordered(chain); if (!list.length) tb.innerHTML = `<tr><td class="muted" colspan="5">${show === 'all' ? 'none yet' : 'none of that kind'}</td></tr>`;
      for (const r of list) {
        const here = coins.filter(c => c.index === r.index), bal = here.reduce((a, c) => a + c.value, 0);
        const used = hist.some(x => x.index === r.index);
        const tr = document.createElement('tr'); tr.className = 'rowpick' + (r.current ? ' cur' : '') + (r.future ? ' future' : '');
        tr.innerHTML = `<td class="muted">${r.position}</td><td class="mono" title="${r.address}">${r.address}</td><td>${lblHtml('addr:' + r.address)}</td><td class="r" style="font-variant-numeric:tabular-nums">${bal ? sats(bal) + ' sats<small class="muted" style="display:block;font-size:11.5px">' + sats(here.length) + ' coin' + (here.length === 1 ? '' : 's') + '</small>' : '<span class="muted">—</span>'}</td><td class="r ${r.current ? 'ok' : 'muted'}">${r.current ? (chain === 'change' ? 'next change' : 'next deposit') : r.future ? 'unused · ahead' : used ? (bal ? 'in use' : 'spent') : 'unused'}</td>`;
        tr.addEventListener('click', (e) => { if (e.target.closest('.lbl')) return; openAddress(r); });
        tb.appendChild(tr);
      }
    }
    $('#addr-gap').textContent = `Gap limit ${vault._gap || 20}: showing ${addrAhead} ahead of the next unused on each chain.`;
    card.hidden = false;
  } catch (e) { card.hidden = true; }
}
$('#addr-more').onclick = () => { addrAhead = Math.min(addrAhead + 10, 200); renderAddresses(); };

// ---------- messages ----------
let lastMessageOut = null;
function outRow(label, val, mono = true) { return `<div class="kv"><span class="muted">${label}</span><span class="${mono ? 'mono' : ''}">${val}</span><button class="btn small" data-copy="${encodeURIComponent(val)}">Copy</button></div>`; }
document.addEventListener('click', (e) => { const b = e.target.closest('[data-copy]'); if (b) navigator.clipboard.writeText(decodeURIComponent(b.dataset.copy)).then(() => toast('Copied')); });
async function renderMessages() {
  if (!vault) return;
  const c = cards.find(c => c.pubkey);
  $('#msg-key-sign').disabled = !c; $('#msg-key-who').textContent = c ? `${cardLabel(c)}${vault.keys.some(k => k.pubkey === c.pubkey) ? ' · ' + vault.keys.find(k => k.pubkey === c.pubkey).name : ''}` : 'plug in a key';
  $('#msg-key-warn').hidden = !c;
  $('#msg-vault-note').textContent = `Proves the vault controls one of its addresses (BIP 322). It takes ${vault.threshold} of the vault's ${vault.keys.length} keys, one after another on the Sign screen, and nothing is broadcast. Verifiable here and by tools that understand BIP 322 script proofs.`;
  try { const rows = await invoke('vault_addresses', { name: vault.name, ahead: 0 }); const sel = $('#msg-vault-addr'); sel.innerHTML = rows.filter(r => !r.future).slice().reverse().map(r => `<option value="${r.index}">${vault._rotating ? fmtIndex(r.index) + ' · ' : ''}${r.address}${r.current && vault._rotating ? ' (next)' : ''}</option>`).join(''); } catch (e) { fail(e); }
  if (lastMessageOut) { const r = lastMessageOut; $('#msg-vault-out').hidden = false; $('#msg-vault-out').innerHTML = outRow('Address', r.address) + outRow('Message', r.message.replace(/</g, '&lt;'), false) + outRow('Signature', r.signature); }
}
$('#msg-key-sign').onclick = async () => {
  const c = cards.find(c => c.pubkey); const text = $('#msg-key-text').value; const pin = $('#msg-key-pin').value;
  if (!c) return toast('Plug in a key'); if (!text.trim()) return toast('Write the message first'); if (!pin) return toast('Enter the card PIN');
  $('#msg-key-sign').disabled = true; $('#msg-key-who').textContent = 'TOUCH THE KEY NOW · it is blinking';
  try {
    const r = await invoke('message_sign_key', { reader: c.reader, pin, message: text }); $('#msg-key-pin').value = '';
    $('#msg-key-out').hidden = false;
    $('#msg-key-out').innerHTML = outRow('Key', r.pubkey) + outRow('Legacy address', r.p2pkh) + outRow('Segwit address', r.p2wpkh) + outRow('Signature', r.legacy) + outRow('Segwit form', r.segwit) + `<div class="muted">Verify against the legacy address with the first signature in any wallet (Sparrow, Electrum, Bitcoin Core). Some wallets verify the segwit address with the second form.</div>`;
    toast('Signed');
  } catch (e) { fail(e); }
  renderMessages();
};
$('#msg-vault-stage').onclick = async () => {
  const text = $('#msg-vault-text').value; const index = +$('#msg-vault-addr').value;
  if (!text.trim()) return toast('Write the message first');
  try { await invoke('message_stage', { name: vault.name, index, message: text }); built = null; lastMessageOut = null; $('#msg-vault-out').hidden = true; show('sign'); } catch (e) { fail(e); }
};
$('#ver-btn').onclick = async () => {
  const out = $('#ver-out');
  try { const r = await invoke('message_verify', { address: $('#ver-addr').value, message: $('#ver-text').value, signature: $('#ver-sig').value }); out.innerHTML = r.ok ? `<span class="ok">✓ valid</span> · ${r.detail}` : `<span class="warn">✗ not valid</span> · ${r.detail}`; }
  catch (e) { out.innerHTML = `<span class="warn">${String(e)}</span>`; }
};

$('#gap-save').onclick = async () => {
  const gap = +($('#gap-in').value.replace(/[^0-9]/g, '') || 0);
  try { store = await invoke('settings_set', { network: netName(), custom: store.settings.custom, electrum: store.settings.electrum, explorer: store.settings.explorer, gap }); renderSettings(); toast(`Gap limit ${gap}`); if (vault) vault._coins = undefined; } catch (e) { fail(e); }
};
$('#notify-test').onclick = async () => { notice({ vault: vault ? vault.name : '', kind: 'in', title: (vault ? vault.name : 'vault') + ': 9,171 sats arrived', body: 'a sample · this is how the window itself tells you' }); try { $('#notify-out').textContent = await invoke('notify_test'); } catch (e) { $('#notify-out').textContent = String(e); } };
// Labels backup: the status line under Activity says whether this vault's labels are on the label host (encrypted,
// recoverable with any k keys) and why not otherwise. Cheap: one signed GET when nothing changed.
async function labelsSync() {
  if (!vault) return;
  const me = vault.name;
  try {
    const st = await invoke('labels_sync', { name: me });
    if (!vault || vault.name !== me) return;
    store = await invoke('store_get'); vault = store.vaults.find(x => x.name === me) || vault;
    const host = st.host.replace(/^https?:\/\//, '').replace(/\/vss$/, '');
    $('#labels-out').textContent = st.backed_up
      ? `${st.count} label${st.count === 1 ? '' : 's'} · backed up encrypted at ${host} · any ${vault.threshold} keys bring them back · in step ${new Date(st.synced * 1000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}`
      : (st.note || 'Labels are this vault’s own notes on its addresses, coins and transactions (BIP 329, as Sparrow uses).');
    if (st.backed_up) renderActivityLabels();
  } catch (e) { $('#labels-out').textContent = 'Label backup: ' + String(e); }
}
function renderActivityLabels() { $$('#activity .lbl, #act-all .lbl, #coin-rows .lbl, #addr-recv .lbl, #addr-chg .lbl').forEach(box => { const key = box.dataset.lkey; const t = box.querySelector('.text'); if (t && t.textContent !== labelOf(key)) t.textContent = labelOf(key); }); }
$('#labels-sync').onclick = () => { $('#labels-out').textContent = 'syncing labels…'; labelsSync(); };
$('#labels-export').onclick = async () => { try { $('#labels-out').textContent = 'Saved ' + await invoke('labels_export', { name: vault.name }); } catch (e) { if (!/cancelled/.test(String(e))) $('#labels-out').textContent = String(e); } };
$('#labels-import').onclick = async () => { try { const n = await invoke('labels_import', { name: vault.name }); labelsSync(); store = await invoke('store_get'); vault = store.vaults.find(x => x.name === vault.name) || vault; $('#labels-out').textContent = `${n} label${n === 1 ? '' : 's'} imported into ${vault.name}.`; } catch (e) { if (!/cancelled/.test(String(e))) $('#labels-out').textContent = String(e); } };
$('#bsms-export').onclick = async () => { try { toast('Saved ' + await invoke('bsms_export', { name: vault.name }), 6000); } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
$('#bsms-open').onclick = async () => { try { const r = await invoke('bsms_import'); $('#desc-in').value = r.descriptor; readDescriptor(r.descriptor); if (r.first_address) toast('BSMS read. First address in the file: ' + r.first_address.slice(0, 12) + '…', 6000); } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };

// ---------- keys ----------
// The recovery line says what the vault holds. Until Reveal is pressed the page carries a scrambled stand-in of the same
// shape, blurred — so a screenshot, a screen share or a screen reader gets nothing. Copy and Save as PDF work without revealing.
const standIn = (line) => { let x = line.length * 2654435761 >>> 0; return line.replace(/[a-z0-9]/gi, () => { x = (x * 1664525 + 1013904223) >>> 0; return '0123456789abcdef'[x >>> 28]; }); };
// the cards in a vault of mixed signers: fixed keys, kept bare, made on the card and never copied off it
const fixedCards = (v) => (v && v.ext ? v.keys.filter(k => /^0[23][0-9a-f]{64}$/i.test(k.pubkey)) : []);
function paintDescWords() {
  const ext = !!(vault && vault.ext);
  $('#keys-desc-cards').hidden = ext; $('#keys-desc-ext').hidden = !ext;
  const fc = fixedCards(vault);
  if (ext && fc.length) $('#keys-desc-ext').textContent = `This is the wallet descriptor: complete and standard. Bitcoin Core reads it whole; whether Sparrow accepts a card\u2019s plain key beside the others is not yet tested. With any ${vault.threshold} of the keys \u2014 a device\u2019s seed words, or a card itself \u2014 it rebuilds this vault anywhere; on its own it can only watch.`;
  else if (ext) $('#keys-desc-ext').textContent = `This is the wallet descriptor: complete and standard \u2014 Sparrow, Bitcoin Core and the signing devices read it whole. With any ${vault.threshold} of the keys\u2019 seed words it rebuilds this vault anywhere; on its own it can only watch. Keep it apart from the seed words: together they are the vault.`;
}
function paintDesc() {
  paintDescWords();
  const line = (vault && vault._recovery) || '', shown = !!hideDesc.shown && !!line;   // an empty line is never 'revealed'
  $('#keys-desc').textContent = shown ? line : standIn(line); $('#keys-desc').setAttribute('aria-hidden', String(!shown));
  $('#desc-wrap').dataset.hidden = shown ? '0' : '1'; $('#desc-hide').hidden = !shown; $('#desc-copy').hidden = !shown;
}
function hideDesc() { clearTimeout(hideDesc.t); hideDesc.shown = false; paintDesc(); }
// The vault object is replaced whenever the store is re-read (after a seal, a rename, a label), and the fresh one has not
// fetched its line yet: Reveal used to show an empty box then. It fetches when it has to, and never reveals nothing.
$('#desc-reveal').onclick = async () => {
  const name = vault.name;
  if (!vault._recovery) { try { const l = await invoke('vault_recovery', { name }); if (!vault || vault.name !== name) return; vault._recovery = l; } catch (e) { return fail(e); } }
  if (!vault._recovery) return toast('The recovery line could not be read');
  hideDesc.shown = true; paintDesc(); clearTimeout(hideDesc.t); hideDesc.t = setTimeout(hideDesc, 120000);   // two minutes, then it goes away again
};
$('#desc-hide').onclick = hideDesc;
$('#desc-copy').onclick = async () => { try { if (!vault._recovery) vault._recovery = await invoke('vault_recovery', { name: vault.name }); await navigator.clipboard.writeText(vault._recovery); const b = $('#desc-copy'); b.classList.add('done'); b.title = 'Copied'; toast('Recovery line copied'); setTimeout(() => { b.classList.remove('done'); b.title = 'Copy the recovery line'; }, 1800); } catch (e) { fail(e); } };
window.addEventListener('blur', hideDesc);   // switching to another app is when a screen gets shared

// Keys is ONE list. Each row is one key and says everything about it: what it is, whether it is plugged in, whether it
// carries its share — and offers to seal it right there. (It used to be three cards listing the same three keys.)
// ---------- what a key can honestly be said to know about itself ----------
// A YubiKey carries a serial, a firmware version, a form factor and its OpenPGP version. It does NOT carry a
// manufacture date, an end-of-life date or a warranty date. Yubico declares end of life per PRODUCT LINE and sets
// no dates in advance — six months' notice before a product stops being sold, support for a year after — and
// warranty runs from a purchase this app knows nothing about. So no end-of-life or warranty date is shown, because
// there is none to show and a guessed one would be worse than silence.
//
// What CAN be done is check the firmware the key does report against advisories known when this build was made —
// here, on this computer. Asking anyone "is serial 12345678 still supported?" would hand over a list of exactly
// which keys guard this vault, which is the one thing this app must never send.
const ADVISORIES_AS_OF = '21 September 2026';
const verLt = (a, b) => { const p = (v) => String(v || '').split('.').map(n => parseInt(n, 10) || 0); const [x, y] = [p(a), p(b)];
  for (let i = 0; i < 3; i++) { if ((x[i] || 0) !== (y[i] || 0)) return (x[i] || 0) < (y[i] || 0); } return false; };
const ADVISORIES = [{
  id: 'YSA-2024-03',
  title: 'Infineon ECDSA private key recovery (\u201cEUCLEAK\u201d)',
  // a YubiKey 5 Series advisory: only a 5.x firmware can be hit by it (a Nitrokey 3's 1.9.1 is not "before 5.7.0")
  hits: (card) => !!card.firmware && /^5\./.test(card.firmware) && verLt(card.firmware, '5.7.0'),
  says: 'Yubico\u2019s advisory covers YubiKey 5 Series firmware before 5.7.0; 5.7.0 and newer are not affected. Someone who had the key in their hands for a time, with laboratory equipment, could recover the secret it holds. Yubico does not allow firmware to be updated on a key that already exists, so this one cannot be moved to 5.7 \u2014 only replaced. Their own advice is to keep physical control of it.',
  here: 'In a vault like this one that matters less on its own: an attacker would need this key AND enough of the others to sign. It is still a reason to keep it somewhere only you reach, and a reason to prefer 5.7.0 or newer for a key you buy next.',
}];
const keyAdvisories = (card) => card ? ADVISORIES.filter(a => a.hits(card)) : [];
const firstSeen = (pubkey) => ((store && store.key_first) || {})[pubkey];
function firstSeenWords(pubkey) {
  const t = firstSeen(pubkey); if (!t) return '';
  const days = Math.floor((Date.now() / 1000 - t) / 86400);
  const when = new Date(t * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'long', year: 'numeric' });
  return `known to this computer since ${when}${days > 60 ? ` \u00b7 ${sats(days)} days` : ''}`;
}
const keysBusy = () => $$('#keys-list input').some(i => i.value || i === document.activeElement);   // a poll must not wipe a half-typed PIN or name
const PENCIL = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/></svg>';
function extKeyRow(k, i) {
  // a card in a mixed vault is a fixed key, kept bare: shown as the card it is, plugged in or not
  if (/^0[23][0-9a-f]{64}$/i.test(k.pubkey)) {
    const card = cards.find(c => c.pubkey === k.pubkey), d = deviceOf(k.device && k.device !== 'card' ? k.device : 'yubikey');
    return `<div class="keyrow vk" data-key-open="${esc(k.pubkey)}" title="Open this key\u2019s card"><span class="devart">${d.svg}</span><div class="who"><div class="name"><span class="name-text">${esc(k.name)}</span><button class="pencil" data-edit-idx="${i}" title="Rename this key">${PENCIL}</button></div><div class="muted" style="font-size:13px">${esc(d.name)} · a fixed key in this vault · <span class="mono">${esc(k.pubkey.slice(0, 12))}…</span></div></div>`
      + `<div class="kright"><div class="state ${card ? 'here' : ''}"><i></i>${card ? 'plugged in' : 'not plugged in'}</div></div></div>`;
  }
  const m = /^\[([0-9a-f]{8})\/([^\]]+)\](\S+)$/i.exec(k.pubkey) || [];
  const art = k.device ? `<span class="devart">${deviceOf(k.device).svg}</span>` : '<span class="keyart"><svg width="24" height="24"><use href="#ring"/></svg></span>';
  return `<div class="keyrow vk" data-key-open="${esc(k.pubkey)}" title="Open this key\u2019s card">${art}<div class="who"><div class="name"><span class="name-text">${esc(k.name)}</span><button class="pencil" data-edit-idx="${i}" title="Rename this key">${PENCIL}</button></div><div class="muted" style="font-size:13px">${k.device ? esc(deviceOf(k.device).name) + ' \u00b7 ' : ''}fingerprint <span class="mono">${esc(m[1] || '?')}</span> \u00b7 m/${esc(m[2] || '?')} \u00b7 <span class="mono">${esc((m[3] || '').slice(0, 14))}\u2026</span></div></div>`
    + `<div class="kright"><div class="state"><i></i>signs on another device</div></div></div>`;
}
function vaultKeyRow(k, i) {
  if (vault && vault.ext) return extKeyRow(k, i);
  const card = cards.find(c => c.pubkey === k.pubkey), pf = packedForm(k.pubkey);
  const art = card ? keyArt(card.form || 3, !!card.nfc, 52) : pf ? keyArt(pf, !!(pf & 0x80), 52) : '<svg width="24" height="24"><use href="#ring"/></svg>';
  const model = card ? modelName(card.form, card.nfc) : pf ? modelName(pf, !!(pf & 0x80), !!(pf & 0x40)) : 'YubiKey';
  const fw = card && card.firmware ? card.firmware : ((store && store.key_fw) || {})[k.pubkey] || '';
  const facts = [model, k.serial ? 'serial ' + k.serial : 'serial not seen on this computer yet']
    .concat(fw ? ['firmware ' + fw + (card ? '' : ' (as it said when last plugged in)')] : [])
    .concat(card ? ['touch ' + String(card.touch).toLowerCase()] : []);
  const seen = firstSeenWords(k.pubkey); if (seen) facts.push(seen);
  const adv = fw ? keyAdvisories({ firmware: fw }) : [];
  return `<div class="keyrow vk" data-key-open="${esc(k.pubkey)}" title="Open this key\u2019s card"><span class="keyart">${art}</span><div class="who"><div class="name"><span class="name-text">${esc(k.name)}</span><button class="pencil" data-edit-idx="${i}" title="Rename this key">${PENCIL}</button></div><div class="muted" style="font-size:13px">${esc(facts.join(' · '))} · <span class="mono">${k.pubkey.slice(0, 12)}…</span></div>${adv.map(a => `<div class="warn advline" style="font-size:13px;margin-top:6px"><b>${esc(a.id)}: ${esc(a.title)}.</b> ${esc(a.says)} ${esc(a.here)}</div>`).join('')}</div>`
    + `<div class="kright"><div class="state ${card ? 'here' : ''}"><i></i>${card ? 'plugged in' : 'not plugged in'}</div><div class="kseal" data-seal-slot="${i}"></div></div></div>`;
}
// ---------- a key's card: everything kept about one key, behind a proof of presence ----------
let keyCardOpenUntil = 0;
function closeKeyCard(repaint = true) {
  if (recPage.kind !== 'key') return;
  const d = $('#key-detail'); if (d.contains($('#tx-notes'))) recHome.appendChild($('#tx-notes'));
  d.hidden = true; d.innerHTML = ''; $('#keys-main').hidden = false; recPage.kind = null; recPage.id = null; recPage.leaf = '';
  const from = keyFrom; keyFrom = null;
  if (repaint && from === 'allkeys') { show('allkeys'); return; }
  if (repaint) paintCrumbs('keys');
}
// the vault whose record holds what is kept with a key that signs for several: the first that has it, always the same one
const keyHome = (pub) => store.vaults.find(v => v.keys.some(k => k.pubkey === pub)) || null;
function openKeyCard(pub) {
  if (!vault) return; const i = vault.keys.findIndex(k => k.pubkey === pub); if (i < 0) return;
  const k = vault.keys[i];
  recPage.kind = 'key'; recPage.id = pub; recPage.leaf = k.name;
  $('#keys-main').hidden = true; const d = $('#key-detail'); d.hidden = false;
  // a card's key, or a device's (a SeedSigner, a Coldcard) named by fingerprint and path
  const bare = /^0[23][0-9a-f]{64}$/i.test(pub), ext = bare ? [] : (/^\[([0-9a-f]{8})\/([^\]]+)\](\S+)$/i.exec(pub) || []);
  const card = bare ? cards.find(c => c.pubkey === pub) : null, pf = bare ? packedForm(pub) : 0;
  const dev = k.device && k.device !== 'card' && k.device !== 'yubikey' ? deviceOf(k.device) : null;
  const art = dev ? `<span class="devart" style="transform:scale(2.2);display:inline-block">${dev.svg}</span>` : card ? keyArt(card.form || 3, !!card.nfc, 132) : pf ? keyArt(pf, !!(pf & 0x80), 132) : '<svg width="64" height="64"><use href="#ring"/></svg>';
  const model = dev ? dev.name : card ? modelName(card.form, card.nfc) : pf ? modelName(pf, !!(pf & 0x80), !!(pf & 0x40)) : 'YubiKey';
  const fw = card && card.firmware ? card.firmware : ((store && store.key_fw) || {})[pub] || '';
  const elsewhere = store.vaults.filter(v => v.name !== vault.name && v.keys.some(x => x.pubkey === pub)).map(v => v.name);
  const seen = bare ? firstSeenWords(pub) : '';
  const sub = bare ? [model, k.serial ? 'serial ' + k.serial : '', fw ? 'firmware ' + fw : '', vault.ext ? 'a fixed key in this vault' : ''].filter(Boolean).join(' · ') : `${model} · signs on its own device, from its seed words`;
  const facts = [
    bare ? ['Public key', `<span class="mono" style="font-size:12px;word-break:break-all;-webkit-user-select:text;user-select:text">${esc(pub)}</span>`] : ['Fingerprint', `<span class="mono">${esc(ext[1] || '?')}</span> · m/${esc(ext[2] || '?')}`],
    !bare ? ['Extended key', `<span class="mono" style="font-size:12px;word-break:break-all;-webkit-user-select:text;user-select:text">${esc(ext[3] || pub)}</span>`] : null,
    ['Now', bare ? (card ? `<span class="ok">✓ plugged in</span>${card.touch ? ' · touch ' + esc(String(card.touch).toLowerCase()) : ''}` : 'not plugged in') : 'on its own device'],
    ['Signs for', [vault.name].concat(elsewhere).map(esc).join(' · ') + (elsewhere.length ? `<small class="muted"> · ${sats(elsewhere.length + 1)} vaults share this key</small>` : '')],
    seen ? ['Seen', esc(seen)] : null].filter(Boolean);
  d.innerHTML = `<div class="txnav"><button class="btn small" type="button" id="key-back">‹ ${keyFrom === 'allkeys' ? 'Every key' : 'This vault\u2019s keys'}</button><span class="muted">${esc(k.name)}</span><span></span></div>
    <div class="guide-row">
      <div class="guide" style="display:flex;align-items:center;justify-content:center;min-height:196px;line-height:0">${art}</div>
      <div class="guide-body">
        <div class="row" style="gap:10px;align-items:center">${iconHtml('key', pub, 40, 'keys', true)}<div class="cer-act-title">${esc(k.name)}</div></div>
        <div class="muted">${esc(sub)}</div>
        <dl class="tx-dl" style="margin-top:6px">${facts.map(([a, b]) => `<dt>${a}</dt><dd>${b}</dd>`).join('')}</dl>
      </div>
    </div>
    <div id="key-private"></div>`;
  $('#key-back').onclick = () => closeKeyCard();
  paintIcons(d);
  paintKeyPrivate();
  paintCrumbs('keys');
  // ---- the key's signing part can be wiped: a factory reset of its OpenPGP applet, under its admin PIN ----
  if (bare) {
    const here = cards.find(c => c.pubkey === pub);
    const uses = store.vaults.filter(v => v.keys.some(x => x.pubkey === pub));
    const amount = (v) => v === vault && vault._balance != null ? `${sats(vault._balance)} sats` : 'balance not read on this computer yet';
    d.insertAdjacentHTML('beforeend', `<div class="card" style="margin-top:18px;border-color:var(--no)"><div class="label">Reset this key</div>
      <div class="muted" style="font-size:13px">Wipes the signing part of the key (its OpenPGP applet): the signing key, both PINs, any seal share and this computer\u2019s unlock slot on it. The key comes back factory-fresh and can be set up again \u2014 as a <b>different</b> key. Nothing else on it (passkeys, authenticator codes) is touched.</div>
      <div class="warn" style="font-size:13px"><b>This key signs for ${uses.length === 1 ? 'one vault' : sats(uses.length) + ' vaults'}, and each loses this cosigner for good:</b><ul style="margin:6px 0 0 18px;padding:0">${uses.map(v => `<li>${esc(v.name)} \u2014 ${v.network === 'mainnet' ? 'mainnet' : esc(v.network)} \u00b7 any ${v.threshold} of ${v.keys.length} keys \u00b7 ${amount(v)}</li>`).join('')}</ul>${uses.some(v => v.keys.length === 1) ? '<div style="margin-top:6px"><b>A vault of one key is lost outright.</b></div>' : '<div style="margin-top:6px">A vault of several keys still spends with the others, but has one fewer to lose; move its coins to a new vault with a replacement key.</div>'}${!here ? '<div style="margin-top:6px">Plug the key in to go on.</div>' : ''}</div>
      ${here ? `<div class="grid2"><div class="field"><div class="label">Its admin PIN</div><input type="password" id="key-reset-pin" autocomplete="off" style="max-width:260px"></div></div>
      <div class="row"><button class="btn" type="button" id="key-reset" style="border-color:var(--no);color:var(--no)">Reset this key\u2019s signing part\u2026</button></div><div class="muted" id="key-reset-note" aria-live="polite"></div>` : ''}</div>`);
    if (here) askTwice($('#key-reset'), `Wipe the signing part of “${k.name}” (serial ${here.serial})? ${uses.length ? uses.map(v => v.name).join(', ') + (uses.length === 1 ? ' loses' : ' lose') + ' this cosigner for good.' : ''} The key cannot be brought back.`, 'Yes, reset it', async () => {
      const pin = $('#key-reset-pin').value; $('#key-reset-pin').value = ''; if (!pin) return toast('Enter the admin PIN');
      $('#key-reset').disabled = true; $('#key-reset-note').textContent = 'Resetting\u2026';
      try { store = await invoke('card_reset', { reader: here.reader, adminPin: pin, serial: here.serial }); if (vault) vault = store.vaults.find(v => v.name === vault.name) || vault; toast(`${k.name} is factory-fresh again. Its vaults keep the record of the key it was; a new key can be set up on it.`, 9000); closeKeyCard(); renderKeys(); }
      catch (e) { $('#key-reset-note').textContent = String(e); $('#key-reset').disabled = false; }
    });
  }
}
// The map to the custody — where the key is kept, photos, who knows — is drawn only while a proof of presence
// holds: Touch ID, or one of this vault's keys signing a fresh challenge. The proof is the window's for ten minutes.
function paintKeyPrivate() {
  const box = $('#key-private'); if (!box || recPage.kind !== 'key') return;
  const pub = recPage.id;
  if (Date.now() < keyCardOpenUntil) {
    box.innerHTML = '<div class="label" style="margin-top:18px">Where it is kept</div><div id="key-place"></div><div class="label" style="margin-top:18px">Kept with this key</div><div class="muted" style="font-size:13px">Who knows, photos of it and of its sheet, its PIN hint. Sealed on this computer like everything else; shown for ten minutes after you proved you are the one looking.</div><div id="key-rec-slot"></div>';
    paintKeyPlace(pub);
    recPoint('key', 'key:' + pub, $('#key-rec-slot'));
    return;
  }
  const here = cards.filter(c => c.pubkey && vault.keys.some(k => k.pubkey === c.pubkey));
  const nameOfCard = (c) => (vault.keys.find(k => k.pubkey === c.pubkey) || {}).name || 'key';
  box.innerHTML = `<div class="label" style="margin-top:18px">Kept with this key</div>
    <div class="muted" style="font-size:13.5px;max-width:640px">Where it is kept, photos of it and its sheet, notes on who knows — a map to the custody. It is sealed on disk like everything else; putting it on a screen deserves a second look first.</div>
    <div class="row" style="margin-top:12px"><button class="btn primary" type="button" id="key-open-mac">Show it — Touch ID</button></div>
    ${here.length ? `<div class="muted" style="font-size:13px;margin-top:14px">Or with one of this vault’s keys, its PIN and a touch:</div><div style="display:flex;flex-direction:column;gap:8px;margin-top:6px;max-width:640px">${here.map((c, i) => `<form class="lock-key" data-reader="${esc(c.reader)}"><b>${esc(nameOfCard(c))}<small>plugged in · ${esc(modelName(c.form, c.nfc))} ${esc(String(c.serial))}</small></b><input type="password" autocomplete="off" placeholder="Key PIN" aria-label="PIN of ${esc(nameOfCard(c))}"><button class="btn" type="submit">Show it</button></form>`).join('')}</div>` : `<div class="muted" style="font-size:13px;margin-top:10px">Or plug in one of this vault’s keys to open it with a PIN and a touch.</div>`}
    <div class="muted" id="key-open-note" aria-live="polite" style="margin-top:8px"></div>`;
  const opened = (until) => { keyCardOpenUntil = until * 1000; paintKeyPrivate(); setTimeout(() => { if (recPage.kind === 'key' && Date.now() >= keyCardOpenUntil) paintKeyPrivate(); }, Math.max(0, keyCardOpenUntil - Date.now()) + 50); };
  $('#key-open-mac').onclick = async () => { $('#key-open-note').textContent = ''; try { opened(await invoke('key_card_open', { name: vault.name, reader: null, pin: null })); } catch (e) { $('#key-open-note').textContent = String(e); } };
  $$('#key-private form.lock-key').forEach(f => f.onsubmit = async (ev) => {
    ev.preventDefault(); const inp = f.querySelector('input'); let pin = inp.value; inp.value = ''; if (!pin) return toast('Enter the key’s PIN');
    if (/^[cbdefghijklnrtuv]{32,}$/.test(pin)) { pin = ''; return toast('That was the key typing a one-time code, not a PIN. Nothing was sent to the card and no PIN try was used.', 8000); }
    $('#key-open-note').textContent = 'Touch the key when it flashes…';
    try { const req = invoke('key_card_open', { name: vault.name, reader: f.dataset.reader, pin }); pin = ''; opened(await req); } catch (e) { $('#key-open-note').textContent = String(e); }
  });
}
// ---------- where a key is kept: a place, chosen from the ones named or made here ----------
const placeOf = (pub) => (store.places || []).find(p => p.id === (store.key_place || {})[pub]) || null;
const placesSorted = () => (store.places || []).slice().sort((a, b) => a.name.localeCompare(b.name));
// which keys are kept at a place, by vault, with what that means: how many of the vault's keys, against its threshold
function placeHoldings(id) {
  const out = [];
  for (const v of store.vaults) {
    const here = v.keys.filter(k => (store.key_place || {})[k.pubkey] === id);
    if (here.length) out.push({ vault: v, keys: here, alone: v.keys.length > 1 && here.length >= v.threshold });
  }
  return out;
}
// the keys of every vault with no place yet — one entry per public key, since a key may sign for several vaults
function unplacedKeys() {
  const seen = new Map();
  for (const v of store.vaults) for (const k of v.keys) {
    if (!k.pubkey || (store.key_place || {})[k.pubkey]) continue;
    const e = seen.get(k.pubkey) || { key: k, vaults: [] }; e.vaults.push(v.name); seen.set(k.pubkey, e);
  }
  return [...seen.values()];
}
// the map of a place, or the tile that stands where the map would be, so the row keeps its shape either way
function placeTile(p, id) {
  if (p && p.lat != null) return `<div class="ctx-map" id="${id}"></div>`;
  return `<div class="ctx-map nopin"><svg aria-hidden="true"><use href="#i-place"/></svg><b>${p ? 'No pin yet' : 'No place yet'}</b><span>${p ? 'Add an address to see the map' : 'Name where this key lives'}</span></div>`;
}
async function drawPlaceMap(box, p) {
  if (!box) return;
  if (p.lat == null || p.lon == null) { box.hidden = true; return; }
  box.hidden = false; box.innerHTML = '<div class="nomap"><span>fetching the map…</span></div>';
  let pic = null; try { pic = await invoke('tx_map', { lat: p.lat, lon: p.lon, dark: matchMedia('(prefers-color-scheme: dark)').matches }); } catch {}
  if (!box.isConnected) return;
  box.innerHTML = pic ? `<img alt="Map of the area around ${esc(p.place || p.name)}" src="${pic.png}"><span class="pin"></span><small>© OpenStreetMap</small>` : `<div class="nomap"><b>No map just now</b><span>${p.lat.toFixed(3)}, ${p.lon.toFixed(3)}</span></div>`;
}
// the chips: every place named, the current one pressed; New location opens the form in `slot`; `pick(id)` records it
function placeChips(current, slot, pick) {
  const wrap = document.createElement('div'); wrap.className = 'row'; wrap.style.cssText = 'gap:8px;align-items:center;flex-wrap:wrap';
  wrap.innerHTML = `<span class="label" style="margin-right:2px">Kept at</span>${placesSorted().map(x => `<button type="button" class="vchip" data-place-pick="${esc(x.id)}" aria-pressed="${current === x.id ? 'true' : 'false'}">${esc(x.name)}</button>`).join('')}<button type="button" class="btn small" data-place-add>New location…</button>${current ? '<button type="button" class="linkish" data-place-clear style="margin-left:4px">Not recorded</button>' : ''}`;
  $$('[data-place-pick]', wrap).forEach(b => b.onclick = () => pick(b.getAttribute('aria-pressed') === 'true' ? null : b.dataset.placePick));
  wrap.querySelector('[data-place-add]').onclick = () => placeForm(slot, null, (saved) => pick(saved.id));
  const c = wrap.querySelector('[data-place-clear]'); if (c) c.onclick = () => pick(null);
  return wrap;
}
function paintKeyPlace(pub) {
  const box = $('#key-place'); if (!box) return;
  const p = placeOf(pub);
  const others = p ? placeHoldings(p.id).flatMap(x => x.keys.map(k => ({ k, v: x.vault }))).filter(x => x.k.pubkey !== pub) : [];
  box.innerHTML = `<div class="guide-row" style="align-items:stretch">${placeTile(p, 'key-place-map')}<div class="guide-body" id="key-place-body">
      ${p ? `<div class="cer-act-title" style="font-size:18px">${esc(p.name)}</div>${p.place ? `<div class="muted">${esc(p.place)}</div>` : ''}${p.note ? `<div style="font-size:13.5px;white-space:pre-wrap">${esc(p.note)}</div>` : ''}
        <div class="muted" style="font-size:13px">${others.length ? 'Also kept here: ' + others.map(x => `<b>${esc(x.k.name)}</b> · ${esc(x.v.name)}`).join(' · ') : 'Nothing else is recorded here.'}</div>`
      : `<div class="cer-act-title" style="font-size:18px">Where is this key?</div><div class="muted" style="font-size:13.5px">${placesSorted().length ? 'Pick a place, or name a new one. A place can hold keys of several vaults.' : 'Name the first place — the safe, the office, Mum’s. Other keys can be kept there too, and Locations shows what each place holds.'}</div>`}
    </div></div><div id="key-place-form"></div>`;
  const body = $('#key-place-body');
  body.appendChild(placeChips(p ? p.id : null, $('#key-place-form'), (id) => keyPlaceSet(pub, id)));
  if (p) {
    const links = document.createElement('div'); links.className = 'row'; links.style.gap = '14px';
    links.innerHTML = '<button type="button" class="linkish" id="key-place-edit">Edit this place</button><button type="button" class="linkish" id="key-place-open">Open Locations</button>';
    body.appendChild(links);
    $('#key-place-edit').onclick = () => placeForm($('#key-place-form'), p, () => paintKeyPlace(pub));
    $('#key-place-open').onclick = () => show('places');
    drawPlaceMap($('#key-place-map'), p);
  }
}
async function keyPlaceSet(pub, id) { try { store = await invoke('key_place_set', { pubkey: pub, place: id }); if (vault) vault = store.vaults.find(x => x.name === vault.name) || vault; paintKeyPlace(pub); } catch (e) { fail(e); } }
// The form for a place: a name, an address looked up as it is typed (the pin from the match chosen — or, if none was
// chosen, from the best match at save), a note. A narrow column: it is a small thing to fill in.
function placeForm(slot, p, done) {
  const id = p ? p.id : '';
  slot.innerHTML = `<form class="stack tight" id="place-form" style="margin-top:10px;padding:16px;border:1px solid var(--line);border-radius:8px;background:var(--ground);max-width:520px">
      <div class="label">${p ? 'This place' : 'A new place'}</div>
      <div class="field"><div class="label">Name</div><input id="pf-name" maxlength="60" placeholder="the safe, the office, Mum’s" value="${esc(p ? p.name : '')}"></div>
      <div class="field" style="position:relative"><div class="label">Address <span style="font-weight:400;text-transform:none;letter-spacing:0">— optional, for the map</span></div><input id="pf-place" autocomplete="off" spellcheck="false" placeholder="start typing; pick a match to place the pin" value="${esc(p ? p.place : '')}"><ul id="pf-list" class="place-list" role="listbox" hidden></ul><div class="muted" id="pf-pos" style="font-size:12.5px">${p && p.lat != null ? `pin at ${p.lat.toFixed(4)}, ${p.lon.toFixed(4)}` : 'no pin yet'}</div></div>
      <div class="field"><div class="label">Note</div><textarea id="pf-note" rows="2" style="min-height:64px" placeholder="who has access, how it is secured, what else is there">${esc(p ? p.note : '')}</textarea></div>
      <div class="row" style="gap:8px"><button class="btn small primary" type="submit">${p ? 'Save' : 'Add this place'}</button><button class="btn small" type="button" id="pf-cancel">Cancel</button></div>
      <div class="muted" id="pf-note-line" aria-live="polite"></div></form>`;
  let pos = p && p.lat != null ? { lat: p.lat, lon: p.lon } : null, hits = [], timer = null, seq = 0;
  const list = $('#pf-list'), input = $('#pf-place');
  const showHits = (h) => { hits = h; list.innerHTML = h.map((x, i) => `<li role="option" data-i="${i}"><b>${esc(x.label)}</b><small>${esc(x.detail)}</small></li>`).join(''); list.hidden = !h.length; };
  input.addEventListener('input', () => { pos = null; $('#pf-pos').textContent = 'no pin yet — pick a match below, or the closest match is used when you save'; clearTimeout(timer); const q = input.value.trim(), mine = ++seq; if (q.length < 4) return showHits([]);
    timer = setTimeout(async () => { let h = []; try { h = await invoke('place_search', { q }); } catch {} if (mine === seq) showHits(h); }, 350); });
  list.addEventListener('mousedown', (e) => { const li = e.target.closest('li[role=option]'); if (!li) return; e.preventDefault(); const h = hits[+li.dataset.i]; input.value = [h.label, h.detail.replace(/ · /g, ', ')].filter(Boolean).join(', ').slice(0, 160); pos = { lat: h.lat, lon: h.lon }; $('#pf-pos').textContent = `pin at ${h.lat.toFixed(4)}, ${h.lon.toFixed(4)}`; showHits([]); });
  input.addEventListener('blur', () => setTimeout(() => showHits([]), 120));
  $('#pf-cancel').onclick = () => { slot.innerHTML = ''; };
  $('#place-form').onsubmit = async (e) => {
    e.preventDefault(); const name = $('#pf-name').value.trim(); if (!name) return toast('Give the place a name');
    let at = pos; const addr = input.value.trim();
    if (!at && addr) { $('#pf-note-line').textContent = 'Placing the pin…'; try { const h = await invoke('place_search', { q: addr }); if (h.length) at = { lat: h[0].lat, lon: h[0].lon }; } catch {} }
    try {
      const before = new Set((store.places || []).map(x => x.id));
      store = await invoke('place_save', { id, name, note: $('#pf-note').value, place: addr, lat: at ? at.lat : null, lon: at ? at.lon : null });
      if (vault) vault = store.vaults.find(x => x.name === vault.name) || vault;
      const saved = id ? store.places.find(x => x.id === id) : store.places.find(x => !before.has(x.id));
      slot.innerHTML = ''; if (addr && !at) toast('No match for that address, so the place has no pin yet'); if (done) await done(saved);
    } catch (err) { $('#pf-note-line').textContent = String(err); }
  };
  $('#pf-name').focus();
}
// ---------- the proof gate: Keys and Locations together are the map to the custody, so they wait for a proof ----------
function paintProofGate(gate, title, words, onOpened) {
  const here = cards.filter(c => c.pubkey && store.vaults.some(v => v.keys.some(k => k.pubkey === c.pubkey)));
  gate.innerHTML = `<div class="label">${title}</div><div class="muted" style="font-size:13.5px;max-width:640px">${words} It is sealed on disk like everything else; putting it on a screen deserves a second look first.</div>
      <div class="row" style="margin-top:12px"><button class="btn primary" type="button" id="places-open-mac">Show it — Touch ID</button></div>
      ${here.length ? `<div class="muted" style="font-size:13px;margin-top:14px">Or with any key kept here, its PIN and a touch:</div><div style="display:flex;flex-direction:column;gap:8px;margin-top:6px;max-width:640px">${here.map(c => `<form class="lock-key" data-reader="${esc(c.reader)}"><b>${esc((store.vaults.flatMap(v => v.keys).find(k => k.pubkey === c.pubkey) || {}).name || 'key')}<small>plugged in · ${esc(modelName(c.form, c.nfc))} ${esc(String(c.serial))}</small></b><input type="password" autocomplete="off" placeholder="Key PIN"><button class="btn" type="submit">Show it</button></form>`).join('')}</div>` : ''}
      <div class="muted" id="places-open-note" aria-live="polite" style="margin-top:8px"></div>`;
  const opened = (until) => { keyCardOpenUntil = until * 1000; onOpened(); };
  gate.querySelector('#places-open-mac').onclick = async () => { try { opened(await invoke('key_card_open', { name: null, reader: null, pin: null })); } catch (e) { gate.querySelector('#places-open-note').textContent = String(e); } };
  $$('form.lock-key', gate).forEach(f => f.onsubmit = async (ev) => { ev.preventDefault(); const inp = f.querySelector('input'); let pin = inp.value; inp.value = ''; if (!pin) return toast('Enter the key’s PIN'); gate.querySelector('#places-open-note').textContent = 'Touch the key when it flashes…'; try { const req = invoke('key_card_open', { name: null, reader: f.dataset.reader, pin }); pin = ''; opened(await req); } catch (e) { gate.querySelector('#places-open-note').textContent = String(e); } });
}
const proofOpen = () => Date.now() < keyCardOpenUntil;
// ---------- Locations: every place as a row; a place's card is its own screen ----------
function renderLocations() {
  const gate = $('#places-gate'), main = $('#places-main');
  if (!proofOpen()) { main.hidden = true; gate.hidden = false; paintProofGate(gate, 'Where the keys are kept', 'The places, what each holds, and the map — the whole custody on one screen.', renderLocations); return; }
  gate.hidden = true; main.hidden = false;
  const places = placesSorted(), un = unplacedKeys();
  const placed = Object.keys(store.key_place || {}).filter(k => store.vaults.some(v => v.keys.some(x => x.pubkey === k)) && places.some(p => p.id === store.key_place[k])).length;
  const n = (c, w) => `${sats(c)} ${w}${c === 1 ? '' : 's'}`;
  $('#places-sum').innerHTML = places.length ? `${n(places.length, 'place')} · ${n(placed, 'key')} placed${un.length ? ` · <button type="button" class="linkish" data-go="allkeys">${un.length} not yet placed</button> — each key's card says where it is kept` : ''}` : '';
  const list = $('#places-list'); list.innerHTML = '';
  if (!places.length) list.innerHTML = '<div class="sum-empty">No place named yet. Name one here, or from a key’s card.</div>';
  for (const p of places) {
    const h = placeHoldings(p.id), nk = h.reduce((a, x) => a + x.keys.length, 0), row = document.createElement('div'); row.className = 'tp rowpick'; row.dataset.place = p.id; row.setAttribute('role', 'link'); row.tabIndex = 0;
    const holds = h.length ? `${sats(nk)} key${nk === 1 ? '' : 's'} of ${sats(h.length)} vault${h.length === 1 ? '' : 's'}` : 'nothing kept here yet';
    row.innerHTML = `<div class="tp-art">${iconHtml('place', p.id, 40, 'place')}</div><div class="tp-what"><div class="tp-head"><b>${esc(p.name)}</b>${p.place ? ` <span class="muted">· ${esc(p.place)}</span>` : ''}</div><div class="muted" style="font-size:13px">${holds}${h.length ? ': ' + h.map(x => `${esc(x.vault.name)} (${x.keys.map(k => esc(k.name)).join(', ')})`).join(' · ') : ''}</div>${h.some(x => x.alone) ? `<div class="warn" style="font-size:13px">this place alone can spend ${h.filter(x => x.alone).map(x => esc(x.vault.name)).join(' and ')}</div>` : ''}</div><div class="tp-act"><span class="muted" style="font-size:13px">${p.lat != null ? 'on the map' : 'no pin'}</span></div>`;
    row.onclick = () => openPlace(p.id); row.onkeydown = (e) => { if (e.key === 'Enter') openPlace(p.id); };
    list.appendChild(row);
  }
  paintIcons(list);
  $('#place-new').onclick = () => placeForm($('#place-form-slot'), null, (saved) => { if (saved) openPlace(saved.id); else renderLocations(); });
}
function openPlace(id) { placePage = id; show('place'); }
// a place's card: the map or its tile, the name and address edited in place, the keys kept here as a list that
// opens each key's card, the keys with no place yet to be kept here, and the record card everything else has
function renderPlace() {
  const card = $('#place-card'), p = (store.places || []).find(x => x.id === placePage);
  // the record card is one element the whole app shares: it goes home before this card is redrawn over it
  if (card.contains($('#tx-notes'))) recHome.appendChild($('#tx-notes'));
  if (!proofOpen()) { paintProofGate(card, 'This place', 'Where it is, and which keys are kept there.', renderPlace); return; }
  if (!p) { card.innerHTML = '<div class="muted">That place is no longer here.</div><div class="row"><button class="btn small" type="button" data-go="places">All locations</button></div>'; return; }
  const save = async (fields) => { try { store = await invoke('place_save', { id: p.id, name: p.name, note: p.note, place: p.place, lat: p.lat, lon: p.lon, ...fields }); if (vault) vault = store.vaults.find(x => x.name === vault.name) || vault; show('place'); } catch (e) { fail(e); } };   // show, not renderPlace: the title and the crumb carry the name
  // the keys kept here, one row per key, with every vault it signs for that keeps it here
  const here = new Map();
  for (const h of placeHoldings(p.id)) for (const k of h.keys) { const e = here.get(k.pubkey) || { key: k, vaults: [] }; e.vaults.push({ v: h.vault, n: h.keys.length, alone: h.alone }); here.set(k.pubkey, e); }
  const un = unplacedKeys();
  const keyArtOf = (k) => { const pub = k.pubkey, bare = /^0[23][0-9a-f]{64}$/i.test(pub), c = bare ? cards.find(x => x.pubkey === pub) : null, pf = bare ? packedForm(pub) : 0, dev = k.device && k.device !== 'card' && k.device !== 'yubikey' ? deviceOf(k.device) : null;
    return iconOf('key', pub) ? iconHtml('key', pub, 40, 'keys') : dev ? `<span class="devart">${dev.svg}</span>` : c ? keyArt(c.form || 3, !!c.nfc, 40) : pf ? keyArt(pf, !!(pf & 0x80), 40) : '<svg width="24" height="24"><use href="#ring"/></svg>'; };
  const keyRow = (k, sub, act) => `<div class="tp rowpick" data-open-key="${esc(k.pubkey)}" role="link" tabindex="0"><div class="tp-art" style="color:var(--ink)">${keyArtOf(k)}</div><div class="tp-what"><div class="tp-head"><b>${esc(k.name)}</b></div><div class="muted" style="font-size:13px">${sub}</div></div><div class="tp-act">${act}</div></div>`;
  card.innerHTML = `<div class="txnav"><button class="btn small" type="button" data-go="places">‹ All locations</button><span class="muted">${esc(p.name)}</span><span></span></div>
    <div class="guide-row" style="align-items:stretch">${placeTile(p, 'place-map-' + esc(p.id))}<div class="guide-body">
      <div class="row" style="gap:10px;align-items:center;flex-wrap:nowrap">${iconHtml('place', p.id, 40, 'place', true)}<div class="cer-act-title name" id="place-name"><span class="name-text">${esc(p.name)}</span><button class="pencil" id="place-rename" title="Rename this place">${PEN}</button></div></div>
      <div id="place-addr" class="row" style="gap:8px;align-items:center;flex-wrap:nowrap"><span class="${p.place ? '' : 'muted'}" id="place-addr-text">${p.place ? esc(p.place) : 'No address — add one for the map'}</span><button class="pencil" id="place-addr-edit" title="Change the address">${PEN}</button></div>
      <div class="muted" style="font-size:13px">${here.size ? `${sats(here.size)} key${here.size === 1 ? '' : 's'} kept here` : 'Nothing kept here yet'}${p.lat != null ? '' : ' · no pin on the map'}</div>
    </div></div>
    <div class="label" style="margin-top:14px">Kept here</div>
    <div class="tp-list" id="place-keys">${here.size ? [...here.values()].map(e => keyRow(e.key, e.vaults.map(x => `${esc(x.v.name)} <span class="muted">· ${x.v.keys.length > 1 ? `${sats(x.n)} of its ${sats(x.v.keys.length)} keys · ${sats(x.v.threshold)} sign` : 'its only key'}</span>${x.alone ? ' <span class="warn">this place alone can spend it</span>' : ''}`).join('<br>'), '<button type="button" class="linkish" data-unkeep="' + esc(e.key.pubkey) + '">Not kept here</button>')).join('') : '<div class="muted" style="font-size:13px;padding:6px 0">No key is recorded here yet. Keep one here from the list below, or from its own card.</div>'}</div>
    <details class="adv" id="place-unplaced" style="margin-top:10px"${here.size ? '' : ' open'}><summary>Keys with no place yet · ${sats(un.length)}</summary><div class="tp-list" style="margin-top:8px">${un.length ? un.map(u => keyRow(u.key, `Signs for ${u.vaults.map(esc).join(', ')}`, `<button type="button" class="btn small" data-keep="${esc(u.key.pubkey)}">Keep it here</button>`)).join('') : '<div class="muted" style="font-size:13px">Every key has a place recorded.</div>'}</div></details>
    <div class="label" style="margin-top:18px">Kept with this place</div><div class="muted" style="font-size:13px">Who has access, how it is secured, photos of it. Sealed on this computer like everything else.</div><div id="place-rec-slot"></div>
    <div class="row" style="margin-top:14px"><button type="button" class="linkish" id="place-del">Forget this place</button></div>`;
  if (p.lat != null) drawPlaceMap($('#place-map-' + p.id), p); paintIcons(card);
  // rename in place, as a key's row does
  $('#place-rename').onclick = () => { const box = $('#place-name'), inp = document.createElement('input'); inp.className = 'key-input'; inp.value = p.name; inp.maxLength = 60; box.replaceChildren(inp); inp.focus(); inp.select();
    let done = false; const finish = (ok) => { if (done) return; done = true; const v = inp.value.trim(); if (ok && v && v !== p.name) save({ name: v }); else renderPlace(); };
    inp.addEventListener('keydown', (ev) => { if (ev.key === 'Enter') { ev.preventDefault(); finish(true); } if (ev.key === 'Escape') finish(false); }); inp.addEventListener('blur', () => finish(true)); };
  // the address, with the same lookup the form has: a match chosen places the pin; otherwise the closest match at save
  $('#place-addr-edit').onclick = () => {
    const box = $('#place-addr'); box.style.position = 'relative'; box.innerHTML = `<input id="pf-place" autocomplete="off" spellcheck="false" placeholder="start typing; pick a match to place the pin" value="${esc(p.place)}" style="flex:1 1 320px;padding:8px 10px;border:1px solid var(--line);border-radius:6px;background:var(--panel);font:inherit"><ul id="pf-list" class="place-list" role="listbox" hidden></ul><span class="muted" id="pf-pos" style="font-size:12.5px">${p.lat != null ? 'pin kept unless you pick a new match' : 'no pin yet'}</span>`;
    const input = $('#pf-place'), list = $('#pf-list'); let hits = [], timer = null, seq = 0, pos = null, done = false;
    const showHits = (h) => { hits = h; list.innerHTML = h.map((x, i) => `<li role="option" data-i="${i}"><b>${esc(x.label)}</b><small>${esc(x.detail)}</small></li>`).join(''); list.hidden = !h.length; };
    input.addEventListener('input', () => { clearTimeout(timer); const q = input.value.trim(), mine = ++seq; if (q.length < 4) return showHits([]); timer = setTimeout(async () => { let h = []; try { h = await invoke('place_search', { q }); } catch {} if (mine === seq) showHits(h); }, 350); });
    const finish = async (ok) => { if (done) return; done = true; const addr = input.value.trim();
      if (!ok || (addr === p.place && !pos)) return renderPlace();
      let at = pos; if (!at && addr && addr !== p.place) { try { const h = await invoke('place_search', { q: addr }); if (h.length) at = { lat: h[0].lat, lon: h[0].lon }; } catch {} }
      if (!addr) return save({ place: '', lat: null, lon: null });
      save(at ? { place: addr, lat: at.lat, lon: at.lon } : { place: addr }); };
    list.addEventListener('mousedown', (e) => { const li = e.target.closest('li[role=option]'); if (!li) return; e.preventDefault(); const h = hits[+li.dataset.i]; input.value = [h.label, h.detail.replace(/ · /g, ', ')].filter(Boolean).join(', ').slice(0, 160); pos = { lat: h.lat, lon: h.lon }; showHits([]); finish(true); });
    input.addEventListener('keydown', (ev) => { if (ev.key === 'Enter') { ev.preventDefault(); finish(true); } if (ev.key === 'Escape') finish(false); });
    input.addEventListener('blur', () => setTimeout(() => finish(true), 150));
    input.focus(); input.select();
  };
  $$('#place-card [data-open-key]').forEach(r => { const go = () => openKeyFromApp(r.dataset.openKey); r.onclick = (e) => { if (e.target.closest('button')) return; go(); }; r.onkeydown = (ev) => { if (ev.key === 'Enter') go(); }; });
  $$('#place-card [data-keep]').forEach(b => b.onclick = async () => { try { store = await invoke('key_place_set', { pubkey: b.dataset.keep, place: p.id }); renderPlace(); } catch (e) { fail(e); } });
  $$('#place-card [data-unkeep]').forEach(b => b.onclick = async () => { try { store = await invoke('key_place_set', { pubkey: b.dataset.unkeep, place: null }); renderPlace(); } catch (e) { fail(e); } });
  // the note a place was given before it had a record moves into the record, once
  const rec = (store.records || {});
  if (p.note && !((rec.notes || {})['place:' + p.id])) { invoke('note_set', { name: '', key: 'place:' + p.id, note: p.note }).then(st => { store = st; return save({ note: '' }); }).catch(() => {}); }
  recPoint('place', 'place:' + p.id, $('#place-rec-slot'));
  askTwice($('#place-del'), 'Forget this place? The keys kept there stay in their vaults; they just have no place recorded.', 'Yes, forget it', async () => { try { store = await invoke('place_delete', { id: p.id }); show('places'); } catch (e) { fail(e); } });
}
// ---------- Keys: every key across every vault; a row opens the same card the vault's Keys screen opens ----------
function openKeyFromApp(pub) {
  const home = keyHome(pub); if (!home) return;
  keyFrom = 'allkeys'; vault = store.vaults.find(v => v.name === home.name) || home;
  show('keys'); openKeyCard(pub);
}
function renderAllKeys() {
  const gate = $('#allkeys-gate'), main = $('#allkeys-main');
  if (!proofOpen()) { main.hidden = true; gate.hidden = false; paintProofGate(gate, 'Every key', 'Which keys there are, what each signs for, and where each is kept.', renderAllKeys); return; }
  gate.hidden = true; main.hidden = false;
  const seen = new Map();
  for (const v of store.vaults) for (const k of v.keys) { if (!k.pubkey) continue; const e = seen.get(k.pubkey) || { key: k, vaults: [] }; e.vaults.push(v); seen.set(k.pubkey, e); }
  const list = $('#allkeys-list'); list.innerHTML = '';
  if (!seen.size) { list.innerHTML = '<div class="sum-empty">No keys yet: make or restore a vault first.</div>'; return; }
  for (const { key: k, vaults } of [...seen.values()].sort((a, b) => a.key.name.localeCompare(b.key.name))) {
    const pub = k.pubkey, bare = /^0[23][0-9a-f]{64}$/i.test(pub), card = bare ? cards.find(c => c.pubkey === pub) : null, pf = bare ? packedForm(pub) : 0;
    const dev = k.device && k.device !== 'card' && k.device !== 'yubikey' ? deviceOf(k.device) : null;
    const art = dev ? `<span class="devart">${dev.svg}</span>` : card ? keyArt(card.form || 3, !!card.nfc, 40) : pf ? keyArt(pf, !!(pf & 0x80), 40) : '<svg width="24" height="24"><use href="#ring"/></svg>';
    const model = dev ? dev.name : card ? modelName(card.form, card.nfc) : pf ? modelName(pf, !!(pf & 0x80), !!(pf & 0x40)) : bare ? 'YubiKey' : 'signs on its own device';
    const p = placeOf(pub);
    const row = document.createElement('div'); row.className = 'tp rowpick'; row.dataset.key = pub; row.setAttribute('role', 'link'); row.tabIndex = 0;
    row.innerHTML = `<div class="tp-art" style="color:var(--ink)">${iconOf('key', pub) ? iconHtml('key', pub, 40, 'keys') : art}</div><div class="tp-what"><div class="tp-head"><b>${esc(k.name)}</b> <span class="muted">· ${esc(model)}${k.serial ? ' · serial ' + esc(String(k.serial)) : ''}</span></div><div class="muted" style="font-size:13px">Signs for ${vaults.map(v => esc(v.name)).join(', ')}${p ? ` · kept at <b>${esc(p.name)}</b>` : ' · no place recorded'}</div></div><div class="tp-act">${bare ? `<span class="${card ? 'ok' : 'muted'}" style="font-size:13px">${card ? '✓ plugged in' : 'not plugged in'}</span>` : '<span class="muted" style="font-size:13px">own device</span>'}</div>`;
    row.onclick = () => openKeyFromApp(pub); row.onkeydown = (e) => { if (e.key === 'Enter') openKeyFromApp(pub); };
    list.appendChild(row);
  }
  paintIcons(list);
}
// ---------- icons: a mark for an entity, a place or a key — a house glyph, or a sealed picture ----------
const GLYPHS = [['entity', 'People'], ['building', 'Company'], ['shield', 'Trust'], ['bank', 'Bank'], ['briefcase', 'Office'], ['safe', 'Safe'], ['home', 'Home'], ['place', 'Place'], ['keys', 'Key'], ['card', 'Card'], ['star', 'Star']];
const glyphRef = (g) => ['entity', 'home', 'place', 'keys', 'lock', 'coins', 'book'].includes(g) ? '#i-' + g : '#g-' + g;
const iconOf = (kind, id) => (store.icons || {})[kind + ':' + id] || '';
// the mark's html; `fallback` is the glyph drawn when none is set; a picture is filled in by paintIcons
function iconHtml(kind, id, size, fallback, pick) {
  const ic = iconOf(kind, id), img = ic.startsWith('img:') ? ic.slice(4) : '', g = ic.startsWith('glyph:') ? ic.slice(6) : fallback;
  return `<span class="ic s${size}" data-icon="${esc(kind + ':' + id)}"${img ? ` data-icon-img="${esc(img)}"` : ''}${pick ? ' data-icon-pick="1" title="Change the picture" role="button" tabindex="0"' : ''}>${img ? '' : `<svg aria-hidden="true"><use href="${glyphRef(g)}"/></svg>`}</span>`;
}
async function paintIcons(root) {
  for (const el of $$('[data-icon-img]', root)) { const h = el.dataset.iconImg; if (el.querySelector('img')) continue;
    try { if (!thumbCache.has(h)) thumbCache.set(h, await invoke('attach_thumb', { sha256: h })); if (!el.isConnected) continue; const img = new Image(); img.alt = ''; img.src = thumbCache.get(h); el.replaceChildren(img); } catch {} }
}
// the picker, under the mark it changes: the glyphs, a picture, or none
function iconPicker(anchor, kind, id, done) {
  let box = anchor.parentElement.querySelector('.icpick'); if (box) { box.remove(); return; }
  box = document.createElement('div'); box.className = 'icpick';
  const cur = iconOf(kind, id);
  box.innerHTML = GLYPHS.map(([g, w]) => `<span class="ic s40${cur === 'glyph:' + g ? ' on' : ''}" data-glyph="${g}" title="${w}" role="button" tabindex="0"><svg aria-hidden="true"><use href="${glyphRef(g)}"/></svg></span>`).join('') + `<button type="button" class="btn small" data-icon-upload>A picture…</button>${cur ? '<button type="button" class="linkish" data-icon-none>None</button>' : ''}`;
  const set = async (icon) => { try { store = icon === 'pick' ? await invoke('icon_pick', { kind, id }) : await invoke('icon_set', { kind, id, icon }); box.remove(); if (done) done(); } catch (e) { if (String(e) !== 'cancelled') fail(e); } };
  $$('[data-glyph]', box).forEach(b => b.onclick = () => set('glyph:' + b.dataset.glyph));
  box.querySelector('[data-icon-upload]').onclick = () => set('pick');
  const none = box.querySelector('[data-icon-none]'); if (none) none.onclick = () => set(null);
  anchor.insertAdjacentElement('afterend', box);
}
document.addEventListener('click', (e) => { const m = e.target.closest('.ic[data-icon-pick]'); if (!m) return; const [kind, ...rest] = m.dataset.icon.split(':'); iconPicker(m, kind, rest.join(':'), () => { const name = $('.screen.on') && $('.screen.on').id.slice(2); if (name === 'keys' && recPage.kind === 'key') openKeyCard(recPage.id); else if (name) show(name); }); });
// ---------- Entities: who owns what ----------
const ENTITY_KINDS = [['person', 'Person'], ['company', 'Company'], ['trust', 'Trust'], ['smsf', 'Super fund'], ['other', 'Other']];
const kindWord = (k) => (ENTITY_KINDS.find(x => x[0] === k) || ['', 'Other'])[1];
const entGlyph = (e) => e.kind === 'company' ? 'building' : e.kind === 'trust' ? 'shield' : e.kind === 'smsf' ? 'bank' : 'entity';
const vaultsOf = (id) => store.vaults.filter(v => v.entity === id);
const kindOfVault = (v) => v.node ? 'Lightning node' : v.statement ? 'closed wallet' : v.exchange ? 'exchange account' : v.spark ? 'Spark wallet' : v.muun ? 'Muun wallet' : v.watch ? 'watched' : v.keys.length === 1 ? 'one key' : `${v.threshold} of ${v.keys.length}`;
// the chips: every entity, the current one pressed; New entity opens the form in `slot`; `pick(id)` records it
function entityChips(current, slot, pick) {
  const wrap = document.createElement('div'); wrap.className = 'row'; wrap.style.cssText = 'gap:8px;align-items:center;flex-wrap:wrap';
  const ents = (store.entities || []).slice().sort((a, b) => a.name.localeCompare(b.name));
  wrap.innerHTML = `<span class="label" style="margin-right:2px">Owner</span>${ents.map(e => `<button type="button" class="vchip" data-ent-pick="${esc(e.id)}" aria-pressed="${current === e.id ? 'true' : 'false'}">${esc(e.name)}</button>`).join('')}<button type="button" class="btn small" data-ent-add>New entity…</button>${current ? '<button type="button" class="linkish" data-ent-clear style="margin-left:4px">Unassigned</button>' : ''}`;
  $$('[data-ent-pick]', wrap).forEach(b => b.onclick = () => pick(b.getAttribute('aria-pressed') === 'true' ? null : b.dataset.entPick));
  wrap.querySelector('[data-ent-add]').onclick = () => entityForm(slot, null, (saved) => pick(saved.id));
  const c = wrap.querySelector('[data-ent-clear]'); if (c) c.onclick = () => pick(null);
  return wrap;
}
function entityForm(slot, e, done) {
  const id = e ? e.id : '';
  slot.innerHTML = `<form class="stack tight" id="ent-form" style="margin-top:10px;padding:16px;border:1px solid var(--line);border-radius:8px;background:var(--ground);max-width:520px">
      <div class="label">${e ? 'This entity' : 'A new entity'}</div>
      <div class="field"><div class="label">Name</div><input id="ef-name" maxlength="80" placeholder="the company, the trust, your own name" value="${esc(e ? e.name : '')}"></div>
      <div class="field"><div class="label">Kind</div><div class="seg" role="radiogroup" id="ef-kind">${ENTITY_KINDS.map(([k, w]) => `<button type="button" role="radio" aria-checked="${(e ? e.kind : 'person') === k ? 'true' : 'false'}" data-kind="${k}">${w}</button>`).join('')}</div></div>
      <div class="field"><div class="label">Note</div><textarea id="ef-note" rows="2" style="min-height:64px" placeholder="ABN, trustee, who does its books">${esc(e ? e.note : '')}</textarea></div>
      <div class="row" style="gap:8px"><button class="btn small primary" type="submit">${e ? 'Save' : 'Add this entity'}</button><button class="btn small" type="button" id="ef-cancel">Cancel</button></div>
      <div class="muted" id="ef-note-line" aria-live="polite"></div></form>`;
  let kind = e ? e.kind : 'person';
  $$('#ef-kind [data-kind]').forEach(b => b.onclick = () => { kind = b.dataset.kind; $$('#ef-kind [data-kind]').forEach(x => x.setAttribute('aria-checked', x === b ? 'true' : 'false')); });
  $('#ef-cancel').onclick = () => { slot.innerHTML = ''; };
  $('#ent-form').onsubmit = async (ev) => {
    ev.preventDefault(); const name = $('#ef-name').value.trim(); if (!name) return toast('Give the entity a name');
    try {
      const before = new Set((store.entities || []).map(x => x.id));
      store = await invoke('entity_save', { id, name, kind, note: $('#ef-note').value });
      if (vault) vault = store.vaults.find(x => x.name === vault.name) || vault;
      const saved = id ? store.entities.find(x => x.id === id) : store.entities.find(x => !before.has(x.id));
      slot.innerHTML = ''; if (done) await done(saved);
    } catch (err) { $('#ef-note-line').textContent = String(err); }
  };
  $('#ef-name').focus();
}
async function vaultEntitySet(name, id, then) { try { store = await invoke('vault_entity_set', { name, entity: id }); if (vault) vault = store.vaults.find(x => x.name === vault.name) || vault; if (then) then(); } catch (e) { fail(e); } }
function renderEntities() {
  const ents = (store.entities || []).slice().sort((a, b) => a.name.localeCompare(b.name)), un = store.vaults.filter(v => !v.entity || !entityOf(v.entity));
  $('#ent-sum').textContent = ents.length ? `${sats(ents.length)} ${ents.length === 1 ? 'entity' : 'entities'} · ${sats(store.vaults.length - un.length)} vault${store.vaults.length - un.length === 1 ? '' : 's'} with an owner${un.length ? ` · ${un.length} without — open an entity to claim them` : ''}` : '';
  const list = $('#ent-list'); list.innerHTML = '';
  if (!ents.length) list.innerHTML = `<div class="sum-empty">No entity yet. ${store.vaults.length ? 'Name the first — yourself, a company, a trust — and put its vaults under it.' : ''}</div>`;
  for (const e of ents) {
    const vs = vaultsOf(e.id), row = document.createElement('div'); row.className = 'tp rowpick'; row.dataset.entity = e.id; row.setAttribute('role', 'link'); row.tabIndex = 0;
    row.innerHTML = `<div class="tp-art">${iconHtml('entity', e.id, 40, entGlyph(e))}</div><div class="tp-what"><div class="tp-head"><b>${esc(e.name)}</b> <span class="ent-kind" style="margin-left:6px">${esc(kindWord(e.kind))}</span></div><div class="muted" style="font-size:13px">${vs.length ? vs.map(v => esc(v.name)).join(' · ') : 'no vaults yet'}</div></div><div class="tp-act"><span class="muted" style="font-size:13px">${sats(vs.length)} vault${vs.length === 1 ? '' : 's'}</span></div>`;
    row.onclick = () => openEntity(e.id); row.onkeydown = (ev) => { if (ev.key === 'Enter') openEntity(e.id); };
    list.appendChild(row);
  }
  paintIcons(list);
  $('#ent-new').onclick = () => entityForm($('#ent-form-slot'), null, (saved) => { if (saved) openEntity(saved.id); else renderEntities(); });
}
function openEntity(id) { entPage = id; show('entity'); }
function renderEntity() {
  const card = $('#entity-card'), e = entityOf(entPage);
  if (!e) { card.innerHTML = '<div class="muted">That entity is no longer here.</div><div class="row"><button class="btn small" type="button" data-go="entities">All entities</button></div>'; $('#ent-summary').innerHTML = ''; return; }
  const vs = vaultsOf(e.id), keys = new Map();
  for (const v of vs) for (const k of v.keys) if (k.pubkey && !keys.has(k.pubkey)) keys.set(k.pubkey, k);
  card.innerHTML = `<div class="txnav"><button class="btn small" type="button" data-go="entities">‹ All entities</button><span class="muted">${esc(e.name)}</span><span></span></div>
    <div class="guide-row"><div class="guide" style="display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:140px;flex-basis:200px">${iconHtml('entity', e.id, 72, entGlyph(e), true)}</div><div class="guide-body">
      <div class="cer-act-title">${esc(e.name)}</div><div class="ent-kind">${esc(kindWord(e.kind))}</div>${e.note ? `<div style="font-size:13.5px;white-space:pre-wrap">${esc(e.note)}</div>` : ''}
      <div class="label" style="margin-top:6px">Its vaults</div>
      <div class="tp-list" style="margin-top:0">${vs.length ? vs.map(v => `<div class="tp rowpick" data-go-vault="${esc(v.name)}" role="link" tabindex="0"><div class="tp-what"><div class="tp-head"><b>${esc(v.name)}</b> <span class="muted">· ${esc(kindOfVault(v))}${v.network !== 'mainnet' ? ' · ' + esc(v.network) : ''}</span></div></div><div class="tp-act"><button type="button" class="linkish" data-disown="${esc(v.name)}">Not its owner</button></div></div>`).join('') : '<div class="muted" style="font-size:13px">No vault yet. Claim one from the list below, or from its Vault settings.</div>'}</div>
      ${(() => { const un = store.vaults.filter(v => !v.entity || !entityOf(v.entity)); return `<details class="adv" id="ent-unowned" style="margin-top:8px"${vs.length ? '' : ' open'}><summary>Vaults with no owner yet · ${sats(un.length)}</summary><div class="tp-list" style="margin-top:8px">${un.length ? un.map(v => `<div class="tp"><div class="tp-what"><div class="tp-head"><b>${esc(v.name)}</b> <span class="muted">· ${esc(kindOfVault(v))}${v.network !== 'mainnet' ? ' · ' + esc(v.network) : ''}</span></div></div><div class="tp-act"><button type="button" class="btn small" data-claim="${esc(v.name)}">Its owner</button></div></div>`).join('') : '<div class="muted" style="font-size:13px">Every vault has an owner.</div>'}</div></details>`; })()}
      ${keys.size ? `<div class="label" style="margin-top:6px">Its keys</div><div class="muted" style="font-size:13px">${[...keys.values()].map(k => `<b>${esc(k.name)}</b>${placeOf(k.pubkey) ? ' at ' + esc(placeOf(k.pubkey).name) : ''}`).join(' · ')} — <button type="button" class="linkish" data-go="allkeys">every key</button></div>` : ''}
      <div class="row" style="gap:8px;margin-top:6px"><button class="btn small" type="button" id="ent-edit">Edit</button><button class="btn small" type="button" id="ent-del">Forget this entity</button></div>
    </div></div><div id="ent-form"></div>`;
  paintIcons(card);
  $$('#entity-card [data-go-vault]').forEach(r => { const go = () => { vault = store.vaults.find(v => v.name === r.dataset.goVault) || vault; show('home'); }; r.onclick = (ev) => { if (ev.target.closest('button')) return; go(); }; r.onkeydown = (ev) => { if (ev.key === 'Enter') go(); }; });
  $$('#entity-card [data-claim]').forEach(b => b.onclick = () => vaultEntitySet(b.dataset.claim, e.id, renderEntity));
  $$('#entity-card [data-disown]').forEach(b => b.onclick = () => vaultEntitySet(b.dataset.disown, null, renderEntity));
  $('#ent-edit').onclick = () => entityForm($('#ent-form'), e, () => renderEntity());
  askTwice($('#ent-del'), 'Forget this entity? Its vaults stay; they are simply unassigned again.', 'Yes, forget it', async () => { try { store = await invoke('entity_delete', { id: e.id }); show('entities'); } catch (err) { fail(err); } });
  const main = vs.filter(v => v.network === 'mainnet').map(v => v.name), sum = $('#ent-summary');
  if (main.length) renderSummary(sum, main, { title: 'Over time', empty: 'Nothing has arrived in these vaults yet.', extra: `${sats(main.length)} vault${main.length === 1 ? '' : 's'} of ${esc(e.name)}, counted together. A transfer between them is a move, not a sale.` });
  else sum.innerHTML = '<div class="label">Over time</div><div class="sum-empty">No vault on mainnet belongs to this entity yet, so there is nothing to draw.</div>';
}
function renderKeys(force) {
  if (!vault) return;
  if (recPage.kind === 'key' && !vault.keys.some(k => k.pubkey === recPage.id)) closeKeyCard(false);
  paintKeyLife();
  if (force || !keysBusy()) { $('#keys-list').innerHTML = vault.keys.map(vaultKeyRow).join(''); paintSealSlots(); }
  invoke('vault_recovery', { name: vault.name }).then(l => { if (vault._recovery !== l) hideDesc.shown = false; vault._recovery = l; paintDesc(); }); $('#keys-k').textContent = vault.threshold; $$('.keys-k2').forEach(e => e.textContent = vault.threshold);
  renderKeysNow(); renderSeal();
}
function keyRow(name, sub, pub, state, idx) { const pen = idx === undefined ? '' : `<button class="pencil" data-edit-idx="${idx}" title="Rename this key"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/></svg></button>`; const pf = packedForm(pub); return `<div class="keyrow"><span class="keyart">${pf ? keyArt(pf, !!(pf & 0x80), 44) : '<svg width="22" height="22"><use href="#ring"/></svg>'}</span><div class="who"><div class="name"><span class="name-text">${name}</span>${pen}</div><div class="muted" style="font-size:13px">${pf ? modelName(pf, !!(pf & 0x80), !!(pf & 0x40)) + ' · ' : ''}${sub} · <span class="mono">${pub.slice(0, 12)}…</span></div></div><div class="state ${state}"><i></i>${{ ok: 'enrolled', off: 'not plugged in', here: 'plugged in' }[state]}</div></div>`; }
// Said once, plainly, rather than left as a gap the person has to wonder about.
function paintKeyLife() {
  if (vault && vault.ext) { $('#key-life').hidden = true; return; }
  $('#key-life').hidden = false;
  const plugged = cards.length, keys = vault ? vault.keys : [];
  const fwOf = (pk) => { const c = cards.find(c => c.pubkey === pk); return (c && c.firmware) || ((store && store.key_fw) || {})[pk] || ''; };
  const known = keys.filter(k => fwOf(k.pubkey)), hit = known.filter(k => keyAdvisories({ firmware: fwOf(k.pubkey) }).length);
  $('#key-life').innerHTML = `<div class="label">Age, end of life and warranty</div>
    <div class="muted" style="font-size:13px">A key tells this computer its serial and, for a YubiKey, its firmware version and its shape. It does <b>not</b> carry the day it was made, an end-of-life date, or anything about a warranty \u2014 so none is shown here, because a guessed one would be worse than nothing. A Nitrokey 3 keeps its firmware version for Nitrokey App 2, which can also update it.</div>
    <div class="muted" style="font-size:13px;margin-top:8px">Yubico retires a whole <b>product line</b>, not your particular key, and sets no date in advance: their policy is six months\u2019 notice before a product stops being sold, and support for a year after that. A warranty runs from a purchase this app knows nothing about. The nearest thing to an age it can honestly give is when this computer first saw each key, which is a floor and never the real age.</div>
    <div class="muted" style="font-size:13px;margin-top:8px">The firmware each key reports <b>is</b> checked, here on this computer, against advisories known when this app was built (${esc(ADVISORIES_AS_OF)}). Nothing is asked of anyone: a question like \u201cis serial 12345678 still supported?\u201d would hand over a list of exactly which keys guard this vault.</div>
    <div class="${hit.length ? 'warn' : 'muted'}" style="font-size:13px;margin-top:8px">${!keys.length ? '' : `The firmware of ${sats(known.length)} of this vault\u2019s ${sats(keys.length)} keys is known${known.length < keys.length ? ' \u2014 plug in the others once each and it is remembered, since it can never change' : ''}. ${hit.length === keys.length && keys.length ? `<b>All ${sats(keys.length)} are on firmware an advisory covers.</b> A vault of several keys usually means an attacker must beat more than one \u2014 but here every one of them has the same weakness, so that comfort is gone.` : hit.length ? `${sats(hit.length)} ${hit.length === 1 ? 'has' : 'have'} something against ${hit.length === 1 ? 'it' : 'them'} \u2014 see ${hit.length === 1 ? 'its row' : 'their rows'} above.` : known.length ? 'Nothing known stands against them.' : ''}`}</div>`;
}
function renderKeysNow() {
  // keys of this vault say "plugged in" on their own rows; this line is only for what ELSE is in the ports
  const others = cards.filter(c => !vault || !vault.keys.some(k => k.pubkey === c.pubkey));
  const devName = (r) => { const m = r.match(/YubiKey[^,()]*/i); const n = (m ? m[0] : r).replace(/\s+(OTP|FIDO|CCID|U2F)(\+\w+)*.*$/i, '').trim(); return /bio/i.test(r) ? 'YubiKey Bio' : /nitrokey|^8352 /i.test(r) ? 'Nitrokey' : (n || 'A smart card'); };
  const unusable = otherDevices.map(d => `<div class="keyrow"><svg width="22" height="22"><use href="#ring"/></svg><div class="who"><div class="name">${esc(devName(d.reader))} <span class="muted" style="font-weight:400">\u00b7 plugged in, cannot be used here</span></div><div class="muted" style="font-size:13px">${esc(d.why.charAt(0).toUpperCase() + d.why.slice(1))}.${/bio/i.test(d.reader) ? ' A YubiKey Bio signs in to websites with a fingerprint; it does not hold the kind of key a vault needs. It is perfectly fine \u2014 just for something else.' : ''} It is not one of this vault\u2019s keys and nothing was done to it.</div></div><div class="state"><i></i>not usable</div></div>`).join('');
  $('#keys-other').innerHTML = unusable; $('#keys-other').hidden = !unusable;
  const text = others.length ? 'Also plugged in: ' + others.map(c => `${modelName(c.form, c.nfc)} · serial ${c.serial}${c.pubkey ? '' : ' · no signing key yet'}`).join('; ') + '. Not one of this vault’s keys.'
    : cardsErr && !/no (YubiKey|key) found/i.test(cardsErr) ? 'Card reader problem: ' + cardsErr : '';
  $('#keys-now').textContent = text; $('#keys-now').hidden = !text;
}
// rename a key in place from the pencil on its row
$('#keys-list').addEventListener('click', (e) => {
  const open = e.target.closest('[data-key-open]');
  if (open && !e.target.closest('.pencil, input, button, .kseal, a')) { openKeyCard(open.dataset.keyOpen); return; }
  const b = e.target.closest('[data-edit-idx]'); if (!b || $('#keys-list .key-input')) return;   // one rename at a time; a PIN field in another row is not a rename
  const idx = +b.dataset.editIdx, nameEl = b.parentElement, cur = vault.keys[idx].name;
  const inp = document.createElement('input'); inp.className = 'key-input'; inp.value = cur; inp.maxLength = 40;
  nameEl.replaceChildren(inp); inp.focus(); inp.select();
  let done = false;
  const finish = async (save) => {
    if (done) return; done = true;
    const v = inp.value.trim();
    if (!save || !v || v === cur) { renderKeys(true); return; }
    try {
      const names = vault.keys.map((k, i) => i === idx ? v : k.name);
      store = await invoke('vault_rename', { name: vault.name, newName: vault.name, keyNames: names });
      vault = store.vaults.find(x => x.name === vault.name) || vault; toast('Key renamed');
    } catch (err) { fail(err); }
    renderKeys(true);
  };
  inp.addEventListener('keydown', (ev) => { if (ev.key === 'Enter') { ev.preventDefault(); finish(true); } if (ev.key === 'Escape') finish(false); });
  inp.addEventListener('blur', () => finish(true));
});

// rename in place from the title
$('#title-edit').onclick = () => {
  const inp = $('#title-input'); inp.value = vault.name; $('#title').hidden = true; $('#title-edit').hidden = true; inp.hidden = false; inp.focus(); inp.select();
};
async function commitTitle() {
  const inp = $('#title-input'); const newName = inp.value.trim();
  inp.hidden = true; $('#title').hidden = false; $('#title-edit').hidden = false;
  if (!newName || newName === vault.name) return;
  try {
    store = await invoke('vault_rename', { name: vault.name, newName, keyNames: vault.keys.map(k => k.name) });
    vault = store.vaults.find(v => v.name === newName); rememberVault(vault);
    $('#title').textContent = vault.name; paintNet(); toast('Renamed');
  } catch (e) { fail(e); }
}
$('#title-input').addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); commitTitle(); } if (e.key === 'Escape') { $('#title-input').value = vault.name; commitTitle(); } });
$('#title-input').addEventListener('blur', () => { if (!$('#title-input').hidden) commitTitle(); });
$('#vault-pdf').onclick = async () => { try { toast('Saved ' + await invoke('vault_pdf', { name: vault.name }), 6000); } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
// never fall back to `vault.descriptor`: for a rotating vault that is the line WITHOUT its chain code, which restores the wrong addresses
$('#copy-desc').onclick = async () => { try { if (!vault._recovery) vault._recovery = await invoke('vault_recovery', { name: vault.name }); if (!vault._recovery) throw 'the recovery line could not be read'; await navigator.clipboard.writeText(vault._recovery); toast('Recovery line copied'); } catch (e) { fail(e); } };
askTwice($('#forget-vault'), 'Forget this vault on this computer? If this profile syncs, it is forgotten on every device it syncs to. The coins stay on the chain; two sealed keys, or the recovery line and two keys, bring it back.', 'Yes, forget it here', async () => { store = await invoke('vault_forget', { name: vault.name }); await refreshTransferLinks(); vault = null; boot(); });

// ---------- setup (enrol) ----------
function renderSetup() {
  paintNet();
  if (!setup.network) setup.network = store.settings.network;
  $('#n-show').textContent = setup.n;
  $$('#preset-pick .opt').forEach(o => o.classList.toggle('on', +o.dataset.k === setup.k && +o.dataset.n === setup.n));
  $$('#setup-net-pick .opt').forEach(o => o.classList.toggle('on', o.dataset.net === setup.network));
  $('#n-less').disabled = setup.n <= Math.max(2, setup.keys.length) || setup.n <= 1; $('#n-more').disabled = setup.n >= 15;
  $('#n-less').closest('.field').hidden = setup.n === 1;
  const kp = $('#k-pick'); kp.innerHTML = '';
  for (let k = 1; k <= setup.n; k++) { const o = document.createElement('div'); o.className = 'opt' + (k === setup.k ? ' on' : ''); o.textContent = `${k} of ${setup.n}`; o.onclick = () => { setup.k = k; renderSetup(); }; kp.appendChild(o); }
  $('#k-note').textContent = setup.k === 1 ? 'Any one key spends: convenient, but one stolen key with its PIN spends everything.' : setup.k === setup.n ? 'Every key must sign: losing any one key loses the funds.' : `${setup.k} of ${setup.n} is the balance: one key can be lost without loss, and one stolen key with its PIN spends nothing.`;
  $('#setup-done').innerHTML = setup.keys.map((k, i) => (k.expression ? deviceRow(k) : keyRow(k.name, `serial ${k.serial}`, k.pubkey, 'ok')).replace(/<\/div>$/, `<button class="btn small" data-drop="${i}" type="button" title="Take this key out of the vault being made">Remove</button></div>`)).join('');
  $$('#setup-done [data-drop]').forEach(b => b.onclick = () => { setup.keys.splice(+b.dataset.drop, 1); renderSetup(); });
  $('#setup-warn').innerHTML = setupWarnings().map(w => `<div class="warnbox">${w}</div>`).join('');
  const box = $('#setup-now');
  const remaining = setup.n - setup.keys.length;
  $('#setup-finish').disabled = remaining > 0;
  $('#add-device').closest('.row').hidden = setup.n < 2;
  $('#add-device').disabled = remaining <= 0;
  if (remaining === 0) { box.innerHTML = `<div class="ok">All ${setup.n} keys ${setup.keys.some(k => k.expression) ? 'added' : 'enrolled'}.</div>`; return; }
  // every plugged-in key that is not in the vault yet is offered, each in its own panel: with two keys in the
  // computer the person chooses which one is next, and is never shown one of them as though it were the only one
  const waiting = cards.filter(c => !setup.keys.some(k => k.serial === c.serial));
  if (!waiting.length) { box.innerHTML = `<div class="muted">Key ${setup.keys.length + 1} of ${setup.n}: plug in a YubiKey or a Nitrokey 3${setup.n > 1 ? ', or add a signing device' : ''}.</div>`; renderSetup.nowSig = null; return; }
  const spare = ['laptop', 'keyring', 'off-site'].filter(n => !setup.keys.some(k => k.name === n));
  const rows = waiting.map((c, i) => {
    // a card already named in another vault keeps that name: it is the same key, and the person knows it by it
    const known = c.pubkey ? store.vaults.flatMap(v => v.keys).find(k => k.pubkey === c.pubkey && k.name && !/^key \d+/.test(k.name)) : null;
    // a key set up here (its PINs chosen, its key made, its first signature checked) is known by the name it was given then
    const ready = c.pubkey ? (store.key_ready || {})[c.pubkey] : null;
    const has = !!c.pubkey;
    // a key with no signing key: is it untouched (set it up here, PINs and all) or does it have PINs of its own already?
    const looked = has ? null : kiSeen.get(c.serial), isNew = !!(looked && looked.state === 'fresh'), begun = isNew && kiBegun(c.serial);
    return { c, known, ready, has, looked, isNew, begun, nameGuess: known ? known.name : ready && ready.name ? ready.name : spare[i] || `key ${setup.keys.length + 1 + i}` };
  });
  // The card poll redraws this screen every 2 s. Rebuilding the panels each time put a fresh "Name this key" box
  // under the cursor — focus gone and the typing reset after a keystroke. They are rebuilt only when what they show
  // changes (another card, another step); what was typed is kept across even that.
  const sig = rows.map(r => [r.c.serial, r.c.pubkey || '', r.isNew, r.begun].join(':')).concat([setup.keys.length, setup.n]).join('|');
  if (renderSetup.nowSig === sig && box.querySelector('.enrol')) return;
  renderSetup.nowSig = sig;
  const typedBy = renderSetup.typedBy || (renderSetup.typedBy = {});
  // the first panel keeps the ids this screen has always had; the others are reached through their own panel
  const id = (i, name) => i === 0 ? ` id="${name}"` : '';
  box.innerHTML = `<div style="font-family:var(--display);font-weight:600;font-size:15px">Key ${setup.keys.length + 1} of ${setup.n}${rows.length > 1 ? `: ${rows.length} keys are plugged in. Choose the one to add next` : ''}</div>`
    + rows.map((r, i) => { const c = r.c, date = r.ready ? new Date(r.ready.at * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'long', year: 'numeric' }) : '';
      const head = `<span class="keyart">${keyArt(c.form || 3, !!c.nfc, 52)}</span><div class="enrol-body"><div style="font-family:var(--display);font-weight:600;font-size:15px">${esc(modelName(c.form, c.nfc))} · serial ${c.serial}${c.firmware ? ' · firmware ' + esc(c.firmware) : ''}</div>`;
      if (r.isNew) return `<div class="enrol" data-serial="${c.serial}">${head}
        <div class="muted" style="font-size:13px">${r.begun ? 'This key’s PINs were set here, but its setup was not finished: it has no signing key yet.' : 'This key is new: it has never been set up, and both its PINs are still the ones every key ships with.'} Setting it up here takes a few minutes and needs no other software.</div>
        <div class="row"><button class="btn primary enrol-init"${id(i, 'enrol-init')} type="button">${r.begun ? 'Finish setting this key up' : 'Set this key up'}</button></div></div></div>`;
      return `<div class="enrol" data-serial="${c.serial}">${head}
        <div class="muted" style="font-size:13px">${r.has ? (r.ready ? `Set up here on ${date}: its PINs are yours, its signing key is made and its first signature was checked. Nothing more to do but add it.` : 'This card already holds a signing key (touch ' + esc(c.touch) + '). Use it: the same card key can belong to any number of vaults.') : 'This key has PINs of its own already and no signing key: one will be made on it.'}</div>
        <div class="grid2"><div class="field"><div class="label">Name this key</div><input class="enrol-name"${id(i, 'enrol-name')} value="${esc(r.nameGuess)}">${r.known ? `<div class="muted" style="font-size:12.5px">Its name in ${esc(store.vaults.find(v => v.keys.includes(r.known)).name)}</div>` : ''}</div>${r.has ? '' : `<div class="field"><div class="label">Admin PIN</div><input class="enrol-admin"${id(i, 'enrol-admin')} type="password" autocomplete="off"></div>`}</div>
        ${r.has ? '' : '<div class="muted" style="font-size:13px">Enter the admin PIN this key was given. A touch will be needed for every signature from then on, which cannot be undone without wiping the key.</div>'}
        <div class="row">${r.has ? `<button class="btn primary use-btn"${id(i, 'use-btn')} type="button">Use this card’s key</button><button class="btn replace-btn"${id(i, 'replace-btn')} type="button" style="border-color:var(--no);color:var(--no)">Replace it with a new key…</button>` : `<button class="btn primary enrol-btn"${id(i, 'enrol-btn')} type="button">Generate key on card</button>`}</div></div></div>`; }).join('');
  rows.forEach((r) => {
    const c = r.c, el = box.querySelector(`.enrol[data-serial="${c.serial}"]`), q = (sel) => el.querySelector(sel);
    if (r.isNew) { q('.enrol-init').onclick = () => kiOpen(r.looked); return; }
    const nameEl = q('.enrol-name'), typed = typedBy[c.serial];
    if (typed) { nameEl.value = typed.name; if (typed.focused) { nameEl.focus(); nameEl.setSelectionRange(nameEl.value.length, nameEl.value.length); } }
    nameEl.addEventListener('input', () => { typedBy[c.serial] = { name: nameEl.value, focused: true }; });
    nameEl.addEventListener('blur', () => { if (typedBy[c.serial]) typedBy[c.serial].focused = false; });
    const wireGenerate = () => { q('.enrol-btn').onclick = async () => {
      const name = nameEl.value.trim() || r.nameGuess, admin = q('.enrol-admin').value;
      if (!admin) return toast('Enter the admin PIN');
      q('.enrol-btn').disabled = true;
      try { const pub = await invoke('card_generate', { reader: c.reader, adminPin: admin }); setup.keys.push({ name, serial: c.serial, pubkey: pub, device: cardDevice(c) }); delete typedBy[c.serial]; toast(`${name} enrolled`); }
      catch (e) { fail(e); }
      renderSetup.nowSig = null; renderSetup();
    }; };
    if (r.has) {
      q('.use-btn').onclick = () => { setup.keys.push({ name: nameEl.value.trim() || r.nameGuess, serial: c.serial, pubkey: c.pubkey, device: cardDevice(c) }); delete typedBy[c.serial]; toast('enrolled with its existing key'); renderSetup(); };
      askTwice(q('.replace-btn'), 'Generate a NEW key on this card? The key it holds now will be destroyed, and any vault it belongs to loses that cosigner for good. Only for a card that has never held funds.', 'Yes, replace its key', async () => {
        q('.enrol-body').insertAdjacentHTML('beforeend', '<div class="field" style="margin-top:12px"><div class="label">Admin PIN</div><input class="enrol-admin" type="password" autocomplete="off" style="max-width:260px"></div><button class="btn enrol-btn" type="button" style="border-color:var(--no);color:var(--no);margin-top:10px">Generate a new key, destroying the old one</button>');
        wireGenerate();
      });
    } else wireGenerate();
  });
}
// the card says who made it (its AID), and cards.rs turns that into the devices.js id; a reader's name says nothing
function cardDevice(c) { return c.device || 'yubikey'; }
function deviceRow(k) {
  const d = deviceOf(k.device), m = /^\[([0-9a-f]{8})\/([^\]]+)\](\w+)/.exec(k.expression) || [];
  return `<div class="keyrow"><span class="devart">${d.svg}</span><div class="who"><div class="name"><span class="name-text">${esc(k.name)}</span></div><div class="muted" style="font-size:13px">${esc(d.name)} · fingerprint <span class="mono">${esc(m[1] || '?')}</span> · m/${esc(m[2] || '?')} · <span class="mono">${esc((m[3] || '').slice(0, 12))}…</span></div></div><div class="state ok"><i></i>added</div></div>`;
}
/// What the shape of the keys chosen so far means, said before the vault exists — warnings, never a block.
function setupWarnings() {
  const out = [], keys = setup.keys;
  // who made each key: an unknown signer counts as its own maker (it is at least not the same device as another)
  const maker = (k) => deviceOf(k.device || 'yubikey').maker || ('signer ' + (k.fingerprint || k.name));
  const by = new Map(); for (const k of keys) by.set(maker(k), (by.get(maker(k)) || []).concat([k]));
  const cardsIn = keys.filter(k => !k.expression), devsIn = keys.filter(k => k.expression);
  if (by.size > 1) for (const [m, ks] of by) if (ks.length >= setup.k)
    out.push(`<b>${ks.length} of these keys are ${esc(m)}’s, and ${setup.k} can spend.</b> ${esc(ks.map(k => k.name).join(' and '))} alone reach the quorum, so one flaw in ${esc(m)}’s design reaches it too, and the other signers are never needed. Mixing makers only adds independence when no one maker holds ${setup.k} keys — for example a SeedSigner, a YubiKey and a Nitrokey 3.`);
  if (cardsIn.length && devsIn.length)
    out.push(`<b>In a vault with other signers, a card’s key is fixed.</b> Addresses still change with every deposit, and any wallet can rebuild them from the recovery line; but ${cardsIn.length > 1 ? 'each card’s key is' : esc(cardsIn[0].name) + '’s key is'} the same in every spend it signs, so those spends can be linked to one another on the chain. A vault of cards alone keeps them private.`);
  if (cardsIn.length && devsIn.some(k => /^coldcard/.test(k.device)))
    out.push(`<b>A Coldcard may not sign for this vault.</b> It registers a multisig from extended keys only, and a card\u2019s key here is a plain key, so the Coldcard will probably refuse to register it \u2014 and it refuses to sign an unregistered multisig. Not yet tested: try a small spend before funding it.`);
  const reg = devsIn.filter(k => /^coldcard|^jade|^ledger|^bitbox/.test(k.device));
  if (reg.length) out.push(`${esc([...new Set(reg.map(k => deviceOf(k.device).name))].join(', '))} sign${reg.length === 1 ? 's' : ''} for a multisig only once the vault is registered on ${reg.length === 1 ? 'it' : 'them'}. The vault’s file for that comes with signing.`);
  return out;
}

// ---------- adding a signing device: pick it, then scan, open or paste its key ----------
const devSt = { dev: null, parts: new Set(), stream: null, timer: null };
function devStop() { if (devSt.timer) clearInterval(devSt.timer); devSt.timer = null; if (devSt.stream) devSt.stream.getTracks().forEach(t => t.stop()); devSt.stream = null; devSt.parts = new Set(); }
function devClose() { devStop(); $('#dev-modal').classList.remove('on'); }
function devList() {
  devStop(); devSt.dev = null;
  $('#dev-title').textContent = 'Add a signing device'; $('#dev-pick').hidden = false; $('#dev-one').hidden = true; $('#dev-back').hidden = true;
  const groups = [['airgap', 'By QR code or SD card — nothing plugged in'], ['usb', 'By USB or Bluetooth — paste its key for now'], ['card', 'Security keys — plug in'], ['other', 'Anything else']];
  $('#dev-pick').innerHTML = groups.map(([g, t]) => `<div class="devgroup"><div class="cap">${t}</div><div class="devrow">${DEVICES.filter(d => d.group === g).map(d => `<button class="devopt" data-dev="${d.id}" type="button">${d.svg}<span>${esc(d.name)}</span><small>${d.offers.map(o => DEVICE_WORDS[o]).join(' · ') || 'text, file or code'}</small></button>`).join('')}</div></div>`).join('');
  $$('#dev-pick [data-dev]').forEach(b => b.onclick = () => devShow(b.dataset.dev));
}
function devShow(id) {
  const d = deviceOf(id); devSt.dev = d;
  $('#dev-title').textContent = d.name; $('#dev-pick').hidden = true; $('#dev-one').hidden = false; $('#dev-back').hidden = false;
  const chips = d.offers.map(o => `<span class="chip${DEVICE_NOW[o] ? ' on' : ''}" title="${DEVICE_NOW[o] ? 'This app uses it' : 'The device has it; this app does not use it yet'}">${DEVICE_WORDS[o]}${DEVICE_NOW[o] ? '' : ' · later'}</span>`).join('');
  const head = `<div class="devhead">${d.svg}<div style="display:flex;flex-direction:column;gap:6px"><div class="muted" style="font-size:13px">${esc(d.maker)}</div><div class="chips">${chips}</div></div></div>`;
  if (d.group === 'card') {
    $('#dev-one').innerHTML = head + `<div>Plug it in and close this: a ${esc(d.name)} appears on the setup screen by itself, ready to enrol${d.id === 'nitrokey3' ? ' (proven on firmware 1.9.0, 9 October 2026)' : ''}.</div>`;
    return;
  }
  const qr = d.offers.includes('qr') || d.group === 'other', sd = d.offers.includes('sd') || d.group === 'other';
  const usbNote = d.group === 'usb' ? `<div class="muted" style="font-size:13px">A direct ${d.offers.includes('ble') ? 'USB or Bluetooth ' : 'USB '}connection comes later. For now the key can come from the device’s own app, or from Sparrow with the device plugged in: copy its <b>multisig</b> key, <span class="mono">[fingerprint/48h/0h/0h/2h]xpub…</span>, and paste it below.</div>` : '';
  $('#dev-one').innerHTML = head + (d.how ? `<div>${esc(d.how)}</div>` : '') + usbNote
    + `<div class="row">${qr ? '<button class="btn" id="dev-scan" type="button">Scan its code</button>' : ''}${sd ? '<button class="btn" id="dev-file" type="button">Open a file…</button>' : ''}</div>
      <div class="field" hidden><div class="cap">Camera</div><div class="camrow" id="dev-pick-cam" role="radiogroup" aria-label="Which camera"></div></div>
      <div class="cam" id="dev-cam" hidden><video id="cam3" autoplay playsinline muted></video><div class="frame"></div></div><div class="muted" style="font-size:13px" id="dev-cam-note" hidden></div>
      <div class="field"><div class="label">Or paste it</div><textarea id="dev-text" class="mono" rows="3" spellcheck="false" placeholder="[a1b2c3d4/48h/0h/0h/2h]xpub…  ·  a Coldcard JSON  ·  ur:crypto-account/…"></textarea></div>
      <div class="row"><button class="btn" id="dev-read" type="button">Read it</button></div>
      <div id="dev-got" class="stack tight"></div>`;
  if ($('#dev-scan')) $('#dev-scan').onclick = () => devScan();
  if ($('#dev-file')) $('#dev-file').onclick = async () => { try { devGot(await invoke('device_key_file')); } catch (e) { if (String(e) !== 'cancelled') devSay(e); } };
  $('#dev-read').onclick = async () => { try { devGot(await invoke('device_key_read', { text: $('#dev-text').value })); } catch (e) { devSay(e); } };
}
function devSay(e) { $('#dev-got').innerHTML = `<div class="warnbox">${esc(String(e && e.message || e))}</div>`; }
async function devScan(deviceId) {
  if (typeof deviceId !== 'string') { scanReset($('#dev-cam-note')); devSt.partsSeen = 0; }
  const parts = devSt.parts; devStop(); if (typeof deviceId === 'string') devSt.parts = parts;   // switching camera keeps the parts already read
  const note = $('#dev-cam-note'); note.hidden = false; $('#dev-cam').hidden = false;
  try { devSt.stream = await camOpen(typeof deviceId === 'string' ? deviceId : undefined); $('#cam3').srcObject = devSt.stream; camChooser($('#dev-pick-cam'), devSt.stream, devScan); } catch (e) { note.textContent = 'No camera available. Open its file, or paste the key instead.'; $('#dev-cam').hidden = true; return; }
  note.textContent = 'Hold the device’s code in the frame.';
  const reader = makeQrReader(note);
  let busy = false;
  devSt.timer = setInterval(async () => {
    const v = $('#cam3'); if (busy || !v || !v.videoWidth) return;
    const data = reader.read(v); if (!data) return;
    const t = data.trim();
    // an animated UR arrives a part at a time, in any order; a plain code is the whole thing
    if (/^ur:/i.test(t)) { const before = devSt.parts.size; devSt.parts.add(t.toLowerCase()); if (devSt.parts.size === before) return; devSt.partsSeen = devSt.parts.size; }
    busy = true;
    try { const r = await invoke('device_key_read', { text: /^ur:/i.test(t) ? [...devSt.parts].join(' ') : t }); devStop(); note.textContent = 'Read its key. Check it below.'; scanDone(note, Math.max(1, devSt.partsSeen || 1), 'Read its key'); devGot(r); }
    catch (e) { if (!scanShow(note, e)) { note.textContent = String(e); scanBar(note, null); } }
    busy = false;
  }, 250);
}
/// "Still reading" from the decoder's own count. An animated UR is a fountain code: after its first lap every frame
/// is a MIXTURE numbered above the total ("7-4"), so counting frames against that total reads "7 of 4" and means
/// nothing. What the decoder has recovered (UR), or which parts it holds (BBQr, which does cycle), can be trusted.
function scanProgress(e) {
  const m = /(\d+) of (\d+) (pieces recovered|shares read|\))/.exec(String(e)) || /missing \((\d+) of (\d+)\)/.exec(String(e));
  if (!/missing/i.test(String(e))) return null;
  if (!m) return { have: 0, of: 0, text: 'Reading an animated code. Keep it in the frame.' };
  const have = +m[1], of = +m[2], left = of - have;
  const shares = m[3] === 'shares read';
  if (shares) return { have, of, shares, text: `${have} of ${of} shares read — put the next share in front of the camera (any order).` };
  return { have, of, shares, text: `${have} of ${of} pieces read — ${left} more to go. Keep it in the frame, or show the next part.` };
}
// ---------- signing on a device that reads codes: scan, check, change, show it back ----------
// The same shape as signing with a card (plug in · PIN · touch): numbered steps, a picture of what to do, and one
// thing on screen at a time. Written with a SeedSigner in hand, but any QR signer follows these beats — the words
// name whichever device is signing, and nothing here is particular to one maker.
const devCer = { key: null, i: 0, sig: '', steps: [] };
function devScene(kind, deviceId) {
  const d = deviceOf(deviceId);
  const dev = (x, y, w) => `<svg x="${x}" y="${y}" width="${w}" height="${Math.round(w * 56 / 72)}" viewBox="0 0 72 56">${d.art}</svg>`;
  const lines = (x, y, n) => Array.from({ length: n }, (_, i) => `<rect x="${x}" y="${y + i * 9}" width="${i === n - 1 ? 26 : 44}" height="3" rx="1.5" fill="currentColor" opacity=".45"/>`).join('');
  // a code, drawn as a code is: three corners and a scatter that does not change between redraws
  const code = (x, y, s) => { let g = `<rect class="g-cap" x="${x}" y="${y}" width="${s}" height="${s}" rx="2"/>`;
    const u = s / 11, fin = (fx, fy) => `<rect class="g-ink" x="${fx}" y="${fy}" width="${u * 2.4}" height="${u * 2.4}" rx="1"/><rect class="g-cap" x="${fx + u * 0.72}" y="${fy + u * 0.72}" width="${u}" height="${u}"/>`;
    g += fin(x + u * 0.8, y + u * 0.8) + fin(x + s - u * 3.2, y + u * 0.8) + fin(x + u * 0.8, y + s - u * 3.2);
    let seed = 7;
    for (let r = 0; r < 11; r++) for (let c = 0; c < 11; c++) { seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      if ((seed >> 16) % 3 === 0 && !(r < 4 && c < 4) && !(r < 4 && c > 7) && !(r > 7 && c < 4)) g += `<rect class="g-ink" x="${x + u * (c + 0.4)}" y="${y + u * (r + 0.4)}" width="${u * 0.8}" height="${u * 0.8}"/>`; }
    return g; };
  const beam = (x1, y1, x2, y2) => `<path class="c-beam g-blink" d="M${x1} ${y1}L${x2} ${y2}"/>`;
  const screen = (inner) => `<rect class="g-lap" x="10" y="16" width="124" height="96" rx="9"/><rect class="g-cap" x="18" y="24" width="108" height="72" rx="3"/>${inner}<path class="g-line" d="M44 112h56"/>`;
  switch (kind) {
    // this screen shows the code; the device is held up to it and reads
    case 'scan':   return `<svg viewBox="0 0 250 132" role="img">${screen(code(40, 30, 64))}${dev(152, 32, 84)}${beam(196, 72, 130, 64)}</svg>`;
    // the device shows what it is about to sign; the person compares it with this screen
    case 'review': return `<svg viewBox="0 0 250 132" role="img"><g transform="translate(8,14)"><svg width="118" height="92" viewBox="0 0 72 56">${d.art}</svg></g><g transform="translate(150,30)" color="var(--muted)">${lines(0, 0, 4)}</g><path class="g-tick" fill="none" d="M150 78l9 10 20-22"/></svg>`;
    // the same, with the wallet's own line on screen, which is what lets it prove the change is yours
    case 'change': return `<svg viewBox="0 0 250 132" role="img">${screen(code(40, 34, 64))}<text x="24" y="21" style="font:700 8px var(--display);fill:var(--muted)">WALLET DESCRIPTOR</text>${dev(152, 32, 84)}${beam(196, 72, 130, 68)}</svg>`;
    // turned around: now the device shows its signature and this computer's camera reads it
    default:       return `<svg viewBox="0 0 250 132" role="img">${screen(`<circle class="g-ink" cx="72" cy="31" r="3.4"/><g color="var(--muted)">${lines(46, 46, 3)}</g>`)}<g transform="translate(152,32)"><svg width="84" height="65" viewBox="0 0 72 56">${d.art}</svg>${code(20, 12, 34)}</g>${beam(152, 56, 84, 40)}</svg>`;
  }
}
/// Which steps this spend needs, in order: a spend with nothing coming back says so rather than skipping it.
function devStepsFor(st) {
  const change = (st.pays || []).filter(p => p.ours);
  return [{ id: 'scan', word: 'Scan' }, { id: 'review', word: 'Check' }, { id: 'change', word: change.length ? 'Change' : 'No change' }, { id: 'back', word: 'Show it back' }];
}
/// The ceremony for one device key of this vault, at its current step.
function devCeremony(st, k) {
  const q = (n) => `\u2018${n}\u2019`;
  const d = deviceOf((vault.keys.find(x => x.pubkey === k.pubkey) || {}).device), steps = devStepsFor(st);
  devCer.steps = steps;
  if (devCer.key !== k.pubkey) { devCer.key = k.pubkey; devCer.i = 0; }
  const i = Math.min(devCer.i, steps.length - 1);
  const ccy = (txFacts[vault.name] || {}).ccy, now = vault._now && vault._now.q;
  const out = (st.pays || []).filter(p => !p.ours), change = (st.pays || []).filter(p => p.ours);
  const worth = (n) => now && ccy ? ` <span class="muted">· ${money(n / 1e8 * now.price, ccy)}</span>` : '';
  $('#dev-cer').hidden = false; $('#dev-sign-row').hidden = true; $('#act-row').hidden = true; $('#gsteps').hidden = true; $('#guide').hidden = true;
  $('#dev-steps').innerHTML = steps.map((s, n) => `<li class="${n === i ? 'now' : n < i ? 'done' : ''}">${esc(s.word)}</li>`).join('');
  $('#dev-cer-back').disabled = i === 0; $('#dev-cer-next').textContent = i === steps.length - 1 ? 'Open the camera' : 'Next ›';
  const sig = [k.pubkey, i, st.txid, st.pays.length].join('|');
  if (devCer.sig === sig) return;   // the 2 s card poll must not restart the code's animation
  devCer.sig = sig;
  $('#dev-guide').hidden = false; $('#dev-guide').innerHTML = devScene(steps[i].id, d.id);
  const T = $('#dev-cer-title'), X = $('#dev-cer-text'), S = $('#dev-stage');
  stopDevQr();
  if (steps[i].id === 'scan') {
    T.textContent = `Show ${q(k.name)} the transaction`;
    X.textContent = `On the ${d.name}: choose to scan, and hold it to this code. It reads the frames as they go by — keep it there until it has them all.`;
    S.innerHTML = '<div class="qr-box small"><canvas id="dev-qr" width="520" height="520"></canvas></div><div class="muted" id="dev-qr-note" style="font-size:13px"></div>';
    playDevQr();
  } else if (steps[i].id === 'review') {
    T.textContent = `Check what ${q(k.name)} shows`;
    X.textContent = `The ${d.name} now lists what this spend pays before it will sign. It should say exactly this. If anything differs — an address, an amount, the fee — do not approve it.`;
    S.innerHTML = `<dl class="tx-dl">${out.map(p => `<dt>Pays</dt><dd><span class="mono" style="word-break:break-all">${esc(p.address)}</span><br>${sats(p.sats)} sats${worth(p.sats)}</dd>`).join('')}
      ${change.map(p => `<dt>Change</dt><dd><span class="mono" style="word-break:break-all">${esc(p.address)}</span><br>${sats(p.sats)} sats${worth(p.sats)} <span class="tag ours">back to ${esc(vault.name)}</span></dd>`).join('')}
      <dt>Fee</dt><dd>${sats(st.fee)} sats${worth(st.fee)}</dd></dl>`;
  } else if (steps[i].id === 'change') {
    if (change.length) {
      T.textContent = 'If it asks about the change';
      X.textContent = `${sats(change[0].sats)} sats come back to this vault. A device can only prove that address is yours from the wallet's own line, so when the ${d.name} asks, show it this code. It cannot spend anything.`;
      S.innerHTML = '<div class="qr-box small"><canvas id="dev-desc-qr" width="520" height="520"></canvas></div><div class="muted" style="font-size:13px">The same line is in Vault settings › Save the recovery sheet, if it will not scan here.</div>';
      // the recovery line, not the stored text: it gives each fixed key the origin it implies, which a device needs
      // before it will show the wallet at all (see wallet::with_key_origins)
      invoke('vault_recovery', { name: vault.name })
        .then(line => { const c = $('#dev-desc-qr'); if (c) drawQr(line, c); })
        .catch(() => { const line = vault.ext || vault._recovery || vault.descriptor || ''; const c = $('#dev-desc-qr');
          if (line && c) drawQr(line, c); else $('#dev-stage').innerHTML = '<div class="muted" style="font-size:13px">Reveal the recovery line on the Keys screen first, then come back.</div>'; });
    } else {
      T.textContent = 'This spend leaves no change';
      X.textContent = `Everything these coins hold is going out, so nothing comes back to this vault. The ${d.name} may say it has no change to check, or show none — that is right for this spend, not a warning about it.`;
      S.innerHTML = '';
    }
  } else {
    T.textContent = `Show ${q(k.name)}'s signed code back`;
    X.textContent = `Once it has signed, the ${d.name} shows the signed transaction as a code. Turn it around to face this computer's camera, then open the camera here.`;
    S.innerHTML = '<div class="row"><button class="btn" id="dev-file-alt" type="button">Or open a signed file…</button></div>';
    $('#dev-file-alt').onclick = () => loadPsbt(null);
  }
}
const devQr = { timer: null, parts: [], i: 0 };
function stopDevQr() { clearInterval(devQr.timer); devQr.timer = null; }
async function playDevQr() {
  stopDevQr();
  try { const r = await invoke('spend_qr'); devQr.parts = (renderSign.devFormat === 'bbqr' ? r.bbqr : null) || r.parts; devQr.i = 0; } catch (e) { return fail(e); }
  const note = $('#dev-qr-note'), plain = Math.round(devQr.parts.length / 2);
  const tick = () => { const cv = $('#dev-qr'); if (!cv) return stopDevQr(); const n = devQr.i++ % devQr.parts.length; drawQr(devQr.parts[n], cv);
    if (note) note.textContent = devQr.parts.length > 1 ? `Frame ${n + 1} of ${devQr.parts.length} · the later frames are spares, so a camera that starts late still finishes.` : 'One code.'; };
  tick(); if (devQr.parts.length > 1) devQr.timer = setInterval(tick, 260);
}
$('#dev-cer-next').onclick = () => { if (devCer.i >= devCer.steps.length - 1) { stopDevQr(); return pscanOpen(); } devCer.i++; devCer.sig = ''; renderSign(); };
$('#dev-cer-back').onclick = () => { if (devCer.i > 0) { devCer.i--; devCer.sig = ''; renderSign(); } };

/// Progress drawn ON the camera: the ceremony's ring in its corner, a segment per plate, share or piece, and a big
/// tick each time one more is read — so the moment a plate registers is unmissable. `p` null clears it.
function scanBar(note, p) {
  const cam = note.parentElement && note.parentElement.querySelector('.cam'); if (!cam) return;
  let st = cam.querySelector('.scanstate');
  if (!p || !p.of) { if (st && !cam.classList.contains('done')) st.remove(); return; }
  if (!st) { st = document.createElement('div'); st.className = 'scanstate'; st.setAttribute('role', 'status'); st.innerHTML = '<svg class="ring corner" viewBox="0 0 120 120" aria-hidden="true"></svg><div class="flash" hidden><svg class="ring big" viewBox="0 0 120 120" aria-hidden="true"></svg><div class="flash-text"></div></div>'; cam.appendChild(st); }
  const word = p.shares ? 'share' : 'piece', was = +(st.dataset.have || 0);
  st.dataset.have = p.have; st.setAttribute('aria-label', `${p.have} of ${p.of} ${word}s read`);
  st.querySelector('.corner').innerHTML = ringMarkup(p.have, p.of, `<text x="60" y="68" text-anchor="middle" class="count">${p.have}/${p.of}</text>`);
  if (p.have > was) {
    const f = st.querySelector('.flash');
    f.querySelector('.big').innerHTML = ringMarkup(p.have, p.of, TICK);
    f.querySelector('.flash-text').textContent = `${p.shares ? 'Share' : 'Part'} read · ${p.have} of ${p.of}`;
    f.hidden = false; f.classList.remove('go'); void f.offsetWidth; f.classList.add('go');
    clearTimeout(f._t); f._t = setTimeout(() => { f.hidden = true; }, 1600);
  }
}
/// Everything is in: the camera stops, and in its place the full ring with a tick — never a black box.
function scanDone(note, of, what) {
  const cam = note.parentElement && note.parentElement.querySelector('.cam'); if (!cam) return;
  // whatever the picture was like before, it read: no hint about it may outlive the success
  note.parentElement.querySelectorAll('.scanhint').forEach(h => { h.hidden = true; });
  cam.classList.add('done');
  let st = cam.querySelector('.scanstate'); if (!st) { st = document.createElement('div'); st.className = 'scanstate'; st.setAttribute('role', 'status'); cam.appendChild(st); }
  st.innerHTML = `<div class="complete"><svg class="ring big ready" viewBox="0 0 120 120" aria-hidden="true">${ringMarkup(of || 1, of || 1, TICK)}</svg><div class="flash-text">${esc(what)}</div><button class="btn small again" type="button">Scan another</button></div>`;
  st.querySelector('.again').onclick = () => { scanReset(note); note.textContent = 'Hold the code in the frame.'; if (st.closest('#tab-scan')) startCam(); else if (st.closest('#dev-one')) devScan(); };
  st.setAttribute('aria-label', what);
}
/// A fresh scan: the camera shows again, with nothing left over from the last one.
function scanReset(note) { if (note && note.parentElement) note.parentElement.querySelectorAll('.scanhint').forEach(h => { h.hidden = true; }); const cam = note && note.parentElement && note.parentElement.querySelector('.cam'); if (!cam) return; cam.classList.remove('done'); const st = cam.querySelector('.scanstate'); if (st) st.remove(); }
function scanShow(note, e) { const p = scanProgress(e); if (!p) return false; note.textContent = p.text; scanBar(note, p); return true; }
/// A device's export read: choose its multisig key, check it belongs in THIS vault, and let it be named.
function devGot(r) {
  const d = devSt.dev, keys = (r && r.keys) || [];
  const k = r.multisig || (keys.length === 1 ? keys[0] : null);
  if (!k) return devSay(keys.length ? `That export holds ${keys.length} keys, none of them the multisig (native segwit) one. On the device, choose its multisig export.` : 'No key found in that.');
  const main = /^xpub/.test(k.xpub);
  if (main !== (setup.network === 'mainnet')) return devSay(`That key is for ${main ? 'mainnet' : 'a test network'}, and this vault is on ${setup.network}. Switch the device (or this vault) so they match.`);
  if (setup.keys.some(x => x.fingerprint === k.fingerprint)) return devSay(`A key from that device (fingerprint ${k.fingerprint}) is already in this vault. One device holding two of its keys is one point of failure, not two.`);
  const path = k.path.replace(/^m\//, ''), expression = `[${k.fingerprint}/${path}]${k.xpub}`;
  const usual = /^48h\/[01]h\/\d+h\/2h$/.test(path);
  // a device key already in another vault keeps its name there (the same fingerprint and xpub)
  const known = store.vaults.flatMap(v => v.keys.map(kk => ({ kk, v }))).find(({ kk }) => kk.pubkey.includes(k.xpub) && kk.name && !/^key \d+/.test(kk.name));
  const taken = new Set(setup.keys.map(x => x.name)); let name = known ? known.kk.name : d.name, i = 2; const base = name; while (taken.has(name)) name = `${base} ${i++}`;
  $('#dev-got').innerHTML = `<div class="ok">Found its key: fingerprint <span class="mono">${esc(k.fingerprint)}</span> · m/${esc(path)} · <span class="mono">${esc(k.xpub.slice(0, 14))}…</span></div>
    ${usual ? '' : `<div class="warnbox">That is not the usual multisig path (m/48h/…/2h). Some signers will not recognise a multisig spend from it. Use it only if you know this device exported it for multisig.</div>`}
    <div class="field"><div class="label">Name this key</div><input id="dev-name" value="${esc(name)}">${known ? `<div class="muted" style="font-size:12.5px">Its name in ${esc(known.v.name)}</div>` : ''}</div>
    <div class="row"><button class="btn primary" id="dev-add" type="button">Add to the vault</button></div>`;
  $('#dev-add').onclick = () => { setup.keys.push({ name: $('#dev-name').value.trim() || name, device: d.id, expression, fingerprint: k.fingerprint, serial: 0 }); devClose(); toast(`${d.name} added`); renderSetup(); };
}
$('#add-device').onclick = () => { devList(); $('#dev-modal').classList.add('on'); };
$('#dev-back').onclick = devList;
$('#dev-close').onclick = devClose;
$('#preset-pick').addEventListener('click', (e) => { const o = e.target.closest('.opt'); if (!o) return; if (setup.keys.length > +o.dataset.n) return toast('More keys already enrolled than that shape holds'); setup.n = +o.dataset.n; setup.k = +o.dataset.k; renderSetup(); });
$('#n-less').onclick = () => { if (setup.n > Math.max(2, setup.keys.length)) { setup.n--; if (setup.k > setup.n) setup.k = setup.n; renderSetup(); } };
$('#n-more').onclick = () => { if (setup.n < 15) { setup.n++; renderSetup(); } };
$('#setup-net-pick').addEventListener('click', (e) => { const o = e.target.closest('.opt'); if (o) { setup.network = o.dataset.net; renderSetup(); } });
$('#setup-finish').onclick = async () => {
  const name = $('#setup-name').value.trim() || 'vault';
  try { vault = setup.keys.some(k => k.expression)
      ? await invoke('vault_create_signers', { name, threshold: setup.k, network: setup.network, keys: setup.keys.map(k => k.expression ? { name: k.name, device: k.device, expression: k.expression, serial: 0 } : { name: k.name, device: k.device || 'yubikey', card: k.pubkey, serial: k.serial }) })
      : await invoke('vault_create', { name, threshold: setup.k, keys: setup.keys.map(k => ({ name: k.name, serial: k.serial, pubkey: k.pubkey })), rotating: true, network: setup.network });
    store = await invoke('store_get'); rememberVault(vault); toast('Vault created'); paintNet(); show('keys'); }
  catch (e) { fail(e); }
};

// single key
$('#single-btn').onclick = () => { $('#single-ack').checked = false; $('#single-go').disabled = true; $('#single-modal').classList.add('on'); };
$('#single-ack').onchange = () => { $('#single-go').disabled = !$('#single-ack').checked; };
$('#single-instead').onclick = () => { $('#single-modal').classList.remove('on'); setup = { k: 2, n: 3, keys: [], network: store.settings.network }; show('setup'); };
$('#single-go').onclick = () => { $('#single-modal').classList.remove('on'); setup = { k: 1, n: 1, keys: [], network: store.settings.network }; show('setup'); };

// ---------- the seal: recovery shares on the keys ----------
function paintSealSlots() {
  if (vault && vault.ext) return;
  const st = renderSeal.st; if (!st || !vault || st.forVault !== vault.name) return;
  const n = st.keys.length;
  st.keys.forEach((key, i) => {
    const slot = $(`[data-seal-slot="${i}"]`); if (!slot) return;
    const here = cards.some(c => c.pubkey === key.pubkey);
    // sealing takes two PINs: the key's own (it locks the share, so holding the key is not enough to read it) and the admin PIN (it writes the card)
    const form = (label, replace) => `<input type="password" autocomplete="off" placeholder="key PIN" data-seal-upin="${i}" aria-label="PIN of ${esc(key.name)}"><input type="password" autocomplete="off" placeholder="admin PIN" data-seal-pin="${i}" aria-label="Admin PIN of ${esc(key.name)}"><button class="btn" data-seal-go="${i}" data-replace="${replace ? 1 : ''}">${label}</button>`;
    slot.title = `share ${i + 1} of ${n}`;
    slot.innerHTML = key.state === 'sealed' ? '<div class="state ok"><i></i>sealed · PIN-locked</div>'
      : key.state === 'open' ? (here ? '<div class="state warnish" title="Anyone holding this key can read its share without a PIN"><i></i>sealed, not locked</div>' + form('Lock it with the PIN', false) : '<div class="state warnish"><i></i>sealed, not locked · plug it in</div>')
      : key.state === 'foreign' ? '<div class="state"><i></i>certificate slot in use · left alone</div>'
      : !here ? '<div class="state"><i></i>seal not confirmed · plug it in</div>'
      : key.state === 'other' ? form('Replace its share', true) : form('Seal this key', false);
  });
  $$('[data-seal-go]').forEach(b => b.onclick = async () => {
    const i = +b.dataset.sealGo, key = st.keys[i], card = cards.find(c => c.pubkey === key.pubkey), pin = $(`[data-seal-pin="${i}"]`).value, upin = $(`[data-seal-upin="${i}"]`).value;
    if (!card) return toast('That key is no longer plugged in');
    if (!upin || !pin) return toast('Enter both: the key’s own PIN locks the share, its admin PIN writes it');
    if (/^[cbdefghijklnrtuv]{32,}$/.test(upin) || /^[cbdefghijklnrtuv]{32,}$/.test(pin)) { $(`[data-seal-upin="${i}"]`).value = ''; $(`[data-seal-pin="${i}"]`).value = ''; return toast('That was the key typing a one-time code, not a PIN. Nothing was sent to the card.', 8000); }
    b.disabled = true; b.textContent = 'writing…';
    try { const r = await invoke('seal_write', { name: vault.name, reader: card.reader, adminPin: pin, pin: upin, replace: !!b.dataset.replace }); toast(`${key.name}: ${r}`, 5000); store = await invoke('store_get'); vault = store.vaults.find(v => v.name === vault.name) || vault; }
    catch (e) { fail(e); }
    $$(`[data-seal-pin="${i}"], [data-seal-upin="${i}"]`).forEach(x => x.value = ''); renderSeal(true);
  });
  $$('[data-seal-pin]').forEach(inp => inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); $(`[data-seal-go="${inp.dataset.sealPin}"]`).click(); } }));
  $$('[data-seal-upin]').forEach(inp => inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); $(`[data-seal-pin="${inp.dataset.sealUpin}"]`).focus(); } }));
}
async function renderSeal(force) {
  if (!vault) return;
  const fc = fixedCards(vault);
  if (fc.length) { const nd = vault.keys.length - fc.length; $('#seal-note').textContent = `Any ${vault.threshold} of these ${vault.keys.length} keys sign. ${nd === 1 ? 'One signs on its own device, from its seed words' : nd + ' sign on their own devices, from their seed words'}; ${fc.length === 1 ? 'the card\u2019s key was made on the card and cannot be copied off it' : 'each card\u2019s key was made on the card and cannot be copied off it'}, so a lost card is a lost key \u2014 which is what the other keys are for. Nothing is sealed on a card in a vault like this: the seed words, the cards, and the wallet descriptor below are how it comes back.`; return; }
  if (vault.ext) { $('#seal-note').textContent = `Any ${vault.threshold} of these ${vault.keys.length} keys sign, each on its own device from its own seed words. Nothing is sealed on a card: the seed words, plus the wallet descriptor below, are how this vault comes back.`; return; }
  const name = vault.name;
  let st; try { st = await invoke('seal_status', { name }); } catch (e) { $('#seal-note').textContent = String(e); return; }
  if (!vault || vault.name !== name) return;
  st.forVault = name; renderSeal.st = st;
  const n = st.keys.length, k = st.threshold;
  $('#seal-note').textContent = st.sealed === n
    ? `Any ${k} of these ${n} keys sign, and any ${k} of them, with their PINs, rebuild this vault on a new computer with no sheet and no file.` + (st.open ? '' : ' Without the PINs the keys give nothing away.')
    : st.sealed ? `Any ${k} of these ${n} keys sign. ${st.sealed} of ${n} are confirmed sealed; recovery from the keys alone needs sealed keys, and you cannot choose which ones you will be left with, so plug in the rest.`
    : `Any ${k} of these ${n} keys sign. Each can also carry a share of this vault's recovery record, locked by its PIN, so that any ${k} of them rebuild the vault on a new computer with nothing else. Sealing takes the key's PIN and its admin PIN once, and never touches its signing key.`;
  if (st.open) $('#seal-note').textContent += ` ${st.open === 1 ? 'One key holds its share' : st.open + ' keys hold their shares'} from before shares were PIN-locked: whoever holds ${k} such keys can read what this vault holds without any PIN. Plug each in and lock it.`;
  if (st.earlier) $('#seal-note').textContent += ` These keys carry shares this computer does not know. Open ${k} of them under New vault › Restore › From the keys and it will offer to adopt them.`;
  if (keysBusy() && !force) return;   // a poll must not wipe a half-typed PIN
  paintSealSlots();
}

// restore, from the keys: shares are remembered across card swaps, so one port is enough
let sealFound = [];                       // shares that are open: read without a PIN (old sealings) or unlocked with one
async function readSeals(force) {
  const sig = cards.map(c => c.serial + ':' + c.pubkey).sort().join(',') + '|' + sealFound.length;
  // the card poll must not wipe a half-typed PIN; the redraw after an unlock is asked for, and the field it would protect is already empty
  if (!force && (sig === readSeals.sig || $$('#seal-read-list input').some(i => i.value || i === document.activeElement))) return; readSeals.sig = sig;
  let r; try { r = await invoke('seal_read'); } catch (e) { $('#seal-read-list').innerHTML = `<div class="warn">${esc(e)}</div>`; readSeals.sig = null; return; }
  const have = (f) => sealFound.some(o => o.entry.id === f.entry.id && o.entry.x === f.entry.x);
  for (const f of r.found) if (!f.entry.locked && !have(f)) sealFound.push(f);
  const lockedHere = cards.filter(c => r.found.some(f => f.serial === c.serial && f.entry.locked && !have(f)));
  const seen = [...new Set(sealFound.map(f => f.serial))];
  $('#seal-read-list').innerHTML =
    lockedHere.map(c => `<div class="keyrow vk"><span class="keyart">${keyArt(c.form || 3, !!c.nfc, 52)}</span><div class="who"><div class="name">${esc(modelName(c.form, c.nfc))} · serial ${c.serial}</div><div class="muted" style="font-size:13px">Its share is locked. The key gives it up for its PIN, the same one it signs with.</div></div><div class="kseal"><input type="password" autocomplete="off" placeholder="key PIN" data-unlock-pin="${esc(c.reader)}" aria-label="PIN of key ${c.serial}"><button class="btn" data-unlock="${esc(c.reader)}">Unlock</button></div></div>`).join('')
    + seen.map(sn => `<div class="keyrow"><svg width="22" height="22"><use href="#ring"/></svg><div class="who"><div class="name">Key ${sn}</div><div class="muted" style="font-size:13px">${sealFound.filter(f => f.serial === sn).length} share(s) open${cards.some(c => c.serial === sn) ? '' : ' · unplugged, remembered'}</div></div><div class="state ok"><i></i>open</div></div>`).join('')
    + (r.cards && !r.found.length ? `<div class="muted" style="font-size:13px">${r.foreign.length ? 'This key\'s certificate object holds something that is not a seal.' : 'The key plugged in now is not sealed. A vault sealed by this app writes a share to each key from the Keys screen.'}</div>` : '')
    + (!r.cards && !seen.length ? '<div class="muted" style="font-size:13px">No key plugged in.</div>' : '');
  $$('[data-unlock]').forEach(b => { const inp = $(`[data-unlock-pin="${CSS.escape(b.dataset.unlock)}"]`);
    const go = async () => { const pin = inp.value; if (!pin) return toast('Enter this key’s PIN');
      if (/^[cbdefghijklnrtuv]{32,}$/.test(pin)) { inp.value = ''; return toast('That was the key typing a one-time code, not a PIN. Nothing was sent to the card.', 8000); }
      b.disabled = true; b.textContent = 'opening…';
      try { const opened = await invoke('seal_unlock', { reader: b.dataset.unlock, pin }); inp.value = ''; for (const f of opened) if (!have(f)) sealFound.push(f); toast(`${opened.length} share${opened.length === 1 ? '' : 's'} opened`); }
      catch (e) { inp.value = ''; fail(e); }
      readSeals(true); };
    b.onclick = go; inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); go(); } }); });
  const first = $('#seal-read-list [data-unlock-pin]'); if (first && (!document.activeElement || document.activeElement === document.body)) first.focus();
  if (!sealFound.length && !r.found.length) return;
  const cands = await invoke('seal_preview', { found: sealFound.concat(r.found.filter(f => f.entry.locked && !have(f))) });
  $('#restore-card').innerHTML = cands.map((c, ci) => c.record ? `<div style="display:flex;flex-direction:column;gap:10px${ci ? ';border-top:1px solid var(--line);padding-top:14px' : ''}">
      <div class="ok">Rebuilt from ${c.need} keys: ${esc(c.record.name)} · ${c.record.threshold} of ${c.record.pubkeys.length} · ${c.record.network}${c.record.chain_code ? ' · rotating addresses' : ' · one fixed address'}</div>
      <div class="muted" style="font-size:13px">Keys: ${c.record.key_names.map(esc).join(' · ')}. First address: <span class="mono">${c.address}</span></div>
      ${c.known_as ? `<div class="muted" style="font-size:13px">Already on this computer as “${esc(c.known_as)}”.</div><button class="btn" data-seal-adopt="${esc(c.known_as)}">Let “${esc(c.known_as)}” learn its shares from these keys</button>` : `<div class="field"><div class="label">Vault name</div><input data-seal-name="${c.id}" value="${esc(c.record.name)}"></div><button class="btn primary" data-seal-restore="${c.id}">Restore this vault</button>`}</div>`
    : `<div class="muted"${ci ? ' style="border-top:1px solid var(--line);padding-top:14px"' : ''}>A vault that needs ${c.need} keys: ${c.have} open${c.locked ? `, ${c.locked} still locked (enter ${c.locked === 1 ? 'its' : 'their'} PIN on the left)` : ''}. ${c.have + c.locked < c.need ? 'Unplug this key and plug in another of the same vault; what was opened is remembered.' : ''}</div>`).join('') || '<div class="muted">Nothing read yet.</div>';
  $$('[data-seal-restore]').forEach(b => b.onclick = async () => {
    const id = b.dataset.sealRestore;
    try { vault = await invoke('seal_restore', { id, name: $(`[data-seal-name="${id}"]`).value, found: sealFound }); store = await invoke('store_get'); rememberVault(vault); sealFound = []; readSeals.sig = null; toast('Vault restored from its keys'); paintNet(); renderTabs(); show('home'); }
    catch (e) { fail(e); }
  });
  $$('[data-seal-adopt]').forEach(b => b.onclick = async () => { try { store = await invoke('seal_adopt', { name: b.dataset.sealAdopt, found: sealFound }); toast('Shares learnt. That vault’s Keys screen now knows its seal.'); } catch (e) { fail(e); } });
}

// ---------- restore ----------
// Every tab of Restore, back to how it looks the first time: no line, no key, no words, no file, nothing read.
// Called on the way in — never on a tab switch within the screen, where what was typed is still wanted.
function restoreFresh() {
  stopCam(); restoreParts.clear(); parsed = null;
  $$('#s-restore input:not([type=checkbox]):not([type=number]):not([type=date]), #s-restore textarea').forEach(i => { if (!i.closest('.seed-grid')) i.value = ''; });
  $$('#s-restore input[type=checkbox]').forEach(c => { c.checked = false; });
  seedClear();
  if (typeof sparkResetEnrollment === 'function') sparkResetEnrollment();
  $$('#s-restore [id$="-note"], #s-restore [id$="-note-off"], #spark-words-count').forEach(n => { n.textContent = ''; });
  $('#restore-card').innerHTML = '<div class="muted">Nothing read yet.</div>';
  if ($('#stmt-preview')) { stmtText = ''; $('#stmt-preview').hidden = true; }
  if ($('#node-preview')) { nodeChecked = null; $('#node-preview').hidden = true; }
  const adv = $('#spark-words-adv'); if (adv) adv.open = false;
  restoreTab.now = 'keys';
}
function restoreTab(name) { if (name !== 'spark' && restoreTab.now === 'spark') { seedClear(); if (sparkSelected || sparkBusy) sparkCancel().catch(() => {}); } restoreTab.now = name; $$('.tabs div').forEach(x => x.classList.toggle('on', x.dataset.tab === name)); for (const t of ['keys', 'scan', 'paste', 'muun', 'spark', 'statement', 'exchange', 'node']) $('#tab-' + t).hidden = t !== name; $('#sheet-note').hidden = !['scan', 'paste'].includes(name);   // it is about a recovery sheet, and only two of these are
  // a result from another way in is not about this one: it goes when the choice changes
  if (restoreTab.last && restoreTab.last !== name) { $('#restore-card').innerHTML = ''; parsed = null; }
  restoreTab.last = name;
  restoreRoom(); if (name === 'scan') startCam(); else stopCam(); if (name === 'keys') { readSeals.sig = null; readSeals(); } }
/// The right-hand card only earns its place once something has been read.
function restoreRoom() { const g = $('#restore-grid'); if (g) g.classList.toggle('alone', !$('#restore-card').innerHTML.trim()); }
const restoreObserver = new MutationObserver(restoreRoom);
$('#restore-card') && restoreObserver.observe($('#restore-card'), { childList: true, subtree: true });
$$('.tabs div').forEach(t => t.onclick = () => restoreTab(t.dataset.tab));
// ---------- a Spark wallet, watched ----------
// Wallet of Satoshi and anything else on Spark. Its address alone reads it: the operators answer balance and
// transfer questions without authentication unless that wallet turned privacy on. Nothing here can spend it, and
// nothing here can check it either — the figures are Spark's word, which the screens say plainly.
$('#spark-read') && ($('#spark-read').onclick = async () => {
  const address = $('#spark-addr').value.trim(), name = $('#spark-name').value.trim() || 'Spark wallet';
  const note = $('#spark-note');
  if (!address) return toast('Paste the wallet\u2019s Spark address');
  note.textContent = 'Asking Spark\u2026';
  try {
    const v = await invoke('spark_restore', { name, address });
    store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v;
    note.textContent = '';
    toast(`Watching \u2018${v.name}\u2019. Its balance and transfers come from Spark, and cannot be checked against the chain.`, 8000);
    renderTabs(); show('home');
  } catch (e) { note.textContent = 'An anonymous read may be empty or private. Open the seed-word section to enrol a YubiKey reader.'; fail(e); }
});

// One box per seed word. Only the box being edited shows its word; every other is masked, so the whole phrase is
// never on the screen at once. Words complete from the BIP 39 list — the same list the parser checks against — and
// each finished word is checked as it is left. When the set is full the checksum is checked and the verdict shown,
// before anything is derived. A paste into any box fills the rest.
let seedWords = null, seedRoot = null;   // the .seed-root the boxes were last built in: Spark's restore step, or a vault's settings
const seedBoxes = () => seedRoot ? [...seedRoot.querySelectorAll('.seed-grid input')] : [];
const seedPhrase = () => seedBoxes().map(i => i.value.trim().toLowerCase()).filter(Boolean).join(' ');
function seedClear() { for (const root of $$('.seed-root')) { root.querySelectorAll('.seed-grid input').forEach(i => { i.value = ''; i.parentElement.className = ''; }); const s = root.querySelector('.seed-suggest'), c = root.querySelector('.seed-count'); if (s) s.innerHTML = ''; if (c) c.textContent = ''; } }
async function seedBuild(n, root) {
  seedRoot = root || seedRoot || $('#spark-words-adv');
  const grid = seedRoot && seedRoot.querySelector('.seed-grid'); if (!grid) return;
  if (!seedWords) { try { seedWords = await invoke('seed_wordlist'); } catch { seedWords = []; } }
  const set = new Set(seedWords);
  grid.innerHTML = Array.from({ length: n }, (_, i) => `<label><span>${i + 1}</span><input type="password" data-i="${i}" spellcheck="false" autocomplete="off" autocapitalize="off" autocorrect="off" aria-label="word ${i + 1}"></label>`).join('');
  const strip = seedRoot.querySelector('.seed-suggest'), count = seedRoot.querySelector('.seed-count');
  const boxes = seedBoxes();
  const mark = (box) => { const w = box.value.trim().toLowerCase(); box.parentElement.className = !w ? '' : set.has(w) ? 'ok' : 'bad'; };
  const verdict = async () => {
    const filled = boxes.filter(b => b.value.trim()).length, bad = boxes.filter(b => b.parentElement.className === 'bad').length;
    if (filled < n) { count.innerHTML = `${filled} of ${n}${bad ? ` \u00b7 <span class="warn">${bad} not in the list</span>` : ''}`; return; }
    // 23 drawn words (Entropia) carry no checksum yet: the 24th is chosen next
    if (n === 23) { count.innerHTML = bad ? `<span class="warn">${bad} not in the list</span>` : '<span class="ok">\u2713 23 drawn words, all on the list \u2014 the 24th is chosen next</span>'; return; }
    try { await invoke('seed_check', { words: seedPhrase() }); count.innerHTML = '<span class="ok">\u2713 all ' + n + ' words are in the list and the checksum matches</span>'; }
    catch (e) { count.innerHTML = `<span class="warn">\u2717 ${esc(String(e && e.message || e))}</span>`; }
  };
  const suggest = (box) => {
    const typing = box.value.trim().toLowerCase();
    const hits = typing && !set.has(typing) ? seedWords.filter(w => w.startsWith(typing)).slice(0, 8) : [];
    strip.innerHTML = hits.map(w => `<button type="button" class="btn small" data-word="${w}">${w}</button>`).join('');
    strip.querySelectorAll('[data-word]').forEach(b => { b.onmousedown = (e) => e.preventDefault(); b.onclick = () => { box.value = b.dataset.word; advance(box); }; });
  };
  const advance = (box) => { mark(box); strip.innerHTML = ''; const next = boxes[+box.dataset.i + 1]; if (next) next.focus(); else box.blur(); verdict(); };
  boxes.forEach(box => {
    box.addEventListener('focus', () => { box.type = 'text'; suggest(box); });
    box.addEventListener('blur', () => { box.type = 'password'; mark(box); verdict(); });
    box.addEventListener('input', () => suggest(box));
    box.addEventListener('keydown', (e) => {
      const first = strip.firstElementChild;
      if ((e.key === 'Tab' || e.key === 'Enter' || e.key === ' ') && !e.shiftKey) {
        if (first && !set.has(box.value.trim().toLowerCase())) { e.preventDefault(); box.value = first.dataset.word; advance(box); }
        else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); advance(box); }
      }
    });
    box.addEventListener('paste', (e) => {
      const words = (e.clipboardData.getData('text') || '').trim().split(/\s+/);
      if (words.length < 2) return;
      e.preventDefault();
      words.forEach((w, k) => { const b = boxes[+box.dataset.i + k]; if (b) { b.value = w.toLowerCase(); mark(b); } });
      verdict();
    });
  });
  count.textContent = '';
}
document.addEventListener('click', (e) => { const o = e.target.closest('.seed-len .opt'); if (!o) return; const root = o.closest('.seed-root'); $$('.seed-len .opt', root).forEach(x => x.classList.toggle('on', x === o)); seedBuild(+o.dataset.n, root); });
$('#spark-words-adv') && $('#spark-words-adv').addEventListener('toggle', () => { const root = $('#spark-words-adv'); if (root.open) { if (!root.querySelector('.seed-grid input')) seedBuild(12, root); else seedRoot = root; } });

// The words unlock the wallet; what comes out is its public identity. They are cleared from the boxes on success
// AND on failure — a failure is not a reason to leave them sitting on a screen.
$('#spark-words-go') && ($('#spark-words-go').onclick = async () => {
  const words = seedPhrase(), note = $('#spark-words-note');
  const name = $('#spark-name').value.trim() || 'Wallet of Satoshi';
  const n = words.split(/\s+/).filter(Boolean).length;
  if (![12, 15, 18, 21, 24].includes(n)) return toast(`Seed words come in 12, 15, 18, 21 or 24 (this has ${n})`);
  note.textContent = 'Working out the identity and asking Spark\u2026';
  try {
    const request = invoke('spark_restore_words', { name, words }); seedClear();
    const v = await request;
    seedClear();
    store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v;
    note.textContent = '';
    toast(`Watching \u2018${v.name}\u2019 as ${v.address.slice(0, 14)}\u2026 The words were not kept.`, 8000);
    renderTabs(); show('home');
  } catch (e) { seedClear(); note.textContent = 'The words were cleared. An empty anonymous answer can also mean private. Enter them again and choose Gate this wallet with a YubiKey for authenticated discovery.'; fail(e); }
});

// ---------- a Muun wallet, watched ----------
// Muun publishes no xpub and no export. Its kit holds both keys, encrypted with the Recovery Code, so the code is
// the only way to read which addresses are yours. What is kept afterwards cannot spend: two account xpubs.
$('#muun-read') && ($('#muun-read').onclick = async () => {
  const key1 = $('#muun-k1').value.trim(), key2 = $('#muun-k2').value.trim(), code = $('#muun-code').value.trim();
  const name = $('#muun-name').value.trim() || 'Muun';
  const note = $('#muun-note');
  if (!key1 || !key2) return toast('Paste both of the kit\u2019s encrypted keys');
  if (!code) return toast('Type the recovery code from the kit');
  note.textContent = 'Reading the kit\u2026';
  try {
    const seen = await invoke('muun_check', { key1, key2 });
    note.textContent = `The kit reads. Unlocking it\u2026${seen.birthday ? ' (this wallet was made at block ' + sats(seen.birthday) + ')' : ''}`;
    const v = await invoke('muun_restore', { name, key1, key2, code, expect: $('#muun-desc').value.trim() || null, names: [] });
    $('#muun-code').value = '';   // the code has done its work and is not kept, on screen or anywhere else
    store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v;
    note.textContent = '';
    toast(`Watching \u2018${v.name}\u2019, keys ${v.muun.fp_user} and ${v.muun.fp_cosign} \u2014 they should be the two fingerprints your kit names. Reading its history from the chain.`, 9000);
    renderTabs(); show('home');
  } catch (e) { note.textContent = ''; fail(e); }
});
async function startCam(deviceId) {
  if (camStream && !deviceId) return;
  if (!deviceId) scanReset($('#cam-note'));   // a fresh scan starts clean; switching camera keeps what is read
  if (camTimer) { clearInterval(camTimer); camTimer = null; }
  if (camStream) { camStream.getTracks().forEach(t => t.stop()); camStream = null; }
  try { camStream = await camOpen(deviceId); $('#cam').srcObject = camStream; camChooser($('#scan-pick2'), camStream, startCam); const reader = makeQrReader($('#cam-note'));
    camTimer = setInterval(() => { const v = $('#cam'); if (!v.videoWidth) return; const data = reader.read(v); if (data) restoreCode(data.trim()); }, 400);
  } catch (e) { $('#cam-note').textContent = 'No camera available. Paste the line instead.'; }
}
// what the restore camera has read: a descriptor as text, or a UR one in parts gathered until it is whole
const restoreParts = new Set(); let restoreBusy = false;
async function restoreCode(t) {
  const note = $('#cam-note');
  if (/^(wsh|wpkh|sh|tr)\(/.test(t)) { stopCam(); note.textContent = 'Read the descriptor. Check it on the right.'; scanDone(note, 1, 'Read the descriptor'); return readDescriptor(t); }
  // a share off a METAL PLATE: the QR carries the share's raw bytes (the scanner hands them over as a
  // byte string), the text side is base32 - either is re-wrapped as ur:envelope by the app and joins as usual
  if (!/^ur:/i.test(t)) {
    const compact = t.replace(/\s+/g, '');
    const asText = /^[A-Za-z2-7]{120,}$/.test(compact), asBytes = t.length >= 60 && /[\x80-\xff]/.test(t);
    if (!asText && !asBytes) return;
    try { t = await invoke('share_rewrap', asBytes ? { bytes: Array.from(t, c => c.charCodeAt(0) & 255), text: null } : { bytes: null, text: compact }); }
    catch (e) { note.textContent = String(e); return; }
  }
  if (!/^ur:(crypto-output|output-descriptor|envelope)\//i.test(t)) return;
  const n = restoreParts.size; restoreParts.add(t.toLowerCase()); if (restoreParts.size === n || restoreBusy) return;
  restoreBusy = true;
  // whole: the camera's work is done — stopped, so a plate still in view does not start a second reading
  try { const all = [...restoreParts].join('\n'), n = restoreParts.size, shares = /^ur:envelope\//i.test(t); await invoke('descriptor_parse', { text: all }); stopCam(); restoreParts.clear();
    const what = shares ? `All ${n} shares read — the recovery line is rebuilt` : n > 1 ? `All ${n} parts read — the whole descriptor` : 'Read the whole descriptor';
    note.textContent = what + '. Check it on the right.'; scanDone(note, n, what); readDescriptor(all); }
  catch (e) { if (!scanShow(note, e)) { note.textContent = String(e); scanBar(note, null); restoreParts.clear(); } }
  restoreBusy = false;
}
function stopCam() { if (camTimer) clearInterval(camTimer); camTimer = null; if (camStream) { camStream.getTracks().forEach(t => t.stop()); camStream = null; } }
$('#desc-read').onclick = () => readDescriptor($('#desc-in').value);
/// One exported key, watched. A zpub or ypub says which kind of address it is for; a plain xpub does not, and
/// wallets export a plain one for all of them — so every kind is watched at once rather than one being guessed.
function readBareKey(key) {
  const kind = /^z|^v/.test(key) ? 'native segwit (BIP 84)' : /^y|^u/.test(key) ? 'wrapped segwit (BIP 49)' : '';
  $('#restore-card').innerHTML = `<div class="ok">Read: one extended key \u2014 a wallet to watch, with no way to spend</div>
    <div class="mono muted" style="background:var(--ground);padding:12px;border-radius:6px;border:1px solid var(--line);overflow-wrap:anywhere">${esc(key.slice(0, 24))}\u2026${esc(key.slice(-8))}</div>
    <div class="muted" style="font-size:13px">${kind ? `This key says it is for <b>${kind}</b> addresses, so that is what will be watched.`
      : 'A plain <span class="mono">xpub</span> does not say which kind of address it is for, and wallets export one for all of them \u2014 so <b>legacy, wrapped segwit, native segwit and taproot are all watched</b>. Guessing one would show a confidently wrong balance.'}</div>
    <div class="field"><div class="label">Call this vault</div><input id="watch-name" value="watched wallet" maxlength="60"></div>
    <button class="btn primary" id="watch-go">Watch this wallet</button>`;
  $('#watch-go').onclick = async () => {
    try {
      const v = await invoke('xpub_restore', { name: $('#watch-name').value.trim() || 'watched wallet', text: key, names: [] });
      store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v;
      renderTabs(); show('home');
    } catch (e) { fail(e); }
  };
}
let parsed = null;
async function readDescriptor(text) {
  try {
    // A bare extended key is what most wallets mean by "export xpub". It is not a descriptor and never will be, so
    // it is offered as what it is: a wallet to watch, with no way to spend.
    const bare = text.trim();
    if (/^(x|y|z|t|u|v)pub[1-9A-HJ-NP-Za-km-z]{50,}$/.test(bare)) return readBareKey(bare);
    parsed = await invoke('descriptor_parse', { text });
    if (parsed.ext) return readExtended(parsed);
    const names = ['laptop', 'keyring', 'off-site'];
    $('#restore-card').innerHTML = `<div class="ok">Read: ${parsed.threshold} of ${parsed.keys.length} over card keys, checksum ok${parsed.rotating ? ', with a chain code (rotating addresses)' : ' (one fixed address)'}</div>
      <div class="mono muted" style="background:var(--ground);padding:12px;border-radius:6px;border:1px solid var(--line)">${parsed.descriptor}</div>
      <div style="display:flex;flex-direction:column;gap:8px">${parsed.keys.map((k, i) => `<div class="row" style="justify-content:space-between"><span class="mono">${k.slice(0, 16)}…</span><input data-kname="${i}" value="${names[i] || 'key ' + (i + 1)}" style="max-width:160px;padding:6px 10px;border:1px solid var(--line);border-radius:6px;background:var(--panel);color:var(--ink)"><span class="${cards.some(c => c.pubkey === k) ? 'ok' : 'muted'}" style="font-size:13px">${cards.some(c => c.pubkey === k) ? 'plugged in' : 'not seen yet'}</span></div>`).join('')}</div>
      <div class="muted" style="font-size:13px">First address on ${netName()}: <span class="mono">${parsed.address}</span>. The vault can be watched now; spending needs ${parsed.threshold} of these keys.</div>
      <div class="field"><div class="label">Vault name</div><input id="restore-name" value="vault"></div>
      <button class="btn primary" id="restore-go">Restore this vault</button>`;
    $('#restore-go').onclick = async () => {
      try { vault = await invoke('vault_restore', { name: $('#restore-name').value.trim() || 'vault', text: parsed.descriptor, names: $$('[data-kname]').map(i => i.value.trim()) }); store = await invoke('store_get'); rememberVault(vault); stopCam(); toast('Vault restored'); paintNet(); show('home'); }
      catch (e) { fail(e); }
    };
  } catch (e) { $('#restore-card').innerHTML = `<div class="warn">${e}</div>`; }
}

// A wallet of extended keys — a SeedSigner multisig, Sparrow, Coldcard — read AS WRITTEN. Its keys are other
// devices' keys, named by the fingerprint and path those devices recognise, never mistaken for cards.
function readExtended(p) {
  const kind = { sortedmulti: 'sortedmulti', multi: 'multi — the order of the keys is part of every address, and is kept', single: 'a single key' }[p.kind] || p.kind;
  $('#restore-card').innerHTML = `<div class="ok">Read: ${p.threshold} of ${p.keys.length}, ${esc(kind)}, over extended keys held by other signers \u2014 checksum ok, a fresh address for every deposit.</div>
    <div class="mono muted" style="background:var(--ground);padding:12px;border-radius:6px;border:1px solid var(--line);word-break:break-all;font-size:12px">${esc(p.descriptor)}</div>
    <div style="display:flex;flex-direction:column;gap:8px">${p.ext.map((k, i) => `<div class="row" style="justify-content:space-between;gap:10px;flex-wrap:wrap"><span><span class="mono">${esc(k.fingerprint)}</span> <span class="muted" style="font-size:12.5px">${esc(k.path)}</span></span><input data-kname="${i}" value="${esc('key ' + (i + 1) + ' \u00b7 ' + k.fingerprint)}" style="max-width:190px;padding:6px 10px;border:1px solid var(--line);border-radius:6px;background:var(--panel);color:var(--ink)"><span class="muted" style="font-size:13px">signs on another device</span></div>`).join('')}</div>
    <div class="muted" style="font-size:13px">First address on ${netName()}: <span class="mono">${esc(p.address)}</span>. Check it is the address your other wallet shows first. From now on this vault is watched here \u2014 balance, activity, coins, addresses \u2014 and a spend is built here and signed on its devices, by their codes or files.</div>
    <div class="field"><div class="label">Vault name</div><input id="restore-name" value="${esc(p.name || 'multisig')}"></div>
    <button class="btn primary" id="restore-go">Watch this vault</button>`;
  $('#restore-go').onclick = async () => {
    try { vault = await invoke('vault_restore', { name: $('#restore-name').value.trim() || 'multisig', text: p.descriptor, names: $$('[data-kname]').map(i => i.value.trim()) }); store = await invoke('store_get'); rememberVault(vault); stopCam(); toast('Watching this vault'); paintNet(); show('home'); }
    catch (e) { fail(e); }
  };
}

/// A vault with signing devices: say, for THIS vault's keys, how a spend gets signed after it is built.
function paintExtSend() {
  const ext = !!(vault && vault.ext);
  $('#ext-send').hidden = !ext;
  if (!ext) return;
  const cardsIn = vault.keys.filter(k => /^0[23][0-9a-f]{64}$/i.test(k.pubkey)), devs = vault.keys.filter(k => k.pubkey.startsWith('['));
  const names = (ks) => ks.map(k => `‘${esc(k.name)}’`).join(ks.length === 2 ? ' and ' : ', ');
  $('#ext-send-text').innerHTML = `Build it here, then sign it with any ${vault.threshold} of the keys on the next screen: `
    + (devs.length ? `${names(devs)} by scanning the code shown there, then showing it the signed code${devs.length > 1 ? 's' : ''} back` : '')
    + (devs.length && cardsIn.length ? '; ' : '')
    + (cardsIn.length ? `${names(cardsIn)} by plugging ${cardsIn.length > 1 ? 'them' : 'it'} in, with ${cardsIn.length > 1 ? 'their PINs' : 'its PIN'} and a touch` : '')
    + '. Nothing is sent until every signature needed is on and you press Send.';
}

// ---------- settings ----------
// Which nav items are shown is chrome, not data: it says nothing about the vault, so it sits with the other
// per-computer preferences rather than in the sealed store.
const coinsTab = () => { try { return localStorage.getItem('a2k-coins-tab') !== 'off'; } catch { return true; } };
function paintCoinsTab() {
  const on = coinsTab();
  $('#nav-coins').hidden = !on;
  $('#coins-switch').classList.toggle('on', on);
  if (!on && $('#s-coins').classList.contains('on')) show('home');
}
// How a vault stands with its drills. `last` is the ISO date the drill wrote; days is how long ago.
function drillState(v) {
  const raw = ((v && v.notes) || {})['vault:drill'] || '';
  const iso = (raw.match(/^\d{4}-\d{2}-\d{2}/) || [])[0] || '';
  const every = store && store.settings ? (store.settings.drill_days === undefined ? 90 : store.settings.drill_days) : 90;
  if (!iso) return { raw, iso: '', every, never: true, due: every > 0, days: null };
  const days = Math.floor((Date.now() - Date.parse(iso + 'T00:00:00Z')) / 86400000);
  return { raw, iso, every, never: false, days, due: every > 0 && days >= every, keys: raw.split('\u00b7')[1] || '' };
}
const drillWords = (d) => d.iso ? new Date(d.iso + 'T00:00:00Z').toLocaleDateString(undefined, { day: 'numeric', month: 'long', year: 'numeric' }) + (d.keys ? ' \u00b7' + d.keys : '') : '';
function paintDrillNudge() {
  const row = $('#drill-nudge');
  if (!vault || vault.keys.length < 2 || vault.ext) { row.hidden = true; return; }   // no cards to read on a vault of other devices' keys
  const d = drillState(vault);
  if (!d.due || paintDrillNudge.hidden === vault.name) { row.hidden = true; return; }
  row.hidden = false;
  $('#drill-nudge-text').innerHTML = d.never
    ? '<b>This vault has never been proved recoverable.</b> A drill reads its cards and checks they really do rebuild it. Nothing is spent.'
    : `<b>It has been ${sats(d.days)} days since this vault was proved recoverable.</b> A key can die, be wiped or lock itself out without saying so.`;
}
$('#drill-nudge-hide').onclick = () => { paintDrillNudge.hidden = vault && vault.name; paintDrillNudge(); };
// ---------- Backup of the keys: the card keys of this vault, and the spare that a key made outside its card can have ----------
function paintBackupKeys() {
  const box = $('#vs-backup'), rows = $('#vs-backup-rows'); if (!box) return;
  const cardKeys = vault ? vault.keys.filter(k => /^0[23][0-9a-f]{64}$/i.test(k.pubkey)) : [];
  box.hidden = !cardKeys.length; if (!cardKeys.length) return;
  const ready = (store && store.key_ready) || {};
  const open = paintBackupKeys.open || {};
  rows.innerHTML = cardKeys.map((k, i) => {
    const r = ready[k.pubkey] || {}, off = !!r.offkey, c = cards.find(c => c.pubkey === k.pubkey), pf = packedForm(k.pubkey);
    const model = c ? modelName(c.form, c.nfc) : pf ? modelName(pf, !!(pf & 0x80), !!(pf & 0x40)) : 'card';
    const spares = cards.filter(c => !c.pubkey);
    const form = open[k.pubkey] ? `<div class="stack" data-backup-form="${esc(k.pubkey)}" style="gap:8px;max-width:560px">
        <div class="muted" style="font-size:13px">The words from the page are checked against this key before anything is written: they must make exactly this key. The other card must be set up (its own PINs) with its signing slot empty; it is written with the same key, so the vault does not change.</div>
        <div class="field seed-root bk-seed"><div class="row" style="justify-content:space-between;align-items:center;flex-wrap:wrap;gap:8px"><div class="label">The words, from the page</div><div class="pick tight seed-len">${[12, 15, 18, 21, 24].map(n => `<div class="opt${n === 24 ? ' on' : ''}" data-n="${n}">${n}</div>`).join('')}</div></div><div class="muted" style="font-size:13px">Only the word you are on is shown; the rest stay hidden. Type the first letters and pick, or paste the whole set into the first box.</div><div class="seed-grid"></div><div class="row seed-suggest" style="gap:var(--s2);flex-wrap:wrap;min-height:34px" aria-live="polite"></div><div class="seed-count" style="font-size:14px" aria-live="polite"></div></div>
        <div class="grid2"><div class="field"><label class="cap">The other card</label><select class="bk-card">${spares.length ? spares.map(c => `<option value="${esc(c.reader)}">${esc(modelName(c.form, c.nfc))} \u00b7 serial ${c.serial}</option>`).join('') : '<option value="">plug in a set-up card with an empty slot</option>'}</select></div>
        <div class="field"><label class="cap">Its admin PIN</label><input type="password" class="bk-admin" autocomplete="off"></div></div>
        <div class="row"><button class="btn primary bk-go" type="button"${spares.length ? '' : ' disabled'}>Write the key to it</button><button class="btn bk-metal" type="button">Engrave the words on metal\u2026</button><button class="btn bk-cancel" type="button">Cancel</button></div><div class="muted bk-note" aria-live="polite"></div></div>` : '';
    return `<div class="vs-backrow" data-backup="${esc(k.pubkey)}"><div><b>${esc(k.name)}</b> <span class="muted">\u00b7 ${esc(model)}${k.serial ? ' \u00b7 serial ' + k.serial : ''}</span></div>
      <div class="muted" style="font-size:13px">${off ? 'Made <b>outside</b> its card at setup: its words are its backup, and can write it to a spare card or onto metal.' : 'Made <b>inside</b> its card: no copy exists. If the card is lost, the other keys replace it through a new vault.'}</div>
      ${off && !open[k.pubkey] ? `<div class="row"><button class="btn small bk-open" type="button">Write it to another card\u2026</button></div>` : ''}${form}</div>`;
  }).join('');
  $$('#vs-backup-rows .bk-open').forEach(b => b.onclick = () => { const pub = b.closest('[data-backup]').dataset.backup; paintBackupKeys.open = { [pub]: true }; paintBackupKeys(); });
  const seedRootHere = $('#vs-backup-rows .bk-seed'); if (seedRootHere && !seedRootHere.querySelector('.seed-grid input')) seedBuild(24, seedRootHere);
  $$('#vs-backup-rows .bk-cancel').forEach(b => b.onclick = () => { paintBackupKeys.open = {}; paintBackupKeys(); });
  $$('#vs-backup-rows .bk-metal').forEach(b => b.onclick = () => { const f = b.closest('[data-backup-form]'), words = [...f.querySelectorAll('.seed-grid input')].map(i => i.value.trim().toLowerCase()).filter(Boolean); if (![12, 15, 18, 21, 24].includes(words.length)) return toast('Type the words from the page first'); const k = vault.keys.find(k => k.pubkey === f.dataset.backupForm); metalEngraveWords(words, `${k ? k.name : 'key'} \u00b7 ${vault.name}`).catch(fail); });
  $$('#vs-backup-rows .bk-go').forEach(b => b.onclick = async () => {
    const f = b.closest('[data-backup-form]'), pub = f.dataset.backupForm, words = [...f.querySelectorAll('.seed-grid input')].map(i => i.value.trim().toLowerCase()).filter(Boolean).join(' '), reader = f.querySelector('.bk-card').value, admin = f.querySelector('.bk-admin').value, note = f.querySelector('.bk-note');
    if (!words.trim()) return toast('Type the words from the page'); if (!reader) return toast('Plug in the other card'); if (!admin) return toast('Enter the other card\u2019s admin PIN');
    b.disabled = true; note.textContent = 'Checking the words, then writing\u2026';
    try { const got = await invoke('card_import', { reader, adminPin: admin, words, expect: pub }); seedClear(); f.querySelector('.bk-admin').value = '';
      const c = cards.find(c => c.reader === reader); paintBackupKeys.open = {}; paintBackupKeys(); toast(`The same key is now on ${c ? modelName(c.form, c.nfc) + ' serial ' + c.serial : 'that card'} too (${got.slice(0, 12)}\u2026). Keep it somewhere else.`, 9000); }
    catch (e) { note.textContent = String(e); b.disabled = false; }
  });
}
function renderVaultSettings() {
  paintShares();
  if (!vault) return;
  { const others = profile && profilesView ? profilesView.profiles.filter(p => p.id !== profile.id) : [];
    $('#vs-move').hidden = !others.length; $('#vs-move-box').hidden = true;
    $('#vs-move-pick').innerHTML = others.map(p => `<div class="opt" data-to="${esc(p.id)}">${esc(p.name)}<small>${PCLASS[p.class][0]}</small></div>`).join(''); $('#vs-move-go').disabled = true; }
  $('#vs-method').value = vault.cost_method || '';
  const own = vault.cost_method, app = store.settings.cost_method || 'utxo';
  $('#vs-method-note').textContent = own
    ? `This vault follows ${COST_METHODS[own].short}, not the app\u2019s ${COST_METHODS[app].short}. That is right only if it belongs to a different taxpayer \u2014 a company\u2019s, a client\u2019s \u2014 whose rules differ. One person cannot use different methods for different wallets to choose the result.`
    : `Follows the app\u2019s method: ${COST_METHODS[app].short}. Change it here only for a vault that belongs to a different taxpayer.`;
  $('#vs-method-note').classList.toggle('warn', !!own);
  const n = vault.keys.length, single = n === 1;
  const drill = (vault.notes || {})['vault:drill'] || '';
  { const box = $('#vs-owner'); box.innerHTML = ''; box.appendChild(entityChips(vault.entity && entityOf(vault.entity) ? vault.entity : null, box, (id) => vaultEntitySet(vault.name, id, renderVaultSettings))); }
  paintExchangeSettings();
  $('#vs-facts').innerHTML = [
    ['Name', esc(vault.name)],
    ['Network', vault.network === 'mainnet' ? 'mainnet \u2014 real coins' : esc(vault.network) + ' \u2014 a test network; these coins are worth nothing'],
    ['Kind', vault.node ? `a ${vault.node.kind === 'cln' ? 'Core Lightning' : 'LND'} node, ${esc(vault.node.alias)}, read with a read-only credential` : vault.exchange ? `an exchange account at ${esc(vault.exchange.provider)}, read from its statements` : vault.statement ? `a closed custodial wallet, read from the statement ${esc(vault.statement.source)} gave` : vault.watch ? `watched \u2014 one exported key, ${vault.watch.kinds.length > 1 ? 'every kind of address' : vault.watch.kinds[0]}${vault.watch.sp ? ', and silent payments' : ''}` : vault.spark ? (vault.spark.authenticated ? 'watched \u2014 Spark, through a YubiKey reader' : 'watched \u2014 Spark') : vault.muun ? 'watched \u2014 a Muun wallet' : single ? 'a single key' : `${vault.threshold} of ${n} keys \u2014 any ${vault.threshold} of them sign`],
    ['Keys', vault.keys.map(k => esc(k.name)).join(' \u00b7 ')],
    ['Made', vault.created ? new Date(vault.created * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'long', year: 'numeric' }) : '\u2014'],
  ].map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
  const def = store.settings.custom ? [store.settings.electrum, store.settings.explorer] : [$('#def-electrum').textContent, $('#def-explorer').textContent];
  $('#vs-servers').innerHTML = [['Electrum server', esc(def[0] || '\u2014')], ['Block explorer', esc(def[1] || '\u2014')], ['Gap limit', sats(store.settings.gap || 20) + ' addresses past the last one used']]
    .map(([k, v]) => `<dt>${k}</dt><dd class="mono" style="font-size:12.5px">${v}</dd>`).join('');
  // hidden until asked for, as on Keys: showing it here in full would undo the care taken there
  const line = vault.ext || vault._recovery || vault.descriptor || '', shown = renderVaultSettings.shown === vault.name;
  $('#vs-desc').textContent = shown ? line : standIn(line); $('#vs-desc').setAttribute('aria-hidden', String(!shown));
  $('#vs-desc-wrap').dataset.hidden = shown ? '0' : '1'; $('#vs-desc-reveal').hidden = shown;
  $('#vs-rewrite').hidden = !vault.ext;
  renderSpCard();
  paintBackupKeys();
  const ds = drillState(vault);
  $('#vs-drill-when').innerHTML = ds.never
    ? '<b>This vault has never been drilled.</b> Until it has, "the keys can recover it" is something you believe rather than something you have seen.'
    : `Last drill: <b>${esc(drillWords(ds))}</b> \u2014 ${ds.days === 0 ? 'today' : sats(ds.days) + ' day' + (ds.days === 1 ? '' : 's') + ' ago'}. ${ds.every ? (ds.due ? 'That is past the ' + sats(ds.every) + ' days you asked for.' : 'The next is due in ' + sats(ds.every - ds.days) + ' day' + (ds.every - ds.days === 1 ? '' : 's') + '.') : 'You have asked not to be reminded.'}`;
  $('#vs-drill-when').classList.toggle('warn', ds.due);
  $('#vs-drill').hidden = single || !!vault.ext || !!(vault.watch || vault.spark || vault.muun || vault.statement || vault.node);
  if (fixedCards(vault).length) { $('#vs-drill-when').classList.remove('warn'); $('#vs-drill-when').textContent = ''; $('#vs-drill-what').textContent = `To check this vault can be recovered: plug in each card \u2014 the Keys screen shows it plugged in only if it holds this vault\u2019s key; on each signing device, load its seed words and confirm it shows the fingerprint on the Keys screen; then load the wallet descriptor below into a wallet and confirm the first address matches this vault\u2019s.`; }
  else if (vault.ext) { $('#vs-drill-when').classList.remove('warn'); $('#vs-drill-when').textContent = ''; $('#vs-drill-what').textContent = `This vault's keys live on other devices, each from its own seed words, so there are no cards here to drill. To check it can be recovered: on each signing device, load its seed words and confirm it shows the fingerprint on this vault's Keys screen; then load the wallet descriptor below into it and confirm the first address matches this vault's.`; }
  else $('#vs-drill-what').textContent = single
    ? 'This vault has one key and no shares to rebuild from: its recovery is the recovery sheet, which you can save below and should keep somewhere else.'
    : 'A drill rebuilds this vault from the cards themselves \u2014 each with its own PIN, exactly as a real recovery would \u2014 and checks that what comes back is this vault and derives the same first address. It reads the cards only. Nothing is spent, nothing is written to the chain, and the vault is not changed.';
}
// The drill keeps its OWN opened shares: borrowing what the Restore screen already opened would report a success
// the cards in front of you did not earn.
let drillFound = [];
const drillSay = (html) => { const o = $('#vs-drill-out'); o.hidden = false; o.innerHTML = html; };
function wireDrill() {
  $$('[data-drill-unlock]').forEach(b => { const inp = $(`[data-drill-pin="${CSS.escape(b.dataset.drillUnlock)}"]`);
    const go = async () => { const pin = inp.value; if (!pin) return toast('Enter this key\u2019s PIN');
      if (/^[cbdefghijklnrtuv]{32,}$/.test(pin)) { inp.value = ''; return toast('That was the key typing a one-time code, not a PIN. Nothing was sent to the card.', 8000); }
      b.disabled = true; b.textContent = 'opening\u2026';
      try { const opened = await invoke('seal_unlock', { reader: b.dataset.drillUnlock, pin }); inp.value = ''; for (const f of opened) if (!drillFound.some(o => o.entry.id === f.entry.id && o.entry.x === f.entry.x)) drillFound.push(f); }
      catch (e) { inp.value = ''; b.disabled = false; b.textContent = 'Unlock'; return fail(e); }
      runDrill(); };
    b.onclick = go; inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); go(); } }); });
}
async function runDrill() {
  if (!vault) return;
  drillSay('<div class="muted">Reading the keys\u2026</div>');
  let r; try { r = await invoke('seal_read'); } catch (e) { return drillSay(`<div class="warn">${esc(String(e))}</div>`); }
  const had = (f) => drillFound.some(o => o.entry.id === f.entry.id && o.entry.x === f.entry.x);
  for (const f of r.found) if (!f.entry.locked && !had(f)) drillFound.push(f);
  const stillLocked = r.found.filter(f => f.entry.locked && !had(f));
  let cands = [];
  try { cands = await invoke('seal_preview', { found: drillFound.concat(stillLocked) }); } catch (e) { return drillSay(`<div class="warn">${esc(String(e))}</div>`); }
  const mine = cands.find(c => c.known_as === vault.name) || cands.find(c => c.address && c.address === vault.address) || cands[0];
  const serials = [...new Set(drillFound.map(f => f.serial))];
  const pinRows = [...new Set(stillLocked.map(f => f.serial))].map(sn => { const c = cards.find(c => c.serial === sn); return c ? `<div class="keyrow vk"><div class="who"><div class="name">Key ${sn}</div><div class="muted" style="font-size:13px">Its share is locked. The drill is only a drill if the PIN opens it.</div></div><div class="kseal"><input type="password" autocomplete="off" placeholder="key PIN" data-drill-pin="${esc(c.reader)}" aria-label="PIN of key ${sn}"><button class="btn" data-drill-unlock="${esc(c.reader)}">Unlock</button></div></div>` : ''; }).join('');
  if (!mine || !mine.record) {
    const need = mine ? mine.need : vault.threshold;
    drillSay(pinRows + `<div class="muted" style="font-size:13px;margin-top:10px">${r.cards ? sats(r.cards) + ' key' + (r.cards === 1 ? '' : 's') + ' plugged in. ' : 'No key plugged in. '}${drillFound.length ? sats(drillFound.length) + ' share(s) open' : 'Nothing open yet'} \u2014 this vault needs ${need}. ${stillLocked.length ? 'Open the locked one(s) above.' : 'Plug in another key of this vault; what has been opened is remembered until you leave this screen.'}</div>`);
    return wireDrill();
  }
  const rec = mine.record, sameAddr = !!(mine.address && mine.address === vault.address);
  const sameShape = rec.threshold === vault.threshold && rec.pubkeys.length === vault.keys.length && rec.network === vault.network;
  const ok = sameAddr && sameShape, untried = vault.keys.length - serials.length;
  drillSay(`<div class="${ok ? 'ok' : 'warn'}" style="font-weight:600;margin-bottom:8px">${ok ? 'Recovered. The cards rebuilt this vault.' : 'It rebuilt something, but not this vault.'}</div>
    <dl class="tx-dl"><dt>Rebuilt from</dt><dd>${sats(mine.need)} key${mine.need === 1 ? '' : 's'}: ${serials.map(x => 'serial ' + x).join(' \u00b7 ')}</dd>
    <dt>What came back</dt><dd>${esc(rec.name)} \u00b7 ${rec.threshold} of ${rec.pubkeys.length} \u00b7 ${esc(rec.network)}</dd>
    <dt>First address</dt><dd class="mono" style="font-size:12px;word-break:break-all">${esc(mine.address || '\u2014')}</dd>
    <dt>Matches this vault</dt><dd class="${ok ? 'ok' : 'warn'}">${ok ? 'yes \u2014 same shape, same network, same first address' : (sameShape ? 'no \u2014 the address it derives is not this vault\u2019s' : 'no \u2014 a different shape or network')}</dd></dl>
    ${untried > 0 ? `<div class="muted" style="font-size:13px">${sats(untried)} of this vault\u2019s ${sats(vault.keys.length)} keys ${untried === 1 ? 'was' : 'were'} not tried. This drill proves the ${sats(serials.length)} used can recover it; it says nothing about the ${untried === 1 ? 'other' : 'others'}. Run it again with a different pair to prove ${untried === 1 ? 'that one' : 'those'} too.</div>` : '<div class="muted" style="font-size:13px">Every key of this vault was read.</div>'}`);
  if (ok) { const when = new Date().toISOString().slice(0, 10) + ` \u00b7 keys ${serials.join(', ')}`;   // ISO first so it can be compared, shown in words
    const html = $('#vs-drill-out').innerHTML;
    try { store = await invoke('note_set', { name: vault.name, key: 'vault:drill', note: when }); vault = store.vaults.find(v => v.name === vault.name) || vault; paintDrillNudge.hidden = null; renderVaultSettings(); paintDrillNudge(); drillSay(html); } catch {} }
}
$('#vs-drill').onclick = () => { drillFound = []; runDrill(); };

// ---------- silent payments on a watched wallet ----------
function renderSpCard() {
  const card = $('#vs-sp-card'); card.hidden = !(vault && vault.watch);
  if (card.hidden) return;
  const sp = vault.watch.sp;
  $('#vs-sp-on').hidden = !sp; $('#vs-sp-off').hidden = !!sp; $('#vs-sp-note').textContent = ''; $('#vs-sp-note-off').textContent = '';
  if (sp) {
    const n = sp.found.length, behind = sp.scanned < sp.from;
    $('#vs-sp-facts').innerHTML = [
      ['Address', '<span class="mono" id="vs-sp-addr" style="font-size:12px;word-break:break-all">…</span>'],
      ['Read', behind ? `not yet — starts at block ${sats(sp.from)} on the next refresh` : `blocks ${sats(sp.from)} to ${sats(sp.scanned)}`],
      ['Found', n ? `${sats(n)} silent payment output${n === 1 ? '' : 's'}` : 'none yet'],
    ].concat(sp.found.map((f, i) => [`#${i}`, `<a href="#" class="mono" data-go-tx="${esc(f.txid)}" style="font-size:12px">${esc(f.txid.slice(0, 12))}\u2026:${f.vout}</a> \u00b7 ${sats(f.value)} sats \u00b7 block ${sats(f.height)}${f.label === 0 ? ' \u00b7 change' : ''}`])).map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
    const me = vault.name; invoke('sp_address', { name: me }).then(a => { const el = $('#vs-sp-addr'); if (el && vault && vault.name === me) el.textContent = a; }).catch(() => {});
  } else {
    $('#vs-sp-oracle').textContent = (store.settings.sp_oracle || '').replace(/^https:\/\//, '') || 'no oracle set in App settings';
    const root = $('#vs-sp-off');
    if (!root.querySelector('.seed-grid input')) seedBuild(12, root); else seedRoot = root;
    if (!$('#vs-sp-made').value) $('#vs-sp-made').value = new Date(Date.now() - 120 * 86400000).toISOString().slice(0, 10);
  }
}
$('#vs-sp-watch').onclick = async () => {
  if (!vault) return;
  seedRoot = $('#vs-sp-off');
  let words = seedPhrase();
  if (![12, 15, 18, 21, 24].includes(words.split(/\s+/).filter(Boolean).length)) return toast('Enter the wallet’s seed words first');
  const made = $('#vs-sp-made').value; if (!made) return toast('Say when the wallet was made');
  $('#vs-sp-watch').disabled = true; $('#vs-sp-note-off').textContent = 'Checking the words are this wallet’s and sealing its scan key…';
  try {
    const request = invoke('sp_watch', { name: vault.name, words, from: made }); words = ''; seedClear();
    const v = await request; store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v;
    renderVaultSettings(); toast('Watching silent payments: the words were cleared, the scan key is sealed. The first read starts now.', 8000);
    vault._coins = undefined; show('home');
  } catch (e) { words = ''; seedClear(); $('#vs-sp-note-off').textContent = String(e); }
  finally { $('#vs-sp-watch').disabled = false; }
};
$('#vs-sp-rescan').onclick = async () => {
  if (!vault) return; const from = $('#vs-sp-from').value; if (!from) return toast('Pick the day to read again from');
  try { const v = await invoke('sp_rescan', { name: vault.name, from }); store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v; renderVaultSettings(); toast('Reading again from that day on the next refresh'); } catch (e) { fail(e); }
};
askTwice($('#vs-sp-forget'), 'Stop watching this wallet’s silent payments? Its scan key is deleted from this computer; the words would be needed to start again. What was found stays in the vault’s history.', 'Yes, stop', async () => {
  try { const v = await invoke('sp_forget', { name: vault.name }); store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v; renderVaultSettings(); vault._coins = undefined; toast('No longer watching silent payments'); } catch (e) { fail(e); }
});
window.__TAURI__.event.listen('sp-scan', (ev) => {
  const p = ev.payload || {}; if (!vault || p.vault !== vault.name) return;
  if (p.error) { toast('Silent payments: ' + p.error, 8000); return; }
  const bar = $('#scanbar'); if (bar) bar.hidden = false;
  const done = p.at - p.from + 1, total = p.to - p.from + 1;
  $('#balance-sub').textContent = `reading blocks for silent payments — ${sats(done)} of ${sats(total)}${p.found ? ', ' + sats(p.found) + ' found' : ''}`;
});
$('#vs-desc-reveal').onclick = () => { renderVaultSettings.shown = vault.name; renderVaultSettings(); };
// the recovery line as SSKR shares: any k of n, one printable sheet each
function paintShares() {
  const kSel = $('#vs-shares-k'), nSel = $('#vs-shares-n');
  const n = +nSel.value || 3, k = Math.min(+kSel.value || 2, n);
  nSel.innerHTML = Array.from({ length: 15 }, (_, i) => i + 2).map(x => `<option value="${x}"${x === n ? ' selected' : ''}>${x}</option>`).join('');
  kSel.innerHTML = Array.from({ length: n - 1 }, (_, i) => i + 2).map(x => `<option value="${x}"${x === Math.max(2, k) ? ' selected' : ''}>${x}</option>`).join('');
  const kk = +kSel.value;
  $('#vs-shares-note').textContent = `${n} sheets, each with one share as a code and as text. Any ${kk} of them rebuild the recovery line; ${kk - 1 === 1 ? 'one alone reveals' : kk - 1 + ' together reveal'} nothing. ${kk === n ? 'Every share is needed: losing one loses the line.' : `Up to ${n - kk} can be lost.`}`;
}
$('#vs-shares-n').onchange = paintShares; $('#vs-shares-k').onchange = paintShares;
$('#vs-shares-go').onclick = async () => {
  try { const dir = await invoke('vault_shares_pdf', { name: vault.name, k: +$('#vs-shares-k').value, n: +$('#vs-shares-n').value }); toast(`Share sheets saved in ${dir}`, 6000); }
  catch (e) { if (!/cancelled/.test(String(e))) fail(e); }
};
$('#vs-copy-desc').onclick = () => navigator.clipboard.writeText(vault.ext || vault._recovery || vault.descriptor || '').then(() => toast('Recovery line copied'));
$('#vs-rewrite-go').onclick = async () => { const text = $('#vs-rewrite-in').value.trim(), name = vault.name; if (!text) return toast('Paste the other wallet\u2019s version first');
  try { store = await invoke('vault_ext_rewrite', { name, text }); vault = store.vaults.find(v => v.name === name) || vault; $('#vs-rewrite-in').value = ''; renderVaultSettings.shown = name; renderVaultSettings(); toast('Same wallet \u2014 its recovery line now reads as your other wallet writes it'); }
  catch (e) { fail(e); } };
$('#vs-pdf').onclick = () => invoke('vault_pdf', { name: vault.name }).then((p) => toast('Saved to ' + p)).catch(fail);
$('#vs-rename').onclick = () => { show('home'); setTimeout(() => $('#title-edit').click(), 80); };
$('#vs-move').onclick = () => { $('#vs-move-box').hidden = false; };
$('#vs-move-cancel').onclick = () => { $('#vs-move-box').hidden = true; };
$('#vs-move-pick').addEventListener('click', (e) => { const o = e.target.closest('.opt'); if (!o) return; $$('#vs-move-pick .opt').forEach(x => x.classList.toggle('on', x === o)); $('#vs-move-go').disabled = false; });
$('#vs-move-go').onclick = async () => { const o = $('#vs-move-pick .opt.on'); if (!o || !vault) return; const to = o.dataset.to; $('#vs-move-go').disabled = true;
  try { const st = await invoke('vault_move_start', { name: vault.name, to }); if (st.missing.length) toast(`${st.missing.length} attachment${st.missing.length === 1 ? ' was' : 's were'} not on this computer and stayed behind`, 8000); await invoke('profile_open', { id: to }); location.reload(); }
  catch (e) { fail(e); $('#vs-move-go').disabled = false; } };
askTwice($('#vs-forget'), 'Forget this vault on this computer? If this profile syncs, it is forgotten on every device it syncs to. The coins stay on the chain; two sealed keys, or the recovery line and two keys, bring it back.', 'Yes, forget it here', async () => { store = await invoke('vault_forget', { name: vault.name }); await refreshTransferLinks(); vault = null; boot(); });
$('#coins-switch').onclick = () => { try { localStorage.setItem('a2k-coins-tab', coinsTab() ? 'off' : 'on'); } catch {} paintCoinsTab(); };
// A change of method changes every realised figure, so every ledger is worked out again.
const methodChanged = () => { vaultLedger.memo = {}; if ($('#s-home').classList.contains('on')) renderHomeSummary(); };
function paintMethodNote() { const m = $('#cost-method').value; $('#cost-method-note').textContent = COST_METHODS[m] ? COST_METHODS[m].note : ''; $('#cost-method-note').classList.toggle('warn', m === 'average'); }
$('#cost-method').onchange = async () => { try { store = await invoke('cost_method_set', { method: $('#cost-method').value }); paintMethodNote(); methodChanged(); toast('Every realised figure is now worked out by ' + COST_METHODS[$('#cost-method').value].short); } catch (e) { fail(e); } };
$('#vs-method').onchange = async () => { const m = $('#vs-method').value || null, name = vault.name;
  try { store = await invoke('vault_cost_method', { name, method: m }); vault = store.vaults.find(v => v.name === name) || vault; methodChanged(); renderVaultSettings(); } catch (e) { fail(e); } };
$('#drill-every').onchange = async () => { try { store = await invoke('drill_every', { days: +$('#drill-every').value }); paintDrillNudge(); } catch (e) { fail(e); } };
function renderSettings() {
  paintCoinsTab();
  $('#cost-method').value = store.settings.cost_method || 'utxo'; paintMethodNote();
  $('#drill-every').value = String(store.settings.drill_days === undefined ? 90 : store.settings.drill_days);
  renderLock(); renderPlaces();
  $$('#net-pick .opt').forEach(o => o.classList.toggle('on', o.dataset.net === store.settings.network));
  const def = store.settings.network === 'mainnet' ? ['ssl://electrum.mineracks.com:50002', 'https://mempool.mineracks.com'] : ['tcp://100.94.4.10:50001', 'https://signet.mineracks.com'];
  $('#def-electrum').textContent = def[0]; $('#def-explorer').textContent = def[1];
  $('#own-servers').classList.toggle('on', !store.settings.custom);
  $('#offline-switch').classList.toggle('on', !!store.settings.offline);
  $('#c-electrum').value = store.settings.electrum || ''; $('#c-explorer').value = store.settings.explorer || '';
  $('#custom-card').style.opacity = store.settings.custom ? 1 : .6;
  $('#gap-in').value = store.settings.gap || 20;
  $('#c-prices').value = store.settings.prices || '';
  $('#c-vss').value = store.settings.vss ?? '';
  $('#c-sp').value = store.settings.sp_oracle ?? '';
  const dev = deviceCurrency(), cur = store.settings.fiat ?? 'auto';
  $('#fiat-pick').innerHTML = `<option value="auto">Automatic: ${dev.ccy} · ${CCYS[dev.ccy] || ''}</option><option value="">None: show sats only</option>` + Object.entries(CCYS).map(([c, n]) => `<option value="${c}">${c} · ${n}</option>`).join('');
  $('#fiat-pick').value = cur;
  $('#fiat-note').textContent = (cur === 'auto' ? `Chosen because ${dev.why}. ` : '') + (store.settings.custom ? (store.settings.prices ? `Prices come from ${store.settings.prices}.` : 'You run your own servers and have named no price server, so no values are shown.') : 'Prices come from prices.mineracks.com: the hour of each transaction is asked for, never an address or an amount.') + ' Test networks show no values.';
  invoke('store_get').then(() => {}); $('#watch-log-path').textContent = profile ? `~/Library/Application Support/Any Two Keys/profiles/${profile.id}/watch.log` : '~/Library/Application Support/Any Two Keys/watch.log';
  renderProfileCard();
}
function renderProfileCard() {
  const card = $('#profile-card'); if (!profile) { card.hidden = true; return; } card.hidden = false;
  $('#pc-pic').innerHTML = `<span class="ic s64 ${PCLASS[profile.class][2]}" data-pc-pick="1" role="button" tabindex="0" title="Change the picture" style="cursor:pointer">${picHtml(profile, 64, false).replace(/^<span[^>]*>|<\/span>$/g, '')}</span>`;
  $('#pc-name').value = profile.name;
  $('#pc-class').textContent = { private: 'A private profile: always asked when it opens, never opens on start, black in screenshots and screen shares, and closed while another display is attached.',
    work: 'A work profile: demo wallets for working on the app. It opens on a click and carries a DEMO tag.', fiction: 'A fiction profile: a made-up person for showing to a room. It opens on a click and carries a DEMO · fictional tag.' }[profile.class];
  $('#pc-start-row').hidden = profile.class === 'private';
  $('#pc-start').classList.toggle('on', !!profile.start); $('#pc-start').setAttribute('aria-checked', String(!!profile.start));
  $('#pc-discreet').classList.toggle('on', !!profile.discreet); $('#pc-discreet').setAttribute('aria-checked', String(!!profile.discreet));
  $('#pc-pic [data-pc-pick]').onclick = (e) => profilePicker(e.currentTarget);
  const pp = $('#pc-persona'); pp.hidden = profile.class !== 'fiction';
  if (profile.class === 'fiction') { invoke('persona_defaults').then(d => { if (!$('#pc-seed').value) $('#pc-seed').value = profile.persona_seed || d.seed; renderPersonaCard.defaults = d; }).catch(() => {});
    $('#pc-persona-say').textContent = profile.persona_seed ? `A persona was generated into this profile from the seed below. Generate again to rebuild it, or change the seed for a different person.` : `Nothing has been generated yet. Generate a made-up person — four entities, two places, two signet vaults, two exchange accounts with years of history — for showing to a room.`;
    $('#pc-generate').textContent = profile.persona_seed ? 'Regenerate' : 'Generate'; }
}
function renderPersonaCard() {}
askTwice($('#pc-generate'), 'Generate a persona into this profile? Everything in it is replaced: every vault, record, entity and place.', 'Generate it', async () => {
  const seed = $('#pc-seed').value.trim(); $('#pc-generate').disabled = true;
  try { const made = await invoke('persona_generate', { seed: seed || null }); store = await invoke('store_get'); vault = store.vaults[0] || null; renderTabs(); profilesView = await invoke('profiles_list').catch(() => profilesView); profile = (profilesView && profilesView.current) || profile;
    const out = $('#pc-persona-out'); out.hidden = false;
    out.innerHTML = `<b>${esc(made.name)}</b> — ${made.entities} entities, ${made.vaults.length} vaults, ${sats(made.exchange_rows)} exchange rows. ` + (made.funded ? 'The signet vaults were funded when this persona was made; the exchange withdrawals match those deposits.' : `These signet addresses need funding for the story to close:<br>${made.fund.map(([v, a, n]) => `<span class="mono" style="font-size:12px">${esc(a)}</span> · ${sats(n)} sats → ${esc(v)}`).join('<br>')}`);
    renderProfileCard(); paintProfileChip(); toast(`${made.name} is here`); }
  catch (e) { fail(e); } finally { $('#pc-generate').disabled = false; } });
function profilePicker(anchor) {
  const row = anchor.closest('.row') || anchor.parentElement;   // under the whole row, never inside it
  let box = row.parentElement.querySelector('.icpick'); if (box) { box.remove(); return; }
  box = document.createElement('div'); box.className = 'icpick';
  const cur = profile.picture || '';
  box.innerHTML = [['lock', 'Lock'], ...GLYPHS].map(([g, w]) => `<span class="ic s40${cur === 'glyph:' + g ? ' on' : ''}" data-glyph="${g}" title="${w}" role="button" tabindex="0"><svg aria-hidden="true"><use href="${glyphRef(g)}"/></svg></span>`).join('')
    + `<button type="button" class="btn small" data-pc-upload>Choose a picture…</button><small class="muted" style="flex-basis:100%">The picture is kept in the clear, like the name.</small>`;
  const busy = (on) => { box.classList.toggle('busy', on); const b = box.querySelector('[data-pc-upload]'); b.disabled = on; b.textContent = on ? 'Reading the picture…' : 'Choose a picture…'; let bar = box.querySelector('.busybar'); if (on && !bar) { bar = document.createElement('div'); bar.className = 'busybar'; bar.setAttribute('role', 'progressbar'); bar.setAttribute('aria-label', 'Reading the picture'); box.appendChild(bar); } if (!on && bar) bar.remove(); };
  const set = async (pic) => { if (pic === 'pick') busy(true);
    try { profile = pic === 'pick' ? await invoke('profile_picture_pick', { id: profile.id }) : await invoke('profile_update', { id: profile.id, picture: pic }); box.remove(); renderProfileCard(); paintProfileChip(); }
    catch (e) { busy(false); if (String(e) !== 'cancelled') fail(e); } };
  $$('[data-glyph]', box).forEach(b => b.onclick = () => set('glyph:' + b.dataset.glyph));
  box.querySelector('[data-pc-upload]').onclick = () => set('pick');
  row.insertAdjacentElement('afterend', box);
}
const pcSet = async (args) => { try { profile = await invoke('profile_update', { id: profile.id, ...args }); renderProfileCard(); paintProfileChip(); } catch (e) { fail(e); } };
$('#pc-save').onclick = () => pcSet({ name: $('#pc-name').value });
$('#pc-start').onclick = () => pcSet({ start: !profile.start }); $('#pc-start').onkeydown = (e) => { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); pcSet({ start: !profile.start }); } };
$('#pc-discreet').onclick = () => pcSet({ discreet: !profile.discreet }); $('#pc-discreet').onkeydown = (e) => { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); pcSet({ discreet: !profile.discreet }); } };
$('#pc-switch').onclick = closeProfile;
askTwice($('#pc-delete'), 'Delete this profile and everything in it? Its key leaves the Keychain first, then its folder. Coins stay on the chain; every record here goes.', 'Delete it', async () => {
  try { const id = profile.id; await invoke('profile_close'); await invoke('profile_delete', { id }); location.reload(); } catch (e) { fail(e); location.reload(); } });
$('#net-pick').addEventListener('click', async (e) => { const o = e.target.closest('.opt'); if (!o) return; store = await invoke('settings_set', { network: o.dataset.net, custom: store.settings.custom, electrum: store.settings.electrum, explorer: store.settings.explorer }); toast('New vaults will use ' + o.dataset.net); renderSettings(); $('#welcome-net').textContent = store.settings.network; });
$('#own-servers').onclick = async () => { store = await invoke('settings_set', { network: netName(), custom: !store.settings.custom, electrum: $('#c-electrum').value, explorer: $('#c-explorer').value }); renderSettings(); };
$('#save-settings').onclick = async () => { try { store = await invoke('settings_set', { network: netName(), custom: true, electrum: $('#c-electrum').value, explorer: $('#c-explorer').value, prices: $('#c-prices').value, vss: $('#c-vss').value, spOracle: $('#c-sp').value }); renderSettings(); toast('Using your servers'); } catch (e) { fail(e); } };
$('#fiat-pick').onchange = async () => { try { store = await invoke('settings_set', { network: netName(), custom: store.settings.custom, electrum: store.settings.electrum, explorer: store.settings.explorer, fiat: $('#fiat-pick').value }); renderSettings(); toast($('#fiat-pick').value ? 'Values shown in ' + fiatCcy() : 'Values hidden'); } catch (e) { fail(e); } };
$('#test-servers').onclick = async () => { $('#test-out').textContent = 'testing…'; try { $('#test-out').textContent = await invoke('servers_test', { electrum: $('#c-electrum').value, explorer: $('#c-explorer').value }); } catch (e) { $('#test-out').textContent = String(e); } };

// ---------- is the node there? a light at the foot of the sidebar ----------
// Green only on a fresh answer from the Electrum server of the network on screen; asked every 30 s, on a vault switch, and on a click.
function paintNode(state, text, title) { const n = $('#node'); n.dataset.state = state; $('#node-text').textContent = text; n.title = title; }
async function checkNode() {
  if (store && store.settings.offline) return paintNode('paused', 'Offline · by choice', 'Nothing leaves Any Two Keys while this is on. Click to go back online.');
  const net = vault ? vault.network : store.settings.network; checkNode.net = net; const mine = ++checkNode.n;
  if ($('#node').dataset.state !== 'online') paintNode('checking', 'Connecting…', 'Asking the ' + net + ' server…');
  try {
    const r = await invoke('node_status', { network: net }); if (mine !== checkNode.n) return;
    const host = r.electrum.replace(/^\w+:\/\//, '');
    paintNode('online', 'Online · ' + (r.ours ? 'mineracks node' : host), `${net} · ${host} · block ${sats(r.height)} · answered in ${r.ms} ms · checked ${new Date().toLocaleTimeString()}. Click to check again.`);
  } catch (e) { if (mine !== checkNode.n) return; paintNode('offline', 'Offline · no answer', `${net}: ${String(e)}. Balances and sends need the server. Click to try again.`); }
}
checkNode.n = 0;
// The light pulses on traffic the app is REALLY making: Rust reports each request as it goes out (net.rs), so this
// is what the app did, not what the window supposed it was doing. Restarting the animation needs the attribute to go
// and come back, hence the reflow.
function netPing(what) {
  const n = $('#node'); if (!n) return;
  n.dataset.last = what || ''; n.removeAttribute('data-busy'); void n.offsetWidth; n.dataset.busy = what || '1';
  clearTimeout(netPing.t); netPing.t = setTimeout(() => n.removeAttribute('data-busy'), 780);
}
window.__TAURI__.event.listen('net', (ev) => netPing(ev.payload));
// how far a scan has got. It cannot say how far there is to go — the search ends when enough addresses in a row come
// back empty — so it says what it has actually done, which is true.
// which operator is being asked what, while a Spark import runs — so a slow answer is seen as a slow answer and
// not as a screen that has stopped
window.__TAURI__.event.listen('spark', (ev) => {
  const p = ev.payload || {}; const what = p.method === 'query_all_transfers' ? 'transfers' : p.method === 'query_balance' ? 'balance' : p.method;
  const line = `asking ${p.operator} for the ${what}\u2026 (a transfer page can take up to a minute)`;
  for (const id of ['#spark-words-note', '#spark-note']) { const n = $(id); if (n && n.textContent && !n.textContent.startsWith('The words')) n.textContent = line; }
});
window.__TAURI__.event.listen('scan', (ev) => {
  const p = ev.payload || {}; if (!vault || p.vault !== vault.name) return;
  const bar = $('#scanbar'); if (bar) bar.hidden = false;
  const seen = p.checked * (p.each || 1);
  $('#balance-sub').textContent = `reading the chain \u2014 ${sats(seen)} ${p.change ? 'change ' : ''}address${seen === 1 ? '' : 'es'} looked at, ${p.used ? sats(p.used) + ' with history' : 'none used yet'}`;
});

const netMenu = (on) => {
  $('#net-menu').hidden = !on; $('#node').setAttribute('aria-expanded', on ? 'true' : 'false');
  if (!on) return;
  const off = !!store.settings.offline;
  $('#net-toggle').textContent = off ? 'Go back online' : 'Work offline';
  $('#net-recheck').hidden = off;
  $('#net-menu-note').textContent = off ? 'Nothing is leaving the app.' : ($('#node').title.split('. Click')[0] || '');
};
async function setOffline(on) {
  try { const name = vault && vault.name; store = await invoke('net_offline', { on }); if (name) vault = store.vaults.find(v => v.name === name) || vault; }
  catch (e) { return fail(e); }
  $('#offline-switch').classList.toggle('on', on);
  if (on) { checkNode(); toast('Working offline. Nothing leaves Any Two Keys until you turn this off.', 7000); }
  else { paintNode('checking', 'Connecting…', ''); checkNode(); toast('Back online.'); }
}
$('#node').onclick = (e) => { e.stopPropagation(); netMenu($('#net-menu').hidden); };
$('#net-menu').onclick = (e) => e.stopPropagation();
$('#net-recheck').onclick = () => { netMenu(false); paintNode('checking', 'Checking…', ''); checkNode(); };
$('#net-toggle').onclick = () => { netMenu(false); setOffline(!store.settings.offline); };
$('#offline-switch').onclick = () => setOffline(!store.settings.offline);
document.addEventListener('click', () => { if (!$('#net-menu').hidden) netMenu(false); });
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && !$('#net-menu').hidden) netMenu(false); });
setInterval(() => { if (!document.hidden) checkNode(); }, 30000);
document.addEventListener('visibilitychange', () => { if (!document.hidden) checkNode(); });

// money moved: say so in the window too. Cards stack, stay ten seconds, and a click opens that vault's Home.
function notice(ev, stay) {
  const el = document.createElement('div'); el.className = 'notice-card ' + (ev.kind || ''); el.setAttribute('role', 'status');
  el.innerHTML = `<div><div class="nt">${esc(ev.title)}</div><div class="nb">${esc(ev.body)}</div></div><button type="button" class="nx" aria-label="Dismiss">×</button>`;
  const close = () => el.remove();
  el.querySelector('.nx').onclick = (e) => { e.stopPropagation(); close(); };
  el.onclick = () => { const v = store.vaults.find(v => v.name === ev.vault); if (v) { vault = v; rememberVault(v); paintNet(); renderTabs(); show('home'); } close(); };
  $('#notices').appendChild(el); while ($('#notices').children.length > 4) $('#notices').firstChild.remove();
  setTimeout(close, stay || 10000);
}
window.__TAURI__.event.listen('vault-event', (ev) => notice(ev.payload));
// the background watcher saw a deposit, a spend or a confirmation: refresh what is on screen
// past prices arrived in the background: redraw the money column from what is now kept (no chain request)
window.__TAURI__.event.listen('prices-filled', async (ev) => {
  if (vault && ev.payload === vault.name && vault._snap) {fillActivity(vault.name, vault._snap);return;}
  const name=ev.payload,f=txFacts[name]; if(!f?.history)return;
  try {const a=await invoke('vault_activity',{name,history:f.history,ccy:f.ccy});const next=keepActivity(name,a);next.history=f.history;await refreshTransferLinks();
    if(vault) {repaintFacts(txFacts[vault.name]);renderHomeSummary();}
  } catch {} // an expired reader stays locked; the next explicit refresh can unlock it
});
window.__TAURI__.event.listen('vault-changed', (ev) => {
  if(txFacts[ev.payload]) txFacts[ev.payload].stale=true;
  for (const [k, l] of Object.entries(LIVE)) if (k.startsWith(ev.payload + '|')) l._stale = true;
  if (!vault || ev.payload !== vault.name) { return; }
  vault._stale = true;
  if ($('#s-home').classList.contains('on')) sync();
  if ($('#s-receive').classList.contains('on')) drawAddress();
});

// sidebar width: drag the divider; remembered per machine
(() => {
  const min = 200, max = 440;
  let w = 220; try { w = Math.min(max, Math.max(min, +localStorage.getItem('a2k-side') || 220)); } catch {}
  document.documentElement.style.setProperty('--side', w + 'px');
  const d = $('#divider'); let dragging = false, startX = 0, startW = w;
  d.addEventListener('mousedown', (e) => { dragging = true; startX = e.clientX; startW = w; d.classList.add('dragging'); document.body.classList.add('resizing'); e.preventDefault(); });
  window.addEventListener('mousemove', (e) => { if (!dragging) return; w = Math.min(max, Math.max(min, startW + e.clientX - startX)); document.documentElement.style.setProperty('--side', w + 'px'); });
  window.addEventListener('mouseup', () => { if (!dragging) return; dragging = false; d.classList.remove('dragging'); document.body.classList.remove('resizing'); try { localStorage.setItem('a2k-side', String(w)); } catch {} });
  d.addEventListener('dblclick', () => { w = 220; document.documentElement.style.setProperty('--side', '220px'); try { localStorage.setItem('a2k-side', '220'); } catch {} });
})();

boot();

// ---------- Spark authenticated discovery, enrolment and reader unlock ----------
let sparkFound = [], sparkSelected = null, sparkCards = [], sparkBusy = false, sparkDiscoverySeq = 0;
function sparkSetBusy(on) {
  sparkBusy = on;
  for (const id of ['spark-gate', 'spark-words-go', 'spark-enroll', 'spark-generate', 'spark-cards-refresh']) $('#' + id).disabled = on;
}
function sparkResetEnrollment() {
  sparkSelected = null; sparkFound = [];
  $('#spark-candidates').innerHTML = ''; $('#spark-enrollment').hidden = true;
  $('#spark-discovery-stop').hidden = true;
  $('#spark-enroll-pin').value = ''; $('#spark-admin-pin').value = '';
  $('#spark-owner-confirm').checked = false; $('#spark-replace-reader').checked = false;
  $('#spark-read').closest('.row').hidden = false; $('#spark-rediscover').hidden = true;
}
// a discovery that is spent — expired, or the wallet was touched — is said so, with the one next action, not a form
// that still looks live
function sparkDiscoverySpent(msg) {
  sparkResetEnrollment();
  $('#spark-words-note').textContent = msg + ' Nothing was saved.';
  $('#spark-rediscover').hidden = false;
}
$('#spark-rediscover').onclick = () => {
  $('#spark-rediscover').hidden = true; $('#spark-words-note').textContent = '';
  $('#spark-words-adv').open = true;
  const first = $('#seed-grid input'); if (first) { first.scrollIntoView({ block: 'center' }); first.focus(); }
};
async function sparkCancel() {
  ++sparkDiscoverySeq; seedClear(); sparkResetEnrollment();
  await invoke('spark_discovery_cancel');
  $('#spark-words-note').textContent = 'Discovery cancelled; temporary owner sessions cleared.';
}
$('#spark-cancel').onclick = () => sparkCancel().catch(fail);
$('#spark-discovery-stop').onclick = () => sparkCancel().catch(fail);
async function sparkLoadCards() {
  const picked = $('#spark-card').value;
  try { sparkCards = await invoke('cards_list'); } catch { sparkCards = []; }
  $('#spark-card').innerHTML = sparkCards.length ? sparkCards.map(c => `<option value="${esc(c.reader)}">${esc(modelName(c.form, c.nfc))} ${c.serial} · ${c.pubkey ? 'key present · touch ' + c.touch : 'empty or unsupported slot'}</option>`).join('') : '<option value="">Plug in a key and refresh</option>';
  if (sparkCards.some(c => c.reader === picked)) $('#spark-card').value = picked;
}
$('#spark-cards-refresh').onclick = () => sparkLoadCards().catch(fail);
$('#spark-gate').onclick = async () => {
  if (sparkBusy) return;
  let words = seedPhrase();
  if (![12,15,18,21,24].includes(words.split(/\s+/).filter(Boolean).length)) return toast('Enter the wallet’s seed words first');
  const seq = ++sparkDiscoverySeq;
  sparkResetEnrollment(); sparkSetBusy(true); $('#spark-discovery-stop').hidden = false;
  $('#spark-words-note').textContent = 'Signing in as the owner to check the selected accounts. No wallet settings are changed during discovery.';
  try {
    const request = invoke('spark_discover', { words, start: Number($('#spark-account-start').value), count: Number($('#spark-account-count').value), scheme: $('#spark-scheme').value, expected: $('#spark-expected').value.trim() });
    words = ''; seedClear();
    const result = await request;
    if (seq !== sparkDiscoverySeq) return;
    sparkFound = result.wallets;
    $('#spark-words-note').textContent = `Checked ${result.checked} account(s). The seed words were cleared. ` + (sparkFound.length ? 'Select the identity you recognise; temporary owner sessions expire within ten minutes.' : 'No wallet with funds or history matched. This does not prove your wallet is empty: check the identity and derivation options.') + (result.errors.length ? ` ${result.errors.length} checks failed: ${result.errors.join(' · ')}` : '');
    $('#spark-candidates').innerHTML = sparkFound.map((c, i) => `<button class="btn" data-spark-candidate="${i}">Select \u2192 Account ${c.account} · ${sats(c.balance)} sats · ${c.private ? 'private' : 'public'}<br><span class="mono">${esc(c.identity)}</span></button>`).join('');
    $$('[data-spark-candidate]').forEach(b => b.onclick = async () => {
      if (sparkBusy) return;
      sparkSelected = sparkFound[Number(b.dataset.sparkCandidate)]; const c = sparkSelected;
      $('#spark-selected').textContent = `Selected account ${c.account}: ${sats(c.balance)} sats. Identity ${c.identity}. ${c.balance === 0 ? 'This identity has transfer history but no available balance.' : 'Compare this with the wallet you control.'} ${c.existing_reader ? 'An existing designated reader is set: ' + c.existing_reader : 'No designated reader is set.'}`;
      $('#spark-owner-confirm').checked = false; $('#spark-replace-reader').checked = false; $('#spark-make-private').checked = false;
      $('#spark-replace-row').hidden = !c.existing_reader;
      $('#spark-enroll-name').value = $('#spark-enroll-name').value || $('#spark-name').value;
      $('#spark-read').closest('.row').hidden = true;   // one next action: the anonymous read steps out of the way
      $('#spark-enrollment').hidden = false; $('#spark-enroll-note').textContent = '';
      await sparkLoadCards();
    });
  } catch (e) { if (seq === sparkDiscoverySeq) $('#spark-words-note').textContent = 'The words were cleared. ' + String(e); }
  finally { words = ''; seedClear(); sparkSetBusy(false); }
};
$('#spark-generate').onclick = async () => {
  if (sparkBusy) return;
  const c = sparkCards.find(c => c.reader === $('#spark-card').value);
  if (!c) return toast('Plug in and select a key');
  if (c.pubkey) return toast('That slot already holds a key. It will not be replaced.');
  let adminPin = $('#spark-admin-pin').value; $('#spark-admin-pin').value = '';
  if (!adminPin) return toast('Enter the card’s admin PIN');
  sparkSetBusy(true); $('#spark-enroll-note').textContent = 'Checking the empty slot, generating its reader key and locking touch on…';
  try { const request = invoke('spark_reader_generate', { reader: c.reader, serial: c.serial, adminPin }); adminPin = ''; await request; await sparkLoadCards(); $('#spark-enroll-note').textContent = 'Reader key generated on the card; physical touch is locked on.'; }
  catch (e) { $('#spark-enroll-note').textContent = String(e); await sparkLoadCards(); }
  finally { adminPin = ''; sparkSetBusy(false); }
};
$('#spark-enroll').onclick = async () => {
  if (sparkBusy || !sparkSelected) return;
  if (!$('#spark-owner-confirm').checked) return toast('First confirm that you recognise the discovered wallet');
  const c = sparkCards.find(c => c.reader === $('#spark-card').value);
  if (!c?.pubkey || c.touch !== 'FIXED') return toast('Choose a secp256k1 key with touch locked to FIXED');
  if (sparkSelected.existing_reader && sparkSelected.existing_reader !== c.pubkey && !$('#spark-replace-reader').checked) return toast('This wallet already has a reader; explicitly allow its replacement to continue');
  let pin = $('#spark-enroll-pin').value; $('#spark-enroll-pin').value = '';
  if (!pin) return toast('Enter the reader PIN');
  sparkSetBusy(true); $('#spark-cancel').disabled = true; $('#spark-discovery-stop').disabled = true;
  $('#spark-enroll-note').textContent = 'Touch the YubiKey when it flashes. Then Spark will register the reader and verify its access.';
  try {
    const request = invoke('spark_enroll', { candidateId: sparkSelected.id, name: $('#spark-enroll-name').value.trim() || $('#spark-name').value.trim() || 'Wallet of Satoshi', reader: c.reader, pubkey: c.pubkey, pin, makePrivate: $('#spark-make-private').checked, replaceReader: $('#spark-replace-reader').checked });
    pin = ''; const v = await request;
    sparkResetEnrollment(); store = await invoke('store_get'); vault = store.vaults.find(x => x.name === v.name) || v;
    toast('Reader enrolled and verified. Only public references were saved; the seed was not kept.', 8000);
    renderTabs(); show('home');
  } catch (e) {
    // spent (expired, locked, or the wallet may already have changed) → the explicit state; anything before that point
    // (wrong PIN, wrong card, a name clash) keeps the discovered wallet selected for another go
    const msg = String(e);
    if (/discovery expired|find the wallet again|discover again|discover and review again|app locked/i.test(msg)) sparkDiscoverySpent(msg);
    else { $('#spark-enroll-note').textContent = msg + ' The discovered wallet is still selected: check the card and PIN and try again.'; $('#spark-enroll-pin').focus(); }
  }
  finally { pin = ''; sparkSetBusy(false); $('#spark-cancel').disabled = false; $('#spark-discovery-stop').disabled = false; }
};
window.__TAURI__.event.listen('spark-discovery', ev => { if (sparkBusy) $('#spark-words-note').textContent = `Checking account ${ev.payload.account} (${ev.payload.checked} of ${ev.payload.count})… No wallet settings are changed during discovery.`; });
let sparkUnlockPending = null;
async function ensureSparkSession(v) {
  if (!v.spark?.authenticated) return true;
  if (await invoke('spark_session_status', { name: v.name })) return true;
  if (sparkUnlockPending) return sparkUnlockPending.name === v.name ? sparkUnlockPending.promise : false;
  let resolve; const promise = new Promise(r => { resolve = r; });
  sparkUnlockPending = { name: v.name, v, promise, resolve, busy: false, focus: document.activeElement };
  $('#spark-unlock-title').textContent = `Open the reader for ${v.name}`;
  $('#spark-unlock-description').textContent = `This wallet is read through a YubiKey registered as its reader. Its PIN and a touch open a temporary read session \u2014 it can look, not spend \u2014 which refreshes use until it expires or the app locks.`;
  $('#spark-unlock-note').textContent = ''; $('#spark-unlock-pin').value = '';
  $('#spark-unlock-modal').classList.add('on');
  sparkUnlockStage('plug', null);
  await sparkUnlockCards();
  // the key is looked for on its own, the way the lock screen does: no "look again" button to find
  sparkUnlockPending.timer = setInterval(() => { if (sparkUnlockPending && !sparkUnlockPending.busy) sparkUnlockCards().catch(() => {}); }, 1500);
  return promise;
}
// The same three steps as signing — plug in, PIN, touch — with the same pictures, so opening a reader looks like
// every other thing a key does here. The software steps back for the touch: no PIN box, no buttons, one line.
function sparkUnlockStage(step, card) {
  const p = sparkUnlockPending; if (!p) return;
  const sp = p.v.spark, form = (card && card.form) || p.form || 3, nano = [2, 4].includes(form & 0x0F);
  $$('#spark-unlock-steps li').forEach(li => { const order = ['plug', 'pin', 'touch'], i = order.indexOf(li.dataset.step), n = order.indexOf(step); li.classList.toggle('now', i === n); li.classList.toggle('done', i < n); });
  const g = $('#spark-unlock-guide'), sig = step + ':' + (form & 0x0F); if (g._sig !== sig) { g._sig = sig; g.innerHTML = guideArt(step, form); }
  const name = store.vaults.flatMap(v => v.keys).find(k => k.pubkey === sp.reader_pubkey)?.name || ((lockInfo && lockInfo.cards) || []).find(k => k.pubkey === sp.reader_pubkey)?.name || 'The reader key';
  const keyBox = $('#spark-unlock-key');
  if (card) {
    keyBox.className = 'lock-key'; keyBox.style.gridTemplateColumns = 'auto minmax(0,1fr)';
    keyBox.innerHTML = `${keyArt(form, !!card.nfc, 40)}<b>${esc(name)}<small>YubiKey ${esc(String(card.serial))} \u00b7 <span style="color:var(--signal)">\u2713 plugged in \u2014 the registered reader</span></small></b>`;
  } else {
    keyBox.className = 'lock-key off'; keyBox.style.gridTemplateColumns = 'auto minmax(0,1fr)';
    keyBox.innerHTML = `${keyArt(form, false, 40)}<b>${esc(name)}<small>YubiKey ${esc(String(sp.reader_card))} \u00b7 not plugged in${p.other ? ` \u2014 the key that is (${esc(String(p.other))}) is not this wallet\u2019s reader` : ''}</small></b>`;
  }
  $('#spark-unlock-pin-row').hidden = step === 'touch'; $('#spark-unlock-actions').hidden = step === 'touch'; $('#spark-unlock-warn').hidden = step !== 'pin';
  $('#spark-unlock-pin').disabled = step !== 'pin';
  $('#spark-unlock-go').disabled = step !== 'pin' || !$('#spark-unlock-pin').value;
  $('#spark-unlock-act').textContent = step === 'plug' ? `Plug in ${name === 'The reader key' ? 'the registered key' : name}` : step === 'pin' ? 'Enter its PIN' : `Touch the ${nano ? 'nano' : 'key'} now`;
  $('#spark-unlock-text').textContent = step === 'plug' ? `The reader is YubiKey ${sp.reader_card}. It is looked for as soon as it is in.`
    : step === 'pin' ? 'Then press Open: the key will ask for a touch.'
    : `${nano ? 'It runs along the key\u2019s top edge. Touch it' : 'Touch it'} when its light flashes. Keep the key plugged in.`;
}
async function sparkUnlockCards() {
  const p = sparkUnlockPending; if (!p) return;
  const sp = p.v.spark;
  let cs = []; try { cs = await invoke('cards_list'); } catch {}
  if (!sparkUnlockPending || sparkUnlockPending !== p || p.busy) return;
  const mine = cs.find(c => c.pubkey === sp.reader_pubkey && String(c.serial) === sp.reader_card && c.touch === 'FIXED');
  p.other = !mine && cs.length ? cs[0].serial : null;
  if (mine) p.form = mine.form || p.form;
  $('#spark-unlock-card').innerHTML = mine ? `<option value="${esc(mine.reader)}">YubiKey ${mine.serial}</option>` : '';
  const was = p.present; p.present = !!mine;
  sparkUnlockStage(mine ? 'pin' : 'plug', mine || null);
  if (mine && !was) $('#spark-unlock-pin').focus();
}
function sparkUnlockClose(ok) {
  const p = sparkUnlockPending; if (!p || p.busy) return;
  clearInterval(p.timer);
  $('#spark-unlock-pin').value = ''; $('#spark-unlock-modal').classList.remove('on'); sparkUnlockPending = null;
  if (p.focus?.isConnected) p.focus.focus(); p.resolve(ok);
}
$('#spark-unlock-pin').addEventListener('input', () => { const p = sparkUnlockPending; if (p && !p.busy) $('#spark-unlock-go').disabled = !p.present || !$('#spark-unlock-pin').value; });
$('#spark-unlock-cancel').onclick = () => sparkUnlockClose(false);
$('#spark-unlock-form').onsubmit = async e => {
  e.preventDefault(); const p = sparkUnlockPending; if (!p || p.busy) return;
  const reader = $('#spark-unlock-card').value; let pin = $('#spark-unlock-pin').value; $('#spark-unlock-pin').value = '';
  if (!reader) return toast('Plug in the registered key first');
  if (!pin) return toast('Enter the key\u2019s PIN');
  // a brushed key with Yubico OTP on types a code, not a PIN: nothing is sent, no try is used
  if (/^[cbdefghijklnrtuv]{32,}$/.test(pin)) { pin = ''; return toast('That was the key typing a one-time code, not a PIN. Nothing was sent to the card and no PIN try was used. Turn off Yubico OTP on this key to stop it happening.', 9000); }
  p.busy = true; $('#spark-unlock-note').textContent = '';
  sparkUnlockStage('touch', { serial: p.v.spark.reader_card, form: p.form, nfc: false });
  let opened = false;
  try { const request = invoke('spark_unlock', { name: p.name, reader, pin }); pin = ''; await request; opened = true; }
  catch (e) { $('#spark-unlock-note').textContent = String(e) + ' Check the key before trying again.'; }
  finally { pin = ''; p.busy = false; if (opened) sparkUnlockClose(true); else { sparkUnlockStage(p.present ? 'pin' : 'plug', p.present ? { serial: p.v.spark.reader_card, form: p.form, nfc: false } : null); $('#spark-unlock-pin').focus(); } }
};
document.addEventListener('keydown', e => {
  if (!sparkUnlockPending) return;
  if (e.key === 'Escape') { e.preventDefault(); sparkUnlockClose(false); }
  if (e.key === 'Tab') {
    const focusable = [...$('#spark-unlock-form').querySelectorAll('button:not(:disabled),input:not(:disabled)')].filter(el => !el.closest('[hidden]'));
    const first = focusable[0], last = focusable[focusable.length - 1];
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  }
});

function sparkTransferStatus(status) { return status === 5 ? "completed · Spark" : status === 6 ? "expired · Spark" : status === 7 ? "returned · Spark" : "not completed · Spark"; }
