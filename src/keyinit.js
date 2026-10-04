// Any Two Keys — setting up a brand-new YubiKey without leaving the app.
//
// A key that has never been used is noticed wherever the person happens to be, and setup is OFFERED in a quiet strip:
// it never takes the focus, never appears over a PIN being typed, and waits until the window is the one in front.
// The walk itself is one idea per screen, a picture on the left: what the key is and what is already on it, what the
// two PINs are for, writing them down BEFORE they are set, setting them, reading them back off the page, two
// switches, the signing key, a first signature that proves the lot.
//
// Looking at a key (card_survey) sends no PIN and writes nothing. The PINs live in this window's memory only while
// the walk is open; nothing here stores, prints or copies them.

const kiSeen = new Map();     // serial → what the key said about itself (null: it would not say)
const kiLater = new Set();    // "Not now" — until the key is unplugged
const ki = { open: false };
const KI_STEPS = ['meet', 'pins', 'record', 'set', 'readback', 'harden', 'key', 'done'];
const KI_NAMES = { meet: 'This key', pins: 'Two PINs', record: 'Write down', set: 'Set', readback: 'Read back', harden: 'Switches', key: 'Signing key', done: 'Ready' };
const kiSleep = (ms) => new Promise(r => setTimeout(r, ms));
const kiSerial = (n) => String(n).replace(/\B(?=(\d{3})+(?!\d))/g, ' ');
const kiBegun = (serial) => !!(store && (store.key_begun || []).includes(serial));
const kiTake = (st) => { store = st; if (vault) vault = st.vaults.find(v => v.name === vault.name) || vault; };

// ---------- noticing, and offering ----------
async function keyOfferTick() {
  if (!store) return;
  const present = new Set(cards.map(c => c.serial));
  for (const s of [...kiSeen.keys()]) if (!present.has(s) && !(ki.open && ki.serial === s)) { kiSeen.delete(s); kiLater.delete(s); }
  // a key reached by NFC is asked for at each step, never "plugged in"
  if (ki.open) { const here = ki.nfc || present.has(ki.serial), el = $('#ki-here'); el.textContent = here ? '' : `Plug key ${kiSerial(ki.serial)} back in to go on`; el.classList.toggle('warn', !here); return; }
  // only a key with no signing key this app can use is worth a second look, and each is looked at once per plugging-in
  if (cards.some(c => !c.pubkey && !kiSeen.has(c.serial))) {
    let all = []; try { all = await invoke('card_survey'); } catch {}
    for (const s of all) kiSeen.set(s.serial, s);
    for (const c of cards) if (!kiSeen.has(c.serial)) kiSeen.set(c.serial, null);
    if ($('#s-setup').classList.contains('on')) renderSetup();
  }
  paintOffer();
}
// the key on offer: untouched, not waved away, and not one the person asked never to hear about
const kiCandidate = () => [...kiSeen.values()].find(s => s && s.state === 'fresh' && !kiLater.has(s.serial) && !(store.key_skip || []).includes(s.serial) && cards.some(c => c.serial === s.serial && !c.pubkey));
function paintOffer() {
  const el = $('#keyoffer'), s = kiCandidate();
  // never over a lock, a curtain or the walk itself; never while a PIN is being typed anywhere; never from behind
  const quiet = !s || ki.open || !$('#lock').hidden || !$('#curtain').hidden || !document.hasFocus() || $('#s-setup').classList.contains('on')
    || $$('input[type=password]').some(i => i.offsetParent !== null && (i.value || i === document.activeElement));   // only a field that is on screen: one left filled on a screen long since left must not silence this for good
  document.body.classList.toggle('offer-on', !quiet);
  if (quiet) { el.hidden = true; paintOffer.sig = null; return; }
  const begun = kiBegun(s.serial), sig = s.serial + '|' + begun;
  if (paintOffer.sig === sig) return;
  paintOffer.sig = sig;
  el.innerHTML = `<span class="ko-art">${keyArt(s.form || 3, s.nfc, 58)}</span>
    <div class="ko-body"><b>${begun ? 'This key’s setup is not finished' : `A new ${esc(modelName(s.form, s.nfc))} is plugged in`}</b>
      <span class="muted">serial ${kiSerial(s.serial)}${s.firmware ? ' · firmware ' + esc(s.firmware) : ''} · ${begun ? 'its PINs are set, its signing key is not made yet' : 'never set up, still on its factory PINs'}</span>
      <div class="row"><button type="button" class="btn small primary" id="ko-go">${begun ? 'Finish setting it up' : 'Set it up'}</button><button type="button" class="btn small" id="ko-later">Not now</button><button type="button" class="linkish" id="ko-never">Don’t ask for this key again</button></div></div>`;
  el.hidden = false;
  $('#ko-go').onclick = () => kiOpen(s);
  $('#ko-later').onclick = () => { kiLater.add(s.serial); paintOffer(); };
  $('#ko-never').onclick = async () => { try { kiTake(await invoke('key_setup_skip', { serial: s.serial })); toast('It will not be offered again. New vault can still set it up.', 6000); } catch (e) { fail(e); } paintOffer(); };
}
window.addEventListener('focus', () => { if (store) paintOffer(); });
window.addEventListener('blur', () => { if (store) paintOffer(); });

