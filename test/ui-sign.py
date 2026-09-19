#!/usr/bin/env python3
"""Drive the Sign screen in WebKit against a mocked backend: the ceremony visual in each state, the QR panel
   (its canvas is read back with a real QR decoder when zxing-cpp is installed), PDF and PSBT import wiring.
   usage: <python with playwright> test/ui-sign.py <outdir>      exits non-zero on any failed check"""
import json, sys, io, pathlib
from playwright.sync_api import sync_playwright
here = pathlib.Path(__file__).resolve().parent; src = here.parent / "src"
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
parts = (here / "fixtures/sample-ur-parts.txt").read_text().split(); single = (here / "fixtures/sample-ur-single.txt").read_text().strip()
K = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe", "038410f240613b33fe73d968f331ca82eb1fbd018c6189f89f8c5f9b9fb26f6a47"]
names = ["laptop", "keyring", "off-site"]
vault = dict(name="family vault", network="signet", threshold=2, keys=[dict(name=n, serial=i + 1, pubkey=k) for i, (n, k) in enumerate(zip(names, K))], descriptor="wsh(…)", address="tb1qexample", created=0, chain_code=None, labels={}, seal=dict(shares=[], sealed=[]))
MOCK = """
window.__state = { cards: [], signed: [0, 0, 0], inputs: 2, psbt: true, imported: null };
const V = %(vault)s, K = %(keys)s;
const store = () => ({ forms: __state.forms || {}, vaults: [V], settings: { network: 'signet', custom: false, electrum: '', explorer: '', gap: 20 }, seen: {}, last_psbt: __state.psbt ? 'cHNidP8B' : null, last_message: null });
const ceremony = () => { if (!__state.psbt) throw 'no spend in progress'; const full = __state.signed.filter(n => n >= __state.inputs).length; return { ours: __state.ours !== false, inputs: __state.inputs, need: 2, have: Math.min(full, 2), ready: full >= 2, fee: 190, txid: 'bcd3fb3d', bytes: 398,
  keys: V.keys.map((k, i) => ({ name: k.name, pubkey: k.pubkey, signed: __state.signed[i] })), pays: [{ address: 'tb1qrecipient0000000000000000000000000000000000000000000dest', sats: 5000, ours: false }, { address: 'tb1qchange', sats: 4810, ours: true }] }; };
const H = { store_get: store, cards_list: () => { if (!__state.cards.length) throw 'no YubiKey found'; return __state.cards; }, vault_sync: () => { throw 'offline'; }, fees_get: () => ({ slow: 1, medium: 2, fast: 3 }),
  key_form_seen: (a) => { __state.forms = Object.assign({}, __state.forms, { [a.pubkey]: a.form }); return store(); }, spend_discard: () => { __state.psbt = false; },
  spend_ceremony: ceremony, spend_qr: () => ({ parts: %(parts)s, single: %(single)s, bytes: 398 }), spend_pdf: () => '/tmp/x.pdf',
  spend_import: (a) => { __state.imported = a; __state.psbt = true; __state.signed = [2, 0, 0]; return ceremony(); } };
window.__TAURI__ = { core: { invoke: async (cmd, args) => { if (!H[cmd]) throw 'unmocked: ' + cmd; return H[cmd](args || {}); } }, event: { listen: async () => () => {} } };
"""
card = lambda i: dict(reader=f"Yubico YubiKey {i}", serial=i + 1, version="OpenPGP 3.4", pubkey=K[i], touch="FIXED", form=4 if i == 0 else 3)
fails = []
def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + (f" — {detail}" if detail and not ok else ""))
    if not ok: fails.append(name)
