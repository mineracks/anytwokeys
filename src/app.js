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
    if (save && v !== cur) { try { store = await invoke('label_set', { name: vault.name, key, label: v }); vault = store.vaults.find(x => x.name === vault.name) || vault; } catch (err) { fail(err); } }
    box.innerHTML = `<span class="text">${esc(labelOf(key))}</span><button class="pencil" title="Label">${PEN}</button>`;
  };
  inp.addEventListener('keydown', (ev) => { if (ev.key === 'Enter') { ev.preventDefault(); finish(true); } if (ev.key === 'Escape') finish(false); });
  inp.addEventListener('blur', () => finish(true));
  inp.addEventListener('click', (ev) => ev.stopPropagation());
});

function toast(t, ms = 3500) { const el = $('#toast'); el.textContent = t; el.classList.add('on'); clearTimeout(el._t); el._t = setTimeout(() => el.classList.remove('on'), ms); }
function fail(e) { toast(String(e && e.message || e), 6000); console.error(e); }

// ---------- screens ----------
function show(name) {
  const first = ['setup', 'restore'].includes(name) && !vault;
  $('#welcome').hidden = true; $('#shell').hidden = false;
  $$('.screen').forEach(s => s.classList.toggle('on', s.id === 's-' + name));
  const navName = name === 'sign' ? (paintCrumbs.via || 'send') : name;   // Sign has no sidebar entry of its own: light the screen it came from
  $$('.nav').forEach(n => n.classList.toggle('on', n.dataset.go === navName));
  $('#title').textContent = { home: vault ? vault.name : 'Vault', send: 'Send', receive: 'Receive', keys: 'Keys', settings: 'Settings', setup: 'New vault · enrol keys', restore: 'Restore a vault', sign: 'Sign', messages: 'Messages' }[name] || '';
  $('#title-edit').hidden = !(name === 'home' && vault); $('#title-input').hidden = true; $('#title').hidden = false;
  paintCrumbs(name); paintStaged(name);
  if (name !== 'restore') stopCam();
  if (name !== 'sign') stopQr();
  if (name === 'home') sync();
  if (name === 'receive') { drawAddress(); renderAddresses(); }
  if (name === 'send') { loadFees(); if (vault && !vault._coins) sync().then(renderCoins); else renderCoins(); }
  if (name === 'messages') renderMessages();
  if (name === 'keys') renderKeys();
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
  }
  const here = { home: '', send: 'Send', receive: 'Receive', keys: 'Keys', settings: '', setup: 'New vault', restore: 'Restore', sign: 'Sign', messages: 'Messages' }[name] || '';
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
    const d = document.createElement('div'); d.className = 'nav' + (vault && v.name === vault.name ? ' on' : ''); d.style.fontWeight = '600';
    d.innerHTML = `<span style="width:8px;height:8px;border-radius:50%;background:${v.network === 'mainnet' ? 'var(--gold)' : 'var(--signal)'};display:inline-block;flex:0 0 8px"></span><span style="flex-grow:1">${v.name}</span><span class="muted" style="font-size:11px;font-weight:400">${v.keys.length === 1 ? '1 key' : v.threshold + ' of ' + v.keys.length}</span>`;
    d.onclick = () => { vault = v; try { localStorage.setItem('a2k-vault', v.name); } catch {} paintNet(); show('home'); };
    tabs.appendChild(d);
  }
}

async function boot() {
  store = await invoke('store_get');
  paintNet();
  let want = null; try { want = localStorage.getItem('a2k-vault'); } catch {}
  vault = store.vaults.find(v => v.name === want) || store.vaults[0] || null;
  paintNet();
  if (vault) show('home'); else showWelcome();
  pollCards();
}

