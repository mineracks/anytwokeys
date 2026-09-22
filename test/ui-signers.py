#!/usr/bin/env python3
"""Drive vault setup with mixed signers in WebKit (the Tauri webview's engine) against a mocked backend.
   usage: <python with playwright> test/ui-signers.py <outdir>      exits non-zero on any failed check
   The Rust side (parsing a device's export, the descriptor and its addresses) is tested against embit in cargo test;
   this drives the window: the device menu, its drawings, reading a key, the warnings, and what is sent to be made."""
import json, sys, pathlib
from playwright.sync_api import sync_playwright
src = pathlib.Path(__file__).resolve().parent.parent / "src"
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
fx = json.loads((src.parent / "src-tauri/tests/fixtures/mixed-descriptor.json").read_text())
C1, C2 = fx["cards"]
XPUB = json.loads((src.parent / "src-tauri/tests/fixtures/device-keys.json").read_text())["slip132"]
SS = dict(fingerprint=XPUB["fingerprint"], path="m/" + XPUB["path"], xpub=XPUB["xpub"], script="wsh-multi", source="text")
MIXED = dict(name="mixed", network="mainnet", threshold=2, ext=fx["descriptor"], descriptor=fx["descriptor"], address=fx["recv"][0], created=0, labels={}, seal={},
             keys=[dict(name="SeedSigner", serial=0, pubkey=f"[{SS['fingerprint']}/{XPUB['path']}]{SS['xpub']}", device="seedsigner"),
                   dict(name="laptop", serial=111, pubkey=C1, device="yubikey"), dict(name="keyring", serial=222, pubkey=C2, device="yubikey")])
MOCK = """
window.__calls = []; window.__unmocked = []; window.__created = null;
window.__cards = [%(card1)s];
const store = () => ({ vaults: window.__vaults || [], settings: { network: 'mainnet', custom: false, electrum: '', explorer: '', gap: 20 }, seen: {}, key_fw: {}, key_first: {} });
const H = {
  store_get: store, cards_list: () => { if (!__cards.length) throw 'no YubiKey found'; return __cards; }, cards_other: () => [],
  key_form_seen: () => store(),
  device_key_read: (a) => { const t = (a.text || '').trim(); if (!t) throw 'nothing to read'; if (t.includes('tpub')) return { keys: [Object.assign({}, %(ss)s, { xpub: 'tpubDEADBEEF', fingerprint: '0badf00d' })], multisig: Object.assign({}, %(ss)s, { xpub: 'tpubDEADBEEF', fingerprint: '0badf00d' }) };
    if (t.startsWith('84')) { const k = Object.assign({}, %(ss)s, { path: 'm/84h/0h/0h', script: 'wpkh' }); return { keys: [k, Object.assign({}, k, { path: 'm/44h/0h/0h', script: 'pkh' })], multisig: null }; }
    return { keys: [%(ss)s], multisig: %(ss)s }; },
  device_key_file: () => { throw 'cancelled'; },
  vault_create_signers: (a) => { __created = { cmd: 'signers', a }; window.__vaults = [%(mixed)s]; return window.__vaults[0]; },
  spend_ceremony: (a) => { if (!window.__cer) throw 'no spend in progress'; return window.__cer; },
  spend_qr: () => ({ parts: ['UR:CRYPTO-PSBT/1-2/AAA', 'UR:CRYPTO-PSBT/2-2/BBB', 'UR:CRYPTO-PSBT/3-2/CCC', 'UR:CRYPTO-PSBT/4-2/DDD'], single: null, bytes: 600, bbqr: ['B$2P0200AAAA', 'B$2P0201BBBB'] }),
  spend_import: (a) => { (window.__imports = window.__imports || []).push(a.text); const n = (a.text || '').split('\\n').length; if (n < 2) throw 'some QR parts are missing: 1 of 2 pieces recovered so far; keep the code in the frame until it completes'; window.__cer = Object.assign({}, window.__cer, { have: 1, keys: window.__cer.keys.map(k => k.name === 'SeedSigner' ? Object.assign({}, k, { signed: 1 }) : k) }); return window.__cer; },
  vault_create: (a) => { __created = { cmd: 'cards', a }; throw 'stop here'; },
};
window.__TAURI__ = { core: { invoke: async (cmd, args) => { __calls.push(cmd); if (!H[cmd]) { __unmocked.push(cmd); return null; } return H[cmd](args || {}); } }, event: { listen: async () => () => {} } };
"""
card = lambda pub, sn: dict(reader=f"Yubico YubiKey OTP+FIDO+CCID {sn}", serial=sn, version="OpenPGP 3.4", pubkey=pub, touch="FIXED")
fails = []
def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + (f" — {detail}" if detail and not ok else ""))
    if not ok: fails.append(name)

