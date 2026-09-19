// Any Two Keys — the UI. Every real thing happens in Rust behind `invoke`.
const { invoke } = window.__TAURI__.core;
const $ = (s) => document.querySelector(s);
const $$ = (s) => Array.from(document.querySelectorAll(s));
const sats = (n) => Number(n).toLocaleString('en-AU');
let store = null, vault = null, cards = [], cardsErr = '', fees = null, feeLevel = 'medium', built = null, camStream = null, camTimer = null;
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
  const inVault = vault && !['setup', 'restore', 'settings'].includes(name);
  $$('#vault-tabs .nav').forEach(n => n.classList.toggle('on', !!inVault && n.dataset.vault === vault.name));
  $('#title').textContent = { home: vault ? vault.name : 'Vault', send: 'Send', receive: 'Receive', keys: 'Keys', settings: 'App settings', setup: 'New vault · enrol keys', restore: 'Restore a vault', sign: 'Sign', messages: 'Messages' }[name] || '';
  $('#title-edit').hidden = !(name === 'home' && vault); $('#title-input').hidden = true; $('#title').hidden = false;
  paintCrumbs(name); paintStaged(name);
  $('#side-vault').textContent = vault ? vault.name : 'No vault yet'; $('#side-vault').title = vault ? `These screens are about “${vault.name}”` : '';
  if (checkNode.net !== (vault ? vault.network : store.settings.network)) checkNode();
  if (name !== 'restore') stopCam();
  if (name !== 'sign') stopQr();
  if (name !== 'keys') hideDesc();
  if (name === 'home') { sync(); labelsSync(); }
  if (name === 'receive') { drawAddress(); renderAddresses(); }
  if (name === 'send') { closeBook(); paintTo(); loadFees(); if (vault && !vault._coins) sync().then(renderCoins); else renderCoins(); }
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
    d.onclick = () => { vault = v; try { localStorage.setItem('a2k-vault', v.name); } catch {} paintNet(); show('home'); };
    tabs.appendChild(d);
  }
}

