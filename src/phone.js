// Any Two Keys on a phone: the drawer behind "More". The sidebar is the same sidebar; it only opens and closes.
(() => {
  const more = document.getElementById('tab-more'), side = document.getElementById('side');
  const set = (open) => { document.body.classList.toggle('drawer', open); more.setAttribute('aria-expanded', open); };
  more.onclick = () => set(!document.body.classList.contains('drawer'));
  document.getElementById('side-close').onclick = () => set(false);
  // going anywhere from the drawer puts it away
  side.addEventListener('click', (e) => { if (e.target.closest('.nav')) set(false); });
  document.getElementById('tabbar').addEventListener('click', (e) => { if (e.target.closest('[data-go]')) set(false); });
  // On an iPhone a key is not always in the port: one held to the phone is read when asked, by NFC.
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
    if (what !== 'ios') return;
    document.body.classList.add('ios');
    const tap = document.getElementById('nav-tap'); tap.hidden = false;
    tap.onclick = async () => {
      let s; try { s = await invoke('card_tap'); } catch (e) { if (String(e) !== 'cancelled') fail(e); return; }
      if (s.state === 'fresh') return kiOpen(s);
      const what = s.state === 'ours' ? 'holds a signing key and is ready to use' : s.state === 'blocked' ? 'has a PIN with no tries left' : 'is already in use for something else';
      toast(`${modelName(s.form, s.nfc)} \u00b7 serial ${s.serial}${s.firmware ? ' \u00b7 firmware ' + s.firmware : ''}: it ${what}.`, 9000);
    };
  }).catch(() => {});
})();