import sys as _s; _s.path.insert(0, str(pathlib.Path(__file__).parent)); import audit; audit.install(globals())
with sync_playwright() as p:
    b = p.webkit.launch()
    pg = b.new_page(viewport=dict(width=1180, height=900)); errs = []
    pg.on("pageerror", lambda e: errs.append(str(e)))
    pg.add_init_script(MOCK % dict(card1=json.dumps(card(C1, 111)), ss=json.dumps(SS), mixed=json.dumps(MIXED)))
    pg.goto((src / "index.html").as_uri()); pg.wait_for_timeout(700)
    if errs: print("load errors:", errs[:3])
    pg.evaluate("setup = { k: 2, n: 3, keys: [], network: 'mainnet' }; show('setup')"); pg.wait_for_timeout(400)
    check("the setup screen offers a signing device", pg.is_visible("#add-device"))
    pg.click("#use-btn"); pg.wait_for_timeout(200)
    check("the plugged-in card enrols, remembered as a YubiKey", pg.evaluate("setup.keys.length === 1 && setup.keys[0].device === 'yubikey'"))

    # ---- the menu ----
    pg.click("#add-device"); pg.wait_for_timeout(200)
    n = pg.evaluate("DEVICES.length")
    check("every device in the catalogue is a tile with its drawing", pg.locator("#dev-pick .devopt").count() == n and pg.locator("#dev-pick .devopt svg").count() == n, f"{pg.locator('#dev-pick .devopt').count()} of {n}")
    check("at least 20 devices", n >= 20, str(n))
    check("grouped by how they hand over a key", pg.locator("#dev-pick .devgroup").count() == 4)
    pg.screenshot(path=str(out / "signers-menu.png"))
    pg.click("[data-dev=coldcard-mk4]"); pg.wait_for_timeout(100)
    check("a Coldcard Mk4 offers its SD file, not a camera it does not have", pg.locator("#dev-file").count() == 1 and pg.locator("#dev-scan").count() == 0)
    pg.click("#dev-back"); pg.click("[data-dev=trezor-safe7]"); pg.wait_for_timeout(100)
    t = pg.inner_text("#dev-one")
    check("a USB/Bluetooth device says the direct link comes later, and how to get its key meanwhile", "comes later" in t and "paste" in t and "Bluetooth" in t, t[:200])
    check("its Bluetooth is shown as the device's, not something the app does yet", pg.locator("#dev-one .chip:not(.on)").count() >= 1)
    pg.click("#dev-back"); pg.click("[data-dev=nitrokey3]"); pg.wait_for_timeout(100)
    check("the Nitrokey is honest that it is not yet read", "Not yet" in pg.inner_text("#dev-one") and pg.locator("#dev-read").count() == 0)
    # every scanning screen offers the same camera picker: three cameras, as on Piers's desk
    pg.evaluate("""() => { const cv = document.createElement('canvas'); cv.width = 64; cv.height = 48; cv.getContext('2d').fillRect(0, 0, 64, 48);
      window.__asked = [];
      const cams = [['a', 'IPEVO VZ-X'], ['b', 'Studio Display Camera'], ['c', "Piers' iPhone Camera"]].map(([deviceId, label]) => ({ deviceId, label, kind: 'videoinput' }));
      window.__mru = cams.slice();
      Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { enumerateDevices: async () => window.__mru.slice(),
        getUserMedia: async (c) => { window.__asked.push(c.video && c.video.deviceId ? c.video.deviceId.exact : 'any'); if (c.video && c.video.deviceId) { const i = window.__mru.findIndex(d => d.deviceId === c.video.deviceId.exact); if (i > 0) window.__mru.unshift(window.__mru.splice(i, 1)[0]); } const st = cv.captureStream(5); const tr = st.getVideoTracks()[0]; const id = c.video && c.video.deviceId ? c.video.deviceId.exact : 'b'; tr.getSettings = () => ({ deviceId: id }); return st; } } }); }""")
    pg.click("#dev-back"); pg.click("[data-dev=seedsigner]"); pg.click("#dev-scan"); pg.wait_for_timeout(500)
    check("scanning a device's code offers every camera, drawn as in Take a picture", pg.locator("#dev-pick-cam .camopt").count() == 3 and pg.is_visible("#dev-pick-cam") and pg.locator("#dev-pick-cam .camopt svg").count() == 3)
    pg.locator("#dev-pick-cam .camopt").first.click(); pg.wait_for_timeout(400)
    order0 = pg.evaluate("[...document.querySelectorAll('#dev-pick-cam .camopt')].map(b => b.dataset.id).join()")
    pg.locator("#dev-pick-cam .camopt").nth(2).click(); pg.wait_for_timeout(400)
    check("the cameras keep their places when another is chosen (macOS lists the last used first)", pg.evaluate("[...document.querySelectorAll('#dev-pick-cam .camopt')].map(b => b.dataset.id).join()") == order0 and pg.evaluate("__mru[0].deviceId") == "c", order0)
    pg.locator("#dev-pick-cam .camopt").first.click(); pg.wait_for_timeout(400)
    check("choosing the IPEVO opens that camera and marks it chosen", pg.evaluate("__asked[__asked.length - 1]") == "a" and pg.get_attribute("#dev-pick-cam .camopt[data-id=a]", "aria-checked") == "true", str(pg.evaluate("__asked")))
    pg.click("#dev-close"); pg.wait_for_timeout(100)
    pg.click("#add-device"); pg.click("[data-dev=seedsigner]"); pg.click("#dev-scan"); pg.wait_for_timeout(400)
    check("and it is remembered next time", pg.evaluate("__asked[__asked.length - 1]") == "a")
    pg.evaluate("pscanOpen()"); pg.wait_for_timeout(400)
    check("scanning a signed code back offers the same picker", pg.locator("#pscan-pick .camopt").count() == 3 and pg.is_visible("#pscan-pick"))
    pg.evaluate("pscanClose(); openScan()"); pg.wait_for_timeout(400)
    check("so does scanning an address", pg.locator("#scan-pick .camopt").count() == 3 and pg.is_visible("#scan-pick"))
    pg.evaluate("closeScan()"); pg.evaluate("delete navigator.mediaDevices")
    pg.click("#dev-back"); pg.click("[data-dev=seedsigner]"); pg.wait_for_timeout(100)
    check("a SeedSigner is scanned: camera, no file", pg.locator("#dev-scan").count() == 1 and pg.locator("#dev-file").count() == 0)
    check("with where to find its key", "Export Xpub" in pg.inner_text("#dev-one"))

    # ---- reading its key ----
    pg.fill("#dev-text", "[0badf00d/48h/1h/0h/2h]tpubDEADBEEF"); pg.click("#dev-read"); pg.wait_for_timeout(200)
    check("a test-network key is refused for a mainnet vault", "test network" in pg.inner_text("#dev-got") and pg.locator("#dev-add").count() == 0, pg.inner_text("#dev-got"))
    pg.fill("#dev-text", "84 single-sig export"); pg.click("#dev-read"); pg.wait_for_timeout(200)
    check("an export with no multisig key says so rather than picking a single-sig key", "none of them the multisig" in pg.inner_text("#dev-got") and pg.locator("#dev-add").count() == 0, pg.inner_text("#dev-got"))
    pg.fill("#dev-text", f"[{SS['fingerprint']}/48h/0h/0h/2h]{SS['xpub']}"); pg.click("#dev-read"); pg.wait_for_timeout(200)
    check("its key is found, fingerprint shown", SS["fingerprint"] in pg.inner_text("#dev-got") and pg.locator("#dev-add").count() == 1)
    check("the usual multisig path draws no warning", pg.locator("#dev-got .warnbox").count() == 0)
    pg.locator("#dev-modal .box").screenshot(path=str(out / "signers-device-found.png"))
    pg.click("#dev-add"); pg.wait_for_timeout(200)
    check("the modal closes", not pg.is_visible("#dev-modal"))
    check("the device row carries its drawing", pg.locator("#setup-done .keyrow").count() == 2 and pg.locator("#setup-done .devart svg").count() == 1)
    k = pg.evaluate("setup.keys[1]")
    check("the key is kept as its expression, origin and all", k["expression"] == f"[{SS['fingerprint']}/48h/0h/0h/2h]{SS['xpub']}" and k["device"] == "seedsigner", json.dumps(k)[:200])
    w = pg.inner_text("#setup-warn")
    check("mixing a card with a device says the card's key is fixed and linkable", "fixed" in w and "linked" in w, w)
    check("no quorum warning while no maker holds 2 keys", "reach the quorum" not in w)

    pg.click("#add-device"); pg.click("[data-dev=seedsigner]"); pg.fill("#dev-text", f"[{SS['fingerprint']}/48h/0h/0h/2h]{SS['xpub']}"); pg.click("#dev-read"); pg.wait_for_timeout(200)
    check("the same device cannot be added twice", "already in this vault" in pg.inner_text("#dev-got") and pg.locator("#dev-add").count() == 0)
    pg.click("#dev-close")

    # ---- a second YubiKey: one maker now holds the quorum ----
    pg.evaluate(f"__cards = [{json.dumps(card(C1, 111))}, {json.dumps(card(C2, 222))}]"); pg.wait_for_timeout(2600)
    pg.click("#use-btn"); pg.wait_for_timeout(200)
    w = pg.inner_text("#setup-warn")
    check("two YubiKeys in a 2-of-3 with a SeedSigner: the quorum warning names the maker", "Yubico" in w and "reach the quorum" in w, w)
    check("all three keys: finish is on, and adding is off", pg.is_enabled("#setup-finish") and not pg.is_enabled("#add-device"))
    pg.screenshot(path=str(out / "signers-setup.png"), full_page=True)
    pg.fill("#setup-name", "mixed"); pg.click("#setup-finish"); pg.wait_for_timeout(500)
    c = pg.evaluate("__created")
    ok = c and c["cmd"] == "signers" and c["a"]["threshold"] == 2 and c["a"]["network"] == "mainnet" and len(c["a"]["keys"]) == 3
    check("a vault with a device in it is made as signers", bool(ok), json.dumps(c)[:300])
    ks = c["a"]["keys"] if c else []
    check("cards go as cards, the device as its expression", sorted(k.get("card") or "" for k in ks) == sorted(["", C1, C2]) and sum(1 for k in ks if k.get("expression")) == 1 and all(k.get("device") for k in ks), json.dumps(ks)[:300])

    # ---- the Keys screen of the vault just made ----
    pg.wait_for_timeout(300)
    pg.evaluate("vault = store.vaults[0]; show('keys')"); pg.wait_for_timeout(500)
    kt = pg.inner_text("#keys-list")
    check("the Keys screen draws every signer", pg.locator("#keys-list .devart svg").count() == 3, str(pg.locator("#keys-list .devart svg").count()))
    check("the cards are shown as fixed keys, the device by name", kt.count("a fixed key in this vault") == 2 and "SeedSigner" in kt, kt[:300])
    sn = pg.inner_text("#seal-note")
    check("the Keys screen does not claim a card has seed words", "cannot be copied off it" in sn and "each on its own device from its own seed words" not in sn, sn)
    check("nor that Sparrow is known to read a card's plain key", "not yet tested" in pg.evaluate("document.querySelector('#keys-desc-ext').textContent"))
    pg.screenshot(path=str(out / "signers-keys.png"), full_page=True)

    # ---- signing a spend of the mixed vault ----
    cer = dict(ours=True, inputs=1, need=2, have=0, ready=False, fee=282, txid="ab" * 32, bytes=600,
               pays=[dict(address="bc1q08zdgswgey4kqeau8af0v7ak8e72aqdpnhhsc45uqdush8qs44zs4zwyh7", sats=30000, ours=False), dict(address="bc1qchange", sats=69718, ours=True)],
               keys=[dict(name=k["name"], pubkey=k["pubkey"], signed=0) for k in MIXED["keys"]])
    pg.evaluate(f"__cards = []; window.__cer = {json.dumps(cer)}; show('sign')"); pg.wait_for_timeout(2600)
    check("with no card in, the Sign screen turns to the device", pg.is_visible("#dev-sign-row") and not pg.is_visible("#act-row"), pg.inner_text("#sign-title"))
    check("it names the device and says change shows as change", "SeedSigner" in pg.inner_text("#sign-title") and "change should show as change" in pg.inner_text("#sign-text"), pg.inner_text("#sign-text"))
    check("and that a card would do too", "Or plug in" in pg.inner_text("#sign-text"))
    check("the device's tile carries its drawing", pg.locator("#cer-keys .devart svg").count() == 1)
    check("the guide shows the device, not a key to plug in", pg.locator("#guide .devguide svg").count() == 1)
    pg.click("#dev-sign-show"); pg.wait_for_timeout(300)
    check("Show the code opens the animated UR code", pg.is_visible("#qr-panel") and pg.evaluate("qrShow.fmt") == "ur" and "frame" in pg.inner_text("#qr-title").lower())   # the label is uppercased by CSS
    pg.click("#qr-format"); pg.wait_for_timeout(200)
    check("it can switch to BBQr for a Coldcard Q", pg.evaluate("qrShow.fmt") == "bbqr" and "BBQr" in pg.inner_text("#qr-note"), pg.inner_text("#qr-note"))
    pg.screenshot(path=str(out / "signers-sign.png"), full_page=True)
    # the signed code comes back animated: a fake camera, and the decoder fed the device's parts in turn
    pg.evaluate("""() => { const cv = document.createElement('canvas'); cv.width = 64; cv.height = 48; const x = cv.getContext('2d'); setInterval(() => { x.fillStyle = '#' + Math.floor(Math.random() * 999); x.fillRect(0, 0, 64, 48); }, 50);
      window.camOpen = async () => cv.captureStream(10);
      // a late camera: only fountain mixtures, numbered above the total
      const seq = ['ur:crypto-psbt/7-2/aaa', 'ur:crypto-psbt/7-2/aaa', 'ur:crypto-psbt/9-2/bbb']; let i = 0;
      window.__notes = []; new MutationObserver(() => __notes.push(document.querySelector('#pscan-note').textContent)).observe(document.querySelector('#pscan-note'), { childList: true, characterData: true, subtree: true });
      window.jsQR = () => (i < seq.length ? { data: seq[i++] } : null); }""")
    pg.click("#dev-sign-scan"); pg.wait_for_timeout(1800)
    imports = pg.evaluate("window.__imports || []")
    check("each new part is tried once, a repeated part is not", len(imports) == 2 and imports[1].count("\n") == 1, json.dumps(imports))
    notes = pg.evaluate("__notes")
    check("while reading, progress is the pieces the decoder recovered — never frames against the total ('7 of 2')", any("1 of 2 pieces" in x for x in notes) and not any(" of 2 so far" in x and "pieces" not in x for x in notes), json.dumps(notes))
    check("once complete the scanner closes and the signature counts", not pg.is_visible("#pscan-modal") and "1 of 2 signatures" in pg.inner_text("#toast"), pg.inner_text("#toast"))
    pg.wait_for_timeout(300)
    check("the device's tile then reads signed", "signed" in pg.inner_text("#cer-keys"), pg.inner_text("#cer-keys"))
    # a card of the vault plugged in takes over: the card path, with its PIN
    pg.evaluate(f"__cards = [{json.dumps(card(C1, 111))}]"); pg.wait_for_timeout(2600)
    check("a plugged-in card of this vault gets the PIN path", pg.is_visible("#pin-field") and not pg.is_visible("#dev-sign-row"), pg.inner_text("#sign-title"))

    # ---- removing a key, and cards alone keep the card path ----
    pg.evaluate(f"__cards = [{json.dumps(card(C1, 111))}, {json.dumps(card(C2, 222))}]; setup = {{ k: 2, n: 2, keys: [], network: 'mainnet' }}; show('setup')"); pg.wait_for_timeout(300)
    pg.click("#use-btn"); pg.wait_for_timeout(2600); pg.click("#use-btn"); pg.wait_for_timeout(200)
    pg.click("[data-drop='1']"); pg.wait_for_timeout(100)
    check("a key can be taken out again", pg.evaluate("setup.keys.length") == 1)
    pg.wait_for_timeout(2600); pg.click("#use-btn"); pg.wait_for_timeout(200)
    pg.click("#setup-finish"); pg.wait_for_timeout(300)
    c = pg.evaluate("__created")
    check("cards alone are still made as a card vault, with only card fields", c["cmd"] == "cards" and all(set(k) == {"name", "serial", "pubkey"} for k in c["a"]["keys"]), json.dumps(c)[:300])

    # ---- a single-key vault has no room for a device ----
    pg.evaluate("setup = { k: 1, n: 1, keys: [], network: 'mainnet' }; show('setup')"); pg.wait_for_timeout(300)
    check("a one-key vault does not offer a device", not pg.is_visible("#add-device"))

    check("no page errors", not errs, "; ".join(errs[:3]))
    print("unmocked:", sorted(set(pg.evaluate("__unmocked"))))
    b.close()
print(f"\n{len(fails)} failed" if fails else "\nall passed")
audit.finish()
sys.exit(1 if fails else 0)
