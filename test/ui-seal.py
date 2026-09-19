#!/usr/bin/env python3
"""Drive the seal UI in WebKit (the Tauri webview's engine) against a mocked backend.
   usage: <python with playwright> test/ui-seal.py <outdir>      exits non-zero on any failed check"""
import json, os, sys, pathlib
from playwright.sync_api import sync_playwright
src = pathlib.Path(__file__).resolve().parent.parent / "src"
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
K = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe", "038410f240613b33fe73d968f331ca82eb1a2b43ddb2ad6c4a27b2f6d0c1e2a3b4"]
vault = dict(name="family vault", network="signet", threshold=2, keys=[dict(name=n, serial=s, pubkey=k) for n, s, k in zip(["laptop", "keyring", "off-site"], [27446043, 25944203, 26731747], K)],
             descriptor="wsh(sortedmulti(2,%s))#abcdefgh" % ",".join(K), address="tb1q2n7mza35dzyea6du9dh4amzqxjg6ysgtc45kfklfd7revryppuxqfegkkp", created=0, chain_code="ab" * 32, labels={}, seal=dict(shares=[], sealed=[]))
MOCK = """
window.__calls = [];
window.__state = { cards: %(cards)s, sealed: [], found: [] };
const store = () => ({ vaults: [%(vault)s], settings: { network: 'signet', custom: false, electrum: '', explorer: '', gap: 20 }, seen: {} });
window.__forgot = 0;
const H = { vault_rename: (a) => { window.__renamed = a.keyNames[1]; const st = store(); st.vaults[0].keys[1].name = a.keyNames[1]; return st; }, vault_forget: () => { __forgot++; return { vaults: [], settings: { network: 'signet', custom: false, electrum: '', explorer: '', gap: 20 }, seen: {} }; },
  store_get: store, cards_list: () => { if (!__state.cards.length) throw 'no YubiKey found'; return __state.cards; },
  vault_recovery: () => 'wsh(sortedmulti(2,…))#abcdefgh cc=abab', vault_sync: () => { throw 'offline in the harness'; },
  seal_status: () => ({ id: 'deadbeef', threshold: 2, earlier: false, sealed: __state.sealed.length,
    keys: store().vaults[0].keys.map(k => ({ name: k.name, pubkey: k.pubkey, here: __state.cards.some(c => c.pubkey === k.pubkey), state: __state.sealed.includes(k.pubkey) ? 'sealed' : 'unsealed' })) }),
  seal_write: (a) => { if (a.adminPin !== '12345678') throw 'wrong PIN, 2 tries left'; const c = __state.cards.find(c => c.reader === a.reader); __state.sealed.push(c.pubkey); return 'sealed, 172 bytes on the key'; },
  seal_read: () => ({ cards: __state.cards.length, foreign: [], found: __state.cards.filter(c => __state.sealedCards.includes(c.pubkey)).map(c => ({ pubkey: c.pubkey, serial: c.serial, entry: { id: 'deadbeef', k: 2, x: %(keys)s.indexOf(c.pubkey) + 1, share: 'aa' } })) }),
  seal_preview: (a) => [a.found.length >= 2 ? { id: 'deadbeef', have: a.found.length, need: 2, known_as: null, address: 'tb1qexample', line: 'wsh(sortedmulti(2,…)) cc=abab',
      record: { name: 'family vault', threshold: 2, network: 'signet', pubkeys: %(keys)s, chain_code: 'abab', key_names: ['laptop', 'keyring', 'off-site'] } } : { id: 'deadbeef', have: a.found.length, need: 2, record: null, line: null, address: null, known_as: null }],
  seal_restore: (a) => { __state.restored = a; return store().vaults[0]; },
};
window.__TAURI__ = { core: { invoke: async (cmd, args) => { __calls.push(cmd); if (!H[cmd]) throw 'unmocked: ' + cmd; return H[cmd](args || {}); } }, event: { listen: async () => () => {} } };
"""
card = lambda i: dict(reader=f"Yubico YubiKey {i}", serial=vault["keys"][i]["serial"], version="OpenPGP 3.4", pubkey=K[i], touch="FIXED")
fails = []
def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + (f" — {detail}" if detail and not ok else "")); 
    if not ok: fails.append(name)

