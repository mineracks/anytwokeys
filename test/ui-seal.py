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
const store = () => ({ vaults: [%(vault)s], settings: { network: 'signet', custom: false, electrum: '', explorer: '', gap: 20 }, seen: {}, key_fw: window.__keyFw || {}, key_first: window.__keyFirst || {} });
window.__forgot = 0;
const H = { vault_rename: (a) => { window.__renamed = a.keyNames[1]; const st = store(); st.vaults[0].keys[1].name = a.keyNames[1]; return st; }, vault_forget: () => { __forgot++; return { vaults: [], settings: { network: 'signet', custom: false, electrum: '', explorer: '', gap: 20 }, seen: {} }; },
  store_get: store, cards_list: () => { if (!__state.cards.length) throw 'no YubiKey found'; return __state.cards; },
  cards_other: () => (window.__other || []),
  key_form_seen: (a) => { window.__fwSeen = (window.__fwSeen || []).concat([a.firmware]); window.__keyFw = Object.assign({}, window.__keyFw || {}); if (a.firmware) window.__keyFw[a.pubkey] = a.firmware; window.__keyFirst = Object.assign({ [a.pubkey]: Math.floor(Date.now()/1000) }, window.__keyFirst || {}); return store(); },
  vault_recovery: () => 'wsh(sortedmulti(2,03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2,032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe,038410f240613b33fe73d968f331ca82eb1fbd018c6189f89f8c5f9b9fb26f6a47))#5va274yx cc=' + 'ab'.repeat(32), vault_sync: () => { throw 'offline in the harness'; },
  seal_status: () => ({ id: 'deadbeef', threshold: 2, earlier: false, sealed: __state.sealed.length + (__state.open || []).length, open: (__state.open || []).length,
    keys: store().vaults[0].keys.map(k => ({ name: k.name, pubkey: k.pubkey, here: __state.cards.some(c => c.pubkey === k.pubkey), state: __state.sealed.includes(k.pubkey) ? 'sealed' : (__state.open || []).includes(k.pubkey) ? 'open' : 'unsealed' })) }),
  seal_write: (a) => { __state.lastWrite = a; if (!a.pin) throw "enter the key PIN as well"; if (a.adminPin !== '12345678' || a.pin !== '123456') throw 'wrong PIN, 2 tries left'; const c = __state.cards.find(c => c.reader === a.reader); __state.open = (__state.open || []).filter(p => p !== c.pubkey); __state.sealed.push(c.pubkey); return 'sealed and locked, 188 bytes on the key'; },
  seal_unlock: (a) => { __state.unlockTries = (__state.unlockTries || 0) + 1; if (a.pin !== '123456') throw 'wrong PIN, 2 tries left'; const c = __state.cards.find(c => c.reader === a.reader); return [{ pubkey: c.pubkey, serial: c.serial, entry: { id: 'deadbeef', k: 2, x: %(keys)s.indexOf(c.pubkey) + 1, share: 'aa', locked: false } }]; },
  seal_adopt: (a) => { __state.adopted = a; return store(); },
  seal_read: () => ({ cards: __state.cards.length, foreign: [], found: __state.cards.filter(c => __state.sealedCards.includes(c.pubkey)).map(c => ({ pubkey: c.pubkey, serial: c.serial, entry: { id: 'deadbeef', k: 2, x: %(keys)s.indexOf(c.pubkey) + 1, share: 'aa', locked: !!__state.lockedShares } })) }),
  seal_preview: (a) => [a.found.filter(f => !f.entry.locked).length >= 2 ? { locked: 0, id: 'deadbeef', have: a.found.length, need: 2, known_as: null, address: 'tb1qexample', line: 'wsh(sortedmulti(2,…)) cc=abab',
      record: { name: 'family vault', threshold: 2, network: 'signet', pubkeys: %(keys)s, chain_code: 'abab', key_names: ['laptop', 'keyring', 'off-site'] } } : { id: 'deadbeef', have: a.found.filter(f => !f.entry.locked).length, locked: a.found.filter(f => f.entry.locked).length, need: 2, record: null, line: null, address: null, known_as: null }],
  seal_restore: (a) => { __state.restored = a; return store().vaults[0]; },
};
window.__TAURI__ = { core: { invoke: async (cmd, args) => { __calls.push(cmd); if (!H[cmd]) throw 'unmocked: ' + cmd; return H[cmd](args || {}); } }, event: { listen: async () => () => {} } };
"""
card = lambda i: dict(reader=f"Yubico YubiKey {i}", serial=vault["keys"][i]["serial"], version="OpenPGP 3.4", pubkey=K[i], touch="FIXED")
fails = []
def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + (f" — {detail}" if detail and not ok else "")); 
    if not ok: fails.append(name)

import sys as _s; _s.path.insert(0, str(pathlib.Path(__file__).parent)); import audit; audit.install(globals())
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
    check("the plugged-in key asks for BOTH PINs, absent keys for none", pg.locator("#keys-list [data-seal-pin]").count() == 1 and pg.locator("#keys-list [data-seal-upin]").count() == 1)
    check("the note says what the PIN is for", "locked by its PIN" in pg.inner_text("#seal-note") and "admin PIN" in pg.inner_text("#seal-note"))
    pg.fill("#keys-list [data-seal-pin]", "1234"); pg.wait_for_timeout(2600)   # a card poll happens meanwhile
    check("a poll does not wipe a half-typed PIN", pg.input_value("#keys-list [data-seal-pin]") == "1234")
    pg.click("[data-seal-go]"); pg.wait_for_timeout(300)
    check("sealing without the key's own PIN is refused before anything is sent", pg.evaluate("__state.lastWrite") is None and "Enter both" in pg.inner_text("#toast"), pg.inner_text("#toast"))
    pg.fill("#keys-list [data-seal-upin]", "123456"); pg.click("[data-seal-go]"); pg.wait_for_timeout(400)
    check("wrong PIN is reported", "wrong PIN" in pg.locator("#toast").inner_text(), pg.locator("#toast").inner_text())
    pg.fill("#keys-list [data-seal-upin]", "123456"); pg.fill("#keys-list [data-seal-pin]", "12345678"); pg.click("[data-seal-go]"); pg.wait_for_timeout(500)
    # the store was just re-read, so `vault` is a fresh object that has not fetched its recovery line
    pg.evaluate("vault._recovery = undefined"); pg.locator("#desc-reveal").scroll_into_view_if_needed(); pg.click("#desc-reveal"); pg.wait_for_timeout(400)
    shown_ = pg.evaluate("document.querySelector('#keys-desc').textContent")
    check("Reveal straight after a seal shows the line, never an empty box", shown_.startswith("wsh(sortedmulti(2,0337") and pg.get_attribute("#desc-wrap", "data-hidden") == "0", repr(shown_[:40]))
    pg.click("#desc-hide")
    pg.evaluate("vault._recovery = undefined; window.__copied = null; navigator.clipboard.writeText = async (x) => { window.__copied = x; }"); pg.click("#copy-desc"); pg.wait_for_timeout(400)
    check("Copy descriptor on a fresh vault object copies the FULL line, chain code included, never the bare descriptor", (pg.evaluate("window.__copied") or "").endswith("cc=" + "ab" * 32), repr((pg.evaluate("window.__copied") or "")[-30:]))
    check("both PINs reach the backend", pg.evaluate("__state.lastWrite.pin") == "123456" and pg.evaluate("__state.lastWrite.adminPin") == "12345678")
    check("the row's seal slot turns to sealed, its PIN field gone", "PIN-locked" in rows.nth(0).locator(".kseal").inner_text() and rows.nth(0).locator("[data-seal-pin]").count() == 0)
    check("note counts 1 of 3", "1 of 3" in pg.inner_text("#seal-note"), pg.inner_text("#seal-note"))
    # nothing overflows or collapses: each row is one line high-ish and inside the card
    boxes = [rows.nth(i).bounding_box() for i in range(3)]; cardbox = pg.locator("#keys-list").bounding_box()
    check("rows sit inside the card at sane heights", all(40 <= bx["height"] <= 110 and bx["x"] >= cardbox["x"] and bx["x"] + bx["width"] <= cardbox["x"] + cardbox["width"] + 1 for bx in boxes), str(boxes))
    pg.screenshot(path=str(out / "keys-seal.png"), full_page=True)
    check("ONE list: a row per key, no second or third card repeating them", pg.locator("#s-keys .keyrow").count() == 3 and pg.locator("#seal-list").count() == 0 and "Plugged in now" not in pg.inner_text("#s-keys"))
    r0 = rows.nth(0).inner_text()
    check("the plugged-in key's row says what it is, that it is in, and that it is sealed", "YubiKey" in r0 and "plugged in" in r0 and "sealed" in r0 and "firmware" not in rows.nth(1).inner_text(), r0)
    # ---- what a key can honestly be said to know about itself ----
    check("the screen says plainly that a key carries NO end-of-life or warranty date, rather than leaving a gap",
          "does" in pg.inner_text("#key-life") and "not" in pg.inner_text("#key-life") and "warranty" in pg.inner_text("#key-life") and "guessed one would be worse than nothing" in pg.inner_text("#key-life"), pg.inner_text("#key-life")[:200])
    check("and that Yubico retires a product LINE, not your key, with no date set in advance",
          "product line" in pg.inner_text("#key-life") and "six months" in pg.inner_text("#key-life") and "no date in advance" in pg.inner_text("#key-life"))
    check("the firmware check is done HERE, and says why nothing is asked of anyone",
          "on this computer" in pg.inner_text("#key-life") and "which keys guard this vault" in pg.inner_text("#key-life"))
    check("it dates what it knows, so an old build cannot imply it knows today's advisories", "2026" in pg.inner_text("#key-life"))
    pg.evaluate("__state.cards[0].firmware = '5.4.3'; renderKeys(true)"); pg.wait_for_timeout(400)
    check("a key on firmware before 5.7.0 is told about YSA-2024-03, by name, on its own row",
          "YSA-2024-03" in pg.inner_text("#keys-list") and "5.7.0 and newer are not affected" in pg.inner_text("#keys-list"), pg.inner_text("#keys-list")[:200])
    check("and it says the firmware cannot be changed on a key that exists, so the only remedy is replacement",
          "does not allow firmware to be updated" in pg.inner_text("#keys-list") and "only replaced" in pg.inner_text("#keys-list"), pg.inner_text("#keys-list")[:260])
    check("and what it means for a vault of several keys, rather than sounding an alarm",
          "enough of the others to sign" in pg.inner_text("#keys-list"))
    pg.evaluate("__state.cards[0].firmware = '5.7.1'; renderKeys(true)"); pg.wait_for_timeout(400)
    check("a key on 5.7.0 or newer is told nothing, because nothing is known against it",
          "YSA-2024-03" not in pg.inner_text("#keys-list") and "Nothing known stands against" in pg.inner_text("#key-life"))
    check("the version test is a real comparison, not a string match", pg.evaluate("[verLt('5.10.0','5.7.0'), verLt('5.7','5.7.0'), verLt('5.6.9','5.7.0'), verLt('','5.7.0')]") == [False, False, True, True])
    pg.evaluate("window.__keyFirst = { [__state.cards[0].pubkey]: Math.floor(Date.now()/1000) - 400*86400 }; store.key_first = window.__keyFirst; renderKeys(true)"); pg.wait_for_timeout(400)
    # ---- a device that is plugged in and is NOT a key: named, and why it will not do ----
    pg.evaluate("window.__other = [{ reader: 'Yubico YubiKey Bio FIDO+CCID', why: 'the computer can see it, but it does not answer as a smart card' }]; pollCards()"); pg.wait_for_timeout(700)
    check("a YubiKey Bio plugged in is NAMED, and said to be unusable here rather than ignored",
          pg.locator("#keys-other").is_visible() and "YubiKey Bio" in pg.inner_text("#keys-other") and "cannot be used here" in pg.inner_text("#keys-other"), pg.inner_text("#keys-other")[:200])
    check("it says WHY, in terms of what it is for, and that it is fine — just for something else",
          "fingerprint" in pg.inner_text("#keys-other") and "just for something else" in pg.inner_text("#keys-other") and "nothing was done to it" in pg.inner_text("#keys-other"))
    check("and the vault's own keys are still listed beside it — one device that is not a key must not hide the ones that are",
          rows.count() >= 1 and "plugged in" in rows.nth(0).inner_text())
    pg.evaluate("window.__other = [{ reader: 'Yubico YubiKey OTP+FIDO', why: 'it has no OpenPGP applet, which is what Any Two Keys signs with' }]; pollCards()"); pg.wait_for_timeout(700)
    check("a key with no OpenPGP applet is told so in those words, not mistaken for a key that is failing",
          "no OpenPGP applet" in pg.inner_text("#keys-other") and "YubiKey Bio" not in pg.inner_text("#keys-other"))
    pg.evaluate("window.__other = []; pollCards()"); pg.wait_for_timeout(700)
    check("unplugged, it goes", not pg.locator("#keys-other").is_visible())
    # ---- firmware remembered, so an off-site key can be checked too ----
    fw_key = pg.evaluate("vault.keys[1].pubkey")
    pg.evaluate("window.__keyFw = { [vault.keys[1].pubkey]: '5.4.3' }; store.key_fw = window.__keyFw; renderKeys(true)"); pg.wait_for_timeout(400)
    check("a key NOT plugged in still shows the firmware it gave last time, saying that is where it came from",
          "firmware 5.4.3 (as it said when last plugged in)" in rows.nth(1).inner_text(), rows.nth(1).inner_text()[:200])
    check("and the advisory reaches it — which is the only way an off-site key is ever checked",
          "YSA-2024-03" in rows.nth(1).inner_text())
    pg.evaluate("window.__keyFw = Object.fromEntries(vault.keys.map(k => [k.pubkey, '5.4.3'])); store.key_fw = window.__keyFw; __state.cards.forEach(c => c.firmware = '5.4.3'); renderKeys(true)"); pg.wait_for_timeout(400)
    check("when EVERY key is on affected firmware it says so, and says what that takes away from a vault of several keys",
          "All " in pg.inner_text("#key-life") and "every one of them has the same weakness" in pg.inner_text("#key-life") and "comfort is gone" in pg.inner_text("#key-life"), pg.inner_text("#key-life")[-260:])
    pg.evaluate("window.__keyFw = {}; store.key_fw = {}; __state.cards.forEach(c => c.firmware = '5.7.1'); renderKeys(true)"); pg.wait_for_timeout(400)
    check("the nearest thing to an age it can claim is when this computer first saw the key, shown as that and no more",
          "known to this computer since" in pg.inner_text("#keys-list") and "400 days" in pg.inner_text("#keys-list"), pg.inner_text("#keys-list")[:220])
    # scrolled and measured in ONE step inside the page: the card poll redraws this list, and a redraw landing between
    # a scroll and a measurement detached the row and aborted the whole run (two runs in three, 2026-09-21)
    geom = pg.evaluate("""() => { const svg = document.querySelector('#keys-list .keyrow .keyart svg'); if (!svg) return null;
        svg.scrollIntoView({ block: 'center' }); const r = svg.getBoundingClientRect(), c = document.querySelector('#keys-list').getBoundingClientRect();
        const e = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
        return { x: r.x, w: r.width, listX: c.x, onTop: !!e && !!e.closest('.keyart') }; }""")
    check("its drawing is inside the list and on top", bool(geom) and geom["x"] >= geom["listX"] and geom["w"] > 10 and geom["onTop"], geom)
    check("a key that is away and unconfirmed says so without claiming it is unsealed", "seal not confirmed" in rows.nth(1).inner_text())
    check("no separate Edit names panel: the pencil on each row is the way, and it can be seen without hovering", pg.locator("#edit-names").count() == 0 and pg.locator("#edit-panel").count() == 0 and pg.evaluate("() => { const p = document.querySelector('#keys-list .pencil'), s = getComputedStyle(p); return s.color !== 'rgba(0, 0, 0, 0)' && +s.opacity > 0.3; }"))
    pg.click("#keys-list [data-edit-idx='1']"); pg.keyboard.type("spare"); pg.keyboard.press("Enter"); pg.wait_for_timeout(300)
    check("renaming from the pencil still works with PIN fields in the list", pg.evaluate("window.__renamed") == "spare" or "spare" in pg.inner_text("#keys-list"), pg.inner_text("#keys-list")[:120])
    pg.evaluate("__state.cards = __state.cards.concat([{ reader: 'z', serial: 999, version: 'OpenPGP 3.4', pubkey: '02' + 'cd'.repeat(32), touch: 'FIXED', form: 1, nfc: true, firmware: '5.7.1' }])"); pg.wait_for_timeout(2600)
    check("a key that is not this vault's is named once, under the list", "Also plugged in: YubiKey 5 NFC · serial 999" in pg.inner_text("#keys-now") and pg.locator("#s-keys .keyrow").count() == 3, pg.inner_text("#keys-now"))
    pg.locator("#s-keys .card").first.screenshot(path=str(out / "keys-one-list.png"))
    # ---- the recovery line is not on screen, nor in the page, until asked for ----
    real = pg.evaluate("vault._recovery"); shown = pg.inner_text("#keys-desc") if pg.locator("#keys-desc").count() else ""
    dom = pg.evaluate("document.querySelector('#keys-desc').textContent")
    check("at rest: blurred, and what is in the page is a stand-in of the same shape, not the line", pg.get_attribute("#desc-wrap", "data-hidden") == "1" and dom != real and len(dom) == len(real) and real not in pg.content() and "blur" in pg.evaluate("getComputedStyle(document.querySelector('#keys-desc')).filter"), dom[:40])
    pg.locator("#desc-wrap").scroll_into_view_if_needed(); pg.locator("#desc-wrap").screenshot(path=str(out / "desc-hidden.png"))
    check("no copy icon while it is hidden", not pg.locator("#desc-copy").is_visible())
    pg.click("#desc-reveal"); pg.wait_for_timeout(300)
    check("Reveal shows the real line, selectable, with a way to hide it", pg.evaluate("document.querySelector('#keys-desc').textContent") == real and pg.evaluate("getComputedStyle(document.querySelector('#keys-desc')).filter") == "none" and pg.locator("#desc-hide").is_visible() and not pg.locator("#desc-reveal").is_visible())
    check("a copy icon appears with the revealed line, and not before", pg.locator("#desc-copy").is_visible())
    pg.evaluate("navigator.clipboard.writeText = async (x) => { window.__copied = x; }"); pg.click("#desc-copy"); pg.wait_for_timeout(200)
    check("it copies the REAL line and says so", pg.evaluate("window.__copied") == real and "copied" in pg.inner_text("#toast").lower() and "done" in pg.get_attribute("#desc-copy", "class"))
    pg.click("#desc-hide"); check("Hide it again: gone from the page", pg.evaluate("document.querySelector('#keys-desc').textContent") != real)
    pg.click("#desc-reveal"); pg.evaluate("show('home')"); pg.evaluate("show('keys')"); pg.wait_for_timeout(400)
    check("leaving the screen hides it", pg.get_attribute("#desc-wrap", "data-hidden") == "1" and pg.evaluate("document.querySelector('#keys-desc').textContent") != real)
    pg.click("#desc-reveal"); pg.evaluate("window.dispatchEvent(new Event('blur'))"); check("so does switching to another app", pg.get_attribute("#desc-wrap", "data-hidden") == "1")
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
    check("one key: says one more is needed", "needs 2 keys: 1 open" in pg2.locator("#restore-card").inner_text(), pg2.locator("#restore-card").inner_text())
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
    # ---- PIN-locked shares: holding the keys is not enough; each key is asked for its PIN ----
    pg3 = b.new_page(viewport=dict(width=1180, height=900)); errs3 = []
    pg3.on("pageerror", lambda e: errs3.append(str(e)))
    pg3.add_init_script(MOCK % dict(cards=json.dumps([card(1)]), vault=json.dumps(vault), keys=json.dumps(K)))
    pg3.goto((src / "index.html").as_uri()); pg3.wait_for_timeout(600)
    pg3.evaluate("__state.sealedCards = %s; __state.lockedShares = true; show('restore')" % json.dumps(K)); pg3.wait_for_timeout(2600)
    check("a locked share is NOT usable on sight: the key asks for its PIN, and nothing is rebuilt", pg3.locator("[data-unlock-pin]").count() == 1 and "Rebuilt" not in pg3.inner_text("#restore-card") and "1 still locked" in pg3.inner_text("#restore-card"), pg3.inner_text("#restore-card"))
    check("the PIN field has the focus", pg3.evaluate("document.activeElement && document.activeElement.hasAttribute('data-unlock-pin')"))
    pg3.keyboard.type("000000"); pg3.keyboard.press("Enter"); pg3.wait_for_timeout(400)
    check("a wrong PIN is reported, opens nothing, and the field is cleared", "wrong PIN" in pg3.inner_text("#toast") and "Rebuilt" not in pg3.inner_text("#restore-card") and pg3.input_value("[data-unlock-pin]") == "")
    pg3.fill("[data-unlock-pin]", "123456"); pg3.keyboard.press("Enter"); pg3.wait_for_timeout(500)
    check("the right PIN opens that key's share", "1 share(s) open" in pg3.inner_text("#seal-read-list") and "needs 2 keys: 1 open" in pg3.inner_text("#restore-card"), pg3.inner_text("#restore-card"))
    pg3.evaluate("__state.cards = %s" % json.dumps([card(2)])); pg3.wait_for_timeout(2600)
    pg3.locator("#s-restore").screenshot(path=str(out / "restore-locked.png"))
    pg3.fill("[data-unlock-pin]", "123456"); pg3.click("[data-unlock]"); pg3.wait_for_timeout(600)
    check("second key, second PIN: the vault is rebuilt", "Rebuilt from 2 keys" in pg3.inner_text("#restore-card") and pg3.evaluate("__state.unlockTries") == 3, pg3.inner_text("#restore-card")[:120])
    check("no page errors with locked shares", not errs3, "; ".join(errs3))
    # ---- a key sealed before locking existed: said plainly, and lockable in place ----
    pg4 = b.new_page(viewport=dict(width=1180, height=900))
    pg4.add_init_script(MOCK % dict(cards=json.dumps([card(0)]), vault=json.dumps(vault), keys=json.dumps(K)))
    pg4.goto((src / "index.html").as_uri()); pg4.wait_for_timeout(600)
    pg4.evaluate("__state.open = %s; show('keys')" % json.dumps(K)); pg4.wait_for_timeout(700)
    r4 = pg4.locator("#keys-list .keyrow")
    check("an unlocked share is called what it is, on the row and in the note", "sealed, not locked" in r4.nth(0).inner_text() and "sealed, not locked · plug it in" in r4.nth(1).inner_text() and "without any PIN" in pg4.inner_text("#seal-note"), pg4.inner_text("#seal-note"))
    pg4.locator("#s-keys .card").first.screenshot(path=str(out / "keys-open.png"))
    pg4.fill("#keys-list [data-seal-upin]", "123456"); pg4.fill("#keys-list [data-seal-pin]", "12345678"); pg4.click("[data-seal-go]"); pg4.wait_for_timeout(600)
    check("Lock it with the PIN: the row becomes PIN-locked and the count of open ones drops", "PIN-locked" in r4.nth(0).inner_text() and pg4.evaluate("__state.open.length") == 2 and pg4.evaluate("__state.lastWrite.replace") is False, r4.nth(0).inner_text())
    check("no page errors on Restore", not errs2, "; ".join(errs2))
    b.close()
audit.finish()
sys.exit(1 if fails else 0)