// ---------- the walk ----------
function kiOpen(survey) {
  const n = Object.keys((store && store.key_ready) || {}).length;
  Object.assign(ki, { open: true, serial: survey.serial, survey, nfc: survey.reader === 'NFC', resume: kiBegun(survey.serial), name: ['laptop', 'keyring', 'off-site'][n] || `key ${n + 1}`,
    user: '', admin: '', userDone: false, written: false, sig: !survey.pgp.sig_pin_each, otp: survey.otp_usb === true && !survey.config_locked,
    notes: [], pubkey: null, phase: 'idle', err: '', stop: null, busy: false });
  ki.step = ki.resume ? 'readback' : 'meet';
  $('#keyinit').hidden = false; paintOffer(); kiPaint();
}
function kiClose() {
  // the PINs leave memory with the walk; whatever is on the key stays as it is
  ki.user = ''; ki.admin = ''; ki.open = false; $$('#keyinit input').forEach(i => { i.value = ''; });
  // the card list is only read again at the next poll: say now what the walk just made true
  const c = cards.find(c => c.serial === ki.serial); if (c && ki.step === 'done' && ki.pubkey) { c.pubkey = ki.pubkey; c.touch = 'FIXED'; }
  $('#ki-body').innerHTML = ''; $('#keyinit').hidden = true; kiSeen.delete(ki.serial);
  if ($('#s-setup').classList.contains('on')) { renderSetup.nowSig = null; renderSetup(); }
}
$('#ki-close').onclick = () => {
  const mid = KI_STEPS.indexOf(ki.step) > KI_STEPS.indexOf('set') && ki.step !== 'done' && !ki.stop;
  if (mid) toast(`Both PINs are set on key ${kiSerial(ki.serial)}. Plug it in again to finish: it will ask for them.`, 9000);
  kiClose();
};
const kiGo = (step) => { ki.step = step; ki.err = ''; ki.busy = false; kiPaint(); };

// pictures: our own drawings, in the app's own line and colours
const kiPic = (inner) => `<svg viewBox="0 0 250 132" role="img">${inner}</svg>`;
const kiLock = (x, y, w, label, pips) => `<path class="g-line" d="M${x + w * .25} ${y}v-${w * .22}a${w * .25} ${w * .25} 0 0 1 ${w * .5} 0v${w * .22}"/><rect class="g-ink" x="${x}" y="${y}" width="${w}" height="${w * .78}" rx="6"/>`
  + [0, 1, 2].map(i => `<circle cx="${x + w / 2 + (i - 1) * 11}" cy="${y + w * .39}" r="3.4" class="${i < pips ? 'g-strip' : 'g-hole'}"/>`).join('') + `<text class="ki-t" x="${x + w / 2}" y="${y + w * .78 + 18}" text-anchor="middle">${label}</text>`;