async function boot() {
  store = await invoke('store_get');
  try { osLocale = await invoke('device_locale'); } catch {}   // without it the language list and the clock still answer
  paintNet();
  let want = null; try { want = localStorage.getItem('a2k-vault'); } catch {}
  vault = store.vaults.find(v => v.name === want) || store.vaults[0] || null;
  paintNet();
  if (vault) show('home'); else showWelcome();
  pollCards();
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
  for (const c of cards) if (c.pubkey && c.form && packedForm(c.pubkey) !== ((c.form & 0x0F) | 0x40 | (c.nfc ? 0x80 : 0))) { try { store = await invoke('key_form_seen', { pubkey: c.pubkey, form: c.form, nfc: !!c.nfc }); renderSign.tiles = null; if ($('#s-keys').classList.contains('on') && !keysBusy()) renderKeys(); } catch {} }
  if ($('#s-setup').classList.contains('on')) renderSetup();
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
function openTx(txid) { const h = ((vault && vault._history) || []).find(x => x.txid === txid); txView.txid = txid; txView.height = h ? h.height : 0; show('tx'); }
const txHeadline = () => { const f = ((txFacts[vault.name] || {}).rows || {})[txView.txid], lab = labelOf('tx:' + txView.txid);
  if (lab) return lab; if (!f || f.net === null) return txView.txid.slice(0, 10) + '…';
  return (f.time ? new Date(f.time * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short' }) + ' · ' : '') + (f.net > 0 ? '+' : f.net < 0 ? '−' : '') + sats(Math.abs(f.net)) + ' sats'; };
async function renderTx() {
  if (!vault || !txView.txid) return show('home');
  const name = vault.name, txid = txView.txid, mine = ++txView.seq, key = 'tx:' + txid;
  const facts = txFacts[name] || {}, f = (facts.rows || {})[txid];
  paintCrumbs('tx'); $('#title').textContent = txHeadline();
  // what the activity list already knows is on screen at once; the decoded transaction fills the rest in
  const amt = $('#tx-amt'); amt.className = 'tx-amt' + (f && f.net > 0 ? ' in' : ''); amt.innerHTML = f && f.net !== null ? `${f.net > 0 ? '+' : f.net < 0 ? '−' : ''}${sats(Math.abs(f.net))} <small>sats</small>` : '…';
  const fiat = $('#tx-fiat');
  fiat.hidden = !(f && f.net !== null && (f.quote || facts.now));
  if (!fiat.hidden) { fiat.textContent = [f.quote ? worth(Math.abs(f.net), f.quote, facts.ccy) + ' then' : '', facts.now ? worth(Math.abs(f.net), facts.now, facts.ccy) + ' now' : ''].filter(Boolean).join(' · '); fiat.title = f.quote ? basisWords(f.quote, facts.ccy) : ''; }
  $('#tx-when').textContent = f && f.time ? new Date(f.time * 1000).toLocaleString(undefined, { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric', hour: 'numeric', minute: '2-digit' }) : (txView.height > 0 ? '' : 'Not in a block yet');
  if (document.activeElement !== $('#tx-label')) $('#tx-label').value = labelOf(key);
  if (document.activeElement !== $('#tx-note')) $('#tx-note').value = (vault.notes || {})[key] || '';
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
  } else {                        // a deposit
    rows.push(['From', `outside this vault · ${d.inputs.length} input${d.inputs.length === 1 ? '' : 's'}`]);
    rows.push(['To', oursOut.map(outLine).join('') || '—']);
    rows.push(['Fee', d.fee !== null ? `${sats(d.fee)} sats, paid by the sender` : 'paid by the sender']);
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
    + `<div class="row" style="justify-content:space-between"><div class="label">Raw transaction</div><button class="btn small" id="tx-copyhex">Copy hex</button></div><div class="chain-hex" id="tx-hex">${esc(d.hex)}</div>`;
  $('#tx-copyhex').onclick = () => navigator.clipboard.writeText(d.hex).then(() => toast('Raw transaction copied'));
}
$('#tx-chain').addEventListener('toggle', () => { try { localStorage.setItem('a2k-chain-open', $('#tx-chain').open ? '1' : '0'); } catch {} });
$('#tx-explorer').onclick = () => { const a = document.createElement('a'); a.dataset.tx = txView.txid; document.body.appendChild(a); a.click(); a.remove(); };
$('#tx-copyid').onclick = () => navigator.clipboard.writeText(txView.txid).then(() => toast('Transaction id copied'));
// the record saves itself: the label when you leave the field or press Enter, the note a moment after you stop typing
async function saveTxField(which) {
  if (!vault || !txView.txid) return;
  const name = vault.name, key = 'tx:' + txView.txid, val = which === 'label' ? $('#tx-label').value.trim() : $('#tx-note').value;
  const cur = which === 'label' ? labelOf(key) : ((vault.notes || {})[key] || '');
  if (val.trimEnd() === cur) return;
  $('#tx-saved').textContent = 'saving…';
  try { store = await invoke(which === 'label' ? 'label_set' : 'note_set', which === 'label' ? { name, key, label: val } : { name, key, note: val }); vault = store.vaults.find(v => v.name === name) || vault;
    $('#tx-saved').textContent = 'Saved ' + new Date().toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' }) + (vault.seal && vault.seal.shares && vault.seal.shares.length ? ' · backed up with this vault\'s labels' : ' · on this computer');
    if (which === 'label') { paintCrumbs('tx'); $('#title').textContent = txHeadline(); } }
  catch (e) { $('#tx-saved').textContent = ''; fail(e); }
}
$('#tx-label').addEventListener('blur', () => saveTxField('label'));
$('#tx-label').addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); $('#tx-label').blur(); } });
$('#tx-note').addEventListener('input', () => { clearTimeout(saveTxField.t); $('#tx-saved').textContent = 'not saved yet…'; saveTxField.t = setTimeout(() => saveTxField('note'), 1200); });
$('#tx-note').addEventListener('blur', () => { clearTimeout(saveTxField.t); saveTxField('note'); });
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && $('#s-tx').classList.contains('on') && !/INPUT|TEXTAREA/.test(document.activeElement.tagName)) show('home'); });

