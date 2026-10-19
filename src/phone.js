// Any Two Keys on a phone: the drawer behind "More". The sidebar is the same sidebar; it only opens and closes.
(() => {
  const more = document.getElementById('tab-more'), side = document.getElementById('side');
  const set = (open) => { document.body.classList.toggle('drawer', open); more.setAttribute('aria-expanded', open); };
  more.onclick = () => set(!document.body.classList.contains('drawer'));
  document.getElementById('side-close').onclick = () => set(false);
  // going anywhere from the drawer puts it away
  side.addEventListener('click', (e) => { if (e.target.closest('.nav')) set(false); });
  document.getElementById('tabbar').addEventListener('click', (e) => { if (e.target.closest('[data-go]')) set(false); });
  // On a phone a key is not always in the port: one held to the phone is read when asked, by NFC. iOS puts up its
  // own sheet while it listens; Android shows nothing, so the app says what to do itself and takes it away after.
  invoke('platform').then((what) => {
    if (what === 'android') document.body.classList.add('android');
    // A phone has no open-file dialog for the app to call. The system's own chooser (photos, camera, files) is
    // reached through a file field in the page, and what is chosen goes to the backend as bytes.
    if (what === 'ios' || what === 'android') {
      const pick = document.createElement('input'); pick.type = 'file'; pick.multiple = true; pick.hidden = true; document.body.appendChild(pick);
      const b64 = (file) => new Promise((ok, no) => { const r = new FileReader(); r.onload = () => ok(String(r.result).split(',')[1] || ''); r.onerror = () => no(r.error); r.readAsDataURL(file); });
      let where = null;
      $('#tx-addfiles').onclick = () => { where = { name: recName(recKey()), key: recKey(), keepOriginal: !stripOn() }; pick.value = ''; pick.click(); };
      pick.onchange = async () => {
        const files = Array.from(pick.files || []), at = where; if (!files.length || !at) return;
        paintZone(false, true);
        let added = 0, last = null; const notes = [], errors = [];
        for (const f of files) {
          if (f.size > 25 * 1024 * 1024) { errors.push(`${f.name} is over 25 MB`); continue; }
          try { last = await invoke('attach_bytes', { name: at.name, key: at.key, filename: f.name || 'file', base64: await b64(f), keepOriginal: at.keepOriginal }); added += last.added || 0; notes.push(...(last.notes || [])); errors.push(...(last.errors || [])); }
          catch (e) { errors.push(`${f.name}: ${e}`); }
        }
        paintZone(false);
        if (last) attachDone({ store: last.store, added, notes, errors }, at.name); else if (errors.length) toast(errors.join(' \u00b7 '), 9000);
      };
    }
    if (what !== 'ios' && what !== 'android') return;
    if (what === 'ios') document.body.classList.add('ios');
    const tap = document.getElementById('nav-tap'); tap.hidden = false;
    const nfcWait = (on) => {
      let w = document.getElementById('nfc-wait');
      if (!on) { if (w) w.hidden = true; return; }
      if (!w) { w = document.createElement('div'); w.id = 'nfc-wait'; w.setAttribute('role', 'status'); w.setAttribute('aria-live', 'polite');
        w.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="M5 8a9 9 0 0 1 14 0M8 11a5 5 0 0 1 8 0"/><rect x="7.5" y="14" width="9" height="6.5" rx="1.8"/></svg><b>Hold the key or card against the back of the phone</b><span>Keep it still until the phone has read it.</span>';
        document.body.appendChild(w); }
      w.hidden = false;
    };
    tap.onclick = async () => {
      let s; if (what === 'android') nfcWait(true);
      try { s = await invoke('card_tap'); } catch (e) { if (String(e) !== 'cancelled') fail(e); return; } finally { nfcWait(false); }
      if (s.state === 'fresh') return kiOpen(s);
      const what = s.state === 'ours' ? 'holds a signing key and is ready to use' : s.state === 'blocked' ? 'has a PIN with no tries left' : 'is already in use for something else';
      toast(`${modelName(s.form, s.nfc)} \u00b7 serial ${s.serial}${s.firmware ? ' \u00b7 firmware ' + s.firmware : ''}: it ${what}.`, 9000);
    };
  }).catch(() => {});
})();

// A picture kept with a record, shown in the window: tap anywhere to put it away.
async function phoneView(e) {
  const v = await invoke('attach_view', { sha256: e.sha256 });
  let box = document.getElementById('phone-view');
  if (!box) { box = document.createElement('div'); box.id = 'phone-view'; box.setAttribute('role', 'dialog'); box.setAttribute('aria-label', 'Picture'); document.body.appendChild(box); box.onclick = () => { box.hidden = true; box.innerHTML = ''; }; }
  box.innerHTML = '<img alt=""><span class="pv-name"></span><span class="pv-hint">Tap to close</span>';
  box.querySelector('img').src = `data:${v.mime};base64,${v.base64}`; box.querySelector('.pv-name').textContent = e.name || '';
  box.hidden = false;
}