const kiPage = (x, lines) => `<rect class="g-lap" x="${x}" y="14" width="104" height="104" rx="6"/>` + [0, 1, 2, 3, 4].map(i => `<circle class="g-hole" cx="${x + 9}" cy="${28 + i * 19}" r="2.6" stroke="var(--line)"/>`).join('')
  + lines.map((l, i) => `<text class="ki-t s" x="${x + 20}" y="${36 + i * 21}">${l}</text><path class="ki-rule" d="M${x + 20} ${41 + i * 21}h74"/>`).join('');
const KI_ART = {
  meet: (s) => `<div class="ki-keyart">${keyArt(s.form || 3, s.nfc, 150)}</div>`,
  pins: () => kiPic(kiLock(42, 44, 52, 'KEY PIN', 3) + kiLock(144, 30, 70, 'ADMIN PIN', 3)),
  record: () => kiPic(kiPage(40, ['name', 'serial', 'key PIN', 'admin PIN']) + `<g transform="translate(176 92) rotate(-38)"><rect class="g-finger" x="-5" y="-62" width="10" height="56" rx="2"/><path class="g-ink" d="M-5 -6h10l-5 12z"/></g>`),
  set: (s) => guideArt('in', s.form || 3),
  readback: () => kiPic(kiPage(22, ['name', 'serial', '••••••', '••••••••']) + `<path class="g-arrow" d="M138 66h30m-8 -8l8 8l-8 8"/>` + `<rect class="g-ink" x="182" y="40" width="40" height="56" rx="8"/><circle class="g-gold" cx="202" cy="62" r="9"/><circle cx="202" cy="84" r="4" class="g-hole"/>`),
  harden: () => kiPic([0, 1].map(i => `<rect x="62" y="${30 + i * 44}" width="62" height="30" rx="15" fill="var(--signal)"/><circle cx="109" cy="${45 + i * 44}" r="11" fill="var(--panel)"/><path class="ki-rule" d="M138 ${40 + i * 44}h54M138 ${51 + i * 44}h34"/>`).join('')),
  key: (s) => guideArt(ki.phase === 'touch' ? 'touch' : 'in', s.form || 3),
  done: (s) => `<div class="ki-keyart done">${keyArt(s.form || 3, s.nfc, 150)}<svg class="ki-tick" viewBox="0 0 40 40" width="44" height="44" aria-hidden="true"><circle cx="20" cy="20" r="19" fill="var(--signal)"/><path d="M11 21l6 6l12 -14" fill="none" stroke="#fff" stroke-width="3.6" stroke-linecap="round" stroke-linejoin="round"/></svg></div>`,
};
const kiPinField = (id, label, hint) => `<div class="field"><label class="cap" for="${id}">${label}</label><input id="${id}" type="password" autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false"><div class="ki-say" id="${id}-say">${hint || ''}</div></div>`;
const kiErr = () => ki.err ? `<div class="warnbox ki-err" role="alert">${esc(ki.err)}</div>` : '';

// what else is on the key, said as plainly as each part allows without a PIN
function kiNeighbours(s) {
  const rows = [['Signing (OpenPGP)', 'empty, on the factory PINs', 'set up here']];
  const p = s.piv; rows.push(['Smart-card login (PIV)', !p ? 'switched off, or would not say' : (p.keys || p.certs) ? `${p.keys == null ? '' : p.keys + ' key' + (p.keys === 1 ? '' : 's') + ', '}${p.certs} certificate${p.certs === 1 ? '' : 's'}` : 'nothing stored', 'left alone']);
  const o = s.oath; rows.push(['Authenticator codes', !o ? 'switched off, or would not say' : o.password ? 'behind a password; not looked into' : o.accounts == null ? 'would not say' : o.accounts ? `${o.accounts} account${o.accounts === 1 ? '' : 's'}` : 'none', 'left alone']);
  const t = s.otp; rows.push(['One-time-code slots', !t ? 'switched off' : [t.slot1 ? 'slot 1 in use' + (t.slot2 ? '' : ' (every key leaves the factory that way)') : '', t.slot2 ? 'slot 2 in use' : ''].filter(Boolean).join(', ') || 'both empty', 'left alone']);
  rows.push(['Passkeys and website logins', 'cannot be listed by anyone without the key’s own passkey PIN; not read', 'left alone']);
  return `<div class="ki-rows">${rows.map(r => `<div class="ki-row"><b>${r[0]}</b><span class="muted">${esc(r[1])}</span><span class="tag${r[2] === 'set up here' ? ' out' : ''}">${r[2]}</span></div>`).join('')}</div>`;
}

