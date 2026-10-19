// Any Two Keys — setting up a brand-new key (a YubiKey 5, a Nitrokey 3, a Token2 key or a Token2 smart card) without leaving the app.
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
// 'which' is only walked for a Token2 reached by NFC (form 11): the app cannot tell the key from the card there, and
// they sign differently (the key wants its button pressed, the card has none), so the person says which before the key is made
const KI_STEPS_ALL = ['meet', 'pins', 'record', 'set', 'readback', 'harden', 'which', 'key', 'done'];
const KI_NAMES = { meet: 'This key', pins: 'Two PINs', record: 'Write down', set: 'Set', readback: 'Read back', harden: 'Switches', which: 'Key or card', key: 'Signing key', done: 'Ready' };
const kiAsk = () => !!(ki.survey && (ki.survey.form & 0x0F) === FORM_TOKEN2);
const kiSteps = () => KI_STEPS_ALL.filter(k => k !== 'which' || kiAsk());
const kiAfterHarden = () => kiAsk() ? 'which' : 'key';
// the shape the walk draws and speaks for: a Token2 of unknown shape becomes the card or the key once the person has said
const kiForm = (s) => { const f = s.form || 3; return (f & 0x0F) === FORM_TOKEN2 && ki.kind ? (ki.kind === 'card' ? FORM_TOKEN2_CARD : FORM_TOKEN2_KEY) : f; };
const kiIsCard = (s) => isCard(kiForm(s));
const kiPress = (s) => isNitro(s.form) || isToken2(s.form);   // a Nitrokey 3 and a Token2 key are pressed, a YubiKey is touched
const kiToken2 = (s) => s.maker === 'token2' || s.device === 'token2key' || s.device === 'token2card' || isToken2(s.form);
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
  if (ki.open) { const here = ki.nfc || present.has(ki.serial), el = $('#ki-here'); el.textContent = here ? '' : kiIsCard(ki.survey) ? `Put card ${kiSerial(ki.serial)} back in the reader to go on` : `Plug key ${kiSerial(ki.serial)} back in to go on`; el.classList.toggle('warn', !here); return; }
  // only a key with no signing key this app can use is worth a second look, and each is looked at once per plugging-in
  if (cards.some(c => !c.pubkey && !kiSeen.has(c.serial))) {
    let all = [], failed = false; try { all = await invoke('card_survey'); } catch { failed = true; }
    for (const s of all) kiSeen.set(s.serial, s);
    // a key the survey could not read is looked at again in a while, not written off until it is unplugged
    for (const c of cards) if (!kiSeen.has(c.serial)) { kiSeen.set(c.serial, null); if (failed) setTimeout(() => { if (kiSeen.get(c.serial) === null) kiSeen.delete(c.serial); }, 10000); }
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
    <div class="ko-body"><b>${begun ? 'This key’s setup is not finished' : `A new ${esc(modelName(s.form, s.nfc))} ${s.reader === 'NFC' ? 'was held to the phone' : isCard(s.form) ? 'is in the reader' : 'is plugged in'}`}</b>
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
async function kiOpen(survey) {
  // "setup begun" is this computer's memory, not the key's: a key reset since (by its maker's tool, or gpg) is
  // factory-fresh again, and the walk must start over rather than ask for PINs the key no longer has. The key is
  // asked: its factory key PIN either opens it (fresh) or does not (begun; the try is repaired by the right PIN).
  if (kiBegun(survey.serial)) {
    try { const r = await invoke('card_init_resume_check', { serial: survey.serial }); if (r === 'fresh') { kiTake(await invoke('key_setup_restart', { serial: survey.serial })); } } catch {}
  }
  const n = Object.keys((store && store.key_ready) || {}).length;
  // what the key is told to want for a signature: a touch for every maker but a Token2 card, which has no button; a
  // Token2 reached by NFC is asked (the 'which' step) before its signing key is made
  const f = survey.form & 0x0F, kind = f === FORM_TOKEN2_CARD ? 'card' : f === FORM_TOKEN2_KEY ? 'key' : null;
  Object.assign(ki, { open: true, serial: survey.serial, survey, nfc: survey.reader === 'NFC', resume: kiBegun(survey.serial), name: ['laptop', 'keyring', 'off-site'][n] || `key ${n + 1}`, kind, touch: kind !== 'card',
    user: '', admin: '', userDone: false, written: false, sig: !survey.pgp.sig_pin_each, otp: survey.otp_usb === true && !survey.config_locked,
    notes: [], pubkey: null, phase: 'idle', err: '', stop: null, busy: false, offkey: false, src: null, preview: null, srcWritten: false });
  ki.step = ki.resume ? 'readback' : 'meet';
  $('#keyinit').hidden = false; paintOffer(); kiPaint();
}
function kiClose() {
  // the PINs leave memory with the walk; whatever is on the key stays as it is
  ki.user = ''; ki.admin = ''; ki.src = null; ki.preview = null; ki.open = false; $$('#keyinit input, #keyinit textarea').forEach(i => { i.value = ''; }); seedClear();
  // the card list is only read again at the next poll: say now what the walk just made true
  const c = cards.find(c => c.serial === ki.serial); if (c && ki.step === 'done' && ki.pubkey) { c.pubkey = ki.pubkey; c.touch = 'FIXED'; }
  $('#ki-body').innerHTML = ''; $('#keyinit').hidden = true; kiSeen.delete(ki.serial);
  if ($('#s-setup').classList.contains('on')) { renderSetup.nowSig = null; renderSetup(); }
}
$('#ki-close').onclick = () => {
  const mid = kiSteps().indexOf(ki.step) > kiSteps().indexOf('set') && ki.step !== 'done' && !ki.stop;
  if (mid) toast(`Both PINs are set on key ${kiSerial(ki.serial)}. ${kiIsCard(ki.survey) ? 'Put it in the reader again' : 'Plug it in again'} to finish: it will ask for them.`, 9000);
  kiClose();
};
const kiGo = (step) => { ki.step = step; ki.err = ''; ki.busy = false; kiPaint(); };

// pictures: our own drawings, in the app's own line and colours
const kiPic = (inner) => `<svg viewBox="0 0 250 132" role="img">${inner}</svg>`;
const kiLock = (x, y, w, label, pips) => `<path class="g-line" d="M${x + w * .25} ${y}v-${w * .22}a${w * .25} ${w * .25} 0 0 1 ${w * .5} 0v${w * .22}"/><rect class="g-ink" x="${x}" y="${y}" width="${w}" height="${w * .78}" rx="6"/>`
  + [0, 1, 2].map(i => `<circle cx="${x + w / 2 + (i - 1) * 11}" cy="${y + w * .39}" r="3.4" class="${i < pips ? 'g-strip' : 'g-hole'}"/>`).join('') + `<text class="ki-t" x="${x + w / 2}" y="${y + w * .78 + 18}" text-anchor="middle">${label}</text>`;
const kiPage = (x, lines) => `<rect class="g-lap" x="${x}" y="14" width="104" height="104" rx="6"/>` + [0, 1, 2, 3, 4].map(i => `<circle class="g-hole" cx="${x + 9}" cy="${28 + i * 19}" r="2.6" stroke="var(--line)"/>`).join('')
  + lines.map((l, i) => `<text class="ki-t s" x="${x + 20}" y="${36 + i * 21}">${l}</text><path class="ki-rule" d="M${x + 20} ${41 + i * 21}h74"/>`).join('');
// the key standing on its plug, for the read-back page: a YubiKey with its gold contact, a Nitrokey 3 as its wedge, a
// Token2 key with its button, a Token2 card lying flat with its chip
const kiKeyUpright = (form) => isCard(form)
  ? `<rect class="g-ink" x="166" y="46" width="72" height="46" rx="5"/><rect class="g-strip" x="174" y="58" width="14" height="10" rx="2"/><path class="g-ring" d="M174 61.5h14M174 65h14M181 58v10"/><path class="g-ring" d="M218 62a6 6 0 0 1 0 12M223 58a11 11 0 0 1 0 20"/>`
  : isToken2(form)
  ? `<rect class="g-ink" x="186" y="40" width="32" height="58" rx="7"/><circle class="g-ring" cx="202" cy="60" r="7"/><circle class="g-led" cx="202" cy="60" r="2"/><circle cx="202" cy="86" r="3.5" class="g-hole"/>`
  : isNitro(form)
  ? `<path class="g-ink" d="M184 40h36q4 0 4 4v36q0 14 -12 14h-20q-12 0 -12 -14V44q0 -4 4 -4z"/><circle class="g-ring" cx="202" cy="64" r="7"/><path class="g-ring" d="M212 56l2.4 -2M214 64h3.5M212 72l2.4 2"/><rect class="g-hole" x="198" y="84" width="8" height="4" rx="1.5"/><circle class="g-led" cx="190" cy="46" r="2"/>`
  : `<rect class="g-ink" x="182" y="40" width="40" height="56" rx="8"/><circle class="g-gold" cx="202" cy="62" r="9"/><circle cx="202" cy="84" r="4" class="g-hole"/>`;
// a page of 24 words, for the "made outside" card
const kiWordsPage = () => `<svg class="ki-wordspage" viewBox="0 0 70 62" aria-hidden="true"><rect class="g-lap" x="4" y="3" width="62" height="56" rx="3" transform="rotate(-3 35 31)"/><circle class="g-led" cx="15" cy="13" r="2.4"/><text class="ki-t s" x="21" y="16" style="font-size:8px">24 words</text>${[0,1,2,3,4,5].map(i => `<path class="ki-rule" d="M13 ${25 + i * 5.5}h20M39 ${25 + i * 5.5}h18"/>`).join('')}</svg>`;
// the two ways a signing key can be made, side by side: born inside the key (nothing on paper, no copy), or made
// outside it as 24 words on a page that go into the key — and can go into a spare
function kiChoiceArt(form) {
  // the way the person has just chosen, and only that: the page of words writes the key in — and a spare
  const key = (cx, top, sc) => `<g transform="translate(${(cx - 202 * sc).toFixed(1)} ${(top - 40 * sc).toFixed(1)}) scale(${sc})">${kiKeyUpright(form)}</g>`;
  const page = (x, y, sc, lines) => `<g transform="translate(${x} ${y}) scale(${sc})">${kiPage(0, lines)}</g>`;
  return kiPic(
    `<text class="ki-t cap" x="125" y="14" text-anchor="middle">MADE OUTSIDE, BACKED UP \u2014 YOUR CHOICE</text>`
    + page(14, 28, 0.62, ['24 words', '', '', '']) + `<path class="g-arrow" d="M84 60h22m-7 -7l7 7l-7 7"/>`
    + key(135, 30, 0.95) + `<text class="ki-t c" x="135" y="100">your key</text>`
    + `<path class="g-arrow" stroke-dasharray="3 3" d="M162 60h26m-7 -7l7 7l-7 7"/>` + `<g opacity=".45">${key(216, 38, 0.72)}</g><text class="ki-t c" x="216" y="100">a spare</text>`
    + `<text class="ki-t c" x="125" y="124">the words write it in \u2014 and into a spare, if you want one</text>`);
}
const KI_ART = {
  meet: (s) => `<div class="ki-keyart">${keyArt(kiForm(s), s.nfc, 150)}</div>`,
  firmware: (s) => ki.fw && ki.fw.phase === 'touch' ? guideArt('touch', kiForm(s)) : `<div class="ki-keyart">${keyArt(kiForm(s), s.nfc, 150)}</div>`,
  pins: () => kiPic(kiLock(42, 44, 52, 'KEY PIN', 3) + kiLock(144, 30, 70, 'ADMIN PIN', 3)),
  record: () => kiPic(kiPage(40, ['name', 'serial', 'key PIN', 'admin PIN']) + `<g transform="translate(176 92) rotate(-38)"><rect class="g-finger" x="-5" y="-62" width="10" height="56" rx="2"/><path class="g-ink" d="M-5 -6h10l-5 12z"/></g>`),
  set: (s) => guideArt('in', kiForm(s)),
  readback: (s) => kiPic(kiPage(22, ['name', 'serial', '••••••', '••••••••']) + `<path class="g-arrow" d="M138 66h30m-8 -8l8 8l-8 8"/>` + kiKeyUpright(kiForm(s))),
  harden: () => kiPic([0, 1].map(i => `<rect x="62" y="${30 + i * 44}" width="62" height="30" rx="15" fill="var(--signal)"/><circle cx="109" cy="${45 + i * 44}" r="11" fill="var(--panel)"/><path class="ki-rule" d="M138 ${40 + i * 44}h54M138 ${51 + i * 44}h34"/>`).join('')),
  which: () => `<div class="ki-keyart" style="gap:18px">${keyArt(FORM_TOKEN2_KEY, true, 96)}${keyArt(FORM_TOKEN2_CARD, true, 96)}</div>`,
  key: (s) => ki.phase === 'idle' ? `<div class="ki-keyart">${keyArt(kiForm(s), s.nfc, 150)}</div>` : ki.phase === 'source' ? kiChoiceArt(kiForm(s)) : ki.phase === 'words' ? kiPic(kiPage(40, ['1 word', '2 word', '3 word', '\u2026 ' + ((ki.preview && ki.preview.words) ? ki.preview.words.split(' ').length : 24)]) + `<path class="g-arrow" d="M156 66h30m-8 -8l8 8l-8 8"/>` + kiKeyUpright(kiForm(s)).replace(/translate\([^)]*\)/g, '')) : guideArt(ki.phase === 'touch' && !kiIsCard(s) ? 'touch' : 'in', kiForm(s)),
  done: (s) => `<div class="ki-keyart done">${keyArt(kiForm(s), s.nfc, 150)}<svg class="ki-tick" viewBox="0 0 40 40" width="44" height="44" aria-hidden="true"><circle cx="20" cy="20" r="19" fill="var(--signal)"/><path d="M11 21l6 6l12 -14" fill="none" stroke="#fff" stroke-width="3.6" stroke-linecap="round" stroke-linejoin="round"/></svg></div>`,
};
const kiPinField = (id, label, hint) => `<div class="field"><label class="cap" for="${id}">${label}</label><input id="${id}" type="password" autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false"><div class="ki-say" id="${id}-say">${hint || ''}</div></div>`;
const kiErr = () => ki.err ? `<div class="warnbox ki-err" role="alert">${esc(ki.err)}</div>` : '';

