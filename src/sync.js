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
  const plugged = (cards || []).filter(c => c.pubkey && !enrolled.some(k => k.pubkey === c.pubkey));
  // rebuilt only when what it shows changes: a poll must not wipe a PIN being typed
  const sig = [on, syncInfo.last, syncInfo.files_waiting, syncInfo.url, enrolled.map(k => k.pubkey).join(','), plugged.map(c => c.reader + c.pubkey).join(','), syncPaint.err || ''].join('|');
  if (sig === syncPaint.sig) return; syncPaint.sig = sig;
  let host = ''; try { host = new URL(syncInfo.url).host; } catch {}
  $('#sync-state').innerHTML = on
    ? `This profile is kept the same on every device you bring it to. It is encrypted before it leaves here; ${esc(host)} holds what it cannot read. Last synced ${esc(syncWhen(syncInfo.last))}.${syncInfo.files_waiting ? ` ${syncInfo.files_waiting} file${syncInfo.files_waiting === 1 ? ' is' : 's are'} still being sent.` : ''}${syncPaint.err ? ` <span class="warn">The last try did not go through: ${esc(syncPaint.err)}</span>` : ''}`
    : 'This profile lives on this device only. Switch sync on with one of your keys and its vaults, labels, contacts and places follow you to your other devices: encrypted here, held by your sync host, opened only by your keys. Settings stay per device, and nothing that can spend ever leaves the keys.';
  $('#sync-keys-cap').textContent = on ? 'Keys that can bring this profile to a new device' : 'Switch it on with a key';
  const nameOf = (c) => { for (const v of store.vaults) { const k = v.keys.find(k => k.pubkey === c.pubkey); if (k) return k.name; } const r = (store.key_ready || {})[c.pubkey]; return (r && r.name) || 'key ' + (c.serial || ''); };
  $('#sync-slots').innerHTML = enrolled.map(k => `<div class="slot"><span><b>${esc(k.name)}</b><small>added ${new Date(k.added * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })}</small></span></div>`).join('')
    + plugged.map((c, i) => `<form class="slot add" data-reader="${esc(c.reader)}"><span><b>${esc(nameOf(c))}</b><small>plugged in</small></span><span class="row" style="gap:8px"><input type="password" autocomplete="off" placeholder="Key PIN" id="sync-pin-${i}" aria-label="PIN of ${esc(nameOf(c))}"><button class="btn small" type="submit">${on ? 'Add this key' : 'Switch sync on'}</button></span></form>`).join('')
    + (!plugged.length ? `<div class="muted" style="font-size:13px">${on ? 'Plug in another key to add it. Any one of them, with its PIN, brings the profile to a new device.' : 'Plug in a key that has a signing key on it.'}</div>` : '');
  $('#sync-acts').hidden = !on;
  $$('#sync-slots form').forEach(f => { f.onsubmit = async (ev) => { ev.preventDefault(); const inp = f.querySelector('input'), pin = syncPinOk(inp); if (!pin) return;
    const b = f.querySelector('button'); b.disabled = true; b.textContent = 'Working…';
    try { syncInfo = await invoke('sync_enable', { reader: f.dataset.reader, pin }); syncPaint.err = ''; toast(on ? 'That key can now bring this profile to a new device' : 'Sync is on. This profile now follows you to your other devices.', 7000); }
    catch (e) { inp.value = ''; fail(e); }
    syncPaint.sig = null; syncPaint(true); }; });
}
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
