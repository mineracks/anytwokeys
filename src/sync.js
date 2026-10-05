// Any Two Keys — a profile kept the same on every device.
//
// Two places in the window: App settings › Sync (switch it on with a key, see which keys can bring the profile to
// a new device, add another), and the profile picker (bring a synced profile here with one of those keys and its
// PIN). Everything else is quiet: a tick every few seconds asks the backend whether there is anything to say or
// hear, and what arrives from another device is drawn without disturbing what is being typed.
let syncInfo = null;
const syncWhen = (t) => !t ? 'not yet' : new Date(t * 1000).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: 'numeric', minute: '2-digit' });
const syncPinOk = (inp) => { const pin = inp.value;
  if (OTP.test(pin)) { inp.value = ''; toast('That was the key typing a one-time code, not a PIN. Nothing was sent to the key.', 8000); return null; }
  if (pin.length < 6) { toast('A key PIN has at least 6 characters'); return null; }
  return pin; };

// ---------- App settings › Sync ----------
async function syncPaint(force) {
  const card = $('#sync-card'); if (!card || !store || !lockInfo || !lockInfo.sealed) { if (card) card.hidden = true; return; }
  if (force || !syncInfo) { try { syncInfo = await invoke('sync_status'); } catch { return; } }
  card.hidden = false;
  const on = syncInfo.on, enrolled = syncInfo.cards || [];
  const moved = /was replaced on another device/.test(syncPaint.err || '');   // this device must be shown a key again
  const rotating = on && !!syncPaint.rotating, removing = on ? syncPaint.removing : null;
  const here = (cards || []).filter(c => c.pubkey);
  // a key gets a PIN field when there is something to do with it: add it, give it the new key, carry on with it
  const asks = here.filter(c => { const k = enrolled.find(k => k.pubkey === c.pubkey); return rotating || moved || !k || k.stale; });
  // rebuilt only when what it shows changes: a poll must not wipe a PIN being typed
  const sig = [on, syncInfo.last, syncInfo.files_waiting, syncInfo.face_refresh, syncInfo.url, enrolled.map(k => k.pubkey + (k.stale ? '!' : '')).join(','), here.map(c => c.reader + c.pubkey).join(','), syncPaint.err || '', rotating, removing || ''].join('|');
  if (sig === syncPaint.sig) return; syncPaint.sig = sig;
  let host = ''; try { host = new URL(syncInfo.url).host; } catch {}
  $('#sync-state').innerHTML = !on
    ? 'This profile lives on this device only. Switch sync on with one of your keys and its vaults, labels, contacts, places and files follow you to your other devices: encrypted here, held by your sync host, opened only by your keys. Settings stay per device, and nothing that can spend ever leaves the keys.'
    : moved ? `<span class="warn">This profile’s key was replaced on another device, so this one has stopped syncing.</span> Plug in one of the profile’s keys and enter its PIN to carry on. Nothing here is lost: what changed on this device meanwhile goes across once it has the new key.`
    : rotating ? `<b>Replace this profile’s key.</b> Do this when a device or a key that could fetch the profile is lost. Everything is sealed again under a new key and the old copy on ${esc(host)} is emptied. The lost device keeps what it already had and gets nothing more. Afterwards each of your other devices asks for a key once, and each of your other keys must be added again here.`
    : `This profile is kept the same on every device you bring it to. It is encrypted before it leaves here; ${esc(host)} holds what it cannot read. Last synced ${esc(syncWhen(syncInfo.last))}.${syncInfo.files_waiting ? ` ${syncInfo.files_waiting} file${syncInfo.files_waiting === 1 ? ' is' : 's are'} still being sent.` : ''}${syncPaint.err ? ` <span class="warn">The last try did not go through: ${esc(syncPaint.err)}</span>` : ''}`;
  $('#sync-keys-cap').textContent = !on ? 'Switch it on with a key' : rotating ? 'Replace it with the key in your hand' : moved ? 'Carry on with a key' : 'Keys that can bring this profile to a new device';
  const nameOf = (c) => { for (const v of store.vaults) { const k = v.keys.find(k => k.pubkey === c.pubkey); if (k) return k.name; } const r = (store.key_ready || {})[c.pubkey]; return (r && r.name) || 'key ' + (c.serial || ''); };
  const date = (t) => new Date(t * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
  const act = rotating ? 'Replace the key' : moved ? 'Carry on with this key' : on ? 'Add this key' : 'Switch sync on';
  $('#sync-slots').innerHTML = (rotating || moved ? [] : enrolled).map(k => { const plug = here.find(c => c.pubkey === k.pubkey);
      if (removing === k.pubkey) return plug
        ? `<form class="slot add" data-remove="${esc(k.pubkey)}" data-reader="${esc(plug.reader)}"><span><b>${esc(k.name)}</b><small>its PIN takes this profile off the key itself</small></span><span class="row" style="gap:8px"><input type="password" autocomplete="off" placeholder="Key PIN" aria-label="PIN of ${esc(k.name)}"><button class="btn small" type="submit">Remove it</button><button class="linkish" type="button" data-sync-keep style="align-self:center">Keep it</button></span></form>`
        : `<div class="slot"><span><b>${esc(k.name)}</b><small>not plugged in: it leaves this list, but can still fetch the profile until the profile’s key is replaced</small></span><span class="row" style="gap:8px"><button class="btn small" type="button" data-sync-drop="${esc(k.pubkey)}">Take it off the list</button><button class="linkish" type="button" data-sync-keep style="align-self:center">Keep it</button></span></div>`;
      return `<div class="slot"><span><b>${esc(k.name)}</b><small>${k.stale ? 'still has the old key: plug it in and add it again' : 'added ' + date(k.added)}</small></span><button class="btn small" type="button" data-sync-x="${esc(k.pubkey)}">Remove</button></div>`; }).join('')
    + asks.map((c, i) => `<form class="slot add" data-reader="${esc(c.reader)}"><span><b>${esc(nameOf(c))}</b><small>plugged in</small></span><span class="row" style="gap:8px"><input type="password" autocomplete="off" placeholder="Key PIN" id="sync-pin-${i}" aria-label="PIN of ${esc(nameOf(c))}"><button class="btn small" type="submit">${act}</button></span></form>`).join('')
    + (!asks.length ? `<div class="muted" style="font-size:13px">${rotating || moved ? 'Plug in one of this profile’s keys.' : on ? 'Plug in another key to add it. Any one of them, with its PIN, brings the profile to a new device.' : 'Plug in a key that has a signing key on it.'}</div>` : '');
  $('#sync-acts').hidden = !on || moved;
  $('#sync-face-row').hidden = !on || moved || rotating; $('#sync-face').classList.toggle('on', syncInfo.face_refresh !== false); $('#sync-face').setAttribute('aria-checked', syncInfo.face_refresh !== false ? 'true' : 'false');
  $('#sync-now').hidden = rotating; $('#sync-off').hidden = rotating;
  $('#sync-rotate').textContent = rotating ? 'Leave the key as it is' : 'A device or a key was lost…';
  $$('#sync-slots [data-sync-x]').forEach(b => b.onclick = () => { syncPaint.removing = b.dataset.syncX; syncPaint.sig = null; syncPaint(); });
  $$('#sync-slots [data-sync-keep]').forEach(b => b.onclick = () => { syncPaint.removing = null; syncPaint.sig = null; syncPaint(); });
  $$('#sync-slots [data-sync-drop]').forEach(b => b.onclick = async () => { try { syncInfo = await invoke('sync_remove_card', { pubkey: b.dataset.syncDrop, reader: null, pin: null }); toast('Off the list. To stop that key fetching the profile, replace the profile’s key.', 9000); } catch (e) { fail(e); } syncPaint.removing = null; syncPaint.sig = null; syncPaint(true); });
  $$('#sync-slots form').forEach(f => { f.onsubmit = async (ev) => { ev.preventDefault(); const inp = f.querySelector('input'), pin = syncPinOk(inp); if (!pin) return;
    const b = f.querySelector('button'); b.disabled = true; b.textContent = 'Working…';
    try {
      if (f.dataset.remove) { syncInfo = await invoke('sync_remove_card', { pubkey: f.dataset.remove, reader: f.dataset.reader, pin }); syncPaint.removing = null; toast('That key can no longer fetch this profile'); }
      else if (rotating) { syncInfo = await invoke('sync_rotate', { reader: f.dataset.reader, pin }); syncPaint.rotating = false; syncPaint.err = ''; toast('The profile has a new key. Your other devices will ask for one of its keys; add your other keys again here.', 12000); }
      else { syncInfo = await invoke('sync_enable', { reader: f.dataset.reader, pin }); syncPaint.err = ''; toast(moved ? 'This device has the profile’s new key and is syncing again' : on ? 'That key can now bring this profile to a new device' : 'Sync is on. This profile now follows you to your other devices.', 7000); if (moved) await syncArrived(); }
    } catch (e) { inp.value = ''; fail(e); }
    syncPaint.sig = null; syncPaint(true); }; });
}
const syncFace = async () => { try { syncInfo = await invoke('sync_face_refresh', { on: syncInfo.face_refresh === false }); } catch (e) { fail(e); } syncPaint.sig = null; syncPaint(); };
$('#sync-face').onclick = syncFace; $('#sync-face').onkeydown = (e) => { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); syncFace(); } };
$('#sync-rotate').onclick = () => { syncPaint.rotating = !syncPaint.rotating; syncPaint.removing = null; syncPaint.sig = null; syncPaint(); };
$('#sync-now').onclick = async () => { const b = $('#sync-now'); b.disabled = true;
  try { const o = await invoke('sync_now'); syncPaint.err = ''; if (o.pulled) await syncArrived(); toast(o.pulled ? 'Synced: changes from another device are here' : o.pushed ? 'Synced: this device’s changes are on the host' : 'Already the same everywhere'); }
  catch (e) { syncPaint.err = String(e); fail(e); }
  b.disabled = false; syncPaint(true); };