// what else is on the key, said as plainly as each part allows without a PIN
function kiNeighbours(s) {
  const rows = [['Signing (OpenPGP)', 'empty, on the factory PINs', 'set up here']];
  const p = s.piv; rows.push(['Smart-card login (PIV)', !p ? 'switched off, or would not say' : (p.keys || p.certs) ? `${p.keys == null ? '' : p.keys + ' key' + (p.keys === 1 ? '' : 's') + ', '}${p.certs} certificate${p.certs === 1 ? '' : 's'}` : 'nothing stored', 'left alone']);
  const nitro = s.device === 'nitrokey3', token2 = kiToken2(s);
  const o = s.oath; rows.push(['Authenticator codes', nitro ? 'kept by Nitrokey’s own app; not looked into' : token2 ? 'not looked into on a Token2' : !o ? 'switched off, or would not say' : o.password ? 'behind a password; not looked into' : o.accounts == null ? 'would not say' : o.accounts ? `${o.accounts} account${o.accounts === 1 ? '' : 's'}` : 'none', 'left alone']);
  const t = s.otp; rows.push(['One-time-code slots', nitro ? 'none on a Nitrokey' : token2 ? 'none on a Token2' : !t ? 'switched off' : [t.slot1 ? 'slot 1 in use' + (t.slot2 ? '' : ' (every key leaves the factory that way)') : '', t.slot2 ? 'slot 2 in use' : ''].filter(Boolean).join(', ') || 'both empty', 'left alone']);
  rows.push(['Passkeys and website logins', 'cannot be listed by anyone without the key’s own passkey PIN; not read', 'left alone']);
  return `<div class="ki-rows">${rows.map(r => `<div class="ki-row"><b>${r[0]}</b><span class="muted">${esc(r[1])}</span><span class="tag${r[2] === 'set up here' ? ' out' : ''}">${r[2]}</span></div>`).join('')}</div>`;
}

