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