// ---------- cards ----------
async function pollCards() {
  try { cards = await invoke('cards_list'); cardsErr = ''; } catch (e) { cards = []; cardsErr = String(e); }
  for (const c of cards) if (c.pubkey && c.form && (store.forms || {})[c.pubkey] !== c.form) { try { store = await invoke('key_form_seen', { pubkey: c.pubkey, form: c.form }); renderSign.tiles = null; } catch {} }
  if ($('#s-setup').classList.contains('on')) renderSetup();
  if ($('#s-keys').classList.contains('on')) { const sig = cards.map(c => c.serial + ':' + c.pubkey).sort().join(','); if (sig !== pollCards.sig) { pollCards.sig = sig; if (!$('#keys-list input')) renderKeys(); } else renderKeysNow(); }
  if (vault) for (const c of cards) { const k = vault.keys.find(k => c.pubkey && k.pubkey === c.pubkey && !k.serial); if (k && c.serial) { invoke('vault_key_seen', { name: vault.name, pubkey: c.pubkey, serial: c.serial }).then(st => { store = st; vault = st.vaults.find(v => v.name === vault.name) || vault; if ($('#s-keys').classList.contains('on')) renderKeys(); }).catch(() => {}); } }
  if ($('#s-restore').classList.contains('on') && restoreTab.now === 'keys') readSeals();
  if ($('#s-sign').classList.contains('on')) renderSign();
  if ($('#s-messages').classList.contains('on')) { const c = cards.find(c => c.pubkey); const was = renderMessages.had; renderMessages.had = !!c; if (was !== !!c) renderMessages(); }
  setTimeout(pollCards, 2000);
}
function cardLabel(c) { return `YubiKey 5 · serial ${c.serial} · ${c.version}`; }