with sync_playwright() as p:
    b = p.webkit.launch(); pg = b.new_page(viewport=dict(width=1500, height=1000)); errs = []
    pg.on("pageerror", lambda e: errs.append(str(e)))
    pg.add_init_script(MOCK % dict(vault=json.dumps(vault), keys=json.dumps(K), parts=json.dumps(parts), single=json.dumps(single)))
    pg.goto((src / "index.html").as_uri()); pg.wait_for_timeout(600)
    # ---- nobody has signed; the keyring is plugged in ----
    pg.evaluate("__state.cards = %s; show('sign')" % json.dumps([card(1)])); pg.wait_for_timeout(2600)
    tiles = pg.locator("#cer-keys .ckey")
    check("one tile per vault key", tiles.count() == 3)
    check("0 of 2, no arc filled", pg.inner_text("#cer-have") == "0" and pg.locator("#cer-ring .arc[style*='opacity:0']").count() == 2)
    check("the plugged-in key is the one marked to sign", "here" in tiles.nth(1).get_attribute("class") and "sign now" in tiles.nth(1).inner_text())
    check("the sentence names what is left", "2 more keys, any of laptop, keyring, off-site" in pg.inner_text("#sign-progress"), pg.inner_text("#sign-progress"))
    check("it says what the spend pays", "5,000 sats to tb1qrecipien" in pg.inner_text("#sign-summary") and "4,810 back to this vault" in pg.inner_text("#sign-summary"), pg.inner_text("#sign-summary"))
    pg.locator("#ceremony").screenshot(path=str(out / "ceremony-0.png"))
    # ---- the keyring signs one of two inputs, then both ----
    pg.evaluate("__state.signed = [0, 1, 0]"); pg.wait_for_timeout(2300)
    check("a key that signed some inputs says how many", "1 of 2 inputs" in tiles.nth(1).inner_text() and pg.inner_text("#cer-have") == "0")
    pg.evaluate("__state.signed = [0, 2, 0]; __state.cards = %s" % json.dumps([card(2)])); pg.wait_for_timeout(2600)
    check("1 of 2: keyring signed, off-site now marked to sign", "signed" in tiles.nth(1).get_attribute("class") and "here" in tiles.nth(2).get_attribute("class") and pg.inner_text("#cer-have") == "1")
    check("sentence: who signed, who may finish", "keyring has signed. One more key, any of laptop, off-site" in pg.inner_text("#sign-progress"), pg.inner_text("#sign-progress"))
    check("export card is offered after one signature", pg.locator("#export-card").is_visible() and pg.locator("#export-pdf").is_visible() and pg.locator("#export-qr").is_visible())
    pg.locator("#ceremony").screenshot(path=str(out / "ceremony-1.png"))
    # ---- context: where am I, and the way back ----
    lit = lambda: [e.inner_text().split("\n")[0] for e in pg.locator(".side .nav.on, aside .nav.on, #vault-tabs .nav.on, .nav.on").all()]
    check("sidebar lights the vault you are in AND the screen Sign came from", set(lit()) == {"family vault", "Send"}, str(lit()))
    check("breadcrumb reads vault > Send > Sign", pg.inner_text("#crumbs").replace("\n", " ").split() == "family vault › Send › Sign".split(), pg.inner_text("#crumbs"))
    box = lambda sel: pg.locator(sel).bounding_box()
    check("wide window: export sits beside the ceremony, not under it", box("#export-card")["x"] > box("#ceremony")["x"] + box("#ceremony")["width"] and abs(box("#export-card")["y"] - box("#ceremony")["y"]) < 4)
    bs = [pg.locator(s).bounding_box() for s in ("#export-psbt", "#export-qr", "#export-pdf", "#copy-psbt")]; ecard = box("#export-card")
    check("export buttons: equal widths and heights, on a grid", len({round(b["width"]) for b in bs}) == 1 and len({round(b["height"]) for b in bs}) == 1 and len({round(b["x"]) for b in bs}) <= 2, str([(round(b["x"]), round(b["y"]), round(b["width"])) for b in bs]))
    check("export buttons fill the card edge to edge", abs(min(b["x"] for b in bs) - (ecard["x"] + 23)) < 3 and abs(max(b["x"] + b["width"] for b in bs) - (ecard["x"] + ecard["width"] - 23)) < 3, str(ecard))
    pg.locator("#export-card").screenshot(path=str(out / "export-grid.png"))
    check("the PIN and Sign button live inside the ceremony card", pg.locator("#ceremony #sign-btn").count() == 1 and pg.locator("#ceremony #pin").is_visible() and pg.locator("#sign-ring").count() == 0)
    check("the card that can sign now carries the gold border", "can-sign" in pg.get_attribute("#ceremony", "class"))
    pg.evaluate("__state.cards = %s" % json.dumps([card(0)])); pg.wait_for_timeout(2600)
    check("a Nano is learnt from the card and drawn short", "nano" in tiles.nth(0).get_attribute("class") and pg.evaluate("__state.forms[%s]" % json.dumps(K[0])) == 4 and "nano" not in tiles.nth(1).get_attribute("class"))
    pg.screenshot(path=str(out / "sign-wide.png"))
    pg.click("#crumbs [data-crumb=send]"); pg.wait_for_timeout(500)
    check("walking back to Send keeps the spend in view", pg.locator("#s-send.on").count() == 1 and pg.locator("#staged-bar").is_visible() and "1 of 2" in pg.inner_text("#staged-text") and "5,000 sats" in pg.inner_text("#staged-text"), pg.inner_text("#staged-text"))
    check("the vault stays lit after walking back", "family vault" in lit() and "Send" in lit(), str(lit()))
    pg.evaluate("show('settings')"); pg.wait_for_timeout(200)
    check("app settings are not about a vault: no vault lit", lit() == ["Settings"], str(lit()))
    pg.evaluate("show('send')"); pg.wait_for_timeout(400)
    check("Send's breadcrumb is vault > Send", pg.inner_text("#crumbs").split() == "family vault › Send".split(), pg.inner_text("#crumbs"))
    pg.screenshot(path=str(out / "send-staged.png"))
    pg.click("#staged-bar [data-go=sign]"); pg.wait_for_timeout(500)
    check("Continue signing returns to the ceremony", pg.locator("#s-sign.on").count() == 1 and not pg.locator("#staged-bar").is_visible())
    pg.set_viewport_size(dict(width=1100, height=1000)); pg.wait_for_timeout(200)
    check("narrow window: one column", box("#export-card")["y"] > box("#ceremony")["y"] + box("#ceremony")["height"])
    pg.set_viewport_size(dict(width=1500, height=1000))
    pg.evaluate("__state.cards = %s" % json.dumps([card(2)])); pg.wait_for_timeout(2600)
    # ---- the QR panel: read the canvas back with a real decoder ----
    pg.click("#export-qr"); pg.wait_for_timeout(300)
    check("QR panel opens on the animated code", pg.locator("#qr-panel").is_visible() and pg.inner_text("#qr-title").lower().startswith("part "), pg.inner_text("#qr-title"))
    try:
        import zxingcpp; from PIL import Image
        read = lambda: [r.text for r in zxingcpp.read_barcodes(Image.open(io.BytesIO(pg.locator("#qr-canvas").screenshot())).convert("RGB"))]
        seen = set()
        for _ in range(60):
            seen.update(read()); pg.wait_for_timeout(90)
            if seen >= set(parts): break
        check("the animation shows every part, plain and repair, and each one decodes", seen >= set(parts), f"{len(seen & set(parts))} of {len(parts)}")
        pg.click("#qr-mode"); pg.wait_for_timeout(200)
        check("the still code reads back as the single UR", pg.inner_text("#qr-title").lower() == "one code" and read() == [single], str(read())[:120])
    except ImportError: print("skip QR read-back (pip install zxing-cpp pillow)")
    pg.locator("#export-card").screenshot(path=str(out / "export-qr.png"))
    pg.evaluate("show('home')"); check("leaving the screen stops the animation", pg.evaluate("qrShow.timer") is None)
    # ---- both signatures in ----
    pg.evaluate("__state.signed = [0, 2, 2]; show('sign')"); pg.wait_for_timeout(700)
    check("2 of 2: ring ready, third key not needed, send offered", "ready" in (pg.get_attribute("#cer-ring", "class") or "") and "not needed" in tiles.nth(0).inner_text() and pg.locator("#broadcast-card").is_visible())
    pg.locator("#ceremony").screenshot(path=str(out / "ceremony-2.png"))
    # ---- nothing in progress, then import by paste ----
    pg.evaluate("__state.psbt = false; show('sign')"); pg.wait_for_timeout(500)
    check("no spend: the ceremony and export cards are hidden, and the way to Send is offered", not pg.locator("#ceremony").is_visible() and not pg.locator("#export-card").is_visible() and pg.locator("#nospend [data-go=send]").is_visible())
    pg.evaluate("__state.psbt = true; __state.ours = false; show('sign')"); pg.wait_for_timeout(500)
    check("another vault's spend is named as such, not drawn as ours", "another vault" in pg.inner_text("#nospend-title").lower() and not pg.locator("#ceremony").is_visible())
    pg.evaluate("__state.psbt = false; __state.ours = true")
    pg.evaluate("show('send')"); pg.click("#psbt-paste-toggle"); pg.fill("#psbt-text", "cHNidP8BAH0C"); pg.click("#psbt-load"); pg.wait_for_timeout(700)
    check("pasted PSBT goes to the backend and lands on the Sign screen", pg.evaluate("__state.imported && __state.imported.text") == "cHNidP8BAH0C" and pg.locator("#s-sign.on").count() == 1 and pg.inner_text("#cer-have") == "1")
    check("no page errors", not errs, "; ".join(errs))
    b.close()
sys.exit(1 if fails else 0)
