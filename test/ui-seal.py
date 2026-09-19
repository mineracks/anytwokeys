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
const H = {
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
    rows = pg.locator("#seal-list .keyrow")
    check("three seal rows", rows.count() == 3, str(rows.count()))
    check("plugged-in key offers a PIN field, absent keys do not", pg.locator("#seal-list input").count() == 1)
    check("note explains sealing", "admin PIN" in pg.locator("#seal-note").inner_text())
    pg.fill("#seal-list input", "1234"); pg.wait_for_timeout(2600)   # a card poll happens meanwhile
    check("a poll does not wipe a half-typed PIN", pg.input_value("#seal-list input") == "1234")
    pg.click("[data-seal-go]"); pg.wait_for_timeout(400)
    check("wrong PIN is reported", "wrong PIN" in pg.locator("#toast").inner_text(), pg.locator("#toast").inner_text())
    pg.fill("#seal-list input", "12345678"); pg.click("[data-seal-go]"); pg.wait_for_timeout(500)
    check("row turns to sealed", "sealed" in rows.nth(0).locator(".state").inner_text())
    check("note counts 1 of 3", "1 of 3" in pg.locator("#seal-note").inner_text(), pg.locator("#seal-note").inner_text())
    # nothing overflows or collapses: each row is one line high-ish and inside the card
    boxes = [rows.nth(i).bounding_box() for i in range(3)]; cardbox = pg.locator("#seal-card").bounding_box()
    check("rows sit inside the card at sane heights", all(40 <= bx["height"] <= 90 and bx["x"] >= cardbox["x"] and bx["x"] + bx["width"] <= cardbox["x"] + cardbox["width"] + 1 for bx in boxes), str(boxes))
    pg.screenshot(path=str(out / "keys-seal.png"), full_page=True)
    nb, cb = pg.locator("#keys-now .nowname").bounding_box(), pg.locator("#keys-now").bounding_box()
    hit = pg.evaluate("() => { const r = document.querySelector('#keys-now .nowname').getBoundingClientRect(); const e = document.elementFromPoint(r.x + 8, r.y + r.height / 2); return !!e && !!e.closest('#keys-now'); }")
    check("Plugged in now: the key's name is inside its card and is what a click there would hit", nb and cb["x"] <= nb["x"] and nb["x"] + nb["width"] <= cb["x"] + cb["width"] + 1 and hit and "YubiKey" in pg.inner_text("#keys-now"), f"name={nb} card={cb} hit={hit}")
    ab = pg.locator("#keys-now svg").bounding_box(); check("and so is its drawing", ab and ab["x"] >= cb["x"] and ab["width"] > 10, str(ab))
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