function kiPaint() {
  const s = ki.survey, at = KI_STEPS.indexOf(ki.step);
  $('#ki-prog').innerHTML = KI_STEPS.map((k, i) => `<li class="${i < at ? 'done' : i === at ? 'now' : ''}"${i === at ? ' aria-current="step"' : ''}><span>${KI_NAMES[k]}</span></li>`).join('');
  $('#ki-art').innerHTML = KI_ART[ki.step](s);
  const body = $('#ki-body'), title = (t) => `<div class="cer-act-title" id="ki-title">${t}</div>`;
  const foot = (...b) => `<div class="row ki-foot">${b.join('')}</div>`;
  const back = (to) => `<button type="button" class="btn" data-ki-go="${to}">Back</button>`;

  if (ki.stop) {
    body.innerHTML = title(ki.stop.title) + ki.stop.lines.map(l => `<div>${l}</div>`).join('') + foot('<button type="button" class="btn primary" id="ki-end">Close</button>');
    $('#ki-end').onclick = kiClose; return;
  }

  if (ki.step === 'meet') {
    const adv = keyAdvisories({ firmware: s.firmware }), newer = s.firmware && !verLt(s.firmware, '5.7.0');
    body.innerHTML = title(`A new ${esc(modelName(s.form, s.nfc))}`)
      + `<div class="muted">Serial ${kiSerial(s.serial)}${s.firmware ? ' · firmware ' + esc(s.firmware) : ''}. It has never been set up: no key on it, and both PINs still the ones every YubiKey ships with.</div>`
      + (adv.length ? adv.map(a => `<div class="warnbox"><b>${esc(a.id)}: ${esc(a.title)}.</b> ${esc(a.says)} ${esc(a.here)}</div>`).join('')
        : `<div class="muted">${s.firmware ? `No security advisory is known for firmware ${esc(s.firmware)} as of ${ADVISORIES_AS_OF}. Firmware cannot be changed on a YubiKey, so this is checked here, on this computer, each time the list is updated.` : 'The key would not say its firmware, so it could not be checked against known advisories.'}</div>`)
      + (newer ? `<div class="muted">Signing with this app was proven on firmware 5.4. This key is newer; the last step of setup has it sign and checks the answer, so nothing is taken on trust.</div>` : '')
      + `<div class="cap">Already on this key</div>` + kiNeighbours(s)
      + `<div class="muted">Setting it up changes its two signing PINs and makes one signing key in an empty slot. Nothing is erased, here or anywhere else on the key.</div>`
      + foot('<button type="button" class="btn primary" data-ki-go="pins">Set this key up</button>', '<button type="button" class="btn" id="ki-later">Not now</button>');
    $('#ki-later').onclick = () => { kiLater.add(ki.serial); kiClose(); };
  }

  if (ki.step === 'pins') {
    body.innerHTML = title('Two PINs, two jobs')
      + `<div class="ki-two"><div><b>Key PIN</b><span class="tag">6 or more characters</span><div class="muted">Asked every time this key signs. Three wrong tries in a row and the key refuses to sign until the admin PIN unlocks it.</div></div>
         <div><b>Admin PIN</b><span class="tag">8 or more characters</span><div class="muted">Asked rarely: now, when a vault is sealed onto the key, and to unlock a key PIN that ran out of tries. Three wrong tries in a row and the signing part of the key locks for good. Only wiping it frees it, and the wipe destroys the key it holds.</div></div></div>`
      + `<div class="muted">Nobody can reset either one for you: not Yubico, not this app. That is the point of them, and it is why a vault needs any two keys rather than all of them. One key lost to a forgotten PIN costs nothing while two others still sign.</div>`
      + `<div class="muted">Digits are enough, letters and symbols are allowed. Make them two different PINs you use nowhere else.</div>`
      + foot(back('meet'), '<button type="button" class="btn primary" data-ki-go="record">Next: write them down</button>');
  }

  if (ki.step === 'record') {
    const today = new Date().toLocaleDateString(undefined, { day: 'numeric', month: 'long', year: 'numeric' });
    body.innerHTML = title('Write them down before you set them')
      + `<div class="muted">Decide both PINs now and put them on paper first, so there is never a moment when the key has a PIN that exists only in your head. An offline notebook made for this (a Stonebook, for one) or a card in a safe.</div>`
      + `<div class="field ki-narrow"><label class="cap" for="ki-name">Name this key</label><input id="ki-name" value="${esc(ki.name)}" maxlength="40" autocomplete="off"></div>`
      + `<div class="ki-record" aria-label="What to write"><div><span>Key</span><b id="ki-rec-name">${esc(ki.name)}</b></div><div><span>Serial</span><b class="mono">${kiSerial(s.serial)}</b><small>also printed on the key</small></div><div><span>Key PIN</span><i></i></div><div><span>Admin PIN</span><i></i></div><div><span>Set up</span><b>${today}</b></div></div>`
      + `<div class="muted">Keep the page somewhere other than the key. A PIN without its key opens nothing; the two together are a signature. Not a photo, not a notes app, not a message to yourself: those are copied to places you do not control. This app never stores, prints or copies either PIN.</div>`
      + `<button type="button" class="vchip" id="ki-written" aria-pressed="${ki.written}">Both PINs are written down, away from the key</button>`
      + foot(back('pins'), `<button type="button" class="btn primary" id="ki-next"${ki.written ? '' : ' disabled'}>Next: set them on the key</button>`);
    $('#ki-name').oninput = () => { ki.name = $('#ki-name').value; $('#ki-rec-name').textContent = ki.name; };
    $('#ki-written').onclick = () => { ki.written = !ki.written; $('#ki-written').setAttribute('aria-pressed', ki.written); $('#ki-next').disabled = !ki.written; };
    $('#ki-next').onclick = () => { ki.name = ki.name.trim() || `key ${kiSerial(s.serial)}`; kiGo('set'); };
  }

  if (ki.step === 'set') {
    body.innerHTML = title('Set them on the key')
      + `<div class="muted">Type each PIN twice, from the page. They go to the key and nowhere else.</div>`
      + `<div class="ki-form">${kiPinField('ki-u1', 'Key PIN')}${kiPinField('ki-u2', 'Key PIN, again')}${kiPinField('ki-a1', 'Admin PIN')}${kiPinField('ki-a2', 'Admin PIN, again')}</div>`
      + `<button type="button" class="linkish" id="ki-show">Show what I am typing</button>`
      + (s.pin_complexity ? `<div class="muted">This key also enforces PIN rules of its own. If it refuses one, choose a less regular one.</div>` : '')
      + kiErr() + foot(back('record'), '<button type="button" class="btn primary" id="ki-set" disabled>Set both PINs</button>');
    const f = { u1: $('#ki-u1'), u2: $('#ki-u2'), a1: $('#ki-a1'), a2: $('#ki-a2') };
    if (ki.userDone) { f.u1.value = f.u2.value = ki.user; f.u1.readOnly = f.u2.readOnly = true; $('#ki-u1-say').textContent = 'Already set on the key.'; }
    let verdict = [null, null], asked = 0;
    const say = (id, text, bad) => { const e = $(`#${id}-say`); e.textContent = text || ''; e.classList.toggle('bad', !!bad); };
    const paint = () => {
      const u = f.u1.value, a = f.a1.value;
      if (!ki.userDone) say('ki-u1', verdict[0] || (u ? 'Good.' : ''), verdict[0]);
      say('ki-a1', verdict[1] || (a ? 'Good.' : ''), verdict[1]);
      say('ki-u2', f.u2.value && f.u2.value !== u ? 'Not the same as above.' : '', true); say('ki-a2', f.a2.value && f.a2.value !== a ? 'Not the same as above.' : '', true);
      $('#ki-set').disabled = ki.busy || !u || !a || verdict[0] || verdict[1] || f.u2.value !== u || f.a2.value !== a;
    };
    const judge = async () => { const mine = ++asked; let v = [null, null]; try { v = await invoke('card_pin_judge', { user: f.u1.value, admin: f.a1.value, serial: s.serial }); } catch {} if (mine !== asked || ki.step !== 'set') return; verdict = v; paint(); };
    const order = ['ki-u1', 'ki-u2', 'ki-a1', 'ki-a2'];
    for (const i of Object.values(f)) {
      i.oninput = () => { $('#ki-set').disabled = true; judge(); };
      // Enter moves on to the next field and never submits: a brushed key types a one-time code and presses Enter
      i.onkeydown = (e) => { if (e.key !== 'Enter') return; e.preventDefault(); const n = $('#' + order[order.indexOf(i.id) + 1]); if (n) n.focus(); };
    }
    $('#ki-show').onclick = () => { const on = f.u1.type === 'password'; for (const i of Object.values(f)) i.type = on ? 'text' : 'password'; $('#ki-show').textContent = on ? 'Hide what I am typing' : 'Show what I am typing'; };
    $('#ki-set').onclick = async () => {
      const u = f.u1.value, a = f.a1.value; ki.busy = true; ki.err = ''; $('#ki-set').disabled = true; $('#ki-set').textContent = 'Setting the PINs…';
      let r; try { r = await invoke('card_init_pins', { serial: s.serial, userPin: u, adminPin: a, userDone: ki.userDone }); }
      catch (e) { ki.busy = false; ki.err = String(e); ki.user = ki.userDone ? ki.user : ''; kiPaintKeep(f); return; }
      ki.busy = false; ki.userDone = !!r.user_done; if (ki.userDone) ki.user = u;
      if (r.admin_done) { ki.user = u; ki.admin = a; if (store) store.key_begun = [...new Set([...(store.key_begun || []), s.serial])]; if (r.error) toast(r.error, 8000); kiGo('readback'); return; }
      if (r.not_factory === 'user') { ki.stop = { title: 'This key already has a PIN of its own', lines: [
        `The factory key PIN was refused, so somebody has set one before. Nothing on the key was changed.`,
        `<span class="muted">That used one of its three tries: ${r.tries} remain, and all three come back the next time its real PIN is entered.</span>`,
        `<span class="muted">If the PIN is yours, the key needs no setup here: choose New vault and it will ask for the PINs it already has.</span>`] }; kiPaint(); return; }
      if (r.not_factory === 'admin') { ki.stop = { title: 'The admin PIN was not the factory one', lines: [
        `The key PIN is now the one you typed. The admin PIN was refused, so it has been changed before and only its real one will do.`,
        `<span class="muted">That used one of its three tries: ${r.tries} remain. Do not guess at it: at zero the signing part of this key locks for good.</span>`,
        `<span class="muted">Record the key PIN you just set. With the real admin PIN, New vault can finish this key.</span>`] }; kiPaint(); return; }
      ki.err = (r.error || 'The key did not answer.') + (ki.userDone ? ' The key PIN is already set to the one you typed; pressing the button again carries on from there.' : ' Nothing was changed.');
      kiPaintKeep(f);
    };
    paint(); if (f.u1.value || f.a1.value) judge();
    (ki.userDone ? f.a1 : f.u1).focus();
  }

  if (ki.step === 'readback') {
    body.innerHTML = title(ki.resume ? 'Enter the PINs you set on this key' : 'Now read them back from the page')
      + `<div class="muted">${ki.resume ? 'This key’s PINs were set here earlier and its setup was not finished. Type both from your record to carry on.' : 'Look at what you wrote, not at what you remember. This proves the page opens the key, while a mistake on it can still be put right.'}</div>`
      + `<div class="ki-form one">${kiPinField('ki-ru', 'Key PIN, from the page')}${kiPinField('ki-ra', 'Admin PIN, from the page')}</div>`
      + (ki.resume ? '' : `<button type="button" class="linkish" id="ki-reveal">The page is wrong: show what was set, to correct it</button><div class="ki-reveal mono" id="ki-revealed" hidden></div>`)
      + kiErr() + foot('<button type="button" class="btn primary" id="ki-check">Check them against the key</button>');
    const ru = $('#ki-ru'), ra = $('#ki-ra');
    ru.onkeydown = (e) => { if (e.key === 'Enter') { e.preventDefault(); ra.focus(); } }; ra.onkeydown = (e) => { if (e.key === 'Enter') e.preventDefault(); };
    if (!ki.resume) $('#ki-reveal').onclick = () => { const r = $('#ki-revealed'); r.hidden = false; r.textContent = `key PIN  ${ki.user}\nadmin PIN  ${ki.admin}`; clearTimeout(kiPaint.hide); kiPaint.hide = setTimeout(() => { r.hidden = true; r.textContent = ''; }, 15000); };
    $('#ki-check').onclick = async () => {
      const u = ru.value, a = ra.value, miss = (t) => { ki.err = t; ru.value = ra.value = ''; kiPaint(); };
      if (OTP.test(u) || OTP.test(a)) return miss('That was the key typing a one-time code, not a PIN. Nothing was sent to the key.');
      if (!u || !a) return miss('Both PINs, from the page.');
      // a slip of the finger is caught here, in the window, where it costs no try
      if (!ki.resume && (u !== ki.user || a !== ki.admin)) return miss(`${u !== ki.user && a !== ki.admin ? 'Neither PIN is' : u !== ki.user ? 'The key PIN is not' : 'The admin PIN is not'} what was set on the key. Nothing was sent to the key, so no try was used. Check the page; if it is the page that is wrong, correct it now.`);
      $('#ki-check').disabled = true;
      try { await invoke('card_init_check', { serial: s.serial, userPin: u, adminPin: a }); ki.user = u; ki.admin = a; kiGo('harden'); }
      catch (e) { miss(String(e)); }
    };
    ru.focus();
  }

  if (ki.step === 'harden') {
    const otpSay = s.config_locked ? 'This key’s settings are locked with a code, so this is left as it is.' : s.otp_usb === false ? 'Already off on this key.' : s.otp_usb == null ? 'The key would not say, so this is left as it is.'
      : 'A brushed YubiKey types a 44-letter code and presses Enter, into whatever has the cursor: a chat, a document, a PIN field. Turning that off erases nothing and can be turned back on.';
    body.innerHTML = title('Two switches worth setting')
      + `<div class="ki-two"><div><button type="button" class="vchip" id="ki-sig" aria-pressed="${ki.sig || s.pgp.sig_pin_each}"${s.pgp.sig_pin_each ? ' disabled' : ''}>Ask the key PIN for every signature</button><div class="muted">${s.pgp.sig_pin_each ? 'Already on.' : 'Out of the box a key PIN entered once covers every signature until the key is unplugged, for any program on the computer. This makes each signature ask.'}</div></div>
         <div><button type="button" class="vchip" id="ki-otp" aria-pressed="${ki.otp}"${s.otp_usb === true && !s.config_locked ? '' : ' disabled'}>Stop the key typing one-time codes</button><div class="muted">${otpSay}</div></div></div>`
      + `<div class="muted">Both can be changed later. The key restarts for the second one: its light goes out for a moment.</div>`
      + kiErr() + foot('<button type="button" class="btn primary" id="ki-apply">Apply and go on</button>');
    $('#ki-sig').onclick = () => { ki.sig = !ki.sig; $('#ki-sig').setAttribute('aria-pressed', ki.sig); };
    $('#ki-otp').onclick = () => { ki.otp = !ki.otp; $('#ki-otp').setAttribute('aria-pressed', ki.otp); };
    $('#ki-apply').onclick = async () => {
      const sig = ki.sig && !s.pgp.sig_pin_each, otp = ki.otp && s.otp_usb === true && !s.config_locked;
      if (!sig && !otp) return kiGo('key');
      const b = $('#ki-apply'); b.disabled = true; b.textContent = otp ? 'Applying; the key restarts…' : 'Applying…';
      try {
        const r = await invoke('card_init_harden', { serial: s.serial, adminPin: ki.admin, sigPinEach: sig, otpOff: otp }); ki.notes = r.notes || [];
        if (r.sig_pin_each) s.pgp.sig_pin_each = true;
        if (otp && r.otp_off) { s.otp_usb = false; for (let i = 0; i < 20 && !ki.nfc; i++) { await kiSleep(750); let l = []; try { l = await invoke('cards_list'); } catch {} if (l.some(c => c.serial === s.serial)) break; } }
        kiGo('key');
      } catch (e) { ki.err = String(e); kiPaint(); }
    };
  }

  if (ki.step === 'key') {
    const notes = ki.notes.map(n => `<div class="muted">${esc(n)}</div>`).join('');
    if (ki.phase === 'idle') body.innerHTML = title('Make its signing key')
      + `<div class="muted">The key makes its own secret, inside itself. It cannot be copied out, by you or by anyone, so there is no backup of it and nothing to write down. The vault is the backup: any two of its keys sign.</div>`
      + `<div class="muted">From then on every signature needs a touch of the key. That is set for good here; it cannot be turned off without wiping the key.</div>`
      + notes + kiErr() + foot('<button type="button" class="btn primary" id="ki-make">Make the signing key</button>');
    else if (ki.phase === 'making') body.innerHTML = title('Making the key…') + `<div class="muted">Inside the YubiKey. Leave it plugged in.</div>`;
    else if (ki.phase === 'touch') body.innerHTML = title('Touch the key') + `<div>Its light is blinking. Touch the gold contact now.</div><div class="muted">This is its first signature. The answer is checked against the key it just made, which proves the PIN, the touch and the key together.</div>`;
    else body.innerHTML = title('The key was not touched in time') + `<div class="muted">The signing key is made. Only its first signature is left.</div>` + kiErr() + foot('<button type="button" class="btn primary" id="ki-prove">Try the touch again</button>');
    const prove = async () => {
      ki.phase = 'touch'; ki.err = ''; kiPaint();   // the picture and the words are on screen BEFORE the key starts waiting
      try { kiTake(await invoke('card_init_prove', { serial: s.serial, userPin: ki.user, pubkey: ki.pubkey, name: ki.name })); ki.phase = 'idle'; kiGo('done'); }
      catch (e) { ki.phase = 'retry'; ki.err = String(e); kiPaint(); }
    };
    if ($('#ki-make')) $('#ki-make').onclick = async () => {
      ki.phase = 'making'; ki.err = ''; kiPaint();
      try { ki.pubkey = await invoke('card_init_key', { serial: s.serial, adminPin: ki.admin }); } catch (e) { ki.phase = 'idle'; ki.err = String(e); kiPaint(); return; }
      prove();
    };
    if ($('#ki-prove')) $('#ki-prove').onclick = prove;
  }

  if (ki.step === 'done') {
    ki.user = ''; ki.admin = '';   // nothing left to ask the key: the PINs leave memory here
    const did = ['Both PINs are yours, and the page was checked against the key', s.pgp.sig_pin_each ? 'The key PIN is asked for every signature' : '', s.otp_usb === false ? 'It no longer types one-time codes' : '', 'Its signing key was made inside it, and needs a touch every time', 'Its first signature checked out'].filter(Boolean);
    body.innerHTML = title(`${esc(ki.name)} is ready`)
      + `<div class="ki-did">${did.map(d => `<div><span class="ok">✓</span>${d}</div>`).join('')}</div>`
      + `<div class="muted">It can join a vault with one click now, here or later: New vault will know it by name and ask for nothing more. Put the page back where it lives, away from this key.</div>`
      + foot('<button type="button" class="btn primary" id="ki-vault">Make a vault with it</button>', '<button type="button" class="btn" id="ki-end">Done</button>')
      + `<div class="muted">Setting up another? Unplug this one and plug the next in.</div>`;
    $('#ki-end').onclick = kiClose;
    $('#ki-vault').onclick = () => { kiClose(); if (!$('#s-setup').classList.contains('on')) show('setup'); };
  }
  $$('#ki-body [data-ki-go]').forEach(b => b.onclick = () => kiGo(b.dataset.kiGo));
}
// repaint the Set step after a failure without losing what was typed
function kiPaintKeep(f) { const kept = Object.values(f).map(i => i.value); kiPaint(); ['ki-u1', 'ki-u2', 'ki-a1', 'ki-a2'].forEach((id, i) => { $('#' + id).value = kept[i]; }); $('#ki-u1').dispatchEvent(new Event('input')); }
