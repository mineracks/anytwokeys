#!/usr/bin/env python3
"""Profile sync in the window: App settings › Sync, and bringing a profile here from the picker. WebKit, mocked backend.
   usage: <python with playwright> test/ui-sync.py <outdir>      exits non-zero on any failed check"""
import json, sys, pathlib
from playwright.sync_api import sync_playwright
here = pathlib.Path(__file__).resolve().parent; src = here.parent / "src"
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
K = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe"]
MOCK = """
window.__calls = []; window.__args = {};
document.hasFocus = () => true;
const CFG = %(cfg)s;
const V = (name) => ({ name, network: 'mainnet', threshold: 2, keys: [{ name: 'laptop', serial: 1, pubkey: '%(k0)s' }, { name: 'off-site', serial: 2, pubkey: '%(k1)s' }], descriptor: 'wsh(' + name + ')', address: 'bc1qexample', labels: {}, seal: { shares: [], sealed: [] } });
window.__vaults = CFG.vaults.map(V);
window.SYNC = { on: CFG.on, url: 'https://vss.example.org/vss', last: CFG.on ? 1791000000 : 0, cards: CFG.on ? [{ pubkey: '%(k0)s', name: 'laptop', added: 1790000000 }] : [], waiting: false };
window.__tick = null; window.__profiles = CFG.profiles; window.__current = CFG.current;
const store = () => ({ forms: {}, contacts: [], vaults: __vaults, key_ready: {}, key_skip: [], key_begun: [], settings: { network: 'mainnet', custom: false, electrum: '', explorer: '', gap: 20, fiat: 'AUD', prices: '', vss: 'https://vss.example.org/vss' }, seen: {}, last_psbt: null, last_message: null });
const card = (i) => ({ reader: 'Yubico YubiKey ' + i, serial: 100 + i, version: 'OpenPGP 3.4', pubkey: [ '%(k0)s', '%(k1)s' ][i], touch: 'FIXED', form: 3, nfc: true, firmware: '5.7.1' });
const prof = (id, name, extra) => Object.assign({ id, name, class: 'work', picture: null, created: 1, last_opened: 0, start: false, discreet: false, persona_seed: null, keychain_account: 'profile ' + id, sync_id: null }, extra || {});
const H = {
  profiles_list: () => ({ profiles: __profiles.map(p => prof(p.id, p.name, p)), current: __current ? prof(__current.id, __current.name, __current) : null, moved: { moved: false, files: 0, id: null }, displays: { count: 1, mirrored: false, external: false } }),
  profile_open: (a) => { __current = __profiles.find(p => p.id === a.id); return H.lock_status(); },
  lock_status: () => ({ sealed: true, unlocked: true, finished: true, touch: false, idle_minutes: 0, with_screen: true, cards: [], moved: null }), lock_open_mac: () => H.lock_status(),
  cards_list: () => CFG.cards.map(card), cards_other: () => [], card_survey: () => [], store_get: () => store(), device_locale: () => ({ locale: 'en_AU', currency: 'AUD' }), platform: () => 'macos',
  node_status: () => ({ electrum: 'x', height: 1, ms: 5, own: true }), key_form_seen: () => store(), vault_key_seen: () => store(), guard_displays: () => ({ count: 1, mirrored: false, external: false }), guard_sharing: () => null,
  vault_sync: () => ({ coins: [], balance: 0, height: 1, rotating: false, receive: 'bc1qexample', receive_index: 0, next_change: 0, gap: 20, servers: { electrum: 'ssl://x:50002', explorer: '' }, history: [] }),
  vault_activity: () => ({ rows: [], now: null, ccy: 'AUD' }), labels_status: () => ({}), labels_sync: () => ({}), seal_status: () => ({ cards: [], sealed: [] }), spend_status: () => null, spend_ceremony: () => null,
  sync_status: () => ({ ...SYNC }),
  sync_enable: (a) => { if (a.pin !== '493817') throw 'wrong PIN, 2 tries left'; const c = CFG.cards.map(card).find(c => c.reader === a.reader); SYNC.on = true; SYNC.last = 1791000100; SYNC.cards = SYNC.cards.filter(k => k.pubkey !== c.pubkey).concat([{ pubkey: c.pubkey, name: c.pubkey === '%(k0)s' ? 'laptop' : 'off-site', added: 1791000100 }]); return { ...SYNC }; },
  sync_now: () => { SYNC.last = 1791000200; return { pulled: false, pushed: 1, last: SYNC.last }; },
  sync_tick: () => { const t = __tick; __tick = null; if (t && t.vaults) { __vaults = t.vaults.map(V); } if (t && t.error) throw t.error; return t ? { pulled: !!t.vaults, pushed: 0, last: 1791000300 } : null; },
  sync_disable: () => { SYNC.on = false; SYNC.cards = []; return { ...SYNC }; },
  sync_remove_card: (a) => { if (a.reader && a.pin !== '493817') throw 'wrong PIN, 2 tries left'; window.__removed = a; SYNC.cards = SYNC.cards.filter(k => k.pubkey !== a.pubkey); return { ...SYNC }; },
  sync_rotate: (a) => { if (a.pin !== '493817') throw 'wrong PIN, 2 tries left'; const c = CFG.cards.map(card).find(c => c.reader === a.reader); SYNC.cards = SYNC.cards.map(k => ({ ...k, stale: k.pubkey !== c.pubkey })); window.__rotated = a.reader; return { ...SYNC }; },
  sync_find: (a) => { if (a.pin !== '493817') throw 'wrong PIN, 2 tries left'; return CFG.found; },
  sync_adopt: (a) => { const f = CFG.found.find(p => p.id === a.id); const p = { id: 'new1', name: f.name, class: f.class, sync_id: f.id }; __profiles.push(p); __vaults = []; return prof(p.id, p.name, p); },
};
window.__TAURI__ = { core: { invoke: async (cmd, args) => { __calls.push(cmd); __args[cmd] = args; if (!H[cmd]) throw 'unmocked: ' + cmd; return H[cmd](args || {}); } }, event: { listen: async () => () => {} } };
"""
fails = 0
def check(name, ok, detail=""):
    global fails
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else f" — {detail}"))
    if not ok: fails += 1
