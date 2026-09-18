// Any Two Keys — the UI. Every real thing happens in Rust behind `invoke`.
const { invoke } = window.__TAURI__.core;
const $ = (s) => document.querySelector(s);
const $$ = (s) => Array.from(document.querySelectorAll(s));
const sats = (n) => Number(n).toLocaleString('en-AU');
let store = null, vault = null, cards = [], cardsErr = '', fees = null, feeLevel = 'medium', built = null, camStream = null, camTimer = null;
let setup = { k: 2, n: 3, keys: [] };

function toast(t, ms = 3500) { const el = $('#toast'); el.textContent = t; el.classList.add('on'); clearTimeout(el._t); el._t = setTimeout(() => el.classList.remove('on'), ms); }
function fail(e) { toast(String(e && e.message || e), 6000); console.error(e); }

// ---------- screens ----------
function show(name) {
  const first = ['setup', 'restore'].includes(name) && !vault;
  $('#welcome').hidden = true; $('#shell').hidden = false;
  $$('.screen').forEach(s => s.classList.toggle('on', s.id === 's-' + name));
  $$('.nav').forEach(n => n.classList.toggle('on', n.dataset.go === name));
  $('#title').textContent = { home: vault ? vault.name : 'Vault', send: 'Send', receive: 'Receive', keys: 'Keys', settings: 'Settings', setup: 'New vault · enrol keys', restore: 'Restore from a recovery sheet', sign: 'Sign' }[name] || '';
  if (name !== 'restore') stopCam();
  if (name === 'home') sync();
  if (name === 'receive') drawAddress();
  if (name === 'send') loadFees();
  if (name === 'keys') renderKeys();
  if (name === 'setup') renderSetup();
  if (name === 'restore') startCam();
  if (name === 'settings') renderSettings();
  if (name === 'sign') renderSign();
  void first;
}
document.addEventListener('click', (e) => { const go = e.target.closest('[data-go]'); if (go) { if (go.dataset.k) { setup = { k: +go.dataset.k, n: +go.dataset.n, keys: [] }; } show(go.dataset.go); } });

function netName() { return store.settings.network; }
function paintNet() { const p = $('#netpill'); p.classList.toggle('signet', netName() === 'signet'); p.querySelector('span').textContent = netName(); $('#welcome-net').textContent = netName(); $('#setup-net').textContent = netName(); }

async function boot() {
  store = await invoke('store_get');
  paintNet();
  const mine = store.vaults.filter(v => v.network === netName());
  vault = mine[0] || null;
  if (vault) show('home'); else { $('#shell').hidden = true; $('#welcome').hidden = false; }
  pollCards();
}

// ---------- cards ----------
async function pollCards() {
  try { cards = await invoke('cards_list'); cardsErr = ''; } catch (e) { cards = []; cardsErr = String(e); }
  if ($('#s-setup').classList.contains('on')) renderSetup();
  if ($('#s-keys').classList.contains('on')) renderKeysNow();
  if ($('#s-sign').classList.contains('on')) renderSign();
  setTimeout(pollCards, 2000);
}
function cardLabel(c) { return `YubiKey 5 · serial ${c.serial} · firmware ${c.version}`; }

