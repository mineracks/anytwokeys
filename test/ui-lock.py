#!/usr/bin/env python3
"""The lock screen and App settings › Lock, in WebKit against a mocked backend.
   usage: <python with playwright> test/ui-lock.py <outdir>      exits non-zero on any failed check"""
import json, sys, pathlib
from playwright.sync_api import sync_playwright
here = pathlib.Path(__file__).resolve().parent; src = here.parent / "src"
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
K = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe"]
MOCK = """
window.__calls = []; window.__args = {};
const S = Object.assign({ sealed: true, unlocked: false, finished: true, touch: true, idle_minutes: 0, with_screen: true, cards: [{ pubkey: '%(k0)s', name: 'laptop', added: 1789000000 }], moved: null }, %(status)s);
window.S = S; window.__cards = %(cards)s; window.__macFails = %(macfails)s;
const V = { name: 'SECRET VAULT NAME', network: 'mainnet', threshold: 2, keys: [{ name: 'laptop', serial: 1, pubkey: '%(k0)s' }, { name: 'off-site', serial: 2, pubkey: '%(k1)s' }], descriptor: 'wsh(…)', address: 'bc1qexample', labels: {}, seal: { shares: [], sealed: [] } };
const store = () => ({ forms: {}, contacts: [], vaults: [V], settings: { network: 'mainnet', custom: false, electrum: '', explorer: '', gap: 20, fiat: 'AUD', prices: '' }, seen: {}, last_psbt: null, last_message: null });
const locked = () => { if (S.sealed && !S.unlocked) throw 'locked: open Any Two Keys with Touch ID or one of your keys first'; };
const H = {
  lock_status: () => ({ ...S }),
  lock_start: () => { S.sealed = true; S.unlocked = true; S.finished = true; S.cards = []; return { ...S, moved: { store: true, cache: true, attachments: 2, left: [] } }; },
  lock_open_mac: () => { if (__macFails) throw 'Authentication was cancelled by the user.'; S.unlocked = true; return { ...S }; },
  lock_open_card: (a) => { if (a.pin !== '123456') throw 'wrong PIN: 2 tries left'; S.unlocked = true; return { ...S }; },
  lock_now: () => { S.unlocked = false; try { sessionStorage.setItem('locked-now', '1'); } catch {} return { ...S }; },
  lock_settings: (a) => { if (a.touch !== undefined && a.touch !== null) S.touch = a.touch; if (a.idleMinutes !== undefined) S.idle_minutes = a.idleMinutes; if (a.withScreen !== undefined) S.with_screen = a.withScreen; return { ...S }; },
  lock_add_card: (a) => { S.cards.push({ pubkey: '%(k1)s', name: 'off-site', added: 1789900000 }); return { ...S }; },
  lock_remove_card: (a) => { S.cards = S.cards.filter(c => c.pubkey !== a.pubkey); return { ...S }; },
  cards_list: () => __cards, store_get: () => { locked(); return store(); }, device_locale: () => ({ locale: 'en_AU', currency: 'AUD' }),
  node_status: () => ({ electrum: 'x', height: 1, ms: 5, own: true }), key_form_seen: () => store(), vault_key_seen: () => store(),
  vault_sync: () => { locked(); return { coins: [], balance: 0, height: 1, rotating: false, receive: 'bc1qexample', receive_index: 0, next_change: 0, history: [] }; },
  vault_activity: () => ({ rows: [], now: null, ccy: 'AUD' }), labels_status: () => ({}), labels_sync: () => ({}), seal_status: () => ({ cards: [], sealed: [] }), spend_status: () => null, spend_ceremony: () => null,
};
window.__TAURI__ = { core: { invoke: async (cmd, args) => { __calls.push(cmd); __args[cmd] = args; if (!H[cmd]) throw 'unmocked: ' + cmd; return H[cmd](args || {}); } }, event: { listen: async () => () => {} } };
"""
card = lambda i, r: dict(reader=r, serial=100 + i, pubkey=K[i], form=1, nfc=True, firmware="5.7.1", pin_tries=3, admin_tries=3, touch="on")
fails = 0
def check(name, ok, detail=""):
    global fails
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else f" — {detail}"))
    if not ok: fails += 1
def page(b, **kw):
    pg = b.new_page(viewport=dict(width=1180, height=820))
    pg.add_init_script(MOCK % dict(k0=K[0], k1=K[1], status=json.dumps(kw.get("status", {})), cards=json.dumps(kw.get("cards", [])), macfails=json.dumps(kw.get("macfails", False))))
    pg.goto((src / "index.html").as_uri()); pg.wait_for_timeout(700); return pg