def page(b, **cfg):
    base = dict(on=False, vaults=["Household"], cards=[0], profiles=[dict(id="p1", name="Bench")], current=dict(id="p1", name="Bench"), found=[])
    base.update(cfg)
    pg = b.new_page(viewport=dict(width=1180, height=900))
    pg.add_init_script(MOCK % dict(k0=K[0], k1=K[1], cfg=json.dumps(base)))
    pg.goto((src / "index.html").as_uri()); pg.wait_for_timeout(900); return pg
called = lambda pg, cmd: cmd in pg.evaluate("__calls")
with sync_playwright() as p:
    b = p.webkit.launch()
    # ---- off: switch it on with a key ----
    pg = page(b); pg.evaluate("show('settings')"); pg.wait_for_selector("#sync-card:not([hidden])", timeout=6000); pg.wait_for_timeout(2300)
    t = pg.inner_text("#sync-card")
    check("off, the card says the profile lives here only, what would travel and what never does", "this device only" in t and "Settings stay per device" in t and "nothing that can spend" in t and pg.locator("#sync-acts").is_hidden(), t)
    check("the plugged-in key is offered by its name in the vault, with a PIN field", "laptop" in pg.inner_text("#sync-slots form") and "Switch sync on" in pg.inner_text("#sync-slots form"))
    pg.locator("#sync-card").screenshot(path=str(out / "sync-off.png"))
    pg.fill("#sync-slots form input", "4938"); pg.wait_for_timeout(4300)
    check("a PIN being typed survives the polls", pg.input_value("#sync-slots form input") == "4938")
    pg.fill("#sync-slots form input", "cccccbcgtrdjkbhivnnfgrlnhjrcdtbjvjtlhurkdhtd"); pg.keyboard.press("Enter"); pg.wait_for_timeout(200)
    check("a key typing a one-time code is not sent on as a PIN", not called(pg, "sync_enable") and pg.input_value("#sync-slots form input") == "")
    pg.fill("#sync-slots form input", "493817"); pg.keyboard.press("Enter"); pg.wait_for_timeout(500)
    t = pg.inner_text("#sync-card")
    check("with the key's PIN sync is on: the host is named, the key is listed, and the PIN is gone from the page", "vss.example.org" in t and "Last synced" in t and pg.locator("#sync-slots .slot:not(.add)").count() == 1 and pg.locator("#sync-slots input").count() == 0 and pg.locator("#sync-acts").is_visible(), t)
    pg.locator("#sync-card").screenshot(path=str(out / "sync-on.png"))
    pg.click("#sync-now"); pg.wait_for_timeout(300); check("Sync now says what happened", "changes are on the host" in pg.inner_text("#toast"))
    # ---- something arrives from another device ----
    pg.evaluate("show('home'); window.__tick = { vaults: ['Household', 'Holiday fund'] }; syncTick()"); pg.wait_for_timeout(600)
    check("a vault made on another device appears in the sidebar without a restart", "Holiday fund" in pg.inner_text(".side") and pg.locator("#s-home.on").count() == 1, pg.inner_text(".side")[:300])
    pg.evaluate("show('settings'); window.__tick = { error: 'sync host: unreachable' }; syncTick()"); pg.wait_for_timeout(2600)
    check("a failed try is said on the card, not thrown at the person", "did not go through" in pg.inner_text("#sync-state") and "unreachable" in pg.inner_text("#sync-state"), pg.inner_text("#sync-state"))
    pg.click("#sync-off"); pg.wait_for_timeout(150); check("stopping asks first", pg.evaluate("SYNC.on") and "stays on the host" in pg.inner_text("#sync-card"))
    pg.click("#sync-off"); pg.wait_for_timeout(400); check("and then stops on this device only", not pg.evaluate("SYNC.on") and "this device only" in pg.inner_text("#sync-state"))
    # ---- on already, a second key plugged in ----
    pg = page(b, on=True, cards=[0, 1]); pg.evaluate("show('settings')"); pg.wait_for_selector("#sync-slots form", timeout=8000)
    check("a second key is offered to be added, the first is listed as enrolled", "off-site" in pg.inner_text("#sync-slots form") and "Add this key" in pg.inner_text("#sync-slots form") and "laptop" in pg.inner_text("#sync-slots .slot:not(.add)"))
    pg.fill("#sync-slots form input", "493817"); pg.keyboard.press("Enter"); pg.wait_for_timeout(500)
    check("and after its PIN both can bring the profile to a new device", pg.locator("#sync-slots .slot:not(.add)").count() == 2 and pg.evaluate("__args.sync_enable.reader") == "Yubico YubiKey 1")
    # ---- removing a key, replacing the profile's key, and a device left behind by a replacement ----
    check("each enrolled key can be removed", pg.locator("#sync-slots [data-sync-x]").count() == 2)
    pg.locator("#sync-slots [data-sync-x]").nth(1).click(); pg.wait_for_timeout(200)
    check("removing a plugged-in key asks for its PIN, because the profile is taken off the key itself", pg.locator("#sync-slots form[data-remove] input").count() == 1 and "off the key itself" in pg.inner_text("#sync-slots"))
    pg.fill("#sync-slots form[data-remove] input", "493817"); pg.keyboard.press("Enter"); pg.wait_for_timeout(400)
    check("and with it the key is gone from the list and from its own pointer", pg.evaluate("__removed.reader") == "Yubico YubiKey 1" and pg.locator("#sync-slots .slot:not(.add)").count() == 1)
    pg.click("#sync-rotate"); pg.wait_for_timeout(200)
    t = pg.inner_text("#sync-card")
    check("replacing the key says what it does, what it cannot do, and what is needed afterwards", "keeps what it already had" in t and "asks for a key once" in t and "added again" in t and pg.locator("#sync-slots form").count() == 2 and "Replace the key" in pg.inner_text("#sync-slots"), t)
    pg.locator("#sync-card").screenshot(path=str(out / "sync-rotate.png"))
    pg.fill("#sync-slots form input >> nth=0", "493817"); pg.locator("#sync-slots form").nth(0).locator("button").click(); pg.wait_for_timeout(500)
    check("with a key's PIN the profile's key is replaced, and nothing was replaced before that", pg.evaluate("__rotated") == "Yubico YubiKey 0" and "Replace this profile" not in pg.inner_text("#sync-state"))
    pg.evaluate("window.__tick = { error: 'this profile\u2019s key was replaced on another device. Plug in one of its keys and enter its PIN to carry on.' }; syncTick()"); pg.wait_for_timeout(2600)
    t = pg.inner_text("#sync-card")
    check("a device left behind by a replacement says so, says nothing is lost, and asks for a key", "stopped syncing" in t and "Nothing here is lost" in t and "Carry on with this key" in t and pg.locator("#sync-acts").is_hidden(), t)
    pg.locator("#sync-card").screenshot(path=str(out / "sync-moved.png"))
    pg.fill("#sync-slots form input >> nth=0", "493817"); pg.locator("#sync-slots form").nth(0).locator("button").click(); pg.wait_for_timeout(500)
    check("and carries on once it has been shown one", "Last synced" in pg.inner_text("#sync-state") and pg.locator("#sync-acts").is_visible(), pg.inner_text("#sync-state"))
    # ---- a blank device: bring a profile with a key ----
    found = [dict(id="s1", name="Household money", **{"class": "private"}, added=1790000000, here=False), dict(id="s2", name="Bench", **{"class": "work"}, added=1790000000, here=True)]
    pg = page(b, current=None, vaults=[], found=found)
    check("the picker offers to bring a profile from another device", pg.locator("#profiles").is_visible() and pg.locator("#pb-start").is_visible())
    pg.click("#pb-start"); pg.wait_for_timeout(300)
    check("the plugged-in key is listed with a PIN field, and nothing has been asked of it", pg.locator("#pb-keys form").count() == 1 and "serial 100" in pg.inner_text("#pb-keys") and not called(pg, "sync_find"))
    pg.fill("#pb-keys input", "111111"); pg.keyboard.press("Enter"); pg.wait_for_timeout(300)
    check("a wrong PIN says how many tries are left and clears the field", "2 tries left" in pg.inner_text("#toast") and pg.input_value("#pb-keys input") == "")
    pg.fill("#pb-keys input", "493817"); pg.keyboard.press("Enter"); pg.wait_for_timeout(300)
    t = pg.inner_text("#pb-found")
    check("the key's profiles are listed; one already here is said to be here and cannot be brought twice", "Household money" in t and "already on this device" in t and pg.locator("#pb-found [data-bring]").count() == 1, t)
    pg.locator("#profile-bring").screenshot(path=str(out / "bring.png"))
    pg.evaluate("window.__tick = { vaults: ['Household'] }"); pg.click("#pb-found [data-bring]"); pg.wait_for_timeout(2200)
    check("bringing it opens the new profile and its vaults arrive", pg.evaluate("__args.sync_adopt.id") == "s1" and not pg.locator("#profiles").is_visible() and "Household" in pg.inner_text(".side") and pg.locator("#s-home.on").count() == 1, pg.inner_text("body")[:300])
    b.close()
print(f"{fails} failed"); sys.exit(1 if fails else 0)