// ---------- home ----------
async function sync() {
  if (!vault) return;
  $('#vault-kind').textContent = vault.keys.length === 1 ? 'Single key' : `${vault.threshold} of ${vault.keys.length} keys sign`;
  $('#vault-keys').textContent = vault.keys.map(k => k.name).join(' · ');
  $('#balance-sub').textContent = 'syncing…';
  try {
    const s = await invoke('vault_sync', { name: vault.name });
    vault._coins = s.coins;
    $('#balance').innerHTML = `${sats(s.balance)} <small>sats</small>`;
    const unconf = s.coins.filter(c => c.height === 0).reduce((a, c) => a + c.value, 0);
    $('#balance-sub').textContent = `${s.coins.length} coin${s.coins.length === 1 ? '' : 's'}` + (unconf ? ` · ${sats(unconf)} unconfirmed` : ' · all confirmed');
    $('#conn').textContent = `Connected to ${s.servers.electrum.replace(/^\w+:\/\//, '')} · block ${sats(s.height)}`;
    const tb = $('#activity'); tb.innerHTML = '';
    if (!s.history.length) tb.innerHTML = '<tr><td class="muted" colspan="2">nothing yet</td></tr>';
    for (const h of s.history.slice().reverse()) {
      const tr = document.createElement('tr');
      tr.innerHTML = `<td><a href="#" data-tx="${h.txid}" class="mono">${h.txid.slice(0, 16)}…${h.txid.slice(-6)}</a></td><td class="r ${h.height > 0 ? 'ok' : 'muted'}">${h.height > 0 ? 'confirmed · ' + sats(h.height) : 'unconfirmed'}</td>`;
      tb.appendChild(tr);
    }
    $('#send-have').textContent = `${sats(s.balance)} sats`;
  } catch (e) { $('#balance-sub').textContent = 'could not reach the server'; $('#conn').textContent = String(e); }
}
document.addEventListener('click', (e) => { const a = e.target.closest('[data-tx]'); if (a) { e.preventDefault(); const base = ($('#conn').textContent.includes('signet') ? 'https://signet.mineracks.com' : (store.settings.custom && store.settings.explorer) || (netName() === 'mainnet' ? 'https://mempool.mineracks.com' : 'https://signet.mineracks.com')); window.open(`${base}/tx/${a.dataset.tx}`); } });

// ---------- receive ----------
function drawAddress() {
  if (!vault) return;
  $('#addr-text').textContent = vault.address;
  const qr = qrcode(0, 'M'); qr.addData('bitcoin:' + vault.address); qr.make();
  $('#addr-qr').innerHTML = qr.createSvgTag({ cellSize: 4, margin: 2, scalable: true });
}
$('#copy-addr').onclick = () => { navigator.clipboard.writeText(vault.address).then(() => toast('Address copied')); };

// ---------- send ----------
async function loadFees() {
  if (!vault) return;
  if (!vault._coins) await sync();
  try {
    fees = await invoke('fees_get');
    const vb = vault.keys.length === 1 ? 141 : (11 + inVb(vault.threshold, vault.keys.length) + 31 + 43);
    for (const lvl of ['slow', 'medium', 'fast']) { const r = fees[lvl]; $(`[data-fee="${lvl}"] small`).textContent = `${r} sat/vB · ≈${sats(Math.ceil(r * vb))} sats`; }
    $('#fee-note').textContent = `Rates from the explorer just now. A ${vault.keys.length === 1 ? 'single-key' : vault.threshold + '-of-' + vault.keys.length} spend is about ${vb} vB per input set.`;
  } catch (e) { $('#fee-note').textContent = 'Could not read fee rates: ' + e; }
}
function inVb(k, n) { if (n === 1) return 68; const sl = 1 + 34 * n + 2; return 41 + Math.ceil((1 + 1 + k * 73 + (sl < 76 ? 1 : 2) + sl) / 4); }
$('#fee-pick').addEventListener('click', (e) => { const o = e.target.closest('.opt'); if (!o) return; feeLevel = o.dataset.fee; $$('#fee-pick .opt').forEach(x => x.classList.toggle('on', x === o)); });
$('#send-all').onchange = () => { $('#send-amount').disabled = $('#send-all').checked; };
$('#build-btn').onclick = async () => {
  const to = $('#send-to').value.trim(), all = $('#send-all').checked, amount = +($('#send-amount').value.replace(/[^0-9]/g, '') || 0);
  if (!to) return toast('Enter a destination address');
  if (!all && !amount) return toast('Enter an amount in sats');
  const rate = fees ? fees[feeLevel] : 1;
  try {
    built = await invoke('spend_build', { name: vault.name, to, amount, rate, sendAll: all });
    $('#review').innerHTML = `<div style="display:flex;flex-direction:column;gap:8px;font-size:14px">
      <div class="row" style="justify-content:space-between"><span class="muted">To</span><span class="mono">${to.slice(0, 10)}…${to.slice(-6)}</span></div>
      <div class="row" style="justify-content:space-between"><span class="muted">Amount</span><span>${all ? 'everything' : sats(amount) + ' sats'}</span></div>
      <div class="row" style="justify-content:space-between"><span class="muted">Fee</span><span>${sats(built.fee)} sats · ${built.vsize} vB</span></div>
      <div class="row" style="justify-content:space-between"><span class="muted">Change back to vault</span><span>${sats(built.change)} sats</span></div>
      <div class="muted" style="border-top:1px solid var(--line);padding-top:10px;font-size:13px">Next: sign with ${vault.keys.length === 1 ? 'the key' : 'any ' + vault.threshold + ' of your ' + vault.keys.length + ' keys'}. Nothing is sent until you confirm after the last signature.</div></div>`;
    $('#build-btn').textContent = 'Sign with keys'; $('#build-btn').onclick = () => show('sign');
  } catch (e) { fail(e); }
};