async function fillActivity(name, s) {
  const ccy = s.servers && vault.network === 'mainnet' ? fiatCcy() : '';
  let a; try { a = await invoke('vault_activity', { name, history: s.history.map(h => [h.txid, h.height]), ccy }); }
  catch (e) { if (vault && vault.name === name) { $$('#activity .when, #activity .amt').forEach(td => { if (td.textContent === '…') td.textContent = '—'; }); $('#activity-note').textContent = 'Dates and amounts could not be fetched: ' + e; $('#activity-note').hidden = false; } return; }
  if (!vault || vault.name !== name) return;
  txFacts[name] = { rows: Object.fromEntries(a.rows.map(r => [r.txid, r])), now: a.now, ccy: a.ccy };   // kept by vault NAME and txid, never on the vault object, which is replaced on every store re-read
  if ($('#s-tx').classList.contains('on')) renderTx();
  for (const r of a.rows) {
    const tr = $(`#activity tr[data-txid="${r.txid}"]`); if (!tr) continue;
    const when = tr.querySelector('.when'), amt = tr.querySelector('.amt');
    if (r.time) { const d = new Date(r.time * 1000); when.classList.remove('muted'); when.innerHTML = `${d.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })}<small>${d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}</small>`; when.title = d.toISOString().replace('.000Z', ' UTC').replace('T', ' ') + ' · the time in the block header'; }
    else when.textContent = 'pending';
    if (r.net === null) { amt.textContent = '—'; continue; }
    const q = r.quote || (!r.time ? a.now : null);
    amt.classList.remove('muted'); amt.classList.toggle('in', r.net > 0);
    amt.innerHTML = `${r.net > 0 ? '+' : r.net < 0 ? '−' : ''}${sats(Math.abs(r.net))} sats<small>${q ? worth(Math.abs(r.net), q, a.ccy) + (r.time ? ' then' : ' now') : ''}${r.fee !== null && r.net < 0 ? (q ? ' · ' : '') + 'fee ' + sats(r.fee) : ''}</small>`;
    if (q) amt.title = basisWords(q, a.ccy);
  }
  vault._now = a.now ? { q: a.now, ccy: a.ccy } : null; paintBalance();
  if (a.note && vault.network === 'mainnet') { $('#activity-note').textContent = a.note; $('#activity-note').hidden = false; }
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
  if (sync.shown !== me) { sync.shown = me; vault._balance = undefined; vault._now = null; $('#balance').innerHTML = '— <small>sats</small>'; $('#balance-fiat').hidden = true; $('#activity').innerHTML = '<tr><td class="muted" colspan="5">syncing…</td></tr>'; $('#activity-note').hidden = true; $('#conn').textContent = ''; }
  try {
    const s = await invoke('vault_sync', { name: me });
    if (!vault || vault.name !== me) return;
    vault._coins = s.coins; vault._receive = s.receive; vault._receiveIndex = s.receive_index; vault._rotating = s.rotating; vault._height = s.height; vault._history = s.history; vault._nextChange = s.next_change; vault._gap = s.gap;
    if ($('#s-send').classList.contains('on')) renderCoins();
    if ($('#s-receive').classList.contains('on')) renderAddresses();
    vault._balance = s.balance; vault._now = null; paintBalance();
    const unconf = s.coins.filter(c => c.height === 0).reduce((a, c) => a + c.value, 0);
    $('#balance-sub').textContent = `${s.coins.length} coin${s.coins.length === 1 ? '' : 's'}` + (unconf ? ` · ${sats(unconf)} unconfirmed` : ' · all confirmed');
    $('#conn').textContent = `Connected to ${s.servers.electrum.replace(/^\w+:\/\//, '')} · block ${sats(s.height)}`;
    const tb = $('#activity'); tb.innerHTML = ''; $('#activity-note').hidden = true;
    if (!s.history.length) tb.innerHTML = '<tr><td class="muted" colspan="5">nothing yet</td></tr>';
    for (const h of s.history.slice().reverse()) {
      const tr = document.createElement('tr'); tr.dataset.txid = h.txid;
      tr.innerHTML = `<td class="when muted">${h.height > 0 ? '…' : 'pending'}</td><td><span class="mono">${h.txid.slice(0, 10)}…${h.txid.slice(-6)}</span>${s.rotating ? ' <span class="muted">· ' + fmtIndex(h.index) + '</span>' : ''}</td><td>${lblHtml('tx:' + h.txid)}</td><td class="r amt muted">…</td><td class="r ${h.height > 0 ? 'ok' : 'muted'}">${h.height > 0 ? 'confirmed · ' + sats(h.height) : 'unconfirmed'}</td>`;
      tb.appendChild(tr);
    }
    tb.onclick = (e) => { if (e.target.closest('.lbl, a, button, input')) return; const tr = e.target.closest('tr[data-txid]'); if (tr) openTx(tr.dataset.txid); };
    fillActivity(vault.name, s);   // dates, amounts and values arrive after the balance; the balance never waits for them
    $('#send-have').textContent = `${sats(s.balance)} sats`;
  } catch (e) { if (!vault || vault.name !== me) return; checkNode(); $('#balance-sub').textContent = 'could not reach the server'; $('#conn').textContent = String(e); }
}
document.addEventListener('click', (e) => { const a = e.target.closest('[data-tx]'); if (a) { e.preventDefault(); const base = ($('#conn').textContent.includes('signet') ? 'https://signet.mineracks.com' : (store.settings.custom && store.settings.explorer) || (netName() === 'mainnet' ? 'https://mempool.mineracks.com' : 'https://signet.mineracks.com')); invoke('open_url', { url: `${base}/tx/${a.dataset.tx}` }).catch(fail); } });

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
    for (const lvl of ['slow', 'medium', 'fast']) { const r = fees[lvl]; $(`[data-fee="${lvl}"] small`).textContent = `${r} sat/vB · ≈${sats(Math.ceil(r * vb))} sats`; }
    $('#fee-note').textContent = `Rates from the explorer just now. A ${vault.keys.length === 1 ? 'single-key' : vault.threshold + '-of-' + vault.keys.length} spend is about ${vb} vB per input set.`;
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
$('#scan-to').onclick = async () => {
  $('#scan-modal').classList.add('on');
  try {
    cam2Stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' } });
    $('#cam2').srcObject = cam2Stream;
    const cv = document.createElement('canvas'); const ctx = cv.getContext('2d', { willReadFrequently: true });
    cam2Timer = setInterval(() => { const v = $('#cam2'); if (!v.videoWidth) return; cv.width = v.videoWidth; cv.height = v.videoHeight; ctx.drawImage(v, 0, 0); const img = ctx.getImageData(0, 0, cv.width, cv.height); const code = jsQR(img.data, cv.width, cv.height); if (code && code.data && /^(bitcoin:|bc1|tb1|[13mn2])/i.test(code.data.trim())) { fillDestination(code.data); closeScan(); } }, 350);
  } catch { $('#cam2-note').textContent = 'No camera available on this computer. Paste the address instead.'; }
};
function closeScan() { if (cam2Timer) clearInterval(cam2Timer); cam2Timer = null; if (cam2Stream) { cam2Stream.getTracks().forEach(t => t.stop()); cam2Stream = null; } $('#scan-modal').classList.remove('on'); }
$('#scan-close').onclick = closeScan;

