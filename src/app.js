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
const fmtIndex = (i) => i >= CHANGE ? 'change #' + (i - CHANGE) : '#' + i;
const PEN = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/></svg>';
const labelOf = (key) => (vault && vault.labels && vault.labels[key]) || '';
const esc = (t) => String(t).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/"/g, '&quot;');
// an inline label: text plus a hover pencil; click → field; Enter/blur saves through label_set
function lblHtml(key) { return `<span class="lbl" data-lkey="${esc(key)}"><span class="text">${esc(labelOf(key))}</span><button class="pencil" title="Label">${PEN}</button></span>`; }
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
function toast(t, ms = 3500) { const el = $('#toast'); el.textContent = t; el.classList.add('on'); clearTimeout(el._t); el._t = setTimeout(() => el.classList.remove('on'), ms); }
function fail(e) { toast(String(e && e.message || e), 6000); console.error(e); }

// ---------- screens ----------
function show(name) {
  const first = ['setup', 'restore'].includes(name) && !vault;
  $('#welcome').hidden = true; $('#shell').hidden = false;
  $$('.screen').forEach(s => s.classList.toggle('on', s.id === 's-' + name));
  const navName = name === 'sign' ? (paintCrumbs.via || 'send') : name === 'tx' ? 'home' : name;   // Sign has no sidebar entry of its own: light the screen it came from
  $$('.nav[data-go]').forEach(n => n.classList.toggle('on', n.dataset.go === navName));
  // the vault you are in stays lit on every screen that is about it, matching the breadcrumb; not while making another, nor in app settings
  const inVault = vault && !['setup', 'restore', 'settings', 'overview'].includes(name);
  $$('#vault-tabs .nav').forEach(n => n.classList.toggle('on', !!inVault && n.dataset.vault === vault.name));
  $('#title').textContent = { overview: 'All vaults', home: vault ? vault.name : 'Vault', activity: 'Activity', send: 'Send', receive: 'Receive', coins: 'Coins', addresses: 'Addresses', vault: 'Vault settings', keys: 'Keys', settings: 'App settings', setup: 'New vault · enrol keys', restore: 'Restore a vault', sign: 'Sign', messages: 'Messages' }[name] || '';
  $('#title-edit').hidden = !(name === 'home' && vault); $('#title-input').hidden = true; $('#title').hidden = false;
  paintCrumbs(name); paintStaged(name);
  $('#side-vault').textContent = vault ? vault.name : 'No vault yet'; $('#side-vault').title = vault ? `These screens are about “${vault.name}”` : '';
  if (checkNode.net !== (vault ? vault.network : store.settings.network)) checkNode();
  if (!['tx', 'addresses', 'coins'].includes(name) && $('#tx-notes').parentElement !== recHome) recHome.appendChild($('#tx-notes'));
  if (name !== 'restore') stopCam();
  if (name !== 'sign') stopQr();
  if (name !== 'keys') hideDesc();
  if (name === 'home') { syncIfStale(); labelsSync(); paintDrillNudge(); }
  if (name === 'receive') drawAddress();
  if (name === 'activity') { if (vault && !vault._coins) sync(); else paintActivity(); }
  if (name === 'addresses') { $('#addr-detail').hidden = true; renderAddresses(); }
  if (name === 'coins') { $('#coins-detail').hidden = true; if (vault && !vault._coins) sync().then(() => Promise.resolve(renderAddresses()).then(renderCoins2)); else Promise.resolve(renderAddresses()).then(renderCoins2); }
  if (name === 'vault') renderVaultSettings();
  if (name === 'overview') renderOverview();
  if (name === 'send') { closeBook(); paintTo(); loadFees(); if (vault && !vault._coins) sync().then(renderCoins); else renderCoins(); paintExtSend(); }
  if (name === 'messages') renderMessages();
  if (name === 'keys') renderKeys();
  if (name === 'tx') renderTx();
  if (name === 'setup') renderSetup();
  if (name === 'restore') restoreTab(restoreTab.now || 'keys');
  if (name === 'settings') renderSettings();
  if (name === 'sign') renderSign();
  void first;
}
document.addEventListener('click', (e) => { const go = e.target.closest('[data-go]'); if (go) { if (go.dataset.k) { setup = { k: +go.dataset.k, n: +go.dataset.n, keys: [], network: store.settings.network }; } show(go.dataset.go); } });
// where you are, and the way back: Vaults › this vault › Send › Sign
function paintCrumbs(name) {
  const trail = [];
  if (['setup', 'restore'].includes(name)) trail.push(['Vaults', store.vaults.length ? 'welcome' : null]);
  else if (name !== 'settings' && vault) {
    if (name !== 'home') trail.push([vault.name, 'home']);
    if (name === 'sign') trail.push(paintCrumbs.via === 'messages' ? ['Messages', 'messages'] : ['Send', 'send']);
    if (name === 'tx') trail.push(['Activity', 'home']);
  }
  if (name === 'settings') trail.push(['Any Two Keys', null]);
  const here = { home: '', send: 'Send', receive: 'Receive', keys: 'Keys', settings: 'App settings', setup: 'New vault', restore: 'Restore', sign: 'Sign', messages: 'Messages', tx: name === 'tx' && vault && txView.txid ? txHeadline() : '' }[name] || '';
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
function showWelcome() { $('#shell').hidden = true; $('#welcome').hidden = false; $('#welcome-back').hidden = !store.vaults.length; }
$('#new-vault-nav').onclick = showWelcome;
$('#welcome-back').onclick = () => { if (vault) show('home'); };

function netName() { return vault ? vault.network : store.settings.network; }
function paintNet() { const p = $('#netpill'); p.classList.toggle('signet', netName() === 'signet'); p.querySelector('span').textContent = netName(); $('#welcome-net').textContent = store.settings.network; renderTabs(); }
function renderTabs() {
  const tabs = $('#vault-tabs'); tabs.innerHTML = '';
  for (const v of store.vaults) {
    const d = document.createElement('div'); d.className = 'nav' + (vault && v.name === vault.name ? ' on' : ''); d.style.fontWeight = '600'; d.dataset.vault = v.name;
    d.innerHTML = `<span style="width:8px;height:8px;border-radius:50%;background:${v.network === 'mainnet' ? 'var(--gold)' : 'var(--signal)'};display:inline-block;flex:0 0 8px"></span><span style="flex-grow:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap" title="${esc(v.name)}">${esc(v.name)}</span><span class="muted" style="font-size:11px;font-weight:400">${v.keys.length === 1 ? '1 key' : v.threshold + ' of ' + v.keys.length}</span>`;
    d.onclick = () => { vault = v; rememberVault(v); paintNet(); show('home'); };
    tabs.appendChild(d);
  }
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
async function boot() {
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
  if (vault) show('home'); else showWelcome();
  pollCards(); idleLock(); invoke('location_status').catch(() => {});   // the setting lives in the sealed store, so listening can only start now
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
  setTimeout(pollCards, 2000);
}
function cardLabel(c) { return `YubiKey 5 · serial ${c.serial} · ${c.version}`; }

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
  let d; try { d = await invoke('tx_detail', { name, txid, height: txView.height }); } catch (e) { if (mine === txView.seq) { $('#tx-dl').innerHTML = `<dt>Details</dt><dd class="warn">${esc(e)}</dd>`; $('#tx-proof').textContent = ''; } return; }
  if (mine !== txView.seq || !vault || vault.name !== name) return;   // the person has moved on; this answer is for another screen
  const conf = d.height > 0 ? Math.max(1, d.tip - d.height + 1) : 0, st = $('#tx-status');
  st.className = 'state ' + (conf ? 'ok' : 'here'); st.querySelector('span').textContent = conf ? `confirmed · ${sats(conf)} block${conf === 1 ? '' : 's'} deep` : 'in the mempool';
  if (!f) { amt.className = 'tx-amt' + (d.net > 0 ? ' in' : ''); amt.innerHTML = `${d.net > 0 ? '+' : d.net < 0 ? '−' : ''}${sats(Math.abs(d.net))} <small>sats</small>`; }
  const book = bookOf(), who = (a) => { const c = a && book.find(c => c.address.toLowerCase() === a.toLowerCase()); return c ? `<span class="tag who">${esc(c.name)}</span>` : ''; };
  const place = (i) => `<span class="tag ours">${esc(vault.name)} · ${i >= CHANGE ? 'change #' + (i - CHANGE) : 'receive #' + i}</span>`;
  const outLine = (o) => `<div><span class="mono">${esc(o.address || o.kind)}</span>${o.ours !== null ? place(o.ours) : who(o.address)}<small>${sats(o.sats)} sats</small></div>`;
  const oursIn = d.inputs.filter(i => i.ours !== null), foreignOut = d.outputs.filter(o => o.ours === null), oursOut = d.outputs.filter(o => o.ours !== null);
  const signers = [...new Set(oursIn.flatMap(i => i.signers))];
  const rows = [];
  if (oursIn.length) {            // a spend from this vault
    rows.push(['To', foreignOut.length ? foreignOut.map(outLine).join('') : 'Nowhere outside this vault: the coins moved between its own addresses.']);
    if (oursOut.length) rows.push(['Change', oursOut.map(outLine).join('')]);
    rows.push(['Fee', d.fee !== null ? `${sats(d.fee)} sats<small>${d.fee_rate.toFixed(2)} sat/vB · ${sats(d.vsize)} vB</small>` : 'not known']);
    rows.push(['Signed by', signers.length ? signers.map(esc).join(' and ') + `<small>read from the signatures on the transaction</small>` : 'could not be read from the witness']);
    const rl = realisedRow(name, txid); if (rl) rows.push(['Realised', rl]);
  } else {                        // a deposit
    rows.push(['From', `outside this vault · ${d.inputs.length} input${d.inputs.length === 1 ? '' : 's'}`]);
    rows.push(['To', oursOut.map(outLine).join('') || '—']);
    rows.push(['Fee', d.fee !== null ? `${sats(d.fee)} sats, paid by the sender` : 'paid by the sender']);
    const facts = txFacts[name], q = facts && facts.rows[txid] && facts.rows[txid].quote;
    if (q && vault.network === 'mainnet') rows.push(['Cost', `${money(Math.abs(facts.rows[txid].net) / 1e8 * q.price, facts.ccy)}<small>what it was worth as it arrived \u2014 its cost, if it was bought</small>`]);
  }
  $('#tx-dl').innerHTML = rows.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join('');
  $('#tx-proof').textContent = d.proof === 'proved' ? `the node proved it is in block ${sats(d.height)}` : d.proof === 'unconfirmed' ? 'not in a block yet' : d.proof;
  $('#tx-proof').classList.toggle('warn', d.proof !== 'proved' && d.proof !== 'unconfirmed');
  const kv = [['Transaction id', d.txid], ['Witness id', d.wtxid], ['Version · locktime', `${d.version} · ${d.locktime}`], ['Size', `${sats(d.size)} B · ${sats(d.vsize)} vB · ${sats(d.weight)} WU`], ['Fee', d.fee !== null ? `${sats(d.fee)} sats · ${d.fee_rate.toFixed(2)} sat/vB` : 'unknown (an input could not be fetched)'],
    ['Replaceable (RBF)', d.rbf ? 'signalled' : 'not signalled'], ['Block', d.block ? `${sats(d.height)} · ${d.block}` : '—'], ['Position in block', d.position !== null ? String(d.position) : '—'],
    ['Block time', d.time ? new Date(d.time * 1000).toISOString().replace('.000Z', ' UTC').replace('T', ' ') : '—'], ['Inclusion', d.proof === 'proved' ? 'proved: the server\'s merkle path hashes up to the root in that block\'s header' : d.proof]];
  $('#tx-chain-body').innerHTML = `<dl class="chain-kv">${kv.map(([k, v]) => `<dt>${k}</dt><dd>${esc(v)}</dd>`).join('')}</dl>`
    + `<table class="chain-t"><thead><tr><th>Input</th><th>Address</th><th class="r">Sats</th><th>Sequence</th><th>Witness</th></tr></thead><tbody>${d.inputs.map(i => `<tr><td class="mono">${esc(i.outpoint.slice(0, 12))}…:${esc(i.outpoint.split(':')[1])}</td><td class="mono">${esc(i.address || '—')}${i.ours !== null ? place(i.ours) : ''}</td><td class="r">${i.sats !== null ? sats(i.sats) : '—'}</td><td class="mono">${i.sequence.toString(16)}</td><td>${i.witness} item${i.witness === 1 ? '' : 's'}${i.signers.length ? ' · ' + i.signers.map(esc).join(', ') : ''}</td></tr>`).join('')}</tbody></table>`
    + `<table class="chain-t"><thead><tr><th>Output</th><th>Address</th><th class="r">Sats</th><th>Script</th><th></th></tr></thead><tbody>${d.outputs.map(o => { const unspent = o.ours !== null && (vault._coins || []).some(c => c.txid === d.txid && c.vout === o.n); return `<tr><td>#${o.n}</td><td class="mono">${esc(o.address || '—')}${o.ours !== null ? place(o.ours) : who(o.address)}</td><td class="r">${sats(o.sats)}</td><td>${esc(o.kind)}</td><td class="muted">${o.ours !== null ? (unspent ? 'unspent' : 'spent') : ''}</td></tr>`; }).join('')}</tbody></table>`
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
};
function recPoint(kind, key, slot) {
  recView = { kind, key };
  const card = $('#tx-notes'), want = slot || recHome;
  if (card.parentElement !== want) want.appendChild(card);
  const [note, label] = REC_WORDS[kind] || REC_WORDS.tx;
  $('#tx-note').placeholder = note; $('#tx-label').placeholder = label;
  if (document.activeElement !== $('#tx-label')) $('#tx-label').value = labelOf(key);
  if (document.activeElement !== $('#tx-note')) $('#tx-note').value = (vault.notes || {})[key] || '';
  fitNote(); recSaid(''); renderFiles();
}
const thumbCache = new Map();
const sizeWords = (n) => n >= 1048576 ? (n / 1048576).toFixed(1) + ' MB' : n >= 1024 ? Math.round(n / 1024) + ' KB' : n + ' B';
async function renderFiles() {
  if (!vault || !recKey()) return;
  const name = vault.name, key = recKey(), list = ((vault.files || {})[key] || []).slice();
  let here = []; try { here = await invoke('attach_have', { hashes: list.map(e => e.sha256) }); } catch {}
  if (!vault || vault.name !== name || key !== recKey()) return;
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
function realisedRow(name, txid) {
  const L = vaultLedger(name), r = L && L.realised[txid], ccy = (txFacts[name] || {}).ccy;
  if (!r || !ccy) return '';
  const day = (t) => new Date(t * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
  const held = r.from ? (r.from === r.to ? `bought ${day(r.from)}` : `bought between ${day(r.from)} and ${day(r.to)}`) + ` \u00b7 held ${sats(Math.floor((r.t - r.to) / 86400))}${r.from !== r.to ? '\u2013' + sats(Math.floor((r.t - r.from) / 86400)) : ''} days` : 'bought on a day not known here';
  if (r.gain === null) return `unknown<small>${r.cost === null ? 'some of the coins that left have no known cost' : 'no price for the day they left'} \u00b7 ${held} \u00b7 by ${esc(COST_METHODS[r.method].short)}</small>`;
  return `<span class="${r.gain >= 0 ? 'ok' : 'warn'}">${r.gain >= 0 ? '\u25b2 +' : '\u25bc \u2212'}${money(Math.abs(r.gain), ccy)}</span><small>worth ${money(r.proceeds, ccy)} as ${sats(Math.round(r.sats))} sats left \u00b7 they had cost ${money(r.cost, ccy)} \u00b7 ${held} \u00b7 by ${esc(COST_METHODS[r.method].short)}</small>`;
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
  } catch (e) { $('#shot-note').classList.add('warn'); $('#shot-note').textContent = /NotAllowed|denied/i.test(String(e)) ? 'macOS is not letting Any Two Keys use the camera: allow it in System Settings › Privacy & Security › Camera.' : /NotFound|Overconstrained/i.test(String(e)) ? 'No camera was found.' : 'The camera could not be started: ' + String(e.message || e); }
}
$('#tx-camera').onclick = () => { if (!onRec()) return; $('#shot-modal').classList.add('on'); shotStart(''); };

$('#shot-close').onclick = shotClose; $('#shot-again').onclick = () => shotStart('');
$('#shot-take').onclick = async () => {
  const v = $('#shot-video'), cv = $('#shot-still');
  if (!shot.taken) {   // first press: hold the frame so it can be looked at before it is kept
    if (!v.videoWidth) { $('#shot-note').textContent = 'The camera has not produced a picture yet. Give it a moment.'; return; }
    cv.width = v.videoWidth; cv.height = v.videoHeight; cv.getContext('2d').drawImage(v, 0, 0); shotStop();
    v.hidden = true; cv.hidden = false; shot.taken = true; $('#shot-again').hidden = false; $('#shot-take').textContent = 'Keep it with this transaction'; return; }
  const name = vault.name, key = recKey(); $('#shot-take').disabled = true;
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
$('#tx-addfiles').onclick = async () => { const name = vault.name; paintZone(false, true); try { attachDone(await invoke('attach_pick', { name, key: recKey(), keepOriginal: !stripOn() }), name); } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } finally { paintZone(false); } };
// an OS drop reaches a Tauri window as PATHS in an event, never as File objects in the page
for (const [ev, on] of [['tauri://drag-enter', true], ['tauri://drag-over', true], ['tauri://drag-leave', false]]) window.__TAURI__.event.listen(ev, () => { const lit = on && onRec(); if (lit && !$('#tx-addfiles').classList.contains('over')) $('#tx-addfiles').scrollIntoView({ block: 'nearest', behavior: 'smooth' }); paintZone(lit); });
window.__TAURI__.event.listen('tauri://drag-drop', async (e) => {
  paintZone(false);
  const paths = (e.payload && e.payload.paths) || []; if (!paths.length) return;
  if (!onRec()) return toast('Open a transaction, an address or a coin first: files attach to the record on screen');
  const name = vault.name; paintZone(false, true); try { attachDone(await invoke('attach_paths', { name, key: recKey(), paths, keepOriginal: !stripOn() }), name); } catch (err) { fail(err); } finally { paintZone(false); }
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
  if (!vault || !recKey()) return;
  const name = vault.name, key = recKey(), val = which === 'label' ? $('#tx-label').value.trim() : $('#tx-note').value;
  const cur = which === 'label' ? labelOf(key) : ((vault.notes || {})[key] || '');
  if (val.trimEnd() === cur) return;
  recSaid('Saving…');
  try { store = await invoke(which === 'label' ? 'label_set' : 'note_set', which === 'label' ? { name, key, label: val } : { name, key, note: val }); vault = store.vaults.find(v => v.name === name) || vault;
    const sealed = vault.seal && vault.seal.shares && vault.seal.shares.length;
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

async function fillActivity(name, s) {
  const ccy = s.servers && vault.network === 'mainnet' ? fiatCcy() : '';
  let a; try { a = await invoke('vault_activity', { name, history: s.history.map(h => [h.txid, h.height]), ccy }); }
  catch (e) { clearTimeout(fillLater.t); if (vault && vault.name === name) { $$('#activity .when, #activity .amt, #act-all .when, #act-all .amt').forEach(td => { if (td.textContent === '…') td.textContent = '—'; }); $('#activity-note').textContent = 'Dates and amounts could not be fetched: ' + e; $('#activity-note').hidden = false; } return; }
  if (!vault || vault.name !== name) return;
  txFacts[name] = { rows: Object.fromEntries(a.rows.map(r => [r.txid, r])), now: a.now, ccy: a.ccy };   // kept by vault NAME and txid, never on the vault object, which is replaced on every store re-read
  if ($('#s-tx').classList.contains('on')) renderTx();
  repaintFacts(txFacts[name]);
  vault._now = a.now ? { q: a.now, ccy: a.ccy } : null; paintBalance();
  renderHomeSummary();
  if (a.note && vault.network === 'mainnet') { $('#activity-note').textContent = a.note; $('#activity-note').hidden = false; }
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
  $('#vault-kind').textContent = vault.keys.length === 1 ? 'Single key' : `${vault.threshold} of ${vault.keys.length} keys sign`;
  $('#vault-keys').textContent = vault.keys.map(k => k.name).join(' · ');
  $('#balance-sub').textContent = 'syncing…';
  // Nothing of another vault may sit under this vault's name, not even for the seconds a sync takes:
  // a vault that has not been drawn yet starts from a blank screen, and an answer that comes back after the person has
  // moved to another vault is for that OTHER vault and is dropped here.
  const me = vault.name;
  if (sync.shown !== me && vault._snap && vault._snapFor === liveKey(vault)) { sync.shown = me; paintSnap(vault._snap); const f = txFacts[me]; if (f) { repaintFacts(f); vault._now = f.now ? { q: f.now, ccy: f.ccy } : null; paintBalance(); } }   // last known, at once, while the chain is asked
  if (sync.shown !== me) { sync.shown = me; vault._balance = undefined; vault._now = null; $('#balance').innerHTML = '— <small>sats</small>'; $('#balance-fiat').hidden = true; $('#activity').innerHTML = '<tr><td class="muted" colspan="5">syncing…</td></tr>'; $('#activity-note').hidden = true; $('#conn').textContent = ''; }
  try {
    const s = await invoke('vault_sync', { name: me });
    if (!vault || vault.name !== me) return;
    vault._snap = s; vault._snapFor = liveKey(vault); vault._syncedAt = Date.now(); vault._stale = false;
    paintSnap(s);
    fillActivity(vault.name, s);   // dates, amounts and values arrive after the balance; the balance never waits for them
  } catch (e) { if (!vault || vault.name !== me) return; checkNode(); $('#balance-sub').textContent = 'could not reach the server'; $('#conn').textContent = String(e); }
}
/// Draw one vault_sync answer: balance, coins, history, where it came from.
function paintSnap(s) {
  {
    vault._coins = s.coins; vault._receive = s.receive; vault._receiveIndex = s.receive_index; vault._rotating = s.rotating; vault._height = s.height; vault._history = s.history; vault._nextChange = s.next_change; vault._gap = s.gap;
    if ($('#s-send').classList.contains('on')) renderCoins();
    if ($('#s-receive').classList.contains('on')) renderAddresses();
    vault._balance = s.balance; vault._now = null; paintBalance();
    const unconf = s.coins.filter(c => c.height === 0).reduce((a, c) => a + c.value, 0);
    $('#balance-sub').textContent = `${s.coins.length} coin${s.coins.length === 1 ? '' : 's'}` + (unconf ? ` · ${sats(unconf)} unconfirmed` : ' · all confirmed');
    $('#conn').textContent = `Connected to ${s.servers.electrum.replace(/^\w+:\/\//, '')} · block ${sats(s.height)}`;
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
  return `<td class="when muted">${h.height > 0 ? '…' : 'pending'}</td><td><span class="mono">${h.txid.slice(0, 10)}…${h.txid.slice(-6)}</span>${vault._rotating ? ' <span class="muted">· ' + fmtIndex(h.index) + '</span>' : ''}</td><td>${lblHtml('tx:' + h.txid)}</td><td class="r amt muted">…</td><td class="r ${h.height > 0 ? 'ok' : 'muted'}">${h.height > 0 ? 'confirmed · ' + sats(h.height) : 'unconfirmed'}</td>`;
}
function actFill(tb, list) {
  tb.innerHTML = '';
  if (!list.length) { tb.innerHTML = '<tr><td class="muted" colspan="5">nothing yet</td></tr>'; return; }
  for (const h of list) { const tr = document.createElement('tr'); tr.dataset.txid = h.txid; tr.innerHTML = actRowHtml(h); tb.appendChild(tr); }
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
function actMatch(h, q) {
  if (!q) return true;
  const k = 'tx:' + h.txid;
  return (h.txid + ' ' + labelOf(k) + ' ' + ((vault.notes || {})[k] || '')).toLowerCase().includes(q);
}
function renderActScreen() {
  if (!vault) return;
  const all = txList[vault.name] || [], q = ($('#act-find').value || '').trim().toLowerCase();
  const hits = all.filter(h => actMatch(h, q));
  actFill($('#act-all'), hits.slice(0, actShown));
  $('#act-count').textContent = q ? `${sats(hits.length)} of ${sats(all.length)}` : `${sats(all.length)} transaction${all.length === 1 ? '' : 's'}`;
  $('#act-more').hidden = hits.length <= actShown;
  $('#act-more').textContent = `Show ${sats(Math.min(ACT_PAGE, hits.length - actShown))} more`;
  $('#act-foot').textContent = q && !hits.length ? 'Nothing here matches that. A label or a note you have written counts, as well as the id.'
    : `Showing ${sats(Math.min(actShown, hits.length))}. Amounts and dates arrive from the server a moment after the list.`;
  if (vault.name) fillLater();
}
const fillLater = () => { clearTimeout(fillLater.t); fillLater.t = setTimeout(() => { const f = txFacts[vault.name]; if (f) repaintFacts(f); }, 0); };
// the amounts already fetched are put back into whichever rows have just been drawn
function repaintFacts(facts) {
  if (!facts) return;
  for (const [txid, r] of Object.entries(facts.rows || {})) paintFactRow(txid, r, facts);
}
function paintFactRow(txid, r, facts) {
  for (const tr of $$(`#activity tr[data-txid="${txid}"], #act-all tr[data-txid="${txid}"]`)) {
    const when = tr.querySelector('.when'), amt = tr.querySelector('.amt');
    if (!when || !amt) continue;
    if (r.time) { const d = new Date(r.time * 1000); when.classList.remove('muted'); when.innerHTML = `${d.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })}<small>${d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}</small>`; when.title = d.toISOString().replace('.000Z', ' UTC').replace('T', ' ') + ' · the time in the block header'; }
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
  const all = (vault._coins || []).slice().sort((a, b) => b.value - a.value);
  const q = ($('#coin-find').value || '').trim().toLowerCase();
  const hits = all.filter(c => !q || (coinKey(c) + ' ' + coinAddr(c) + ' ' + labelOf(coinKey(c)) + ' ' + ((vault.notes || {})[coinKey(c)] || '')).toLowerCase().includes(q));
  const tb = $('#coin-rows'); tb.innerHTML = '';
  if (!hits.length) tb.innerHTML = `<tr><td class="muted" colspan="5">${all.length ? 'nothing here matches that' : 'no coins yet: nothing has arrived, or nothing is left unspent'}</td></tr>`;
  for (const c of hits) {
    const k = coinKey(c), addr = coinAddr(c), tip = vault._height || 0, deep = c.height > 0 && tip ? tip - c.height + 1 : 0;
    const tr = document.createElement('tr'); tr.className = 'rowpick'; tr.dataset.coin = k;
    tr.innerHTML = `<td><span class="mono">${c.txid.slice(0, 8)}…:${c.vout}</span>${vault._rotating && c.index !== undefined ? ' <span class="muted">· ' + fmtIndex(c.index) + '</span>' : ''}</td><td class="mono" title="${esc(addr)}">${addr ? esc(addr.slice(0, 18)) + '…' : '—'}</td><td>${lblHtml(k)}</td><td class="r" style="font-variant-numeric:tabular-nums">${sats(c.value)} sats</td><td class="r ${c.height > 0 ? 'ok' : 'muted'}">${c.height > 0 ? (deep ? sats(deep) + ' deep' : 'confirmed') : 'unconfirmed'}</td>`;
    tr.addEventListener('click', (e) => { if (e.target.closest('.lbl')) return; openCoin(c); });
    tb.appendChild(tr);
  }
  const tot = hits.reduce((a, c) => a + c.value, 0);
  $('#coin-count').textContent = `${sats(hits.length)} coin${hits.length === 1 ? '' : 's'} · ${sats(tot)} sats`;
}
$('#coin-find').addEventListener('input', renderCoins2);
function openCoin(c) {
  const k = coinKey(c), addr = coinAddr(c), tip = vault._height || 0, deep = c.height > 0 && tip ? tip - c.height + 1 : 0;
  const box = $('#coins-detail'); box.hidden = false;
  box.innerHTML = `<div class="rec-head"><h3>This coin</h3><button class="btn small" type="button" id="rec-close">Close</button></div>
    <dl class="tx-dl"><dt>Amount</dt><dd>${sats(c.value)} sats</dd>
    <dt>Output</dt><dd class="mono"><a href="#" data-tx="${c.txid}">${c.txid}</a>:${c.vout}</dd>
    <dt>Address</dt><dd class="mono">${addr ? esc(addr) : 'not known until the next sync'}</dd>
    <dt>Status</dt><dd>${c.height > 0 ? `in block ${sats(c.height)}${deep ? ' · ' + sats(deep) + ' deep' : ''}` : 'not in a block yet'}</dd></dl>
    <div class="row" style="gap:10px"><button class="btn small" type="button" id="coin-open-tx">Open the transaction it came from</button><button class="btn small" type="button" id="coin-spend">Spend this coin</button></div>
    <div id="rec-slot" style="margin-top:16px"></div>`;
  $('#rec-close').onclick = () => { box.hidden = true; recPoint('tx', recKey(), recHome); };
  $('#coin-open-tx').onclick = () => openTx(c.txid);
  $('#coin-spend').onclick = () => { coinPick.clear(); coinPick.add(c.txid + ':' + c.vout); show('send'); setTimeout(() => { const d = $('#coins-adv'); if (d) d.open = true; renderCoins(); coinSum(); }, 60); };
  recPoint('coin', k, $('#rec-slot'));
  box.scrollIntoView({ block: 'nearest' });
}
function renderRecHead() { if (recView.kind === 'coin') renderCoins2(); if (recView.kind === 'addr') renderAddresses(); }

// ---------- one address, with a record of its own ----------
function openAddress(r) {
  const k = 'addr:' + r.address, coins = (vault._coins || []).filter(c => c.index === r.index);
  const bal = coins.reduce((a, c) => a + c.value, 0), used = (vault._history || []).some(x => x.index === r.index);
  const box = $('#addr-detail'); box.hidden = false;
  box.innerHTML = `<div class="rec-head"><h3>This address</h3><button class="btn small" type="button" id="rec-close">Close</button></div>
    <dl class="tx-dl"><dt>Address</dt><dd class="mono" style="word-break:break-all;-webkit-user-select:text;user-select:text">${esc(r.address)}</dd>
    <dt>Chain</dt><dd>${r.chain === 'change' ? 'change — the app sends what is left over here' : 'receive — this is one to hand out'}${vault._rotating ? ' · ' + fmtIndex(r.index) : ''}</dd>
    <dt>Holding</dt><dd>${bal ? sats(bal) + ' sats in ' + sats(coins.length) + ' coin' + (coins.length === 1 ? '' : 's') : used ? 'nothing now — it has been used and spent from' : 'nothing yet — this address has never been used'}</dd></dl>
    <div class="row" style="gap:10px"><button class="btn small" type="button" id="addr-show">Show it on Receive</button><button class="btn small" type="button" id="addr-copy">Copy it</button></div>
    <div class="muted" style="font-size:13px;margin-top:10px">${used ? 'An address that has been used is public: whoever you gave it to can see everything it has ever held.' : 'Labelling an address before you hand it out is the point: when a deposit arrives you already know who it was from.'}</div>
    <div id="rec-slot" style="margin-top:16px"></div>`;
  $('#rec-close').onclick = () => { box.hidden = true; recPoint('tx', recKey(), recHome); };
  $('#addr-show').onclick = () => { drawAddress(r); show('receive'); };
  $('#addr-copy').onclick = () => navigator.clipboard.writeText(r.address).then(() => toast('Address copied'));
  recPoint('addr', k, $('#rec-slot'));
  box.scrollIntoView({ block: 'nearest' });
}

// ---------- a vault over time: balance, value, and what it cost ----------
// The ledger is where correctness lives, so it is a plain function of the transactions and nothing else.
//
// COST BASIS is average cost, and it MOVES WITH THE COINS. A deposit from outside costs what it was worth when it
// arrived. A spend takes its share of the basis with it. A transfer between two vaults in scope is not a sale and a
// purchase: the coins' cost goes across with them and only the fee's share is spent — otherwise moving coins between
// your own vaults would quietly reset their cost to that day's price. Such a transfer is recognised the only reliable
// way there is: the same transaction appears in more than one vault's history.
//
// It is what each deposit was worth on arrival, which is a cost only if the deposit was a purchase. A deposit from a
// wallet this app does not hold (an exchange, Sparrow) is counted at that day's price because nothing better is known.
function buildLedger(inputs, method) {
  method = method || 'average';
  // ---- events: one per transaction, with a leg per vault it touched and the coins it spent and made ----
  const ev = new Map();
  for (const v of inputs) for (const r of v.rows) {
    if (r.net === null || r.net === undefined) continue;
    const t = r.time || Math.floor(Date.now() / 1000);   // not in a block yet: it is happening now
    const e = ev.get(r.txid) || { txid: r.txid, t, legs: [], price: null };
    e.t = Math.min(e.t, t);
    e.legs.push({ vault: v.name, net: r.net, ins: r.ins || [], outs: (r.outs || []).map(([n, sats]) => [r.txid + ':' + n, sats]) });
    const p = r.quote ? r.quote.price : (!r.time && v.now ? v.now.price : null);
    if (p && !e.price) e.price = p;
    ev.set(r.txid, e);
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
  const heldLots = () => method === 'utxo' ? [...coins.values()].flatMap(c => c.lots) : method === 'fifo' ? Object.values(queues).flat() : null;
  const basisNow = () => method === 'average' ? Object.values(pools).reduce((a, p) => a + p.basis, 0) : (heldLots().reduce((a, l) => a + (l.cost || 0), 0));
  const popFifo = (vault, n) => { const q = queues[vault] || (queues[vault] = []), out = []; let need = n;
    while (need > 1e-9 && q.length) { const l = q[0]; if (l.sats <= need + 1e-9) { out.push(q.shift()); need -= l.sats; } else { const f = need / l.sats; out.push({ sats: need, cost: l.cost === null ? null : l.cost * f, t: l.t }); l.sats -= need; if (l.cost !== null) l.cost *= (1 - f); need = 0; } }
    if (need > 1e-9) out.push({ sats: need, cost: null, t: null });   // more left than was known to be held: its cost is unknown, never zero
    return out; };
  for (const e of events) {
    const before = { sats: Object.values(sats).reduce((a, x) => a + x, 0), basis: basisNow() };
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
    if (pool && left > 0) {
      const cost = sumCost(pool), proceeds = e.price ? left / 1e8 * e.price : null;
      const dated = pool.filter(l => l.t !== null);
      realised[e.txid] = { sats: left, cost, proceeds, gain: cost !== null && proceeds !== null ? proceeds - cost : null, t: e.t,
        from: dated.length ? Math.min(...dated.map(l => l.t)) : null, to: dated.length ? Math.max(...dated.map(l => l.t)) : null, lots: pool, method };
    }
    timeline.push({ t: e.t, txid: e.txid, price: e.price, before, sats: Object.values(sats).reduce((a, x) => a + x, 0), basis: basisNow(), internal: isInternal });
  }
  return { timeline, internal, unpriced, realised, method, perVault: { sats, basis: method === 'average' ? Object.fromEntries(Object.entries(pools).map(([k, p]) => [k, p.basis])) : Object.fromEntries(Object.keys(sats).map(v => [v, (method === 'utxo' ? [...coins.values()].filter(c => c.vault === v).flatMap(c => c.lots) : (queues[v] || [])).reduce((a, l) => a + (l.cost || 0), 0)])) } };
}
// Which coins a disposal is taken to be — the taxpayer's rule, not the app's. Sources in roadmap/anytwokeys/ACCOUNTING.md.
const COST_METHODS = {
  utxo:    { name: 'The coins actually spent', short: 'the coins this transaction actually spent', note: 'Specific identification, with the chain as the record: a transaction names exactly which coins it spends. Accepted in Australia (the ATO accepts your identification of the units disposed of, with records) and in the United States.' },
  fifo:    { name: 'First in, first out', short: 'first in, first out', note: 'The oldest coins are taken to leave first, whichever the transaction spent. The fallback in Australia and the default in the United States when units are not identified; counted vault by vault, as US rules have required since 2025.' },
  average: { name: 'Average cost', short: 'average cost', note: 'Every coin costs the same average. NOT generally accepted for bitcoin: the ATO accepts it only for units acquired on the same day, and the US not at all. The UK pools instead, with its own same-day and 30-day rules, which this app does not compute. Kept for reference.' },
};
const methodFor = (v) => (v && v.cost_method) || (store && store.settings && store.settings.cost_method) || 'utxo';
function vaultLedger(name) {
  const f = txFacts[name], v = store && store.vaults.find(x => x.name === name);
  if (!f || !v) return null;
  const m = methodFor(v), key = m + ':' + Object.keys(f.rows).length + ':' + (f.now ? f.now.price : 0);
  if (!vaultLedger.memo[name] || vaultLedger.memo[name].key !== key) vaultLedger.memo[name] = { key, L: buildLedger([{ name, rows: Object.values(f.rows), now: f.now }], m) };
  return vaultLedger.memo[name].L;
}
vaultLedger.memo = {};
const ledgerAt = (L, t) => { let s = { sats: 0, basis: 0 }; for (const e of L.timeline) { if (e.t > t) break; s = e; } return s; };

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
  hit.addEventListener('focus', () => show(at)); hit.addEventListener('blur', hide);
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
async function sumInput(name) {
  const v = store.vaults.find(x => x.name === name); if (!v) return null;
  const ccy = v.network === 'mainnet' ? fiatCcy() : '';
  let f = txFacts[name];
  if (!f || (f.ccy || '') !== ccy) {
    const s = await invoke('vault_sync', { name });
    const a = await invoke('vault_activity', { name, history: s.history.map(h => [h.txid, h.height]), ccy });
    f = txFacts[name] = { rows: Object.fromEntries(a.rows.map(r => [r.txid, r])), now: a.now, ccy: a.ccy };
  }
  return { name, network: v.network, rows: Object.values(f.rows), now: f.now, ccy: f.ccy };
}
async function renderSummary(box, names, opts = {}) {
  const me = (renderSummary.seq = (renderSummary.seq || 0) + 1);
  box.classList.add('sum-loading');   // refetch keeps the frame: the last drawing stays, dimmed, until the new one is ready
  let inputs = [];
  try { inputs = (await Promise.all(names.map(sumInput))).filter(Boolean); } catch (e) { if (me !== renderSummary.seq) return; box.classList.remove('sum-loading'); box.innerHTML = `<div class="label">Over time</div><div class="sum-empty">The history could not be fetched: ${esc(String(e))}</div>`; return; }
  if (me !== renderSummary.seq) return;
  const priced = inputs.length && inputs.every(i => i.network === 'mainnet' && i.ccy);
  const ccy = priced ? inputs[0].ccy : '';
  const now = priced ? (inputs.find(i => i.now) || {}).now : null;
  const methods = [...new Set(inputs.map(i => methodFor(store.vaults.find(v => v.name === i.name))))];
  const method = methods.length === 1 ? methods[0] : methodFor(null);
  const L = buildLedger(inputs, method);
  const rangeKey = sumPref('range', 'all'), R = SUM_RANGES.find(r => r[0] === rangeKey) || SUM_RANGES[3];
  const to = Math.floor(Date.now() / 1000), first = L.timeline.length ? L.timeline[0].t : to;
  const from = R[2] ? Math.min(to - R[2] * 86400, to - 3600) : Math.max(0, first - Math.max(3600, (to - first) * 0.03));
  const run = priced && L.timeline.length ? await pricesRange(ccy, Math.max(from, first), to) : { every: 1, prices: {}, ok: true };
  const prices = run.prices;
  if (me !== renderSummary.seq) return;
  const S = sumSeries(L, from, to, prices, now);
  const nowState = L.timeline.length ? L.timeline[L.timeline.length - 1] : { sats: 0, basis: 0 };
  const value = now ? nowState.sats / 1e8 * now.price : null, gain = value !== null ? value - nowState.basis : null;
  box.classList.remove('sum-loading');
  box.innerHTML = '';
  const topRow = document.createElement('div'); topRow.className = 'sum-top';
  const lbl = document.createElement('div'); lbl.className = 'label'; lbl.textContent = opts.title || 'Over time';
  const filters = document.createElement('div'); filters.className = 'sum-filters';
  if (opts.chips) filters.appendChild(opts.chips);
  const seg = document.createElement('div'); seg.className = 'seg'; seg.setAttribute('role', 'radiogroup'); seg.setAttribute('aria-label', 'Time range');
  for (const [k, n] of SUM_RANGES) { const b = document.createElement('button'); b.type = 'button'; b.setAttribute('role', 'radio'); b.setAttribute('aria-checked', k === R[0] ? 'true' : 'false'); b.dataset.range = k; b.textContent = n; b.onclick = () => { sumSave('range', k); renderSummary(box, names, opts); }; seg.appendChild(b); }
  filters.appendChild(seg); topRow.append(lbl, filters); box.appendChild(topRow);
  if (!L.timeline.length) { const e = document.createElement('div'); e.className = 'sum-empty'; e.textContent = opts.empty || 'Nothing has arrived yet, so there is nothing to draw.'; box.appendChild(e); return; }
  // stat tiles first: the numbers a reader came for, before any picture
  const k = document.createElement('div'); k.className = 'kpis';
  const tile = (key, v, sub, cls) => { const d = document.createElement('div'); d.className = 'kpi'; d.innerHTML = `<div class="k"></div><div class="v"></div><div class="s"></div>`; d.querySelector('.k').textContent = key; d.querySelector('.v').textContent = v; const sEl = d.querySelector('.s'); sEl.textContent = sub || ''; if (cls) sEl.classList.add(cls); return d; };
  k.appendChild(tile('Holding', sats(nowState.sats) + ' sats', inputs.length > 1 ? `across ${inputs.length} vaults` : ''));
  if (priced && value !== null) {
    k.appendChild(tile('Worth now', money(value, ccy), '1 BTC = ' + money(now.price, ccy)));
    k.appendChild(tile('Cost basis', money(nowState.basis, ccy), 'what it was worth as it arrived'));
    const pct = nowState.basis > 0 ? gain / nowState.basis * 100 : null;
    // a status colour, so it goes with a sign and an arrow and never by colour alone
    k.appendChild(tile('Unrealised', (gain >= 0 ? '▲ +' : '▼ −') + money(Math.abs(gain), ccy), pct === null ? '' : (gain >= 0 ? '+' : '−') + Math.abs(pct).toFixed(1) + '% on cost', gain >= 0 ? 'up' : 'down'));
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
  if (L.unpriced) notes.push(`${L.unpriced} deposit${L.unpriced === 1 ? ' has' : 's have'} no price, so ${L.unpriced === 1 ? 'its' : 'their'} cost is left out rather than guessed.`);
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
function renderOverview() {
  const main = store.vaults.filter(v => v.network === 'mainnet'), test = store.vaults.filter(v => v.network !== 'mainnet');
  let off = []; try { off = JSON.parse(sumPref('off', '[]')); } catch {}
  const names = main.map(v => v.name).filter(n => !off.includes(n));
  const chips = document.createElement('div'); chips.style.cssText = 'display:flex;gap:var(--s2);flex-wrap:wrap';
  for (const v of main) { const b = document.createElement('button'); b.type = 'button'; b.className = 'vchip'; b.setAttribute('aria-pressed', off.includes(v.name) ? 'false' : 'true'); b.textContent = v.name;
    b.onclick = () => { const set = new Set(off); set.has(v.name) ? set.delete(v.name) : set.add(v.name); sumSave('off', JSON.stringify([...set])); renderOverview(); }; chips.appendChild(b); }
  $('#ov-note').textContent = test.length ? `Not counted: ${test.map(v => '“' + v.name + '”').join(', ')} — on a test network, whose coins are worth nothing and must never be added to real ones.` : '';
  if (!main.length) { $('#ov-summary').innerHTML = '<div class="label">All vaults</div><div class="sum-empty">No vault on mainnet yet. Test-network vaults are never added into a total of real coins.</div>'; return; }
  renderSummary($('#ov-summary'), names, { title: 'All vaults', chips: main.length > 1 ? chips : null,
    empty: names.length ? 'Nothing has arrived in these vaults yet.' : 'Every vault is switched off above. Switch one on to see it.',
    extra: main.length > 1 ? 'Switch a vault off above to leave it out; a transfer from a vault that is switched off then counts as coming from outside.' : '' });
}

// ---------- receive ----------
async function drawAddress(chosen) {
  if (!vault) return;
  if (vault._receive === undefined) await sync();
  const addr = chosen ? chosen.address : (vault._receive || vault.address);
  $('#addr-text').textContent = addr;
  $('#addr-note').innerHTML = chosen && !chosen.current
    ? `<b style="font-family:var(--display);font-weight:600">${chosen.chain === 'change' ? 'Change' : 'Receive'} address ${fmtIndex(chosen.index).replace('change ', '')}${chosen.future ? ', ahead of the next unused one' : ''}.</b> ${chosen.future ? 'Fine to hand out now; the scan will find it as long as it is within the gap limit of the last one used.' : 'Already used. Paying it again works, but links the payments together on the chain.'} <a href="#" id="addr-back">Back to the next unused</a>`
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
    const vb = vault.keys.length === 1 ? 141 : (11 + inVb(vault.threshold, vault.keys.length) + 31 + 43);
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
function inVb(k, n) { if (n === 1) return 68; const sl = 1 + 34 * n + 2; return 41 + Math.ceil((1 + 1 + k * 73 + (sl < 76 ? 1 : 2) + sl) / 4); }
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
const openScan = async (deviceId) => {
  if (cam2Timer) { clearInterval(cam2Timer); cam2Timer = null; }
  if (cam2Stream) { cam2Stream.getTracks().forEach(t => t.stop()); cam2Stream = null; }
  $('#scan-modal').classList.add('on');
  try {
    cam2Stream = await camOpen(deviceId);
    $('#cam2').srcObject = cam2Stream;
    camChooser($('#scan-pick'), cam2Stream, openScan);
    const cv = document.createElement('canvas'); const ctx = cv.getContext('2d', { willReadFrequently: true });
    cam2Timer = setInterval(() => { const v = $('#cam2'); if (!v.videoWidth) return; cv.width = v.videoWidth; cv.height = v.videoHeight; ctx.drawImage(v, 0, 0); const img = ctx.getImageData(0, 0, cv.width, cv.height); const code = jsQR(img.data, cv.width, cv.height); if (code && code.data && /^(bitcoin:|bc1|tb1|[13mn2])/i.test(code.data.trim())) { fillDestination(code.data); closeScan(); } }, 350);
  } catch { $('#cam2-note').textContent = 'No camera available on this computer. Paste the address instead.'; }
};
$('#scan-to').onclick = () => openScan('');
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
$('#send-all').onchange = () => { $('#send-amount').disabled = $('#send-all').checked; };
$('#build-btn').onclick = async () => {
  const to = $('#send-to').value.trim(), all = $('#send-all').checked, amount = +($('#send-amount').value.replace(/[^0-9]/g, '') || 0);
  if (!to) return toast('Enter a destination address');
  if (!all && !amount) return toast('Enter an amount in sats');
  const rate = feeRate();
  if (!rate) return toast('Give a fee rate in sats per virtual byte, or choose Slow, Medium or Fast');
  try {
    built = await invoke('spend_build', { name: vault.name, to, amount, rate, sendAll: all, coins: [...coinPick] });
    $('#review').innerHTML = `<div style="display:flex;flex-direction:column;gap:8px;font-size:14px">
      <div class="row" style="justify-content:space-between"><span class="muted">To</span><span class="mono">${to.slice(0, 10)}…${to.slice(-6)}</span></div>
      <div class="row" style="justify-content:space-between"><span class="muted">Amount</span><span>${all ? 'everything' : sats(amount) + ' sats'}</span></div>
      <div class="row" style="justify-content:space-between"><span class="muted">Coins</span><span>${built.inputs}${coinPick.size ? ' chosen by you' : ' picked by the app'}</span></div>
      <div class="row" style="justify-content:space-between"><span class="muted">Fee</span><span>${sats(built.fee)} sats · ${built.vsize} vB</span></div>
      <div class="row" style="justify-content:space-between"><span class="muted">Change back to vault</span><span>${sats(built.change)} sats</span></div>
      <div class="muted" style="border-top:1px solid var(--line);padding-top:10px;font-size:13px">Next: sign with ${vault.keys.length === 1 ? 'the key' : 'any ' + vault.threshold + ' of your ' + vault.keys.length + ' keys'}. Nothing is sent until you confirm after the last signature.</div></div>`;
    $('#build-btn').textContent = 'Sign with keys'; $('#build-btn').onclick = () => show('sign');
  } catch (e) { fail(e); }
};

// ---------- sign ----------
const packedForm = (pub) => (store.forms || {})[pub] || 0;        // low 4 bits: form factor; top bit: NFC
const isNano = (pub) => [2, 4].includes(packedForm(pub) & 0x0F);
// packed byte: low 4 bits form factor · 0x80 has NFC · 0x40 NFC was actually asked (older builds stored the form alone)
const modelName = (form, nfc, asked = true) => ({ 1: nfc ? 'YubiKey 5 NFC' : asked ? 'YubiKey 5 (USB-A)' : 'YubiKey 5, USB-A', 2: 'YubiKey 5 Nano', 3: nfc ? 'YubiKey 5C NFC' : asked ? 'YubiKey 5C' : 'YubiKey 5, USB-C', 4: 'YubiKey 5C Nano', 5: 'YubiKey 5Ci' }[form & 0x0F] || 'YubiKey 5');
// our own drawings of each shape (not Yubico's artwork): body, gold touch contact, the plug that tells A from C, the keyring hole, an NFC mark
function keyArt(form, nfc, h = 44) {
  const f = form & 0x0F, usbC = [3, 4, 5].includes(f), nano = [2, 4].includes(f);
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
function drawRing(have, need, ready) {
  const R = 44, C = 2 * Math.PI * R, gap = need > 1 ? 7 : 0, seg = C / need - gap;
  let g = `<circle class="halo" cx="60" cy="60" r="56"/>`;
  for (let i = 0; i < need; i++) {
    const rot = -90 + i * 360 / need + (gap / 2) / C * 360;
    g += `<circle class="track" cx="60" cy="60" r="${R}" stroke-dasharray="${seg} ${C - seg}" transform="rotate(${rot} 60 60)"/>`;
    g += `<circle class="arc" cx="60" cy="60" r="${R}" stroke-dasharray="${seg} ${C - seg}" stroke-dashoffset="${i < have ? 0 : seg}" stroke-linecap="butt" transform="rotate(${rot} 60 60)" style="${i < have ? '' : 'opacity:0'}"/>`;
  }
  g += `<circle class="dot" cx="60" cy="60" r="11"/>`;
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
  if (nano) {
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
    return `<div class="ckey ${cls}${isNano(k.pubkey) ? ' nano' : ''}">${glyph}<span class="kname" title="${esc(k.name)}">${esc(k.name)}</span><span class="kstate">${label}</span></div>`;
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
  const q = (n) => `‘${n}’`, list = (ks) => ks.map(k => q(k.name)).join(ks.length === 2 ? ' or ' : ', ');
  $('#gsteps').hidden = false; $('#act-row').hidden = false; $('#sign-waiting').hidden = true; $('#sign-cancel').hidden = false; $('#dev-sign-row').hidden = true;
  const openDev = vault.ext ? st.keys.filter(k => !full(k) && k.pubkey.startsWith('[')) : [];
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
    $('#act-row').hidden = true; $('#gsteps').hidden = true; $('#sign-warn').hidden = true; $('#dev-sign-row').hidden = false;
    $('#dev-sign-show').hidden = !qr; $('#dev-sign-scan').hidden = !qr; $('#dev-sign-save').classList.toggle('primary', !qr);
    renderSign.devFormat = d.id === 'coldcard-q' ? 'bbqr' : 'ur';
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
  const c = usable[0]; const k = vault.keys.find(k => k.pubkey === c.pubkey), nano = [2, 4].includes(c.form & 0x0F), where = nano ? 'gold strip' : 'gold contact';
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
    $('#sign-text').textContent = `${nano ? 'It runs along the key’s top edge. Touch it' : 'Touch it'} when its light flashes${st.inputs > 1 ? `, once for each of the ${st.inputs} inputs` : ''}. Keep the key plugged in.`;
    paintGuide('touch', c.form || 3); renderSign();
    try { const r = await invoke('spend_sign', { name: vault.name, reader: c.reader, pin }); $('#pin').value = ''; toast(`${r.signer} signed ${r.signed_inputs.length} input${r.signed_inputs.length === 1 ? '' : 's'}`); }
    catch (e) { fail(e); }
    renderSign.signing = false; renderSign.tiles = null; renderSign();
  };
}
$('#export-psbt').onclick = async () => { try { const p = await invoke('spend_export', { name: vault.name, what: 'psbt' }); $('#export-note').textContent = 'Saved ' + p; } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
$('#export-tx').onclick = async () => { try { const p = await invoke('spend_export', { name: vault.name, what: 'tx' }); $('#export-note').textContent = 'Saved ' + p + ' — raw hex, broadcastable from any wallet or explorer.'; } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
try { $('#export-card').open = localStorage.getItem('a2k-export-open') === '1'; } catch {}
$('#export-card').addEventListener('toggle', () => { try { localStorage.setItem('a2k-export-open', $('#export-card').open ? '1' : '0'); } catch {} if (!$('#export-card').open) stopQr(); });
$('#export-pdf').onclick = async () => { try { $('#export-note').textContent = 'Saved ' + await invoke('spend_pdf', { name: vault.name }) + ' — the codes and the text on it are the same PSBT.'; } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
// QR: UR crypto-psbt. A looping set of parts (plain ones, then fountain mixtures so a late camera still completes), or one still code when it fits.
const qrShow = { timer: null, parts: [], single: null, still: false, i: 0, bbqr: [], fmt: 'ur' };
function drawQr(text) {
  const qr = qrcode(0, 'L'); qr.addData(text, 'Alphanumeric'); qr.make();
  const n = qr.getModuleCount(), cv = $('#qr-canvas'), ctx = cv.getContext('2d'), m = Math.floor(cv.width / (n + 8)), off = Math.floor((cv.width - m * n) / 2);
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
  const cv = document.createElement('canvas'), ctx = cv.getContext('2d', { willReadFrequently: true });
  let busy = false;
  pscan.timer = setInterval(async () => {
    const v = $('#cam4'); if (busy || !v || !v.videoWidth) return;
    cv.width = v.videoWidth; cv.height = v.videoHeight; ctx.drawImage(v, 0, 0);
    const code = jsQR(ctx.getImageData(0, 0, cv.width, cv.height).data, cv.width, cv.height); if (!code || !code.data) return;
    const t = code.data.trim(), multi = /^ur:/i.test(t) || t.startsWith('B$');
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
$('#sign-cancel').onclick = async () => { await invoke('spend_discard'); built = null; coinPick.clear(); $('#build-btn').textContent = 'Build transaction'; $('#build-btn').onclick = null; show('send'); location.reload(); };
$('#broadcast-btn').onclick = async () => {
  $('#broadcast-btn').disabled = true;
  try { // who is being paid is read from the transaction itself, not from what the Send form remembered: a spend loaded from a file, or finished after a relaunch, counts too
    let dests = []; try { dests = (await invoke('spend_ceremony', { name: vault.name })).pays.filter(p => !p.ours).map(p => p.address.toLowerCase()); } catch {}
    const r = await invoke('spend_broadcast', { name: vault.name }); vault._stale = true; const paid = bookOf().find(c => dests.includes(c.address.toLowerCase()));
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
  list.innerHTML = coins.slice().sort((a, b) => b.value - a.value).map(c => { const op = c.txid + ':' + c.vout; const conf = c.height ? h - c.height + 1 : 0;
    return `<label class="coin"><input type="checkbox" data-op="${op}" ${coinPick.has(op) ? 'checked' : ''}><span class="mono">${c.txid.slice(0, 10)}…${c.txid.slice(-4)}:${c.vout}${vault._rotating ? ' <span class="muted">· ' + fmtIndex(c.index) + '</span>' : ''}</span>${lblHtml('out:' + op)}<span style="font-variant-numeric:tabular-nums">${sats(c.value)} sats</span><span class="muted">${conf ? conf + ' conf' : 'unconfirmed'}</span></label>`; }).join('');
  coinSum();
}
function coinSum() { const n = coinPick.size; const tot = ((vault && vault._coins) || []).filter(c => coinPick.has(c.txid + ':' + c.vout)).reduce((a, c) => a + c.value, 0); $('#coin-sum').textContent = n ? `${n} coin${n === 1 ? '' : 's'} ticked · ${sats(tot)} sats` : 'none ticked: the app picks'; }
$('#coin-list').addEventListener('change', (e) => { const i = e.target.closest('[data-op]'); if (!i) return; if (i.checked) coinPick.add(i.dataset.op); else coinPick.delete(i.dataset.op); coinSum(); });
$('#coins-all').onclick = () => { ((vault && vault._coins) || []).forEach(c => coinPick.add(c.txid + ':' + c.vout)); renderCoins(); };
$('#coins-none').onclick = () => { coinPick.clear(); renderCoins(); };
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
    const rows = await invoke('vault_addresses', { name: vault.name, ahead: addrAhead });
    renderAddresses.rows = rows;
    const coins = vault._coins || [], hist = vault._history || [];
    for (const [chain, sel] of [['receive', '#addr-recv tbody'], ['change', '#addr-chg tbody']]) {
      const tb = $(sel); tb.innerHTML = '';
      for (const r of rows.filter(r => r.chain === chain).slice().reverse()) {
        const bal = coins.filter(c => c.index === r.index).reduce((a, c) => a + c.value, 0);
        const used = hist.some(x => x.index === r.index);
        const tr = document.createElement('tr'); tr.className = 'rowpick' + (r.current ? ' cur' : '') + (r.future ? ' future' : '');
        tr.innerHTML = `<td class="muted">${r.position}</td><td class="mono" title="${r.address}">${r.address}</td><td>${lblHtml('addr:' + r.address)}</td><td class="r" style="font-variant-numeric:tabular-nums">${bal ? sats(bal) + ' sats' : '<span class="muted">—</span>'}</td><td class="r ${r.current ? 'ok' : 'muted'}">${r.current ? (chain === 'change' ? 'next change' : 'next deposit') : r.future ? 'unused · ahead' : used ? (bal ? 'in use' : 'spent') : 'unused'}</td>`;
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
  hits: (card) => !!card.firmware && verLt(card.firmware, '5.7.0'),
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
    return `<div class="keyrow vk"><span class="devart">${d.svg}</span><div class="who"><div class="name"><span class="name-text">${esc(k.name)}</span><button class="pencil" data-edit-idx="${i}" title="Rename this key">${PENCIL}</button></div><div class="muted" style="font-size:13px">${esc(d.name)} · a fixed key in this vault · <span class="mono">${esc(k.pubkey.slice(0, 12))}…</span></div></div>`
      + `<div class="kright"><div class="state ${card ? 'here' : ''}"><i></i>${card ? 'plugged in' : 'not plugged in'}</div></div></div>`;
  }
  const m = /^\[([0-9a-f]{8})\/([^\]]+)\](\S+)$/i.exec(k.pubkey) || [];
  const art = k.device ? `<span class="devart">${deviceOf(k.device).svg}</span>` : '<span class="keyart"><svg width="24" height="24"><use href="#ring"/></svg></span>';
  return `<div class="keyrow vk">${art}<div class="who"><div class="name"><span class="name-text">${esc(k.name)}</span><button class="pencil" data-edit-idx="${i}" title="Rename this key">${PENCIL}</button></div><div class="muted" style="font-size:13px">${k.device ? esc(deviceOf(k.device).name) + ' \u00b7 ' : ''}fingerprint <span class="mono">${esc(m[1] || '?')}</span> \u00b7 m/${esc(m[2] || '?')} \u00b7 <span class="mono">${esc((m[3] || '').slice(0, 14))}\u2026</span></div></div>`
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
  return `<div class="keyrow vk"><span class="keyart">${art}</span><div class="who"><div class="name"><span class="name-text">${esc(k.name)}</span><button class="pencil" data-edit-idx="${i}" title="Rename this key">${PENCIL}</button></div><div class="muted" style="font-size:13px">${esc(facts.join(' · '))} · <span class="mono">${k.pubkey.slice(0, 12)}…</span></div>${adv.map(a => `<div class="warn advline" style="font-size:13px;margin-top:6px"><b>${esc(a.id)}: ${esc(a.title)}.</b> ${esc(a.says)} ${esc(a.here)}</div>`).join('')}</div>`
    + `<div class="kright"><div class="state ${card ? 'here' : ''}"><i></i>${card ? 'plugged in' : 'not plugged in'}</div><div class="kseal" data-seal-slot="${i}"></div></div></div>`;
}
function renderKeys(force) {
  if (!vault) return;
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
    <div class="muted" style="font-size:13px">A YubiKey tells this computer its serial, its firmware version and its shape. It does <b>not</b> carry the day it was made, an end-of-life date, or anything about a warranty \u2014 so none is shown here, because a guessed one would be worse than nothing.</div>
    <div class="muted" style="font-size:13px;margin-top:8px">Yubico retires a whole <b>product line</b>, not your particular key, and sets no date in advance: their policy is six months\u2019 notice before a product stops being sold, and support for a year after that. A warranty runs from a purchase this app knows nothing about. The nearest thing to an age it can honestly give is when this computer first saw each key, which is a floor and never the real age.</div>
    <div class="muted" style="font-size:13px;margin-top:8px">The firmware each key reports <b>is</b> checked, here on this computer, against advisories known when this app was built (${esc(ADVISORIES_AS_OF)}). Nothing is asked of anyone: a question like \u201cis serial 12345678 still supported?\u201d would hand over a list of exactly which keys guard this vault.</div>
    <div class="${hit.length ? 'warn' : 'muted'}" style="font-size:13px;margin-top:8px">${!keys.length ? '' : `The firmware of ${sats(known.length)} of this vault\u2019s ${sats(keys.length)} keys is known${known.length < keys.length ? ' \u2014 plug in the others once each and it is remembered, since it can never change' : ''}. ${hit.length === keys.length && keys.length ? `<b>All ${sats(keys.length)} are on firmware an advisory covers.</b> A vault of several keys usually means an attacker must beat more than one \u2014 but here every one of them has the same weakness, so that comfort is gone.` : hit.length ? `${sats(hit.length)} ${hit.length === 1 ? 'has' : 'have'} something against ${hit.length === 1 ? 'it' : 'them'} \u2014 see ${hit.length === 1 ? 'its row' : 'their rows'} above.` : known.length ? 'Nothing known stands against them.' : ''}`}</div>`;
}
function renderKeysNow() {
  // keys of this vault say "plugged in" on their own rows; this line is only for what ELSE is in the ports
  const others = cards.filter(c => !vault || !vault.keys.some(k => k.pubkey === c.pubkey));
  const devName = (r) => { const m = r.match(/YubiKey[^,()]*/i); const n = (m ? m[0] : r).replace(/\s+(OTP|FIDO|CCID|U2F)(\+\w+)*.*$/i, '').trim(); return /bio/i.test(r) ? 'YubiKey Bio' : (n || 'A Yubico device'); };
  const unusable = otherDevices.map(d => `<div class="keyrow"><svg width="22" height="22"><use href="#ring"/></svg><div class="who"><div class="name">${esc(devName(d.reader))} <span class="muted" style="font-weight:400">\u00b7 plugged in, cannot be used here</span></div><div class="muted" style="font-size:13px">${esc(d.why.charAt(0).toUpperCase() + d.why.slice(1))}.${/bio/i.test(d.reader) ? ' A YubiKey Bio signs in to websites with a fingerprint; it does not hold the kind of key a vault needs. It is perfectly fine \u2014 just for something else.' : ''} It is not one of this vault\u2019s keys and nothing was done to it.</div></div><div class="state"><i></i>not usable</div></div>`).join('');
  $('#keys-other').innerHTML = unusable; $('#keys-other').hidden = !unusable;
  const text = others.length ? 'Also plugged in: ' + others.map(c => `${modelName(c.form, c.nfc)} · serial ${c.serial}${c.pubkey ? '' : ' · no signing key yet'}`).join('; ') + '. Not one of this vault’s keys.'
    : cardsErr && !/no YubiKey found/i.test(cardsErr) ? 'Card reader problem: ' + cardsErr : '';
  $('#keys-now').textContent = text; $('#keys-now').hidden = !text;
}
// rename a key in place from the pencil on its row
$('#keys-list').addEventListener('click', (e) => {
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
askTwice($('#forget-vault'), 'Forget this vault on this computer? The coins stay on the chain; two sealed keys, or the recovery line and two keys, bring it back.', 'Yes, forget it here', async () => { store = await invoke('vault_forget', { name: vault.name }); vault = null; boot(); });

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
  const fresh = cards.find(c => !setup.keys.some(k => k.serial === c.serial));
  if (!fresh) { box.innerHTML = `<div class="muted">Key ${setup.keys.length + 1} of ${setup.n}: plug in a YubiKey${setup.n > 1 ? ', or add a signing device' : ''}.</div>`; return; }
  const nameGuess = ['laptop', 'keyring', 'off-site'][setup.keys.length] || `key ${setup.keys.length + 1}`;
  const has = !!fresh.pubkey;
  box.innerHTML = `<div style="font-family:var(--display);font-weight:600;font-size:15px">Key ${setup.keys.length + 1}: ${cardLabel(fresh)}</div>
    <div class="muted" style="font-size:13px">${has ? 'This card already holds a signing key (touch ' + fresh.touch + '). Use it: the same card key can belong to any number of vaults.' : 'OpenPGP signature slot is empty: a key will be generated on the card.'}</div>
    <div class="grid2"><div class="field"><div class="label">Name this key</div><input id="enrol-name" value="${nameGuess}"></div>${has ? '' : '<div class="field"><div class="label">Admin PIN</div><input id="enrol-admin" type="password" autocomplete="off"></div>'}</div>
    ${has ? '' : '<div class="muted" style="font-size:13px">Factory admin PIN is 12345678. Change it first with YubiKey Manager if you have not. Touch policy will be set to FIXED, which cannot be undone without wiping the key.</div>'}
    <div class="row">${has ? '<button class="btn primary" id="use-btn">Use this card\'s key</button><button class="btn" id="replace-btn" style="border-color:var(--no);color:var(--no)">Replace it with a new key…</button>' : '<button class="btn primary" id="enrol-btn">Generate key on card</button>'}</div>`;
  if (has) {
    $('#use-btn').onclick = () => { setup.keys.push({ name: $('#enrol-name').value.trim() || nameGuess, serial: fresh.serial, pubkey: fresh.pubkey, device: cardDevice(fresh) }); toast('enrolled with its existing key'); renderSetup(); };
    askTwice($('#replace-btn'), 'Generate a NEW key on this card? The key it holds now will be destroyed, and any vault it belongs to loses that cosigner for good. Only for a card that has never held funds.', 'Yes, replace its key', async () => {
      box.innerHTML += '<div class="field" style="margin-top:12px"><div class="label">Admin PIN</div><input id="enrol-admin" type="password" autocomplete="off" style="max-width:260px"></div><button class="btn" id="enrol-btn" style="border-color:var(--no);color:var(--no);margin-top:10px">Generate a new key, destroying the old one</button>';
      wireGenerate();
    });
  } else wireGenerate();
  function wireGenerate() { $('#enrol-btn').onclick = async () => {
    const name = $('#enrol-name').value.trim() || nameGuess, admin = $('#enrol-admin').value;
    if (!admin) return toast('Enter the admin PIN');
    $('#enrol-btn').disabled = true;
    try { const pub = await invoke('card_generate', { reader: fresh.reader, adminPin: admin }); setup.keys.push({ name, serial: fresh.serial, pubkey: pub, device: cardDevice(fresh) }); toast(`${name} enrolled`); }
    catch (e) { fail(e); }
    renderSetup();
  }; }
}
function cardDevice(c) { return /nitrokey/i.test(c.reader || '') ? 'nitrokey3' : 'yubikey'; }
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
    $('#dev-one').innerHTML = head + (d.id === 'yubikey'
      ? '<div>Plug it in and close this: a YubiKey appears on the setup screen by itself, ready to enrol.</div>'
      : '<div>Not yet. A Nitrokey 3 can hold the same kind of key as a YubiKey, and the app will read it once it has been tested on a real one.</div>');
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
  const parts = devSt.parts; devStop(); if (typeof deviceId === 'string') devSt.parts = parts;   // switching camera keeps the parts already read
  const note = $('#dev-cam-note'); note.hidden = false; $('#dev-cam').hidden = false;
  try { devSt.stream = await camOpen(typeof deviceId === 'string' ? deviceId : undefined); $('#cam3').srcObject = devSt.stream; camChooser($('#dev-pick-cam'), devSt.stream, devScan); } catch (e) { note.textContent = 'No camera available. Open its file, or paste the key instead.'; $('#dev-cam').hidden = true; return; }
  note.textContent = 'Hold the device’s code in the frame.';
  const cv = document.createElement('canvas'), ctx = cv.getContext('2d', { willReadFrequently: true });
  let busy = false;
  devSt.timer = setInterval(async () => {
    const v = $('#cam3'); if (busy || !v || !v.videoWidth) return;
    cv.width = v.videoWidth; cv.height = v.videoHeight; ctx.drawImage(v, 0, 0);
    const code = jsQR(ctx.getImageData(0, 0, cv.width, cv.height).data, cv.width, cv.height); if (!code || !code.data) return;
    const t = code.data.trim();
    // an animated UR arrives a part at a time, in any order; a plain code is the whole thing
    if (/^ur:/i.test(t)) { const before = devSt.parts.size; devSt.parts.add(t.toLowerCase()); if (devSt.parts.size === before) return; }
    busy = true;
    try { const r = await invoke('device_key_read', { text: /^ur:/i.test(t) ? [...devSt.parts].join(' ') : t }); devStop(); $('#dev-cam').hidden = true; note.textContent = 'Read.'; scanBar(note, null); devGot(r); }
    catch (e) { if (!scanShow(note, e)) { note.textContent = String(e); scanBar(note, null); } }
    busy = false;
  }, 250);
}
/// "Still reading" from the decoder's own count. An animated UR is a fountain code: after its first lap every frame
/// is a MIXTURE numbered above the total ("7-4"), so counting frames against that total reads "7 of 4" and means
/// nothing. What the decoder has recovered (UR), or which parts it holds (BBQr, which does cycle), can be trusted.
function scanProgress(e) {
  const m = /(\d+) of (\d+) (pieces recovered|\))/.exec(String(e)) || /missing \((\d+) of (\d+)\)/.exec(String(e));
  if (!/missing/i.test(String(e))) return null;
  if (!m) return { have: 0, of: 0, text: 'Reading an animated code. Keep it in the frame.' };
  const have = +m[1], of = +m[2], left = of - have;
  return { have, of, text: `${have} of ${of} pieces read — ${left} more to go. Keep it in the frame, or show the next part.` };
}
/// A bar of `of` segments, `have` of them filled, drawn just above the scanner's note. A code in a handful of parts
/// (one per metal plate, say) shows each part as its own segment; a long animated one, as one bar that fills.
function scanBar(note, p) {
  let bar = note.previousElementSibling && note.previousElementSibling.classList.contains('sbar') ? note.previousElementSibling : null;
  if (!p || !p.of) { if (bar) bar.remove(); return; }
  if (!bar) { bar = document.createElement('div'); bar.className = 'sbar'; bar.setAttribute('role', 'progressbar'); note.parentNode.insertBefore(bar, note); }
  bar.setAttribute('aria-valuemin', '0'); bar.setAttribute('aria-valuemax', String(p.of)); bar.setAttribute('aria-valuenow', String(p.have));
  bar.setAttribute('aria-label', `${p.have} of ${p.of} pieces read`);
  bar.classList.toggle('long', p.of > 24);
  bar.innerHTML = p.of > 24 ? `<i class="fill" style="width:${Math.round(100 * p.have / p.of)}%"></i>` : Array.from({ length: p.of }, (_, i) => `<i class="${i < p.have ? 'on' : ''}"></i>`).join('');
}
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
  const taken = new Set(setup.keys.map(x => x.name)); let name = d.name, i = 2; while (taken.has(name)) name = `${d.name} ${i++}`;
  $('#dev-got').innerHTML = `<div class="ok">Found its key: fingerprint <span class="mono">${esc(k.fingerprint)}</span> · m/${esc(path)} · <span class="mono">${esc(k.xpub.slice(0, 14))}…</span></div>
    ${usual ? '' : `<div class="warnbox">That is not the usual multisig path (m/48h/…/2h). Some signers will not recognise a multisig spend from it. Use it only if you know this device exported it for multisig.</div>`}
    <div class="field"><div class="label">Name this key</div><input id="dev-name" value="${esc(name)}"></div>
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
    + seen.map(sn => `<div class="keyrow"><svg width="22" height="22"><use href="#ring"/></svg><div class="who"><div class="name">YubiKey ${sn}</div><div class="muted" style="font-size:13px">${sealFound.filter(f => f.serial === sn).length} share(s) open${cards.some(c => c.serial === sn) ? '' : ' · unplugged, remembered'}</div></div><div class="state ok"><i></i>open</div></div>`).join('')
    + (r.cards && !r.found.length ? `<div class="muted" style="font-size:13px">${r.foreign.length ? 'This key\'s certificate object holds something that is not a seal.' : 'The key plugged in now is not sealed. A vault sealed by this app writes a share to each key from the Keys screen.'}</div>` : '')
    + (!r.cards && !seen.length ? '<div class="muted" style="font-size:13px">No YubiKey plugged in.</div>' : '');
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
function restoreTab(name) { restoreTab.now = name; $$('.tabs div').forEach(x => x.classList.toggle('on', x.dataset.tab === name)); for (const t of ['keys', 'scan', 'paste']) $('#tab-' + t).hidden = t !== name; $('#sheet-note').hidden = name === 'keys'; if (name === 'scan') startCam(); else stopCam(); if (name === 'keys') { readSeals.sig = null; readSeals(); } }
$$('.tabs div').forEach(t => t.onclick = () => restoreTab(t.dataset.tab));
async function startCam(deviceId) {
  if (camStream && !deviceId) return;
  if (camTimer) { clearInterval(camTimer); camTimer = null; }
  if (camStream) { camStream.getTracks().forEach(t => t.stop()); camStream = null; }
  try { camStream = await camOpen(deviceId); $('#cam').srcObject = camStream; camChooser($('#scan-pick2'), camStream, startCam); const cv = document.createElement('canvas'); const ctx = cv.getContext('2d', { willReadFrequently: true });
    camTimer = setInterval(() => { const v = $('#cam'); if (!v.videoWidth) return; cv.width = v.videoWidth; cv.height = v.videoHeight; ctx.drawImage(v, 0, 0); const img = ctx.getImageData(0, 0, cv.width, cv.height); const code = jsQR(img.data, cv.width, cv.height); if (code && code.data) restoreCode(code.data.trim()); }, 400);
  } catch (e) { $('#cam-note').textContent = 'No camera available. Paste the line instead.'; }
}
// what the restore camera has read: a descriptor as text, or a UR one in parts gathered until it is whole
const restoreParts = new Set(); let restoreBusy = false;
async function restoreCode(t) {
  const note = $('#cam-note');
  if (/^(wsh|wpkh|sh|tr)\(/.test(t)) { note.textContent = 'Read a descriptor from the code.'; scanBar(note, null); return readDescriptor(t); }
  if (!/^ur:(crypto-output|output-descriptor)\//i.test(t)) return;
  const n = restoreParts.size; restoreParts.add(t.toLowerCase()); if (restoreParts.size === n || restoreBusy) return;
  restoreBusy = true;
  // whole: the camera's work is done — stopped, so a plate still in view does not start a second reading
  try { const all = [...restoreParts].join('\n'); await invoke('descriptor_parse', { text: all }); stopCam(); restoreParts.clear(); note.textContent = 'Read the whole descriptor.'; scanBar(note, null); readDescriptor(all); }
  catch (e) { if (!scanShow(note, e)) { note.textContent = String(e); scanBar(note, null); restoreParts.clear(); } }
  restoreBusy = false;
}
function stopCam() { if (camTimer) clearInterval(camTimer); camTimer = null; if (camStream) { camStream.getTracks().forEach(t => t.stop()); camStream = null; } }
$('#desc-read').onclick = () => readDescriptor($('#desc-in').value);
let parsed = null;
async function readDescriptor(text) {
  try {
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
function renderVaultSettings() {
  if (!vault) return;
  $('#vs-method').value = vault.cost_method || '';
  const own = vault.cost_method, app = store.settings.cost_method || 'utxo';
  $('#vs-method-note').textContent = own
    ? `This vault follows ${COST_METHODS[own].short}, not the app\u2019s ${COST_METHODS[app].short}. That is right only if it belongs to a different taxpayer \u2014 a company\u2019s, a client\u2019s \u2014 whose rules differ. One person cannot use different methods for different wallets to choose the result.`
    : `Follows the app\u2019s method: ${COST_METHODS[app].short}. Change it here only for a vault that belongs to a different taxpayer.`;
  $('#vs-method-note').classList.toggle('warn', !!own);
  const n = vault.keys.length, single = n === 1;
  const drill = (vault.notes || {})['vault:drill'] || '';
  $('#vs-facts').innerHTML = [
    ['Name', esc(vault.name)],
    ['Network', vault.network === 'mainnet' ? 'mainnet \u2014 real coins' : esc(vault.network) + ' \u2014 a test network; these coins are worth nothing'],
    ['Kind', single ? 'a single key' : `${vault.threshold} of ${n} keys \u2014 any ${vault.threshold} of them sign`],
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
  const ds = drillState(vault);
  $('#vs-drill-when').innerHTML = ds.never
    ? '<b>This vault has never been drilled.</b> Until it has, "the keys can recover it" is something you believe rather than something you have seen.'
    : `Last drill: <b>${esc(drillWords(ds))}</b> \u2014 ${ds.days === 0 ? 'today' : sats(ds.days) + ' day' + (ds.days === 1 ? '' : 's') + ' ago'}. ${ds.every ? (ds.due ? 'That is past the ' + sats(ds.every) + ' days you asked for.' : 'The next is due in ' + sats(ds.every - ds.days) + ' day' + (ds.every - ds.days === 1 ? '' : 's') + '.') : 'You have asked not to be reminded.'}`;
  $('#vs-drill-when').classList.toggle('warn', ds.due);
  $('#vs-drill').hidden = single || !!vault.ext;
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
  const pinRows = [...new Set(stillLocked.map(f => f.serial))].map(sn => { const c = cards.find(c => c.serial === sn); return c ? `<div class="keyrow vk"><div class="who"><div class="name">YubiKey ${sn}</div><div class="muted" style="font-size:13px">Its share is locked. The drill is only a drill if the PIN opens it.</div></div><div class="kseal"><input type="password" autocomplete="off" placeholder="key PIN" data-drill-pin="${esc(c.reader)}" aria-label="PIN of key ${sn}"><button class="btn" data-drill-unlock="${esc(c.reader)}">Unlock</button></div></div>` : ''; }).join('');
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
$('#vs-desc-reveal').onclick = () => { renderVaultSettings.shown = vault.name; renderVaultSettings(); };
$('#vs-copy-desc').onclick = () => navigator.clipboard.writeText(vault.ext || vault._recovery || vault.descriptor || '').then(() => toast('Recovery line copied'));
$('#vs-rewrite-go').onclick = async () => { const text = $('#vs-rewrite-in').value.trim(), name = vault.name; if (!text) return toast('Paste the other wallet\u2019s version first');
  try { store = await invoke('vault_ext_rewrite', { name, text }); vault = store.vaults.find(v => v.name === name) || vault; $('#vs-rewrite-in').value = ''; renderVaultSettings.shown = name; renderVaultSettings(); toast('Same wallet \u2014 its recovery line now reads as your other wallet writes it'); }
  catch (e) { fail(e); } };
$('#vs-pdf').onclick = () => invoke('vault_pdf', { name: vault.name }).then((p) => toast('Saved to ' + p)).catch(fail);
$('#vs-rename').onclick = () => { show('home'); setTimeout(() => $('#title-edit').click(), 80); };
askTwice($('#vs-forget'), 'Forget this vault on this computer? The coins stay on the chain; two sealed keys, or the recovery line and two keys, bring it back.', 'Yes, forget it here', async () => { store = await invoke('vault_forget', { name: vault.name }); vault = null; boot(); });
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
  const dev = deviceCurrency(), cur = store.settings.fiat ?? 'auto';
  $('#fiat-pick').innerHTML = `<option value="auto">Automatic: ${dev.ccy} · ${CCYS[dev.ccy] || ''}</option><option value="">None: show sats only</option>` + Object.entries(CCYS).map(([c, n]) => `<option value="${c}">${c} · ${n}</option>`).join('');
  $('#fiat-pick').value = cur;
  $('#fiat-note').textContent = (cur === 'auto' ? `Chosen because ${dev.why}. ` : '') + (store.settings.custom ? (store.settings.prices ? `Prices come from ${store.settings.prices}.` : 'You run your own servers and have named no price server, so no values are shown.') : 'Prices come from prices.mineracks.com: the hour of each transaction is asked for, never an address or an amount.') + ' Test networks show no values.';
  invoke('store_get').then(() => {}); $('#watch-log-path').textContent = '~/Library/Application Support/Any Two Keys/watch.log';
}
$('#net-pick').addEventListener('click', async (e) => { const o = e.target.closest('.opt'); if (!o) return; store = await invoke('settings_set', { network: o.dataset.net, custom: store.settings.custom, electrum: store.settings.electrum, explorer: store.settings.explorer }); toast('New vaults will use ' + o.dataset.net); renderSettings(); $('#welcome-net').textContent = store.settings.network; });
$('#own-servers').onclick = async () => { store = await invoke('settings_set', { network: netName(), custom: !store.settings.custom, electrum: $('#c-electrum').value, explorer: $('#c-explorer').value }); renderSettings(); };
$('#save-settings').onclick = async () => { try { store = await invoke('settings_set', { network: netName(), custom: true, electrum: $('#c-electrum').value, explorer: $('#c-explorer').value, prices: $('#c-prices').value, vss: $('#c-vss').value }); renderSettings(); toast('Using your servers'); } catch (e) { fail(e); } };
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
window.__TAURI__.event.listen('prices-filled', (ev) => { if (vault && ev.payload === vault.name && vault._snap) fillActivity(vault.name, vault._snap); });
window.__TAURI__.event.listen('vault-changed', (ev) => {
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