askTwice($('#sync-off'), 'Stop syncing this profile on this device? It stays here as it is, and stays on the host and your other devices. A key can bring it back.', 'Stop syncing here', async () => { syncInfo = await invoke('sync_disable'); syncPaint.sig = null; syncPaint(true); });

// ---------- what arrived from another device ----------
async function syncArrived() {
  const was = vault && vault.name;
  store = await invoke('store_get');
  vault = store.vaults.find(v => v.name === was) || store.vaults[0] || null;
  // the profile's own name and picture may be what arrived
  try { profilesView = await invoke('profiles_list'); if (profilesView && profilesView.current) { profile = profilesView.current; paintProfileChip(); if (typeof renderProfileCard === 'function' && $('#s-settings').classList.contains('on')) renderProfileCard(); } } catch {}
  if (typeof renderTabs === 'function') renderTabs();
  // a profile that was empty a moment ago now has vaults: leave the first-run screen for them
  if (!$('#welcome').hidden && store.vaults.length) { rememberVault(vault); return show('home'); }
  const on = $('.screen.on'); if (!on || $('#shell').hidden) return;
  // a screen with work in progress is left as it is; the others are drawn again with what came
  const name = on.id.slice(2), typing = document.activeElement && /^(INPUT|TEXTAREA)$/.test(document.activeElement.tagName);
  if (!typing && !['setup', 'restore', 'sign', 'send', 'own'].includes(name)) show(vault || ['overview', 'settings', 'places', 'entities', 'allkeys'].includes(name) ? name : 'overview');
}
async function syncTick() {
  if (!store || (lockInfo && lockInfo.sealed && !lockInfo.unlocked) || syncTick.busy) return;
  syncTick.busy = true;
  try { const o = await invoke('sync_tick'); if (o) { syncPaint.err = ''; syncInfo = null; if (o.pulled) await syncArrived(); } }
  catch (e) { syncPaint.err = String(e); syncInfo = null; }
  syncTick.busy = false;
  if ($('#s-settings').classList.contains('on')) syncPaint(!syncInfo);
}
setInterval(syncTick, 15000);
// A phone stops an app's clock while it is in the background. Coming back to the front asks at once, instead of
// leaving the person looking at yesterday's profile until the next tick comes round.
document.addEventListener('visibilitychange', () => { if (!document.hidden) syncTick(); });
window.addEventListener('focus', () => syncTick());
setInterval(() => { if ($('#s-settings').classList.contains('on')) syncPaint(); }, 2000);