// ---------- sign ----------
async function renderSign() {
  if (!vault) return;
  let st; try { st = await invoke('spend_status', { name: vault.name }); } catch { $('#sign-title').textContent = 'No spend in progress'; $('#sign-text').textContent = 'Build one on the Send screen.'; $('#sign-btn').disabled = true; return; }
  const have = Math.min(...st.have), need = st.need;
  $('#sign-summary').textContent = built ? `${sats(built.fee)} sats fee · ${built.inputs} input${built.inputs === 1 ? '' : 's'}` : `${st.inputs} input${st.inputs === 1 ? '' : 's'}`;
  $('#sign-progress').textContent = `${have} of ${need} signature${need === 1 ? '' : 's'} on every input` + (st.signers.length ? ' · signed so far: ' + st.signers.map(p => (vault.keys.find(k => k.pubkey === p) || {}).name || p.slice(0, 8)).join(', ') : '');
  const steps = []; for (let i = 1; i <= need; i++) steps.push(`<b class="${i <= have ? 'done' : i === have + 1 ? 'now' : ''}">${i}</b><span class="${i <= have + 1 ? 'on' : ''}">${i <= have ? 'signed' : 'key ' + i}</span><hr>`);
  steps.push(`<b class="${st.ready ? 'now' : ''}">✓</b><span class="${st.ready ? 'on' : ''}">confirm and send</span>`);
  $('#sign-steps').innerHTML = steps.join('');
  $('#broadcast-card').hidden = !st.ready;
  if (st.ready) { $('#sign-title').textContent = 'All signatures in'; $('#sign-text').textContent = 'Nothing has been sent. Review the summary, then send it.'; $('#pin-field').hidden = true; $('#sign-warn').hidden = true; $('#sign-btn').disabled = true; $('#sign-ring').querySelector('use').classList.remove('pulse'); $('#broadcast-text').textContent = `Broadcast to ${netName()} through the Electrum server.`; return; }
  const usable = cards.filter(c => c.pubkey && vault.keys.some(k => k.pubkey === c.pubkey) && !st.signers.includes(c.pubkey));
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
$('#sign-cancel').onclick = async () => { await invoke('spend_discard'); built = null; $('#build-btn').textContent = 'Build transaction'; $('#build-btn').onclick = null; show('send'); location.reload(); };
$('#broadcast-btn').onclick = async () => {
  $('#broadcast-btn').disabled = true;
  try { const r = await invoke('spend_broadcast', { name: vault.name }); built = null; toast('Sent. ' + r.txid.slice(0, 16) + '…', 8000); $('#broadcast-text').innerHTML = `Broadcast. <a href="#" data-tx="${r.txid}">${r.txid}</a>`; setTimeout(() => show('home'), 2500); }
  catch (e) { fail(e); $('#broadcast-btn').disabled = false; }
};

// ---------- keys ----------
function renderKeys() {
  if (!vault) return;
  $('#keys-list').innerHTML = vault.keys.map(k => keyRow(k.name, k.serial ? `serial ${k.serial}` : 'serial unknown (restored)', k.pubkey, cards.some(c => c.pubkey === k.pubkey) ? 'here' : 'off')).join('');
  $('#keys-desc').textContent = vault.descriptor; $('#keys-k').textContent = vault.threshold;
  renderKeysNow();
}
function keyRow(name, sub, pub, state) { return `<div class="keyrow"><svg width="22" height="22"><use href="#ring"/></svg><div class="who"><div class="name">${name}</div><div class="muted" style="font-size:13px">${sub} · <span class="mono">${pub.slice(0, 12)}…</span></div></div><div class="state ${state}"><i></i>${{ ok: 'enrolled', off: 'not plugged in', here: 'plugged in' }[state]}</div></div>`; }
function renderKeysNow() { $('#keys-now').innerHTML = cards.length ? cards.map(c => `${cardLabel(c)} · ${c.pubkey ? 'signing key present' : 'no signing key'} · touch ${c.touch}`).join('<br>') : (cardsErr && !/no YubiKey found/i.test(cardsErr) ? 'Card reader problem: ' + cardsErr : 'No YubiKey plugged in.'); }
$('#copy-desc').onclick = () => navigator.clipboard.writeText(vault.descriptor).then(() => toast('Descriptor copied'));
$('#forget-vault').onclick = async () => { if (!confirm(`Forget '${vault.name}' on this computer? The coins stay on the chain; the recovery line and two keys bring it back.`)) return; store = await invoke('vault_forget', { name: vault.name }); vault = null; boot(); };

// ---------- setup (enrol) ----------
function renderSetup() {
  paintNet();
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
  box.innerHTML = `<div style="font-family:var(--display);font-weight:600;font-size:15px">Key ${setup.keys.length + 1}: ${cardLabel(fresh)}</div>
    <div class="muted" style="font-size:13px">${fresh.pubkey ? 'This card already holds a signing key (touch ' + fresh.touch + '). Enrolling generates a NEW one and replaces it.' : 'OpenPGP signature slot is empty.'}</div>
    <div class="grid2"><div class="field"><div class="label">Name this key</div><input id="enrol-name" value="${nameGuess}"></div><div class="field"><div class="label">Admin PIN</div><input id="enrol-admin" type="password" autocomplete="off"></div></div>
    <div class="muted" style="font-size:13px">Factory admin PIN is 12345678. Change it first with YubiKey Manager if you have not. Touch policy will be set to FIXED, which cannot be undone without wiping the key.</div>
    <button class="btn primary" id="enrol-btn">Generate key on card</button>`;
  $('#enrol-btn').onclick = async () => {
    const name = $('#enrol-name').value.trim() || nameGuess, admin = $('#enrol-admin').value;
    if (!admin) return toast('Enter the admin PIN');
    $('#enrol-btn').disabled = true;
    try { const pub = await invoke('card_generate', { reader: fresh.reader, adminPin: admin }); setup.keys.push({ name, serial: fresh.serial, pubkey: pub }); toast(`${name} enrolled`); }
    catch (e) { fail(e); }
    renderSetup();
  };
}
$('#setup-finish').onclick = async () => {
  try { vault = await invoke('vault_create', { name: $('#setup-name').value.trim() || 'vault', threshold: setup.k, keys: setup.keys }); store = await invoke('store_get'); toast('Vault created'); show('keys'); }
  catch (e) { fail(e); }
};

// single key
$('#single-btn').onclick = () => { $('#single-ack').checked = false; $('#single-go').disabled = true; $('#single-modal').classList.add('on'); };
$('#single-ack').onchange = () => { $('#single-go').disabled = !$('#single-ack').checked; };
$('#single-instead').onclick = () => { $('#single-modal').classList.remove('on'); setup = { k: 2, n: 3, keys: [] }; show('setup'); };
$('#single-go').onclick = () => { $('#single-modal').classList.remove('on'); setup = { k: 1, n: 1, keys: [] }; show('setup'); };

// ---------- restore ----------
$$('.tabs div').forEach(t => t.onclick = () => { $$('.tabs div').forEach(x => x.classList.toggle('on', x === t)); $('#tab-scan').hidden = t.dataset.tab !== 'scan'; $('#tab-paste').hidden = t.dataset.tab !== 'paste'; if (t.dataset.tab === 'scan') startCam(); else stopCam(); });
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
    $('#restore-card').innerHTML = `<div class="ok">Read: ${parsed.threshold} of ${parsed.keys.length} over fixed keys, checksum ok</div>
      <div class="mono muted" style="background:var(--ground);padding:12px;border-radius:6px;border:1px solid var(--line)">${parsed.descriptor}</div>
      <div style="display:flex;flex-direction:column;gap:8px">${parsed.keys.map((k, i) => `<div class="row" style="justify-content:space-between"><span class="mono">${k.slice(0, 16)}…</span><input data-kname="${i}" value="${names[i] || 'key ' + (i + 1)}" style="max-width:160px;padding:6px 10px;border:1px solid var(--line);border-radius:6px;background:var(--panel);color:var(--ink)"><span class="${cards.some(c => c.pubkey === k) ? 'ok' : 'muted'}" style="font-size:13px">${cards.some(c => c.pubkey === k) ? 'plugged in' : 'not seen yet'}</span></div>`).join('')}</div>
      <div class="muted" style="font-size:13px">Address on ${netName()}: <span class="mono">${parsed.address}</span>. The vault can be watched now; spending needs ${parsed.threshold} of these keys.</div>
      <div class="field"><div class="label">Vault name</div><input id="restore-name" value="vault"></div>
      <button class="btn primary" id="restore-go">Restore this vault</button>`;
    $('#restore-go').onclick = async () => {
      try { vault = await invoke('vault_restore', { name: $('#restore-name').value.trim() || 'vault', text: parsed.descriptor, names: $$('[data-kname]').map(i => i.value.trim()) }); store = await invoke('store_get'); stopCam(); toast('Vault restored'); show('home'); }
      catch (e) { fail(e); }
    };
  } catch (e) { $('#restore-card').innerHTML = `<div class="warn">${e}</div>`; }
}