with sync_playwright() as p:
    b = p.webkit.launch()
    # ---- locked, Touch ID cancelled, a key that can open it and one that cannot ----
    pg = page(b, macfails=True, cards=[card(0, "Yubico YubiKey 0"), card(1, "Yubico YubiKey 1")])
    check("locked: the lock covers the window and the store was never asked for", pg.locator("#lock").is_visible() and "store_get" not in pg.evaluate("__calls") and "SECRET VAULT NAME" not in pg.inner_text("body"), pg.evaluate("__calls"))
    check("Touch ID is asked for by itself, and a cancel is said in the system's words without unlocking", "lock_open_mac" in pg.evaluate("__calls") and "cancelled" in pg.inner_text("#lock-err") and pg.locator("#lock").is_visible())
    rows = pg.locator("#lock-keys > *")
    check("a plugged-in key that was added gets a PIN field; one that was not says so and gets none", rows.count() == 2 and "laptop" in rows.nth(0).inner_text() and rows.nth(0).locator("input").count() == 1 and "not been added" in rows.nth(1).inner_text() and rows.nth(1).locator("input").count() == 0, pg.inner_text("#lock-keys"))
    pg.locator("#lock").screenshot(path=str(out / "lock.png"))
    pg.fill("#lock-keys input", "cccccbcgtrdjkbhivnnfgrlnhjrcdtbjvjtlhurkdhtd"); pg.keyboard.press("Enter"); pg.wait_for_timeout(200)
    check("a key typing a one-time code is not sent to the key as a PIN", "lock_open_card" not in pg.evaluate("__calls") and "one-time code" in pg.inner_text("#lock-err") and pg.input_value("#lock-keys input") == "")
    pg.fill("#lock-keys input", "999999"); pg.keyboard.press("Enter"); pg.wait_for_timeout(300)
    check("a wrong PIN says how many tries are left, clears the field and stays locked", "2 tries left" in pg.inner_text("#lock-err") and pg.input_value("#lock-keys input") == "" and pg.locator("#lock").is_visible())
    pg.wait_for_timeout(2300); pg.fill("#lock-keys input", "1234")
    pg.wait_for_timeout(2300); check("the list of keys is not redrawn under a PIN being typed", pg.input_value("#lock-keys input") == "1234")
    pg.fill("#lock-keys input", "123456"); pg.keyboard.press("Enter"); pg.wait_for_timeout(900)
    check("the right PIN, with Enter, opens the app on the vault", not pg.locator("#lock").is_visible() and pg.evaluate("__args.lock_open_card.reader") == "Yubico YubiKey 0" and "SECRET VAULT NAME" in pg.inner_text(".side"))
    # ---- App settings › Lock ----
    pg.evaluate("show('settings')"); pg.wait_for_timeout(2400)
    check("Lock says where the key is and which keys open the app", "Keychain" in pg.inner_text("#lock-state") and "one of your keys" in pg.inner_text("#lock-state") and "laptop" in pg.inner_text("#lock-slots"))
    check("the other plugged-in key is offered, with its vault name", "off-site" in pg.inner_text("#lock-slots form") and pg.locator("#lock-slots form input").count() == 1)
    pg.locator("#lock-card").screenshot(path=str(out / "lock-settings.png"))
    pg.click("#lock-touch"); pg.wait_for_timeout(200); check("the Touch ID switch turns off", pg.evaluate("__args.lock_settings.touch") is False and pg.get_attribute("#lock-touch", "aria-checked") == "false")
    pg.click("#lock-screen"); pg.wait_for_timeout(200); check("locking with the Mac's screen is on by default and can be turned off", pg.evaluate("S.with_screen") is False and pg.get_attribute("#lock-screen", "aria-checked") == "false")
    pg.select_option("#lock-idle", "15"); pg.wait_for_timeout(200); check("and the idle lock is set", pg.evaluate("S.idle_minutes") == 15)
    pg.fill("#lock-slots form input", "123456"); pg.keyboard.press("Enter"); pg.wait_for_timeout(400)
    check("a second key is added with its PIN", pg.locator("#lock-slots .slot:not(.add)").count() == 2 and "2 of your keys" in pg.inner_text("#lock-state"))
    pg.click("[data-slot-x='%s']" % K[1]); pg.wait_for_timeout(150); check("removing a key asks first", pg.locator("#lock-slots .slot:not(.add)").count() == 2 and "no longer open" in pg.inner_text("#lock-slots"))
    pg.click("[data-slot-x='%s']" % K[1]); pg.wait_for_timeout(300); check("and then removes it", pg.locator("#lock-slots .slot:not(.add)").count() == 1)
    pg.click("#lock-now"); pg.wait_for_timeout(900)
    check("Lock now locks and the window forgets what it knew", pg.evaluate("sessionStorage.getItem('locked-now')") == "1" and pg.evaluate("typeof store === 'undefined' || !store || !store.vaults"), "")
    # ---- this Mac opens it ----
    pg2 = page(b); check("with the Mac's say-so the app opens by itself", not pg2.locator("#lock").is_visible() and "SECRET VAULT NAME" in pg2.inner_text(".side") and pg2.evaluate("__calls.indexOf('lock_open_mac') < __calls.indexOf('store_get')"))
    # ---- the first run of a build that seals ----
    pg3 = page(b, status=dict(sealed=False, unlocked=False, finished=False, cards=[]))
    check("first run: the files are sealed without a lock screen, and the app says so and what to do next", "lock_start" in pg3.evaluate("__calls") and not pg3.locator("#lock").is_visible() and "now encrypted" in pg3.inner_text("#notices") and "2 attachments" in pg3.inner_text("#notices") and "Add one of your keys" in pg3.inner_text("#notices"), pg3.inner_text("#notices"))
    pg3.evaluate("show('settings')"); pg3.wait_for_timeout(300); check("and Lock warns while no key can open it", "No key of yours" in pg3.inner_text("#lock-state") and "warn" in pg3.get_attribute("#lock-state", "class"))
    # ---- no keys added, Touch ID refused ----
    pg4 = page(b, macfails=True, status=dict(cards=[])); check("locked with no key added: says so plainly", "No key has been added yet" in pg4.inner_text("#lock-keys"))
    b.close()
print(f"{fails} failed"); sys.exit(1 if fails else 0)