// ---------- the profile picker: bring one here ----------
function syncBringWire() {
  const start = $('#pb-start'); if (!start) return;
  const box = $('#profile-bring');
  const look = async () => {
    $('#pb-found').innerHTML = '';
    let list = []; try { list = await invoke('cards_list'); } catch {}
    const keys = list.filter(c => c.pubkey);
    $('#pb-keys').innerHTML = keys.map((c, i) => `<form class="slot add" data-reader="${esc(c.reader)}"><span><b>${esc(modelName(c.form, c.nfc))}</b><small>serial ${c.serial}</small></span><span class="row" style="gap:8px"><input type="password" autocomplete="off" placeholder="Key PIN" id="pb-pin-${i}" aria-label="PIN of the key with serial ${c.serial}"><button class="btn small" type="submit">Look on this key</button></span></form>`).join('')
      || '<div class="muted" style="font-size:13px">No key with a signing key is plugged in. Plug one in and look again.</div>';
    $$('#pb-keys form').forEach(f => { f.onsubmit = async (ev) => { ev.preventDefault(); const inp = f.querySelector('input'), pin = syncPinOk(inp); if (!pin) return;
      const b = f.querySelector('button'); b.disabled = true;
      let found; try { found = await invoke('sync_find', { reader: f.dataset.reader, pin, host: $('#pb-host').value.trim() || null }); } catch (e) { inp.value = ''; b.disabled = false; return fail(e); }
      b.disabled = false;
      $('#pb-found').innerHTML = found.length ? found.map(p => `<div class="slot"><span><b>${esc(p.name)}</b><small>${esc(PCLASS[p.class][0])} · this key was added ${new Date(p.added * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })}</small></span>${p.here ? '<span class="muted" style="font-size:13px">already on this device</span>' : `<button class="btn small primary" type="button" data-bring="${esc(p.id)}">Bring it here</button>`}</div>`).join('')
        : '<div class="muted" style="font-size:13px">This key has not been added to any synced profile.</div>';
      $$('#pb-found [data-bring]').forEach(g => g.onclick = async () => { g.disabled = true; g.textContent = 'Bringing…';
        try { const p = await invoke('sync_adopt', { reader: f.dataset.reader, pin, id: g.dataset.bring, host: $('#pb-host').value.trim() || null }); inp.value = ''; box.hidden = true; await openProfile(p.id); setTimeout(syncTick, 800); }
        catch (e) { g.disabled = false; g.textContent = 'Bring it here'; fail(e); } });
    }; });
  };
  start.onclick = () => { box.hidden = false; look(); };
  $('#pb-look').onclick = look;
  $('#pb-host-show').onclick = () => { $('#pb-host-field').hidden = false; $('#pb-host-show').hidden = true; $('#pb-host').focus(); };
  $('#pb-cancel').onclick = () => { box.hidden = true; $('#pb-keys').innerHTML = ''; $('#pb-found').innerHTML = ''; };
}
syncBringWire();
setTimeout(syncTick, 2500);