with sync_playwright() as p:
    b = p.webkit.launch()
    # ---- Keys screen: seal one card ----
    pg = b.new_page(viewport=dict(width=1180, height=900)); errs = []
    pg.on("pageerror", lambda e: errs.append(str(e)))
    pg.add_init_script(MOCK % dict(cards=json.dumps([card(0)]), vault=json.dumps(vault), keys=json.dumps(K)))
    pg.goto((src / "index.html").as_uri()); pg.wait_for_timeout(600)
    pg.evaluate("show('keys')"); pg.wait_for_timeout(500)
    rows = pg.locator("#keys-list .keyrow")
    check("three seal rows", rows.count() == 3, str(rows.count()))
    check("plugged-in key offers a PIN field, absent keys do not", pg.locator("#keys-list [data-seal-pin]").count() == 1)
    check("note explains sealing", "admin PIN" in pg.inner_text("#seal-note"))
    pg.fill("#keys-list [data-seal-pin]", "1234"); pg.wait_for_timeout(2600)   # a card poll happens meanwhile
    check("a poll does not wipe a half-typed PIN", pg.input_value("#keys-list [data-seal-pin]") == "1234")
    pg.click("[data-seal-go]"); pg.wait_for_timeout(400)
    check("wrong PIN is reported", "wrong PIN" in pg.locator("#toast").inner_text(), pg.locator("#toast").inner_text())
    pg.fill("#keys-list [data-seal-pin]", "12345678"); pg.click("[data-seal-go]"); pg.wait_for_timeout(500)
    check("the row's seal slot turns to sealed, its PIN field gone", "sealed" in rows.nth(0).locator(".kseal").inner_text() and rows.nth(0).locator("[data-seal-pin]").count() == 0)
    check("note counts 1 of 3", "1 of 3" in pg.inner_text("#seal-note"), pg.inner_text("#seal-note"))
    # nothing overflows or collapses: each row is one line high-ish and inside the card
    boxes = [rows.nth(i).bounding_box() for i in range(3)]; cardbox = pg.locator("#keys-list").bounding_box()
    check("rows sit inside the card at sane heights", all(40 <= bx["height"] <= 110 and bx["x"] >= cardbox["x"] and bx["x"] + bx["width"] <= cardbox["x"] + cardbox["width"] + 1 for bx in boxes), str(boxes))
    pg.screenshot(path=str(out / "keys-seal.png"), full_page=True)
    check("ONE list: a row per key, no second or third card repeating them", pg.locator("#s-keys .keyrow").count() == 3 and pg.locator("#seal-list").count() == 0 and "Plugged in now" not in pg.inner_text("#s-keys"))
    r0 = rows.nth(0).inner_text()
    check("the plugged-in key's row says what it is, that it is in, and that it is sealed", "YubiKey" in r0 and "plugged in" in r0 and "sealed" in r0 and "firmware" not in rows.nth(1).inner_text(), r0)
    ab, cb = rows.nth(0).locator(".keyart svg").bounding_box(), pg.locator("#keys-list").bounding_box()
    check("its drawing is inside the list and on top", ab and ab["x"] >= cb["x"] and ab["width"] > 10 and pg.evaluate("() => { const r = document.querySelector('#keys-list .keyart svg').getBoundingClientRect(); const e = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2); return !!e && !!e.closest('.keyart'); }"), str(ab))
    check("a key that is away and unconfirmed says so without claiming it is unsealed", "seal not confirmed" in rows.nth(1).inner_text())
    pg.click("#keys-list [data-edit-idx='1']"); pg.keyboard.type("spare"); pg.keyboard.press("Enter"); pg.wait_for_timeout(300)
    check("renaming from the pencil still works with PIN fields in the list", pg.evaluate("window.__renamed") == "spare" or "spare" in pg.inner_text("#keys-list"), pg.inner_text("#keys-list")[:120])
    pg.evaluate("__state.cards = __state.cards.concat([{ reader: 'z', serial: 999, version: 'OpenPGP 3.4', pubkey: '02' + 'cd'.repeat(32), touch: 'FIXED', form: 1, nfc: true, firmware: '5.7.1' }])"); pg.wait_for_timeout(2600)
    check("a key that is not this vault's is named once, under the list", "Also plugged in: YubiKey 5 NFC · serial 999" in pg.inner_text("#keys-now") and pg.locator("#s-keys .keyrow").count() == 3, pg.inner_text("#keys-now"))
    pg.locator("#s-keys .card").first.screenshot(path=str(out / "keys-one-list.png"))
    # ---- a destructive button at the very bottom of the page, in a short window: the question must not sit on top of it ----
    pg.set_viewport_size(dict(width=1180, height=620)); pg.evaluate("window.confirm = () => false")
    pg.locator("#forget-vault").scroll_into_view_if_needed(); pg.click("#forget-vault"); pg.wait_for_timeout(700)
    hit = pg.evaluate("() => { const b = document.querySelector('#forget-vault'), r = b.getBoundingClientRect(); const e = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2); return { armed: b.textContent, onTop: e === b, inView: r.top >= 0 && r.bottom <= innerHeight, toast: /Forget/.test(document.querySelector('#toast').textContent) }; }")
    check("Forget: first press arms it, and the armed button is in view with NOTHING on top of it", hit["armed"] == "Yes, forget it here" and hit["onTop"] and hit["inView"] and not hit["toast"], str(hit))
    look = pg.evaluate("() => { const b = document.querySelector('#forget-vault'), s = getComputedStyle(b); return { color: s.color, bg: s.backgroundColor, text: b.innerText.trim() }; }")
    check("the armed label can be READ: its colour is not its background", look["color"] != look["bg"] and look["color"] == "rgb(255, 255, 255)" and look["text"] == "Yes, forget it here", str(look))
    pg.locator("#forget-vault").screenshot(path=str(out / "armed.png"))
    check("the question is written beside the button, with a way out", "Forget this vault on this computer?" in pg.inner_text(".asknote") and pg.locator(".asknote button").is_visible() and pg.evaluate("__forgot") == 0)
    pg.click(".asknote button"); check("Keep it: disarmed, nothing forgotten", pg.inner_text("#forget-vault") != "Yes, forget it here" and pg.locator(".asknote").count() == 0 and pg.evaluate("__forgot") == 0)
    pg.click("#forget-vault"); pg.wait_for_timeout(500); pg.click("#forget-vault"); pg.wait_for_timeout(400)
    check("second press forgets", pg.evaluate("__forgot") == 1)
    check("no page errors on Keys", not errs, "; ".join(errs))
    # ---- Restore: one port, two cards in turn ----
    pg2 = b.new_page(viewport=dict(width=1180, height=900)); errs2 = []
    pg2.on("pageerror", lambda e: errs2.append(str(e)))
    pg2.add_init_script(MOCK % dict(cards=json.dumps([card(1)]), vault=json.dumps(vault), keys=json.dumps(K)))
    pg2.goto((src / "index.html").as_uri()); pg2.wait_for_timeout(600)
    pg2.evaluate("__state.sealedCards = %s; show('restore')" % json.dumps(K)); pg2.wait_for_timeout(2600)
    check("keys tab is the default and the camera stays off", pg2.locator("#tab-keys").is_visible() and not pg2.locator("#tab-scan").is_visible())
    check("one key: says one more is needed", "1 of the 2" in pg2.locator("#restore-card").inner_text(), pg2.locator("#restore-card").inner_text())
    pg2.evaluate("__state.cards = %s" % json.dumps([card(2)])); pg2.wait_for_timeout(2600)
    txt = pg2.locator("#restore-card").inner_text()
    check("second key swapped in: vault rebuilt", "Rebuilt from 2 keys" in txt and "family vault" in txt, txt)
    check("first key is remembered as unplugged", "unplugged, remembered" in pg2.locator("#seal-read-list").inner_text())
    pg2.screenshot(path=str(out / "restore-keys.png"), full_page=True)
    pg2.click("[data-seal-restore]"); pg2.wait_for_timeout(500)
    sent = pg2.evaluate("__state.restored")
    check("restore sends both shares and the name", sent and len(sent["found"]) == 2 and sent["name"] == "family vault", json.dumps(sent)[:200])
    pg2.evaluate("show('restore'); restoreTab('paste')"); pg2.wait_for_timeout(200)
    check("paste tab still works", pg2.locator("#desc-in").is_visible() and not pg2.locator("#tab-keys").is_visible())
    check("no page errors on Restore", not errs2, "; ".join(errs2))
    b.close()
sys.exit(1 if fails else 0)