// ---------- home ----------
async function sync() {
  if (!vault) return;
  $('#vault-kind').textContent = vault.keys.length === 1 ? 'Single key' : `${vault.threshold} of ${vault.keys.length} keys sign`;
  $('#vault-keys').textContent = vault.keys.map(k => k.name).join(' · ');
  $('#balance-sub').textContent = 'syncing…';
  try {
    const s = await invoke('vault_sync', { name: vault.name });
    vault._coins = s.coins; vault._receive = s.receive; vault._receiveIndex = s.receive_index; vault._rotating = s.rotating; vault._height = s.height; vault._history = s.history; vault._nextChange = s.next_change; vault._gap = s.gap;
    if ($('#s-send').classList.contains('on')) renderCoins();
    if ($('#s-receive').classList.contains('on')) renderAddresses();
    $('#balance').innerHTML = `${sats(s.balance)} <small>sats</small>`;
    const unconf = s.coins.filter(c => c.height === 0).reduce((a, c) => a + c.value, 0);
    $('#balance-sub').textContent = `${s.coins.length} coin${s.coins.length === 1 ? '' : 's'}` + (unconf ? ` · ${sats(unconf)} unconfirmed` : ' · all confirmed');
    $('#conn').textContent = `Connected to ${s.servers.electrum.replace(/^\w+:\/\//, '')} · block ${sats(s.height)}`;
    const tb = $('#activity'); tb.innerHTML = '';
    if (!s.history.length) tb.innerHTML = '<tr><td class="muted" colspan="3">nothing yet</td></tr>';
    for (const h of s.history.slice().reverse()) {
      const tr = document.createElement('tr');
      tr.innerHTML = `<td><a href="#" data-tx="${h.txid}" class="mono">${h.txid.slice(0, 16)}…${h.txid.slice(-6)}</a>${s.rotating ? ' <span class="muted">· ' + fmtIndex(h.index) + '</span>' : ''}</td><td>${lblHtml('tx:' + h.txid)}</td><td class="r ${h.height > 0 ? 'ok' : 'muted'}">${h.height > 0 ? 'confirmed · ' + sats(h.height) : 'unconfirmed'}</td>`;
      tb.appendChild(tr);
    }
    $('#send-have').textContent = `${sats(s.balance)} sats`;
  } catch (e) { $('#balance-sub').textContent = 'could not reach the server'; $('#conn').textContent = String(e); }
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
  $('#send-to').value = t;
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
const isNano = (pub) => [2, 4].includes((store.forms || {})[pub]);
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
    const cls = full(k) ? 'signed' : k.signed ? 'part' : canSign(k) ? 'here' : st.ready ? 'spare' : '';
    const label = full(k) ? '✓ signed' : k.signed ? `${k.signed} of ${st.inputs} inputs` : canSign(k) ? 'plugged in · sign now' : st.ready ? 'not needed' : 'not yet';
    return `<div class="ckey ${cls}${isNano(k.pubkey) ? ' nano' : ''}">${keyGlyph(k.pubkey)}<span class="kname" title="${esc(k.name)}">${esc(k.name)}</span><span class="kstate">${label}</span></div>`;
  }).join('');
  if (renderSign.tiles !== tiles) { renderSign.tiles = tiles; $('#cer-keys').innerHTML = tiles; }   // unchanged tiles keep their pulse running
  const staged = (await invoke('store_get')).last_message;
  paintCrumbs.via = staged ? 'messages' : 'send'; paintCrumbs('sign');
  if (staged) { $('#sign-summary').innerHTML = `Message for <span class="mono">${staged.address.slice(0, 14)}…${staged.address.slice(-6)}</span>: “${staged.message.length > 60 ? staged.message.slice(0, 60) + '…' : staged.message}”`; $('#message-summary').textContent = `The proof for ${staged.address} is fully signed. Nothing is broadcast; the signature is text you can hand to whoever asked.`; }
  $('#message-card').hidden = !(st.ready && staged);
  $('#broadcast-card').hidden = !st.ready || !!staged;
  $('#export-card').hidden = !!staged; $('#export-tx').hidden = !st.ready;
  if (st.ready) { $('#sign-title').textContent = 'All signatures in'; $('#sign-text').textContent = staged ? 'Produce the signature below; nothing is broadcast.' : 'Nothing has been sent. Review the summary, then send it.'; $('#pin-field').hidden = true; $('#sign-warn').hidden = true; $('#sign-btn').disabled = true; $('#broadcast-text').textContent = `Broadcast to ${netName()} through the Electrum server.`; return; }
  const usable = cards.filter(c => c.pubkey && st.keys.some(k => k.pubkey === c.pubkey && !full(k)));
  if (!usable.length) { $('#sign-title').textContent = 'Plug in a key'; $('#sign-text').textContent = cards.length ? 'The plugged-in key is not one of this vault\'s, or it has already signed.' : 'Any of the vault\'s keys that has not signed yet.'; $('#pin-field').hidden = true; $('#sign-warn').hidden = true; $('#sign-btn').disabled = true; return; }
  const c = usable[0]; const k = vault.keys.find(k => k.pubkey === c.pubkey);
  $('#sign-title').textContent = `${k.name} is plugged in`; $('#sign-text').textContent = `${cardLabel(c)}. Enter its PIN, press Sign, then touch the key when it blinks — once per input.`;
  $('#pin-field').hidden = false; $('#sign-warn').hidden = false; $('#sign-btn').disabled = false;
  $('#sign-btn').onclick = async () => {
    const pin = $('#pin').value; if (!pin) return toast('Enter the card PIN');
    $('#sign-btn').disabled = true; $('#sign-title').textContent = 'TOUCH THE KEY NOW'; $('#sign-text').textContent = 'It is blinking. Touch it once per input. This closes by itself when the card has signed.';
    try { const r = await invoke('spend_sign', { name: vault.name, reader: c.reader, pin }); $('#pin').value = ''; toast(`${r.signer} signed ${r.signed_inputs.length} input${r.signed_inputs.length === 1 ? '' : 's'}`); }
    catch (e) { fail(e); }
    renderSign();
  };
}
$('#export-psbt').onclick = async () => { try { const p = await invoke('spend_export', { name: vault.name, what: 'psbt' }); $('#export-note').textContent = 'Saved ' + p; } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
$('#export-tx').onclick = async () => { try { const p = await invoke('spend_export', { name: vault.name, what: 'tx' }); $('#export-note').textContent = 'Saved ' + p + ' — raw hex, broadcastable from any wallet or explorer.'; } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
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
  const tick = () => { const i = qrShow.i++ % qrShow.parts.length; drawQr(qrShow.parts[i]); $('#qr-title').textContent = i < plain ? `Part ${i + 1} of ${plain}` : `Part ${plain} of ${plain} · repair`; };
  $('#qr-note').textContent = 'An animated UR crypto-psbt code, as Sparrow, Keystone, Passport, Jade and SeedSigner scan. Point the other wallet\'s camera at it and wait; it can join at any moment.';
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
$('#staged-discard').onclick = async () => { if (!confirm('Discard the transaction that is waiting for signatures? Nothing has been sent; any signatures on it are lost.')) return; await invoke('spend_discard'); built = null; location.reload(); };
$('#sign-cancel').onclick = async () => { await invoke('spend_discard'); built = null; coinPick.clear(); $('#build-btn').textContent = 'Build transaction'; $('#build-btn').onclick = null; show('send'); location.reload(); };
$('#broadcast-btn').onclick = async () => {
  $('#broadcast-btn').disabled = true;
  try { const r = await invoke('spend_broadcast', { name: vault.name }); built = null; coinPick.clear(); toast('Sent. ' + r.txid.slice(0, 16) + '…', 8000); $('#broadcast-text').innerHTML = `Broadcast. <a href="#" data-tx="${r.txid}">${r.txid}</a>`; setTimeout(() => show('home'), 2500); }
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
$('#notify-test').onclick = async () => { try { $('#notify-out').textContent = await invoke('notify_test'); } catch (e) { $('#notify-out').textContent = String(e); } };
$('#labels-export').onclick = async () => { try { $('#labels-out').textContent = 'Saved ' + await invoke('labels_export', { name: vault.name }); } catch (e) { if (!/cancelled/.test(String(e))) $('#labels-out').textContent = String(e); } };
$('#labels-import').onclick = async () => { try { const n = await invoke('labels_import', { name: vault.name }); store = await invoke('store_get'); vault = store.vaults.find(x => x.name === vault.name) || vault; $('#labels-out').textContent = `${n} label${n === 1 ? '' : 's'} imported into ${vault.name}.`; } catch (e) { if (!/cancelled/.test(String(e))) $('#labels-out').textContent = String(e); } };
$('#bsms-export').onclick = async () => { try { toast('Saved ' + await invoke('bsms_export', { name: vault.name }), 6000); } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };
$('#bsms-open').onclick = async () => { try { const r = await invoke('bsms_import'); $('#desc-in').value = r.descriptor; readDescriptor(r.descriptor); if (r.first_address) toast('BSMS read. First address in the file: ' + r.first_address.slice(0, 12) + '…', 6000); } catch (e) { if (!/cancelled/.test(String(e))) fail(e); } };

// ---------- keys ----------
function renderKeys() {
  if (!vault) return;
  $('#keys-list').innerHTML = vault.keys.map(k => keyRow(k.name, k.serial ? `serial ${k.serial}` : 'serial not yet seen (restored)', k.pubkey, cards.some(c => c.pubkey === k.pubkey) ? 'here' : 'off', vault.keys.indexOf(k))).join('');
  invoke('vault_recovery', { name: vault.name }).then(l => { $('#keys-desc').textContent = l; vault._recovery = l; }); $('#keys-k').textContent = vault.threshold; $$('.keys-k2').forEach(e => e.textContent = vault.threshold);
  renderKeysNow(); renderSeal();
}
function keyRow(name, sub, pub, state, idx) { const pen = idx === undefined ? '' : `<button class="pencil" data-edit-idx="${idx}" title="Rename this key"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/></svg></button>`; return `<div class="keyrow"><svg width="22" height="22"><use href="#ring"/></svg><div class="who"><div class="name"><span class="name-text">${name}</span>${pen}</div><div class="muted" style="font-size:13px">${sub} · <span class="mono">${pub.slice(0, 12)}…</span></div></div><div class="state ${state}"><i></i>${{ ok: 'enrolled', off: 'not plugged in', here: 'plugged in' }[state]}</div></div>`; }
function renderKeysNow() { $('#keys-now').innerHTML = cards.length ? cards.map(c => `${cardLabel(c)} · ${c.pubkey ? 'signing key present' : 'no signing key'} · touch ${c.touch}`).join('<br>') : (cardsErr && !/no YubiKey found/i.test(cardsErr) ? 'Card reader problem: ' + cardsErr : 'No YubiKey plugged in.'); }
// rename a key in place from the pencil on its row
$('#keys-list').addEventListener('click', (e) => {
  const b = e.target.closest('[data-edit-idx]'); if (!b || $('#keys-list input')) return;
  const idx = +b.dataset.editIdx, nameEl = b.parentElement, cur = vault.keys[idx].name;
  const inp = document.createElement('input'); inp.className = 'key-input'; inp.value = cur; inp.maxLength = 40;
  nameEl.replaceChildren(inp); inp.focus(); inp.select();
  let done = false;
  const finish = async (save) => {
    if (done) return; done = true;
    const v = inp.value.trim();
    if (!save || !v || v === cur) { renderKeys(); return; }
    try {
      const names = vault.keys.map((k, i) => i === idx ? v : k.name);
      store = await invoke('vault_rename', { name: vault.name, newName: vault.name, keyNames: names });
      vault = store.vaults.find(x => x.name === vault.name) || vault; toast('Key renamed');
    } catch (err) { fail(err); }
    renderKeys();
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
$('#edit-names').onclick = () => {
  $('#edit-vault-name').value = vault.name;
  $('#edit-key-names').innerHTML = vault.keys.map((k, i) => `<div class="row"><span class="mono muted" style="width:140px;font-size:12px">${k.serial ? 'serial ' + k.serial : k.pubkey.slice(0, 12) + '…'}</span><input data-edit-key="${i}" value="${k.name.replace(/"/g, '&quot;')}" style="max-width:260px;padding:8px 12px;border:1px solid var(--line);border-radius:6px;background:var(--panel);color:var(--ink)"></div>`).join('');
  $('#edit-panel').hidden = false; $('#edit-names').hidden = true;
};
$('#cancel-names').onclick = () => { $('#edit-panel').hidden = true; $('#edit-names').hidden = false; };
$('#save-names').onclick = async () => {
  const old = vault.name;
  try {
    store = await invoke('vault_rename', { name: old, newName: $('#edit-vault-name').value, keyNames: $$('[data-edit-key]').map(i => i.value) });
    vault = store.vaults.find(v => v.name === ($('#edit-vault-name').value.trim() || old)) || store.vaults[0];
    try { localStorage.setItem('a2k-vault', vault.name); } catch {}
    $('#edit-panel').hidden = true; $('#edit-names').hidden = false; paintNet(); renderKeys(); $('#title').textContent = vault.name; toast('Names saved');
  } catch (e) { fail(e); }
};
$('#copy-desc').onclick = () => navigator.clipboard.writeText(vault._recovery || vault.descriptor).then(() => toast('Recovery line copied'));
$('#forget-vault').onclick = async () => { if (!confirm(`Forget '${vault.name}' on this computer? The coins stay on the chain; the recovery line and two keys bring it back.`)) return; store = await invoke('vault_forget', { name: vault.name }); vault = null; boot(); };

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
    $('#replace-btn').onclick = () => {
      if (!confirm('Generate a NEW key on this card?\n\nThe key it holds now will be destroyed. Any vault that key belongs to loses that cosigner for good. Only do this for a card that has never held funds.')) return;
      box.innerHTML += '<div class="field" style="margin-top:12px"><div class="label">Admin PIN</div><input id="enrol-admin" type="password" autocomplete="off" style="max-width:260px"></div><button class="btn" id="enrol-btn" style="border-color:var(--no);color:var(--no);margin-top:10px">Generate a new key, destroying the old one</button>';
      wireGenerate();
    };
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
async function renderSeal(force) {
  if (!vault) return;
  const typing = $$('#seal-list input').some(i => i.value || i === document.activeElement);
  if (typing && !force) return;   // a poll must not wipe a half-typed PIN
  const name = vault.name;
  let st; try { st = await invoke('seal_status', { name }); } catch (e) { $('#seal-note').textContent = String(e); $('#seal-list').innerHTML = ''; return; }
  if (!vault || vault.name !== name) return;
  const n = st.keys.length, k = st.threshold;
  $('#seal-note').textContent = st.sealed === n
    ? `All ${n} keys carry their share. Any ${k} of them rebuild this vault on a new computer, with no sheet and no file. One key alone gives nothing away.`
    : st.sealed ? `${st.sealed} of ${n} keys sealed. Recovery from the keys alone only works with sealed keys, so finish the rest: you cannot choose which keys you will be left with.`
    : `Each key can carry a share of this vault's recovery record. Any ${k} sealed keys then rebuild the vault on a new computer with nothing else. One key alone gives nothing away. Sealing takes the key's admin PIN once; it writes to the card's certificate object and never touches the signing key.`;
  if (st.earlier) $('#seal-note').textContent += ` These keys carry shares from an earlier sealing this computer does not know: plug in ${k} of them together and it is adopted.`;
  $('#seal-list').innerHTML = st.keys.map((key, i) => {
    const card = cards.find(c => c.pubkey === key.pubkey);
    const form = (label, replace) => `<input type="password" autocomplete="off" placeholder="admin PIN" data-seal-pin="${i}" style="max-width:150px;padding:8px 10px;border:1px solid var(--line);border-radius:6px;background:var(--paper,#fff);font:inherit"><button class="btn" data-seal-go="${i}" data-replace="${replace ? 1 : ''}" style="min-height:36px;padding:6px 14px;font-size:13px">${label}</button>`;
    const right = key.state === 'sealed' ? `<div class="state ok"><i></i>sealed${key.here ? '' : ' · not plugged in'}</div>`
      : key.state === 'foreign' ? `<div class="state"><i></i>certificate object in use by something else · left alone</div>`
      : !key.here ? `<div class="state"><i></i>not sealed · plug it in</div>`
      : key.state === 'other' ? form('Replace its share', true) : form('Seal this key', false);
    return `<div class="keyrow"><svg width="22" height="22"><use href="#ring"/></svg><div class="who"><div class="name">${esc(key.name)}</div><div class="muted" style="font-size:13px">${key.state === 'other' ? 'carries a share from a different sealing of this vault' : 'share ' + (i + 1) + ' of ' + n}</div></div>${right}</div>`;
  }).join('');
  $$('[data-seal-go]').forEach(b => b.onclick = async () => {
    const i = +b.dataset.sealGo, key = st.keys[i], card = cards.find(c => c.pubkey === key.pubkey), pin = $(`[data-seal-pin="${i}"]`).value;
    if (!card) return toast('That key is no longer plugged in');
    if (!pin) return toast('Enter the admin PIN of this key');
    b.disabled = true; b.textContent = 'writing…';
    try { const r = await invoke('seal_write', { name: vault.name, reader: card.reader, adminPin: pin, replace: !!b.dataset.replace }); toast(`${key.name}: ${r}`, 5000); store = await invoke('store_get'); vault = store.vaults.find(v => v.name === vault.name) || vault; }
    catch (e) { fail(e); }
    renderSeal(true);
  });
}

// restore, from the keys: shares are remembered across card swaps, so one port is enough
let sealFound = [];
async function readSeals() {
  const sig = cards.map(c => c.serial + ':' + c.pubkey).sort().join(',');
  if (sig === readSeals.sig) return; readSeals.sig = sig;
  let r; try { r = await invoke('seal_read'); } catch (e) { $('#seal-read-list').innerHTML = `<div class="warn">${esc(e)}</div>`; readSeals.sig = null; return; }
  for (const f of r.found) if (!sealFound.some(o => o.entry.id === f.entry.id && o.entry.x === f.entry.x)) sealFound.push(f);
  const seen = [...new Set(sealFound.map(f => f.serial))];
  $('#seal-read-list').innerHTML = (seen.length ? seen.map(sn => `<div class="keyrow"><svg width="22" height="22"><use href="#ring"/></svg><div class="who"><div class="name">YubiKey ${sn}</div><div class="muted" style="font-size:13px">${sealFound.filter(f => f.serial === sn).length} share(s) read${cards.some(c => c.serial === sn) ? '' : ' · unplugged, remembered'}</div></div><div class="state ok"><i></i>read</div></div>`).join('') : '')
    + (r.cards && !r.found.length ? `<div class="muted" style="font-size:13px">${r.foreign.length ? 'This key\'s certificate object holds something that is not a seal.' : 'The key plugged in now is not sealed. A vault sealed by this app writes a share to each key from the Keys screen.'}</div>` : '')
    + (!r.cards && !seen.length ? '<div class="muted" style="font-size:13px">No YubiKey plugged in.</div>' : '');
  if (!sealFound.length) return;
  const cands = await invoke('seal_preview', { found: sealFound });
  $('#restore-card').innerHTML = cands.map((c, ci) => c.record ? `<div style="display:flex;flex-direction:column;gap:10px${ci ? ';border-top:1px solid var(--line);padding-top:14px' : ''}">
      <div class="ok">Rebuilt from ${c.need} keys: ${esc(c.record.name)} · ${c.record.threshold} of ${c.record.pubkeys.length} · ${c.record.network}${c.record.chain_code ? ' · rotating addresses' : ' · one fixed address'}</div>
      <div class="muted" style="font-size:13px">Keys: ${c.record.key_names.map(esc).join(' · ')}. First address: <span class="mono">${c.address}</span></div>
      <div class="mono muted" style="background:var(--ground);padding:12px;border-radius:6px;border:1px solid var(--line);-webkit-user-select:text;user-select:text">${esc(c.line)}</div>
      ${c.known_as ? `<div class="muted" style="font-size:13px">Already on this computer as “${esc(c.known_as)}”.</div>` : `<div class="field"><div class="label">Vault name</div><input data-seal-name="${c.id}" value="${esc(c.record.name)}"></div><button class="btn primary" data-seal-restore="${c.id}">Restore this vault</button>`}</div>`
    : `<div class="muted"${ci ? ' style="border-top:1px solid var(--line);padding-top:14px"' : ''}>A vault with ${c.have} of the ${c.need} keys it needs. Unplug this key and plug in another of the same vault; what was read is remembered.</div>`).join('') || '<div class="muted">Nothing read yet.</div>';
  $$('[data-seal-restore]').forEach(b => b.onclick = async () => {
    const id = b.dataset.sealRestore;
    try { vault = await invoke('seal_restore', { id, name: $(`[data-seal-name="${id}"]`).value, found: sealFound }); store = await invoke('store_get'); try { localStorage.setItem('a2k-vault', vault.name); } catch {} sealFound = []; readSeals.sig = null; toast('Vault restored from its keys'); paintNet(); renderTabs(); show('home'); }
    catch (e) { fail(e); }
  });
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
  invoke('store_get').then(() => {}); $('#watch-log-path').textContent = '~/Library/Application Support/Any Two Keys/watch.log';
}
$('#net-pick').addEventListener('click', async (e) => { const o = e.target.closest('.opt'); if (!o) return; store = await invoke('settings_set', { network: o.dataset.net, custom: store.settings.custom, electrum: store.settings.electrum, explorer: store.settings.explorer }); toast('New vaults will use ' + o.dataset.net); renderSettings(); $('#welcome-net').textContent = store.settings.network; });
$('#own-servers').onclick = async () => { store = await invoke('settings_set', { network: netName(), custom: !store.settings.custom, electrum: $('#c-electrum').value, explorer: $('#c-explorer').value }); renderSettings(); };
$('#save-settings').onclick = async () => { try { store = await invoke('settings_set', { network: netName(), custom: true, electrum: $('#c-electrum').value, explorer: $('#c-explorer').value }); renderSettings(); toast('Using your servers'); } catch (e) { fail(e); } };
$('#test-servers').onclick = async () => { $('#test-out').textContent = 'testing…'; try { $('#test-out').textContent = await invoke('servers_test', { electrum: $('#c-electrum').value, explorer: $('#c-explorer').value }); } catch (e) { $('#test-out').textContent = String(e); } };

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