$('#fee-pick').addEventListener('click', (e) => { const o = e.target.closest('.opt'); if (!o) return; feeLevel = o.dataset.fee; $$('#fee-pick .opt').forEach(x => x.classList.toggle('on', x === o)); });
$('#send-all').onchange = () => { $('#send-amount').disabled = $('#send-all').checked; };
$('#build-btn').onclick = async () => {
  const to = $('#send-to').value.trim(), all = $('#send-all').checked, amount = +($('#send-amount').value.replace(/[^0-9]/g, '') || 0);
  if (!to) return toast('Enter a destination address');
  if (!all && !amount) return toast('Enter an amount in sats');
  const rate = fees ? fees[feeLevel] : 1;
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
    return `<div class="ckey ${cls}${isNano(k.pubkey) ? ' nano' : ''}">${keyGlyph(k.pubkey)}<span class="kname" title="${esc(k.name)}">${esc(k.name)}</span><span class="kstate">${label}</span></div>`;
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
  $('#gsteps').hidden = false; $('#act-row').hidden = false; $('#sign-waiting').hidden = true; $('#sign-cancel').hidden = false;
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
const qrShow = { timer: null, parts: [], single: null, still: false, i: 0 };
function drawQr(text) {
  const qr = qrcode(0, 'L'); qr.addData(text, 'Alphanumeric'); qr.make();
  const n = qr.getModuleCount(), cv = $('#qr-canvas'), ctx = cv.getContext('2d'), m = Math.floor(cv.width / (n + 8)), off = Math.floor((cv.width - m * n) / 2);
  ctx.fillStyle = '#fff'; ctx.fillRect(0, 0, cv.width, cv.height); ctx.fillStyle = '#000';
  for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) if (qr.isDark(r, c)) ctx.fillRect(off + c * m, off + r * m, m, m);
}
function stopQr() { clearInterval(qrShow.timer); qrShow.timer = null; $('#qr-panel').hidden = true; }
function playQr() {
  clearInterval(qrShow.timer); qrShow.timer = null;
  const still = qrShow.still && qrShow.single, plain = qrShow.parts.length / 2;
  $('#qr-mode').hidden = !qrShow.single || qrShow.parts.length < 2; $('#qr-mode').textContent = still ? 'Show the animated code' : 'Show one still code';
  if (still || qrShow.parts.length < 2) { drawQr(qrShow.single || qrShow.parts[0]); $('#qr-title').textContent = 'One code'; $('#qr-note').textContent = 'UR crypto-psbt, the format Sparrow, Keystone, Passport, Jade and SeedSigner scan. Dense: hold the camera steady, or use the animated code.'; return; }
  const tick = () => { const i = qrShow.i++ % qrShow.parts.length; drawQr(qrShow.parts[i]); $('#qr-title').textContent = `Frame ${i + 1} of ${qrShow.parts.length}`; };
  $('#qr-note').textContent = `Point the other wallet's camera at it and wait. Frames 1 to ${plain} carry the transaction; the rest are spare frames, each a mix of several parts, so a camera that starts late or misses one still finishes. UR crypto-psbt, as Sparrow, Keystone, Passport, Jade and SeedSigner scan.`;
  tick(); qrShow.timer = setInterval(tick, 260);
}
$('#export-qr').onclick = async () => { if (!$('#qr-panel').hidden) return stopQr(); try { const r = await invoke('spend_qr'); qrShow.parts = r.parts; qrShow.single = r.single; qrShow.still = !!r.single && r.parts.length < 2; qrShow.i = 0; $('#qr-panel').hidden = false; playQr(); } catch (e) { fail(e); } };
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
    const r = await invoke('spend_broadcast', { name: vault.name }); const paid = bookOf().find(c => dests.includes(c.address.toLowerCase()));
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
  const card = $('#addr-card'); if (!vault || !vault._rotating) { card.hidden = true; return; }
  try {
    const rows = await invoke('vault_addresses', { name: vault.name, ahead: addrAhead });
    const coins = vault._coins || [], hist = vault._history || [];
    for (const [chain, sel] of [['receive', '#addr-recv tbody'], ['change', '#addr-chg tbody']]) {
      const tb = $(sel); tb.innerHTML = '';
      for (const r of rows.filter(r => r.chain === chain).slice().reverse()) {
        const bal = coins.filter(c => c.index === r.index).reduce((a, c) => a + c.value, 0);
        const used = hist.some(x => x.index === r.index);
        const tr = document.createElement('tr'); tr.className = 'rowpick' + (r.current ? ' cur' : '') + (r.future ? ' future' : '');
        tr.innerHTML = `<td class="muted">${r.position}</td><td class="mono" title="${r.address}">${r.address}</td><td>${lblHtml('addr:' + r.address)}</td><td class="r" style="font-variant-numeric:tabular-nums">${bal ? sats(bal) + ' sats' : '<span class="muted">—</span>'}</td><td class="r ${r.current ? 'ok' : 'muted'}">${r.current ? (chain === 'change' ? 'next change' : 'next deposit') : r.future ? 'unused · ahead' : used ? (bal ? 'in use' : 'spent') : 'unused'}</td>`;
        tr.addEventListener('click', (e) => { if (e.target.closest('.lbl')) return; drawAddress(r); window.scrollTo({ top: 0, behavior: 'smooth' }); });
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
function renderActivityLabels() { $$('#activity .lbl').forEach(box => { const key = box.dataset.lkey; const t = box.querySelector('.text'); if (t && t.textContent !== labelOf(key)) t.textContent = labelOf(key); }); }
$('#labels-sync').onclick = () => { $('#labels-out').textContent = 'syncing labels…'; labelsSync(); };
$('#labels-export').onclick = async () => { try { $('#labels-out').textContent = 'Saved ' + await invoke('labels_export', { name: vault.name }); } catch (e) { if (!/cancelled/.test(String(e))) $('#labels-out').textContent = String(e); } };
$('#labels-import').onclick = async () => { try { const n = await invoke('labels_import', { name: vault.name }); labelsSync(); store = await invoke('store_get'); vault = store.vaults.find(x => x.name === vault.name) || vault; $('#labels-out').textContent = `${n} label${n === 1 ? '' : 's'} imported into ${vault.name}.`; } catch (e) { if (!/cancelled/.test(String(e))) $('#labels-out').textContent = String(e); } };
$('#bsms-export').onclick = async () => { try { toast('Saved ' + await invoke('bsms_export', { name: vault.name }), 6000); } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
$('#bsms-open').onclick = async () => { try { const r = await invoke('bsms_import'); $('#desc-in').value = r.descriptor; readDescriptor(r.descriptor); if (r.first_address) toast('BSMS read. First address in the file: ' + r.first_address.slice(0, 12) + '…', 6000); } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };

// ---------- keys ----------
// The recovery line says what the vault holds. Until Reveal is pressed the page carries a scrambled stand-in of the same
// shape, blurred — so a screenshot, a screen share or a screen reader gets nothing. Copy and Save as PDF work without revealing.
const standIn = (line) => { let x = line.length * 2654435761 >>> 0; return line.replace(/[a-z0-9]/gi, () => { x = (x * 1664525 + 1013904223) >>> 0; return '0123456789abcdef'[x >>> 28]; }); };
function paintDesc() {
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
const keysBusy = () => $$('#keys-list input').some(i => i.value || i === document.activeElement);   // a poll must not wipe a half-typed PIN or name
const PENCIL = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/></svg>';
function vaultKeyRow(k, i) {
  const card = cards.find(c => c.pubkey === k.pubkey), pf = packedForm(k.pubkey);
  const art = card ? keyArt(card.form || 3, !!card.nfc, 52) : pf ? keyArt(pf, !!(pf & 0x80), 52) : '<svg width="24" height="24"><use href="#ring"/></svg>';
  const model = card ? modelName(card.form, card.nfc) : pf ? modelName(pf, !!(pf & 0x80), !!(pf & 0x40)) : 'YubiKey';
  const facts = [model, k.serial ? 'serial ' + k.serial : 'serial not seen on this computer yet'].concat(card ? [card.firmware ? 'firmware ' + card.firmware : '', 'touch ' + String(card.touch).toLowerCase()].filter(Boolean) : []);
  return `<div class="keyrow vk"><span class="keyart">${art}</span><div class="who"><div class="name"><span class="name-text">${esc(k.name)}</span><button class="pencil" data-edit-idx="${i}" title="Rename this key">${PENCIL}</button></div><div class="muted" style="font-size:13px">${esc(facts.join(' · '))} · <span class="mono">${k.pubkey.slice(0, 12)}…</span></div></div>`
    + `<div class="kright"><div class="state ${card ? 'here' : ''}"><i></i>${card ? 'plugged in' : 'not plugged in'}</div><div class="kseal" data-seal-slot="${i}"></div></div></div>`;
}
function renderKeys(force) {
  if (!vault) return;
  if (force || !keysBusy()) { $('#keys-list').innerHTML = vault.keys.map(vaultKeyRow).join(''); paintSealSlots(); }
  invoke('vault_recovery', { name: vault.name }).then(l => { if (vault._recovery !== l) hideDesc.shown = false; vault._recovery = l; paintDesc(); }); $('#keys-k').textContent = vault.threshold; $$('.keys-k2').forEach(e => e.textContent = vault.threshold);
  renderKeysNow(); renderSeal();
}
function keyRow(name, sub, pub, state, idx) { const pen = idx === undefined ? '' : `<button class="pencil" data-edit-idx="${idx}" title="Rename this key"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/></svg></button>`; const pf = packedForm(pub); return `<div class="keyrow"><span class="keyart">${pf ? keyArt(pf, !!(pf & 0x80), 44) : '<svg width="22" height="22"><use href="#ring"/></svg>'}</span><div class="who"><div class="name"><span class="name-text">${name}</span>${pen}</div><div class="muted" style="font-size:13px">${pf ? modelName(pf, !!(pf & 0x80), !!(pf & 0x40)) + ' · ' : ''}${sub} · <span class="mono">${pub.slice(0, 12)}…</span></div></div><div class="state ${state}"><i></i>${{ ok: 'enrolled', off: 'not plugged in', here: 'plugged in' }[state]}</div></div>`; }
function renderKeysNow() {
  // keys of this vault say "plugged in" on their own rows; this line is only for what ELSE is in the ports
  const others = cards.filter(c => !vault || !vault.keys.some(k => k.pubkey === c.pubkey));
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
    vault = store.vaults.find(v => v.name === newName); try { localStorage.setItem('a2k-vault', vault.name); } catch {}
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
  $('#setup-done').innerHTML = setup.keys.map(k => keyRow(k.name, `serial ${k.serial}`, k.pubkey, 'ok')).join('');
  const box = $('#setup-now');
  const remaining = setup.n - setup.keys.length;
  $('#setup-finish').disabled = remaining > 0;
  if (remaining === 0) { box.innerHTML = `<div class="ok">All ${setup.n} keys enrolled.</div>`; return; }
  const fresh = cards.find(c => !setup.keys.some(k => k.serial === c.serial));
  if (!fresh) { box.innerHTML = `<div class="muted">Plug in key ${setup.keys.length + 1} of ${setup.n}.</div>`; return; }
  const nameGuess = ['laptop', 'keyring', 'off-site'][setup.keys.length] || `key ${setup.keys.length + 1}`;
  const has = !!fresh.pubkey;
  box.innerHTML = `<div style="font-family:var(--display);font-weight:600;font-size:15px">Key ${setup.keys.length + 1}: ${cardLabel(fresh)}</div>
    <div class="muted" style="font-size:13px">${has ? 'This card already holds a signing key (touch ' + fresh.touch + '). Use it: the same card key can belong to any number of vaults.' : 'OpenPGP signature slot is empty: a key will be generated on the card.'}</div>
    <div class="grid2"><div class="field"><div class="label">Name this key</div><input id="enrol-name" value="${nameGuess}"></div>${has ? '' : '<div class="field"><div class="label">Admin PIN</div><input id="enrol-admin" type="password" autocomplete="off"></div>'}</div>
    ${has ? '' : '<div class="muted" style="font-size:13px">Factory admin PIN is 12345678. Change it first with YubiKey Manager if you have not. Touch policy will be set to FIXED, which cannot be undone without wiping the key.</div>'}
    <div class="row">${has ? '<button class="btn primary" id="use-btn">Use this card\'s key</button><button class="btn" id="replace-btn" style="border-color:var(--no);color:var(--no)">Replace it with a new key…</button>' : '<button class="btn primary" id="enrol-btn">Generate key on card</button>'}</div>`;
  if (has) {
    $('#use-btn').onclick = () => { setup.keys.push({ name: $('#enrol-name').value.trim() || nameGuess, serial: fresh.serial, pubkey: fresh.pubkey }); toast('enrolled with its existing key'); renderSetup(); };
    askTwice($('#replace-btn'), 'Generate a NEW key on this card? The key it holds now will be destroyed, and any vault it belongs to loses that cosigner for good. Only for a card that has never held funds.', 'Yes, replace its key', async () => {
      box.innerHTML += '<div class="field" style="margin-top:12px"><div class="label">Admin PIN</div><input id="enrol-admin" type="password" autocomplete="off" style="max-width:260px"></div><button class="btn" id="enrol-btn" style="border-color:var(--no);color:var(--no);margin-top:10px">Generate a new key, destroying the old one</button>';
      wireGenerate();
    });
  } else wireGenerate();
  function wireGenerate() { $('#enrol-btn').onclick = async () => {
    const name = $('#enrol-name').value.trim() || nameGuess, admin = $('#enrol-admin').value;
    if (!admin) return toast('Enter the admin PIN');
    $('#enrol-btn').disabled = true;
    try { const pub = await invoke('card_generate', { reader: fresh.reader, adminPin: admin }); setup.keys.push({ name, serial: fresh.serial, pubkey: pub }); toast(`${name} enrolled`); }
    catch (e) { fail(e); }
    renderSetup();
  }; }
}
$('#preset-pick').addEventListener('click', (e) => { const o = e.target.closest('.opt'); if (!o) return; if (setup.keys.length > +o.dataset.n) return toast('More keys already enrolled than that shape holds'); setup.n = +o.dataset.n; setup.k = +o.dataset.k; renderSetup(); });
$('#n-less').onclick = () => { if (setup.n > Math.max(2, setup.keys.length)) { setup.n--; if (setup.k > setup.n) setup.k = setup.n; renderSetup(); } };
$('#n-more').onclick = () => { if (setup.n < 15) { setup.n++; renderSetup(); } };
$('#setup-net-pick').addEventListener('click', (e) => { const o = e.target.closest('.opt'); if (o) { setup.network = o.dataset.net; renderSetup(); } });
$('#setup-finish').onclick = async () => {
  try { vault = await invoke('vault_create', { name: $('#setup-name').value.trim() || 'vault', threshold: setup.k, keys: setup.keys, rotating: true, network: setup.network }); store = await invoke('store_get'); try { localStorage.setItem('a2k-vault', vault.name); } catch {} toast('Vault created'); paintNet(); show('keys'); }
  catch (e) { fail(e); }
};

// single key
$('#single-btn').onclick = () => { $('#single-ack').checked = false; $('#single-go').disabled = true; $('#single-modal').classList.add('on'); };
$('#single-ack').onchange = () => { $('#single-go').disabled = !$('#single-ack').checked; };
$('#single-instead').onclick = () => { $('#single-modal').classList.remove('on'); setup = { k: 2, n: 3, keys: [], network: store.settings.network }; show('setup'); };
$('#single-go').onclick = () => { $('#single-modal').classList.remove('on'); setup = { k: 1, n: 1, keys: [], network: store.settings.network }; show('setup'); };

// ---------- the seal: recovery shares on the keys ----------
function paintSealSlots() {
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
    try { vault = await invoke('seal_restore', { id, name: $(`[data-seal-name="${id}"]`).value, found: sealFound }); store = await invoke('store_get'); try { localStorage.setItem('a2k-vault', vault.name); } catch {} sealFound = []; readSeals.sig = null; toast('Vault restored from its keys'); paintNet(); renderTabs(); show('home'); }
    catch (e) { fail(e); }
  });
  $$('[data-seal-adopt]').forEach(b => b.onclick = async () => { try { store = await invoke('seal_adopt', { name: b.dataset.sealAdopt, found: sealFound }); toast('Shares learnt. That vault’s Keys screen now knows its seal.'); } catch (e) { fail(e); } });
}

// ---------- restore ----------
function restoreTab(name) { restoreTab.now = name; $$('.tabs div').forEach(x => x.classList.toggle('on', x.dataset.tab === name)); for (const t of ['keys', 'scan', 'paste']) $('#tab-' + t).hidden = t !== name; $('#sheet-note').hidden = name === 'keys'; if (name === 'scan') startCam(); else stopCam(); if (name === 'keys') { readSeals.sig = null; readSeals(); } }
$$('.tabs div').forEach(t => t.onclick = () => restoreTab(t.dataset.tab));
async function startCam() {
  if (camStream) return;
  try { camStream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' } }); $('#cam').srcObject = camStream; const cv = document.createElement('canvas'); const ctx = cv.getContext('2d', { willReadFrequently: true });
    camTimer = setInterval(() => { const v = $('#cam'); if (!v.videoWidth) return; cv.width = v.videoWidth; cv.height = v.videoHeight; ctx.drawImage(v, 0, 0); const img = ctx.getImageData(0, 0, cv.width, cv.height); const code = jsQR(img.data, cv.width, cv.height); if (code && code.data && /^(wsh|wpkh)\(/.test(code.data)) { $('#cam-note').textContent = 'Read a descriptor from the code.'; readDescriptor(code.data); } }, 400);
  } catch (e) { $('#cam-note').textContent = 'No camera available. Paste the line instead.'; }
}
function stopCam() { if (camTimer) clearInterval(camTimer); camTimer = null; if (camStream) { camStream.getTracks().forEach(t => t.stop()); camStream = null; } }
$('#desc-read').onclick = () => readDescriptor($('#desc-in').value);
let parsed = null;
async function readDescriptor(text) {
  try {
    parsed = await invoke('descriptor_parse', { text });
    const names = ['laptop', 'keyring', 'off-site'];
    $('#restore-card').innerHTML = `<div class="ok">Read: ${parsed.threshold} of ${parsed.keys.length} over card keys, checksum ok${parsed.rotating ? ', with a chain code (rotating addresses)' : ' (one fixed address)'}</div>
      <div class="mono muted" style="background:var(--ground);padding:12px;border-radius:6px;border:1px solid var(--line)">${parsed.descriptor}</div>
      <div style="display:flex;flex-direction:column;gap:8px">${parsed.keys.map((k, i) => `<div class="row" style="justify-content:space-between"><span class="mono">${k.slice(0, 16)}…</span><input data-kname="${i}" value="${names[i] || 'key ' + (i + 1)}" style="max-width:160px;padding:6px 10px;border:1px solid var(--line);border-radius:6px;background:var(--panel);color:var(--ink)"><span class="${cards.some(c => c.pubkey === k) ? 'ok' : 'muted'}" style="font-size:13px">${cards.some(c => c.pubkey === k) ? 'plugged in' : 'not seen yet'}</span></div>`).join('')}</div>
      <div class="muted" style="font-size:13px">First address on ${netName()}: <span class="mono">${parsed.address}</span>. The vault can be watched now; spending needs ${parsed.threshold} of these keys.</div>
      <div class="field"><div class="label">Vault name</div><input id="restore-name" value="vault"></div>
      <button class="btn primary" id="restore-go">Restore this vault</button>`;
    $('#restore-go').onclick = async () => {
      try { vault = await invoke('vault_restore', { name: $('#restore-name').value.trim() || 'vault', text: parsed.descriptor, names: $$('[data-kname]').map(i => i.value.trim()) }); store = await invoke('store_get'); try { localStorage.setItem('a2k-vault', vault.name); } catch {} stopCam(); toast('Vault restored'); paintNet(); show('home'); }
      catch (e) { fail(e); }
    };
  } catch (e) { $('#restore-card').innerHTML = `<div class="warn">${e}</div>`; }
}

// ---------- settings ----------
function renderSettings() {
  $$('#net-pick .opt').forEach(o => o.classList.toggle('on', o.dataset.net === store.settings.network));
  const def = store.settings.network === 'mainnet' ? ['ssl://electrum.mineracks.com:50002', 'https://mempool.mineracks.com'] : ['tcp://100.94.4.10:50001', 'https://signet.mineracks.com'];
  $('#def-electrum').textContent = def[0]; $('#def-explorer').textContent = def[1];
  $('#own-servers').classList.toggle('on', !store.settings.custom);
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
  const net = vault ? vault.network : store.settings.network; checkNode.net = net; const mine = ++checkNode.n;
  if ($('#node').dataset.state !== 'online') paintNode('checking', 'Connecting…', 'Asking the ' + net + ' server…');
  try {
    const r = await invoke('node_status', { network: net }); if (mine !== checkNode.n) return;
    const host = r.electrum.replace(/^\w+:\/\//, '');
    paintNode('online', 'Online · ' + (r.ours ? 'mineracks node' : host), `${net} · ${host} · block ${sats(r.height)} · answered in ${r.ms} ms · checked ${new Date().toLocaleTimeString()}. Click to check again.`);
  } catch (e) { if (mine !== checkNode.n) return; paintNode('offline', 'Offline · no answer', `${net}: ${String(e)}. Balances and sends need the server. Click to try again.`); }
}
checkNode.n = 0;
$('#node').onclick = () => { paintNode('checking', 'Checking…', ''); checkNode(); };
setInterval(() => { if (!document.hidden) checkNode(); }, 30000);
document.addEventListener('visibilitychange', () => { if (!document.hidden) checkNode(); });

// money moved: say so in the window too. Cards stack, stay ten seconds, and a click opens that vault's Home.
function notice(ev) {
  const el = document.createElement('div'); el.className = 'notice-card ' + (ev.kind || ''); el.setAttribute('role', 'status');
  el.innerHTML = `<div><div class="nt">${esc(ev.title)}</div><div class="nb">${esc(ev.body)}</div></div><button type="button" class="nx" aria-label="Dismiss">×</button>`;
  const close = () => el.remove();
  el.querySelector('.nx').onclick = (e) => { e.stopPropagation(); close(); };
  el.onclick = () => { const v = store.vaults.find(v => v.name === ev.vault); if (v) { vault = v; try { localStorage.setItem('a2k-vault', v.name); } catch {} paintNet(); renderTabs(); show('home'); } close(); };
  $('#notices').appendChild(el); while ($('#notices').children.length > 4) $('#notices').firstChild.remove();
  setTimeout(close, 10000);
}
window.__TAURI__.event.listen('vault-event', (ev) => notice(ev.payload));
// the background watcher saw a deposit, a spend or a confirmation: refresh what is on screen
window.__TAURI__.event.listen('vault-changed', (ev) => {
  if (!vault || ev.payload !== vault.name) { return; }
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