// ---------- settings ----------
function renderSettings() {
  $$('#net-pick .opt').forEach(o => o.classList.toggle('on', o.dataset.net === netName()));
  const def = netName() === 'mainnet' ? ['ssl://electrum.mineracks.com:50002', 'https://mempool.mineracks.com'] : ['tcp://100.94.4.10:50001', 'https://signet.mineracks.com'];
  $('#def-electrum').textContent = def[0]; $('#def-explorer').textContent = def[1];
  $('#own-servers').classList.toggle('on', !store.settings.custom);
  $('#c-electrum').value = store.settings.electrum || ''; $('#c-explorer').value = store.settings.explorer || '';
  $('#custom-card').style.opacity = store.settings.custom ? 1 : .6;
}
$('#net-pick').addEventListener('click', async (e) => { const o = e.target.closest('.opt'); if (!o) return; store = await invoke('settings_set', { network: o.dataset.net, custom: store.settings.custom, electrum: store.settings.electrum, explorer: store.settings.explorer }); toast('Network: ' + o.dataset.net); boot(); });
$('#own-servers').onclick = async () => { store = await invoke('settings_set', { network: netName(), custom: !store.settings.custom, electrum: $('#c-electrum').value, explorer: $('#c-explorer').value }); renderSettings(); };
$('#save-settings').onclick = async () => { try { store = await invoke('settings_set', { network: netName(), custom: true, electrum: $('#c-electrum').value, explorer: $('#c-explorer').value }); renderSettings(); toast('Using your servers'); } catch (e) { fail(e); } };
$('#test-servers').onclick = async () => { $('#test-out').textContent = 'testing…'; try { $('#test-out').textContent = await invoke('servers_test', { electrum: $('#c-electrum').value, explorer: $('#c-explorer').value }); } catch (e) { $('#test-out').textContent = String(e); } };

boot();