function kiPaint() {
  const s = ki.survey, steps = kiSteps(), at = steps.indexOf(ki.step === 'firmware' ? 'meet' : ki.step);
  $('#ki-prog').innerHTML = steps.map((k, i) => `<li class="${i < at ? 'done' : i === at ? 'now' : ''}"${i === at ? ' aria-current="step"' : ''}><span>${KI_NAMES[k]}</span></li>`).join('');
  $('#ki-art').innerHTML = KI_ART[ki.step](s);
  const body = $('#ki-body'), title = (t) => `<div class="cer-act-title" id="ki-title">${t}</div>`;
  const foot = (...b) => `<div class="row ki-foot">${b.join('')}</div>`;
  const back = (to) => `<button type="button" class="btn" data-ki-go="${to}">Back</button>`;

  if (ki.stop) {
    body.innerHTML = title(ki.stop.title) + ki.stop.lines.map(l => `<div>${l}</div>`).join('') + foot('<button type="button" class="btn primary" id="ki-end">Close</button>');
    $('#ki-end').onclick = kiClose; return;
  }

  if (ki.step === 'meet') {
    const adv = keyAdvisories({ firmware: s.firmware }), newer = s.firmware && !verLt(s.firmware, '5.7.0'), nitro = s.device === 'nitrokey3', token2 = kiToken2(s), card = kiIsCard(s);
    body.innerHTML = title(`A new ${esc(modelName(s.form, s.nfc))}`)
      + `<div class="muted">Serial ${kiSerial(s.serial)}${s.firmware ? ' · firmware ' + esc(s.firmware) : ''}. It has never been set up: no key on it, and both PINs still the ones every key ships with.</div>`
      + (adv.length ? adv.map(a => `<div class="warnbox"><b>${esc(a.id)}: ${esc(a.title)}.</b> ${esc(a.says)} ${esc(a.here)}</div>`).join('')
        : nitro ? `<div class="muted" id="ki-nkfw">${s.firmware ? `Firmware ${esc(s.firmware)}. Checking it against the newest release known to this app…` : 'Reading its firmware version…'}</div>`
        : token2 ? `<div class="muted">A Token2 does not say its firmware, so there is nothing to check against known advisories.</div>${s.nfc ? `<div class="muted">Held to the phone, a Token2 key and a Token2 card answer exactly alike, so the app cannot tell which this is. Look at what you held up: the next step asks. The serial is not printed on either, so this number is how the app will know it again.</div>` : ''}`
        : `<div class="muted">${s.firmware ? `No security advisory is known for firmware ${esc(s.firmware)} as of ${ADVISORIES_AS_OF}. Firmware cannot be changed on a YubiKey, so this is checked here, on this computer, each time the list is updated.` : 'The key would not say its firmware, so it could not be checked against known advisories.'}</div>`)
      + (newer ? `<div class="muted">Signing with this app was proven on firmware 5.4. This key is newer; the last step of setup has it sign and checks the answer, so nothing is taken on trust.</div>` : nitro ? `<div class="muted">Signing with a Nitrokey 3 was proven on firmware 1.9.0 and 1.9.1. The last step of setup has this key sign and checks the answer, so nothing is taken on trust.</div>` : token2 ? `<div class="muted">Signing with a Token2 is checked the way it is with every key: the last step of setup has it sign and checks the answer, so nothing is taken on trust.</div>` : '')
      + `<div class="cap">Already on this key</div>` + kiNeighbours(s)
      + `<div class="muted">Setting it up changes its two signing PINs and makes one signing key in an empty slot. Nothing is erased, here or anywhere else on the key.</div>`
      + foot(`<button type="button" class="btn primary" data-ki-go="pins">Set this ${card ? 'card' : 'key'} up</button>`, '<button type="button" class="btn" id="ki-later">Not now</button>');
    $('#ki-later').onclick = () => { kiLater.add(ki.serial); kiClose(); };
    if (nitro) kiNkFirmware(s);
  }

  if (ki.step === 'firmware') {
    // the update runs in the backend; the words here follow its progress events. A fresh key is the safest one to
    // update: nothing on it yet to lose, so this is offered before the PINs are set, never after.
    const p = ki.fw || { phase: 'fetch', pct: 0, text: 'Starting' };
    const bar = `<div class="ki-bar"><i style="width:${p.pct}%"></i></div>`;
    if (p.phase === 'error') body.innerHTML = title('The update did not go through') + `<div class="warnbox">${esc(p.text)}</div>`
      + `<div class="muted">Nothing on the key is lost by a failed attempt: its loader refuses an image it cannot verify, and a key left in its loader comes back as a key when it is unplugged and plugged in again.</div>`
      + foot('<button type="button" class="btn primary" id="ki-fw-retry">Try again</button>', '<button type="button" class="btn" data-ki-go="meet">Back</button>');
    else if (p.phase === 'done') body.innerHTML = title(`${esc(p.text)} is on the key`) + `<div class="muted">The key restarted and reported the new version itself. Its setup carries on from the start.</div>`
      + foot('<button type="button" class="btn primary" id="ki-fw-done">Continue</button>');
    else body.innerHTML = title(p.phase === 'touch' ? (isNitro(s.form) ? 'Press the key' : 'Touch the key') : 'Updating the firmware')
      + (p.phase === 'touch' ? `<div>Its light is blinking <b>white</b>. Press the top of the key now: it restarts into its loader.</div>` : '')
      + `<div class="muted">${esc(p.text)}</div>` + bar
      + `<div class="muted">Leave the key plugged in. A steady <b>red</b> light would mean the key's own program has stopped: unplug it, plug it in, and try again. White blinking is a touch wanted; green is done.</div>`;
    if ($('#ki-fw-retry')) $('#ki-fw-retry').onclick = () => kiNkUpdate(s);
    if ($('#ki-fw-done')) $('#ki-fw-done').onclick = async () => { let all = []; try { all = await invoke('card_survey'); } catch {} const again = all.find(x => x.serial === s.serial); if (again) { ki.survey = again; kiSeen.set(again.serial, again); } ki.fw = null; kiGo('meet'); };
  }

  if (ki.step === 'pins') {
    body.innerHTML = title('Two PINs, two jobs')
      + `<div class="ki-two"><div><b>Key PIN</b><span class="tag">6 or more characters</span><div class="muted">Asked every time this key signs. Three wrong tries in a row and the key refuses to sign until the admin PIN unlocks it.</div></div>
         <div><b>Admin PIN</b><span class="tag">8 or more characters</span><div class="muted">Asked rarely: now, when a vault is sealed onto the key, and to unlock a key PIN that ran out of tries. Three wrong tries in a row and the signing part of the key locks for good. Only wiping it frees it, and the wipe destroys the key it holds.</div></div></div>`
      + `<div class="muted">Nobody can reset either one for you: not the maker, not this app. That is the point of them, and it is why a vault needs any two keys rather than all of them. One key lost to a forgotten PIN costs nothing while two others still sign.</div>`
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
    const otpSay = s.device === 'nitrokey3' ? 'A Nitrokey has no one-time-code keyboard, so there is nothing to switch off.' : kiToken2(s) ? 'A Token2 has no one-time-code keyboard, so there is nothing to switch off.' : s.config_locked ? 'This key’s settings are locked with a code, so this is left as it is.' : s.otp_usb === false ? 'Already off on this key.' : s.otp_usb == null ? 'The key would not say, so this is left as it is.'
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
      if (!sig && !otp) return kiGo(kiAfterHarden());
      const b = $('#ki-apply'); b.disabled = true; b.textContent = otp ? 'Applying; the key restarts…' : 'Applying…';
      try {
        const r = await invoke('card_init_harden', { serial: s.serial, adminPin: ki.admin, sigPinEach: sig, otpOff: otp }); ki.notes = r.notes || [];
        if (r.sig_pin_each) s.pgp.sig_pin_each = true;
        if (otp && r.otp_off) { s.otp_usb = false; for (let i = 0; i < 20 && !ki.nfc; i++) { await kiSleep(750); let l = []; try { l = await invoke('cards_list'); } catch {} if (l.some(c => c.serial === s.serial)) break; } }
        kiGo(kiAfterHarden());
      } catch (e) { ki.err = String(e); kiPaint(); }
    };
  }

  if (ki.step === 'which') {
    body.innerHTML = title('Which Token2 is this?')
      + `<div class="muted">Held to a phone, the two look the same to the app, and they sign differently: the key has a button that must be pressed for every signature; the card has no button, so its PIN alone signs. Say which one this is and its signing key is made that way. It cannot be changed afterwards without wiping the key.</div>`
      + `<div class="ki-ways">
          <button type="button" class="ki-way ki-which${ki.kind === 'key' ? ' on' : ''}" data-kind="key"><div class="cap">A security key</div><div class="ki-way-pic">${keyArt(FORM_TOKEN2_KEY, true, 92)}</div><b>A small stick with a button.</b><div class="muted">Every signature asks for the PIN and a press of the button.</div></button>
          <button type="button" class="ki-way ki-which${ki.kind === 'card' ? ' on' : ''}" data-kind="card"><div class="cap">A smart card</div><div class="ki-way-pic">${keyArt(FORM_TOKEN2_CARD, true, 92)}</div><b>A card the size of a bank card.</b><div class="muted">No button: its PIN alone signs, so keep that PIN private.</div></button>
        </div>`
      + kiErr() + foot(back('harden'));
    $$('#ki-body [data-kind]').forEach(b => b.onclick = () => { ki.kind = b.dataset.kind; ki.touch = ki.kind === 'key'; kiGo('key'); });
  }

  if (ki.step === 'key') {
    const notes = ki.notes.map(n => `<div class="muted">${esc(n)}</div>`).join(''), card = kiIsCard(s), form = kiForm(s);
    if (ki.phase === 'idle') body.innerHTML = title('One secret. Two ways to make it.')
      + `<div class="muted">The difference is where the risk sits: inside, the secret can never have been seen; outside, you hold a backup, and the moment it was made had better have been safe.</div>`
      + `<div class="ki-ways">
          <div class="ki-way"><div class="cap">Made inside</div><h4>Only in your key.</h4>
            <div class="ki-way-pic">${keyArt(form, s.nfc, 92)}<div class="ki-way-cap">Created here. Stays here.</div></div>
            <b>No copy. Nothing to write down.</b>
            <ul class="ki-procon"><li class="pro">The secret never exists on any computer, so no laptop, however compromised, can have seen it. This is cryptography at its safest.</li><li class="pro">Nothing to write down, hide, or lose.</li><li class="con">No copy: lose the key or its PINs and that key is gone for good.</li><li class="note">In a vault where any two keys sign, that is fine: the other keys are its backup, and a new key takes its place. For a vault of one key it is a single point of failure.</li></ul>
            <button type="button" class="btn primary" id="ki-make">Make it inside the key</button></div>
          <div class="ki-way"><div class="cap">Made outside</div><h4>A key. And a backup.</h4>
            <div class="ki-way-pic">${kiWordsPage()}<svg class="ki-branch" viewBox="0 0 160 30" aria-hidden="true"><path class="g-arrow" d="M80 0v9M40 9h80M40 9v15m-5 -5l5 5l5 -5M120 9v15m-5 -5l5 5l5 -5"/></svg><div class="ki-way-keys"><div>${keyArt(form, s.nfc, 54)}<small>Your key</small></div><div class="ki-spare">${keyArt(form, s.nfc, 54)}<small>A spare, if needed</small></div></div></div>
            <b>Keep the words. Make a spare.</b>
            <ul class="ki-procon"><li class="pro">24 words on a page restore this key and can write it to a spare: the key is never the only copy.</li><li class="pro">The randomness can be your own, from Entropia tablets or dice, checkable by hand.</li><li class="con">The secret has existed outside the key once \u2014 on this computer, on the page, in the room. It is only as safe as that moment was: a clean machine, nobody watching, no camera.</li><li class="con">Two things to guard from then on, the key and the page. Anyone holding the page needs nothing else.</li></ul>
            <button type="button" class="btn" id="ki-make-out">Make it outside, with a backup\u2026</button></div>
        </div>`
      + `<div class="ki-footnote"><span class="ki-fp" aria-hidden="true"><svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M6 10a6 6 0 0 1 12 0v4M9 10a3 3 0 0 1 6 0v6M12 10v8M4 13a8 8 0 0 0 1 3M20 13a8 8 0 0 1-1 3"/></svg></span><div>${card ? '<b>A card has no button: its PIN alone signs, so keep the key PIN private.</b><div class="muted">Anyone holding the card and its PIN can sign; there is no touch to stop them.</div>' : '<b>Either way, signing needs your touch.</b><div class="muted">That is set for good here; turning it off means wiping the key.</div>'}</div></div>`
      + notes + kiErr();
    else if (ki.phase === 'source') body.innerHTML = kiSourceHtml(s) + kiErr();
    else if (ki.phase === 'words') body.innerHTML = kiWordsHtml(s) + kiErr();
    else if (ki.phase === 'making') body.innerHTML = title('Making the key…') + `<div class="muted">${card ? 'Inside the card. Leave it in the reader.' : 'Inside the key. Leave it plugged in.'}</div>`;
    else if (ki.phase === 'touch') body.innerHTML = card
      ? title('The card is signing') + `<div>Its PIN alone signs: there is no button to press.</div><div class="muted">This is its first signature. The answer is checked against the key it just made, which proves the PIN and the key together.</div>`
      : title(kiPress(s) ? 'Press the key' : 'Touch the key') + `<div>Its light is blinking. ${isToken2(s.form) ? 'Press the button on the key now.' : isNitro(s.form) ? 'Press the top of the key now.' : 'Touch the gold contact now.'}</div><div class="muted">This is its first signature. The answer is checked against the key it just made, which proves the PIN, the ${kiPress(s) ? 'press' : 'touch'} and the key together.</div>`;
    else body.innerHTML = title(card ? 'The first signature did not go through' : 'The key was not touched in time') + `<div class="muted">The signing key is made. Only its first signature is left.</div>` + kiErr() + foot(`<button type="button" class="btn primary" id="ki-prove">${card ? 'Try again' : 'Try the touch again'}</button>`);
    const prove = async () => {
      ki.phase = 'touch'; ki.err = ''; kiPaint();   // the picture and the words are on screen BEFORE the key starts waiting
      try { kiTake(await invoke('card_init_prove', { serial: s.serial, userPin: ki.user, pubkey: ki.pubkey, name: ki.name, offkey: ki.offkey })); ki.phase = 'idle'; ki.src = null; ki.preview = null; kiGo('done'); }
      catch (e) { ki.phase = 'retry'; ki.err = String(e); kiPaint(); }
    };
    if ($('#ki-make')) $('#ki-make').onclick = async () => {
      ki.phase = 'making'; ki.err = ''; ki.offkey = false; kiPaint();
      try { ki.pubkey = await invoke('card_init_key', { serial: s.serial, adminPin: ki.admin, touch: ki.touch !== false }); } catch (e) { ki.phase = 'idle'; ki.err = String(e); kiPaint(); return; }
      prove();
    };
    if ($('#ki-make-out')) $('#ki-make-out').onclick = () => { ki.phase = 'source'; ki.err = ''; ki.src = { kind: 'app', text: '', n: 24, words: 24, short: false }; ki.preview = null; ki.wordAt = null; ki.wordsSeen = null; kiPaint(); };
    if (ki.phase === 'source') kiWireSource(s);
    if (ki.phase === 'words') kiWireWords(s, prove);
    if ($('#ki-prove')) $('#ki-prove').onclick = prove;
  }

  if (ki.step === 'done') {
    ki.user = ''; ki.admin = '';   // nothing left to ask the key: the PINs leave memory here
    const did = ['Both PINs are yours, and the page was checked against the key', s.pgp.sig_pin_each ? 'The key PIN is asked for every signature' : '', s.otp_usb === false ? 'It no longer types one-time codes' : '', ki.offkey ? 'Its signing key was made outside it and written in: the 24 words on your page are its backup, and can write it to a spare key (Vault settings › Backup of the keys)' : kiIsCard(s) ? 'Its signing key was made inside it' : 'Its signing key was made inside it, and needs a touch every time', 'Its first signature checked out'].filter(Boolean);
    body.innerHTML = title(`${esc(ki.name)} is ready`)
      + `<div class="ki-did">${did.map(d => `<div><span class="ok">✓</span>${d}</div>`).join('')}</div>`
      + `<div class="muted">It can join a vault with one click now, here or later: New vault will know it by name and ask for nothing more. Put the page back where it lives, away from this key.</div>`
      + foot('<button type="button" class="btn primary" id="ki-vault">Make a vault with it</button>', '<button type="button" class="btn" id="ki-end">Done</button>')
      + `<div class="muted">Setting up another? ${kiIsCard(s) ? 'Take this one out of the reader and put the next in.' : 'Unplug this one and plug the next in.'}</div>`;
    $('#ki-end').onclick = kiClose;
    $('#ki-vault').onclick = () => { kiClose(); if (!$('#s-setup').classList.contains('on')) show('setup'); };
  }
  $$('#ki-body [data-ki-go]').forEach(b => b.onclick = () => kiGo(b.dataset.kiGo));
}
// ---------- a Nitrokey's firmware: read from its admin app, compared offline, updated on request ----------
// The newest version known to this build is compared without asking anyone, like the advisory list. Only pressing
// "Update" reaches Nitrokey's release list, and that request carries nothing about the key.
async function kiNkFirmware(s) {
  let fw = null; try { fw = await invoke('nk_firmware', { serial: s.serial }); } catch (e) { fw = { error: String(e) }; }
  if (ki.step !== 'meet' || ki.serial !== s.serial) return;
  const el = $('#ki-nkfw'); if (!el) return;
  ki.nkfw = fw;
  if (fw.error) { el.textContent = `Its firmware could not be read over USB (${fw.error}). Nitrokey App 2 shows and updates it.`; return; }
  if (fw.version && !s.firmware) s.firmware = fw.version;
  if (!fw.behind) { el.innerHTML = `Firmware ${esc(fw.version)}: the newest release known to this app as of ${esc(fw.latest_known_as_of)}.`; return; }
  el.innerHTML = `<b>Firmware ${esc(fw.version)}; ${esc(fw.latest_known)} is out</b> (known to this app as of ${esc(fw.latest_known_as_of)}). `
    + (fw.flashable ? `Update it now, before the PINs are set: a new key is the safest one to update, with nothing on it yet to lose. The update is Nitrokey's own signed release, fetched from their release list when you press the button; the key's loader checks the signature itself. One touch, under a minute.`
      : `This is the nRF52 kind of Nitrokey 3 (a 3A Mini); update it with Nitrokey App 2 before setting it up.`)
    + (fw.flashable ? `<div class="row" style="margin-top:8px"><button type="button" class="btn primary" id="ki-fw-go">Update the firmware to ${esc(fw.latest_known)}</button></div>` : '');
  if ($('#ki-fw-go')) $('#ki-fw-go').onclick = () => kiNkUpdate(s);
}
async function kiNkUpdate(s) {
  ki.fw = { phase: 'fetch', pct: 0, text: 'Starting' }; kiGo('firmware');
  const off = await window.__TAURI__.event.listen('nk-firmware', (ev) => { if (ki.step !== 'firmware') return; ki.fw = ev.payload; kiPaint(); });
  try { const after = await invoke('nk_firmware_update', { serial: s.serial }); ki.fw = { phase: 'done', pct: 100, text: `Firmware ${after.version}` }; s.firmware = after.version; ki.nkfw = after; }
  catch (e) { ki.fw = { phase: 'error', pct: 0, text: String(e) }; }
  finally { try { off(); } catch {} }
  if (ki.step === 'firmware') kiPaint();
}
// ---------- a secret made outside the key ----------
// Nothing typed here is stored: the words live in this window only while the walk is open. The words are shown and
// confirmed written BEFORE the key is written, so a key never holds a secret that is not on a page.
const KI_SOURCES = [
  ['app', 'Let this app make it', 'From this computer\u2019s randomness. The words are shown next, one at a time, to write down.'],
  ['words', 'I drew it with Entropia tablets, or I have the words', 'Type the words. A phrase drawn one word short is completed here: the last word carries the checksum, and the valid endings are listed to pick from.'],
  ['dice', 'I rolled a die', 'Enter the rolls, 1 to 6, at least 99 of them. They are hashed once with SHA-256, the way a Coldcard does, so a Coldcard shows the same 24 words for the same rolls.'],
  ['hex', 'I have hex', '32 to 64 hex characters of randomness from any other source.'],
];
const KI_LENGTHS = [12, 15, 18, 21, 24];
// a die face in the house line: the body, and the pips where a real die has them
const KI_PIPS = { 1: [[22, 22]], 2: [[13, 13], [31, 31]], 3: [[13, 13], [22, 22], [31, 31]], 4: [[13, 13], [31, 13], [13, 31], [31, 31]], 5: [[13, 13], [31, 13], [22, 22], [13, 31], [31, 31]], 6: [[13, 13], [31, 13], [13, 22], [31, 22], [13, 31], [31, 31]] };
const kiDie = (n) => `<svg viewBox="0 0 44 44" aria-hidden="true"><rect class="die-body" x="4" y="4" width="36" height="36" rx="8"/>${KI_PIPS[n].map(([x, y]) => `<circle class="die-pip" cx="${x}" cy="${y}" r="3.4"/>`).join('')}</svg>`;
function kiSourceHtml(s) {
  const src = ki.src || { kind: 'app', text: '' }, cur = KI_SOURCES.find(x => x[0] === src.kind) || KI_SOURCES[0];
  return `<div class="cer-act-title" id="ki-title">Where does the secret come from?</div>`
    + `<div class="ki-rows ki-src">${KI_SOURCES.map(([k, t, d]) => `<button type="button" class="ki-row ki-srcopt${src.kind === k ? ' on' : ''}" data-src="${k}"><b>${t}</b><span class="muted">${d}</span></button>`).join('')}</div>`
    + (cur[0] === 'app' ? `<div class="field"><div class="row" style="justify-content:space-between;align-items:center"><div class="label">How many words</div><div class="pick tight" id="ki-app-len">${KI_LENGTHS.map(n => `<div class="opt${(src.words || 24) === n ? ' on' : ''}" data-n="${n}">${n}</div>`).join('')}</div></div><div class="muted" style="font-size:13px">24 words carry 256 bits, 12 carry 128 \u2014 still far beyond any search; 15, 18 and 21 sit between. Fewer words are quicker to write and to check.</div></div>`
      : cur[0] === 'words'
      ? `<div class="field seed-root" id="ki-seed"><div class="row" style="justify-content:space-between;align-items:center;flex-wrap:wrap;gap:8px"><div class="label">The words, in order</div><div class="pick tight seed-len">${KI_LENGTHS.map(n => `<div class="opt${(src.n || 24) === n ? ' on' : ''}" data-n="${n}">${n}</div>`).join('')}</div></div>
          <button type="button" class="vchip" id="ki-short" aria-pressed="${!!src.short}">I drew one word short: the last word is chosen here</button>
          <div class="muted" style="font-size:13px">Only the word you are on is shown; the rest stay hidden. Type the first letters and pick, or paste the whole set into the first box.</div>
          <div class="seed-grid"></div><div class="row seed-suggest" style="gap:var(--s2);flex-wrap:wrap;min-height:34px" aria-live="polite"></div><div class="seed-count" style="font-size:14px" aria-live="polite"></div></div>`
      : cur[0] === 'dice'
      ? `<div class="field"><div class="label">The rolls, in order</div><div class="dice-pad" id="ki-dice"><div class="dice-rolls mono" id="ki-dice-rolls" aria-live="polite"></div><div class="dice-keys">${[1, 2, 3, 4, 5, 6].map(n => `<button type="button" class="dice-key" data-roll="${n}" aria-label="rolled ${n}">${kiDie(n)}<span>${n}</span></button>`).join('')}<button type="button" class="dice-key dice-del" data-roll="del" title="Remove the last roll" aria-label="remove the last roll"><svg viewBox="0 0 44 44" aria-hidden="true"><path class="g-line" d="M14 12h20a4 4 0 0 1 4 4v12a4 4 0 0 1-4 4H14l-8-10z"/><path class="g-line" d="M20 18l8 8M28 18l-8 8"/></svg><span>undo</span></button></div><div class="dice-count" id="ki-dice-count"></div><div class="ki-bar dice-bar" aria-hidden="true"><i id="ki-dice-bar"></i></div><div class="muted" style="font-size:13px">Click the faces or type the digits; paste a run of rolls too. Rolls are shown in tens.</div></div></div>`
      : `<div class="field ki-narrow"><label class="cap" for="ki-hex">The hex</label><input id="ki-hex" class="mono" type="text" autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false" maxlength="80" value="${esc(src.text || '')}"><div class="ki-say" id="ki-hex-say"></div></div>`)
    + (ki.preview && ki.preview.endings ? `<div class="cap">The last word: pick one</div><div class="muted">Any of these ${ki.preview.endings.length} completes the words you drew; each makes a different key. Draw one more tablet if you like and take it if it is here, or just take one.</div><div class="row ki-endings">${ki.preview.endings.map(w => `<button type="button" class="btn small" data-ending="${esc(w)}">${esc(w)}</button>`).join('')}</div>` : '')
    + `<div class="row ki-foot"><button type="button" class="btn" data-ki-phase="idle">Back</button><button type="button" class="btn primary" id="ki-src-next">${cur[0] === 'app' ? 'Make it and show the words' : 'Next'}</button></div>`;
}
function kiWireSource(s) {
  $$('#ki-body [data-src]').forEach(b => b.onclick = () => { seedClear(); ki.src = { kind: b.dataset.src, text: '', n: 24, words: 24, short: false }; ki.preview = null; ki.err = ''; kiPaint(); });
  $$('#ki-body [data-ki-phase]').forEach(b => b.onclick = () => { seedClear(); ki.phase = b.dataset.kiPhase; ki.err = ''; ki.preview = null; kiPaint(); });
  // the app's own: how many words
  $$('#ki-app-len .opt').forEach(o => o.onclick = () => { ki.src.words = +o.dataset.n; $$('#ki-app-len .opt').forEach(x => x.classList.toggle('on', x === o)); });
  // the words go into the house grid: the phrase to type (or paste), or the phrase drawn one word short
  const root = $('#ki-seed');
  if (root) {
    const boxes = (ki.src.n || 24) - (ki.src.short ? 1 : 0);
    if (root.querySelectorAll('.seed-grid input').length !== boxes) { seedBuild(boxes, root).then(() => { const first = root.querySelector('.seed-grid input'); if (first && !ki.src.text) first.focus(); if (ki.src.text) { ki.src.text.split(/\s+/).forEach((w, k) => { const b = root.querySelectorAll('.seed-grid input')[k]; if (b) b.value = w; }); } }); }
    $$('.seed-len .opt', root).forEach(o => { o.onclick = (e) => { e.stopPropagation(); ki.src.n = +o.dataset.n; ki.src.text = ''; ki.preview = null; seedClear(); kiPaint(); }; });
    $('#ki-short').onclick = () => { ki.src.short = !ki.src.short; ki.src.text = ''; ki.preview = null; seedClear(); kiPaint(); };
  }
  // dice: the rolls as a pad, shown in tens, counted against the 99 needed
  const pad = $('#ki-dice');
  if (pad) {
    const paintRolls = () => { const r = ki.src.text || ''; $('#ki-dice-rolls').innerHTML = r ? r.match(/.{1,10}/g).map(g => `<span>${g}</span>`).join('') : '<span class="muted">no rolls yet</span>'; $('#ki-dice-count').innerHTML = r.length >= 99 ? `<span class="ok">\u2713 ${r.length} rolls \u2014 enough for 256 bits</span>` : `${r.length} of 99 rolls`; const bar = $('#ki-dice-bar'); bar.style.width = Math.min(100, r.length * 100 / 99) + '%'; bar.classList.toggle('full', r.length >= 99); };
    $$('#ki-dice .dice-key').forEach(b => b.onclick = () => { ki.src.text = b.dataset.roll === 'del' ? (ki.src.text || '').slice(0, -1) : (ki.src.text || '') + b.dataset.roll; paintRolls(); });
    pad.tabIndex = 0; pad.onkeydown = (e) => { if (/^[1-6]$/.test(e.key)) { ki.src.text = (ki.src.text || '') + e.key; paintRolls(); e.preventDefault(); } else if (e.key === 'Backspace') { ki.src.text = (ki.src.text || '').slice(0, -1); paintRolls(); e.preventDefault(); } };
    pad.onpaste = (e) => { const t = (e.clipboardData.getData('text') || '').replace(/[^1-6]/g, ''); if (t) { ki.src.text = (ki.src.text || '') + t; paintRolls(); e.preventDefault(); } };
    paintRolls(); pad.focus();
  }
  const hx = $('#ki-hex');
  if (hx) { const say = () => { const t = hx.value.replace(/\s/g, ''); ki.src.text = t; const ok = /^[0-9a-fA-F]*$/.test(t); $('#ki-hex-say').textContent = !t ? '' : !ok ? 'Only 0\u20139 and a\u2013f.' : [32, 40, 48, 56, 64].includes(t.length) ? `${t.length} characters: ${t.length * 4 / 11 * 33 / 32 | 0} words` : `${t.length} characters; 32, 40, 48, 56 or 64 make a phrase`; $('#ki-hex-say').classList.toggle('bad', !!t && (!ok || ![32, 40, 48, 56, 64].includes(t.length))); }; hx.oninput = say; say(); hx.focus(); }
  $$('#ki-body [data-ending]').forEach(b => b.onclick = () => { ki.src.text = ki.src.text.trim() + ' ' + b.dataset.ending; ki.src.short = false; ki.preview = null; seedClear(); kiPaint(); setTimeout(() => $('#ki-src-next') && $('#ki-src-next').click(), 150); });
  $('#ki-src-next').onclick = async () => {
    ki.err = ''; $('#ki-src-next').disabled = true;
    if (root) ki.src.text = [...root.querySelectorAll('.seed-grid input')].map(i => i.value.trim().toLowerCase()).filter(Boolean).join(' ');
    let p; try { p = await invoke('secret_preview', { source: { kind: ki.src.kind, text: ki.src.text, words: ki.src.words || 24 } }); } catch (e) { ki.err = String(e); kiPaint(); return; }
    if (p.endings) { ki.preview = p; kiPaint(); return; }
    ki.preview = p; ki.srcWritten = false; ki.wordAt = null; ki.wordsSeen = null; ki.phase = 'words'; kiPaint();
  };
}
// One word on screen at a time, as when they are typed: the person steps through, each word shown while it is
// written, and the page can only be called written once every word has been seen.
function kiWordsHtml(s) {
  const p = ki.preview || {}, words = p.words ? p.words.split(' ') : null, n = words ? words.length : 0;
  const at = ki.wordAt == null ? -1 : ki.wordAt, seen = ki.wordsSeen || new Set();
  return `<div class="cer-act-title" id="ki-title">${words ? `Write these ${n} words down` : 'The words you typed carry this key'}</div>`
    + (words ? `<div class="muted">On the same page as the PINs, or a page of their own kept with it. They ARE the key: anyone with them and nothing else can sign for it. They are not a wallet seed and open no other wallet.</div>
        <div class="muted">One word at a time, so a glance at the screen never takes the whole key: click a number, or step through.</div>
        <div class="seed-grid seed-show ki-words" aria-label="the ${n} words, one shown at a time">${words.map((w, i) => `<button type="button" class="${i === at ? 'on' : seen.has(i) ? 'seen' : ''}" data-word-i="${i}" aria-label="word ${i + 1}${i === at ? ': ' + esc(w) : ''}"><span>${i + 1}</span><b class="mono">${i === at ? esc(w) : '\u2022\u2022\u2022\u2022\u2022\u2022'}</b></button>`).join('')}</div>
        <div class="row" style="align-items:center;gap:10px"><button type="button" class="btn small" id="ki-word-prev"${at <= 0 ? ' disabled' : ''}>\u2039 Previous</button><span class="muted" id="ki-word-pos">${at < 0 ? 'Start with word 1' : `Word ${at + 1} of ${n}`}</span><button type="button" class="btn small primary" id="ki-word-next">${at < 0 ? 'Show word 1' : at + 1 < n ? 'Next word \u203a' : 'Hide'}</button></div>`
      : `<div class="muted">Nothing is shown again: the page you typed them from is the backup.</div>`)
    + `<div class="muted">The key will report this public key once the secret is written in: <span class="mono" style="font-size:12px;word-break:break-all">${esc(p.pubkey || '')}</span></div>`
    + (words ? `<div class="row" style="align-items:center;gap:10px;flex-wrap:wrap"><button type="button" class="vchip" id="ki-src-written" aria-pressed="${ki.srcWritten}"${seen.size < n ? ' disabled' : ''}>All ${n} words are written down, away from the key${seen.size < n ? ` (${n - seen.size} not yet shown)` : ''}</button><button type="button" class="btn small" id="ki-metal">Engrave them on metal\u2026</button></div>` : '')
    + `<div class="row ki-foot"><button type="button" class="btn" data-ki-phase="source">Back</button><button type="button" class="btn primary" id="ki-write"${words && !ki.srcWritten ? ' disabled' : ''}>Write it to the key</button></div>`;
}
function kiWireWords(s, prove) {
  $$('#ki-body [data-ki-phase]').forEach(b => b.onclick = () => { ki.phase = b.dataset.kiPhase; ki.err = ''; ki.wordAt = null; ki.wordsSeen = null; kiPaint(); });
  const p = ki.preview || {}, n = p.words ? p.words.split(' ').length : 0;
  const showWord = (i) => { ki.wordsSeen = ki.wordsSeen || new Set(); if (i >= 0 && i < n) ki.wordsSeen.add(i); ki.wordAt = i; kiPaint(); const b = $(`#ki-body [data-word-i="${i}"]`); if (b) b.focus(); };
  $$('#ki-body [data-word-i]').forEach(b => b.onclick = () => showWord(+b.dataset.wordI === ki.wordAt ? -1 : +b.dataset.wordI));
  if ($('#ki-word-next')) $('#ki-word-next').onclick = () => showWord(ki.wordAt == null || ki.wordAt < 0 ? 0 : ki.wordAt + 1 < n ? ki.wordAt + 1 : -1);
  if ($('#ki-word-prev')) $('#ki-word-prev').onclick = () => showWord(Math.max(0, (ki.wordAt || 0) - 1));
  if ($('#ki-src-written')) $('#ki-src-written').onclick = () => { ki.srcWritten = !ki.srcWritten; $('#ki-src-written').setAttribute('aria-pressed', ki.srcWritten); $('#ki-write').disabled = !ki.srcWritten; };
  // metal instead of paper: the same words go to the SeedHammer; the plate counts as the page once it is engraved
  if ($('#ki-metal')) $('#ki-metal').onclick = () => { const words = (ki.preview.words || '').split(' '); ki.wordsSeen = new Set(words.map((_, i) => i)); metalEngraveWords(words, ki.name || 'key').then(() => kiPaint()).catch(fail); };
  $('#ki-write').onclick = async () => {
    // the words shown are what is written: the same source goes to the key, and the key must report the same public key
    const source = ki.src.kind === 'app' ? { kind: 'words', text: ki.preview.words } : ki.src;
    ki.phase = 'making'; ki.err = ''; ki.offkey = true; ki.wordAt = null; kiPaint();
    try { ki.pubkey = await invoke('card_init_key_from', { serial: s.serial, adminPin: ki.admin, source, touch: ki.touch !== false }); }
    catch (e) { ki.phase = 'words'; ki.err = String(e); kiPaint(); return; }
    if (ki.preview.pubkey && ki.pubkey !== ki.preview.pubkey) { ki.phase = 'words'; ki.err = 'The key reported a different public key from the one these words make. Nothing more was done; do not use this key.'; kiPaint(); return; }
    prove();
  };
}
// repaint the Set step after a failure without losing what was typed
function kiPaintKeep(f) { const kept = Object.values(f).map(i => i.value); kiPaint(); ['ki-u1', 'ki-u2', 'ki-a1', 'ki-a2'].forEach((id, i) => { $('#' + id).value = kept[i]; }); $('#ki-u1').dispatchEvent(new Event('input')); }
