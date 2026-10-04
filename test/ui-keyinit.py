#!/usr/bin/env python3
"""Setting up a brand-new key: the offer and the walk, in WebKit against a mocked backend.
   usage: <python with playwright> test/ui-keyinit.py <outdir>      exits non-zero on any failed check"""
import json, sys, pathlib
from playwright.sync_api import sync_playwright
here = pathlib.Path(__file__).resolve().parent; src = here.parent / "src"
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
K = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe", "038410f2400000000000000000000000000000000000000000000000000000abcd"]
MOCK = """
window.__calls = []; window.__args = {}; window.__all = [];
document.hasFocus = () => window.__focus !== false;
const NEW = %(new)s;      // the key as the card would describe it: { serial, factory: {user, admin}, ... }
window.NEW = NEW; window.__cards = %(cards)s; window.__store = Object.assign({ key_ready: {}, key_skip: [], key_begun: [] }, %(store)s);
const V = { name: 'Household', network: 'mainnet', threshold: 2, keys: [{ name: 'laptop', serial: 1, pubkey: '%(k0)s' }, { name: 'off-site', serial: 2, pubkey: '%(k1)s' }], descriptor: 'wsh(…)', address: 'bc1qexample', labels: {}, seal: { shares: [], sealed: [] } };
const store = () => Object.assign({ forms: {}, contacts: [], vaults: [V], settings: { network: 'mainnet', custom: false, electrum: '', explorer: '', gap: 20, fiat: 'AUD', prices: '' }, seen: {}, last_psbt: null, last_message: null }, __store);
const survey = () => NEW.gone || NEW.pubkey ? [] : [{ reader: 'Yubico YubiKey OTP+FIDO+CCID', serial: NEW.serial, firmware: NEW.firmware, form: 3, nfc: true, pubkey: null, state: NEW.state || 'fresh', why: [],
  pgp: { tries: [NEW.utries, 0, NEW.atries], sig_pin_each: NEW.sig, slots: [false, false, false], fingerprints: false, sig_count: 0, name: false, url: false, login: false, cert: 0, private: false, kdf: false, unread: [] },
  piv: { keys: 0, certs: 0, pin_default: true }, oath: { password: false, accounts: 2 }, otp: NEW.otp ? { slot1: true, slot2: false } : null, otp_usb: NEW.otp, fido: true, config_locked: false, pin_complexity: false }];
const judge = (p, admin) => !p ? null : p.length < (admin ? 8 : 6) ? 'needs at least ' + (admin ? 8 : 6) + ' characters' : (p === '123456' || p === '12345678') ? 'That is the factory PIN every YubiKey ships with.' : null;
const H = {
  lock_status: () => ({ sealed: true, unlocked: true, finished: true, touch: true, idle_minutes: 0, with_screen: true, cards: [], moved: null }), lock_open_mac: () => H.lock_status(),
  cards_list: () => (NEW.gone ? [] : [{ reader: 'Yubico YubiKey OTP+FIDO+CCID', serial: NEW.serial, version: 'OpenPGP 3.4', pubkey: NEW.pubkey || null, touch: NEW.pubkey ? 'FIXED' : 'OFF', form: 3, nfc: true, firmware: NEW.firmware }]).concat(__cards),
  cards_other: () => [], store_get: () => store(), device_locale: () => ({ locale: 'en_AU', currency: 'AUD' }),
  node_status: () => ({ electrum: 'x', height: 1, ms: 5, own: true }), key_form_seen: () => store(), vault_key_seen: () => store(),
  vault_sync: () => ({ coins: [], balance: 0, height: 1, rotating: false, receive: 'bc1qexample', receive_index: 0, next_change: 0, history: [] }),
  vault_activity: () => ({ rows: [], now: null, ccy: 'AUD' }), labels_status: () => ({}), labels_sync: () => ({}), seal_status: () => ({ cards: [], sealed: [] }), spend_status: () => null, spend_ceremony: () => null,
  card_survey: () => survey(),
  card_pin_judge: (a) => [judge(a.user, false), judge(a.admin, true) || (a.admin && a.admin === a.user ? 'The admin PIN must be different from the key PIN' : null)],
  card_init_pins: (a) => {
    if (NEW.fail_once) { NEW.fail_once = false; NEW.user = a.userPin; return { user_done: true, admin_done: false, not_factory: null, tries: 0, error: 'smartcard service: the card was removed' }; }
    if (!a.userDone) { if (NEW.user !== '123456') { NEW.utries--; return { user_done: false, admin_done: false, not_factory: 'user', tries: NEW.utries, error: null }; } NEW.user = a.userPin; }
    if (NEW.admin !== '12345678') { NEW.atries--; return { user_done: true, admin_done: false, not_factory: 'admin', tries: NEW.atries, error: null }; }
    NEW.admin = a.adminPin; __store.key_begun = [NEW.serial]; return { user_done: true, admin_done: true, not_factory: null, tries: 0, error: null };
  },
  card_init_check: (a) => { if (a.userPin !== NEW.user) { NEW.utries--; throw 'key PIN: wrong PIN, ' + NEW.utries + ' tries left'; } if (a.adminPin !== NEW.admin) { NEW.atries--; throw 'admin PIN: wrong PIN, ' + NEW.atries + ' tries left'; } },
  card_init_harden: (a) => { if (a.adminPin !== NEW.admin) throw 'admin PIN: wrong PIN'; if (a.sigPinEach) NEW.sig = true; if (a.otpOff) { NEW.otp = false; NEW.gone = true; setTimeout(() => { NEW.gone = false; }, 900); } return { sig_pin_each: !!a.sigPinEach, otp_off: !!a.otpOff, notes: [] }; },
  card_init_key: (a) => { if (a.adminPin !== NEW.admin) throw 'admin PIN: wrong PIN'; NEW.made = '%(k2)s'; return NEW.made; },
  card_init_prove: async (a) => { if (NEW.notouch) { NEW.notouch = false; throw 'the key was not touched in time'; } await new Promise(r => setTimeout(r, 500)); if (a.userPin !== NEW.user) throw 'wrong PIN'; NEW.pubkey = a.pubkey; __store.key_begun = []; __store.key_ready = { [a.pubkey]: { name: a.name, serial: a.serial, at: 1791000000 } }; return store(); },
  key_setup_skip: (a) => { __store.key_skip = [a.serial]; return store(); },
};
window.__TAURI__ = { core: { invoke: async (cmd, args) => { __calls.push(cmd); __all.push([cmd, args]); __args[cmd] = args; if (!H[cmd]) throw 'unmocked: ' + cmd; return H[cmd](args || {}); } }, event: { listen: async () => () => {} } };
"""
NEWKEY = dict(serial=31204477, firmware="5.7.1", user="123456", admin="12345678", utries=3, atries=3, sig=False, otp=True)
fails = 0
def check(name, ok, detail=""):
    global fails
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else f" — {detail}"))
    if not ok: fails += 1
def page(b, **kw):
    pg = b.new_page(viewport=dict(width=1180, height=860))
    pg.add_init_script(MOCK % dict(k0=K[0], k1=K[1], k2=K[2], new=json.dumps({**NEWKEY, **kw.get("new", {})}), cards=json.dumps(kw.get("cards", [])), store=json.dumps(kw.get("store", {}))))
    pg.goto((src / "index.html").as_uri()); pg.wait_for_timeout(900); return pg
called = lambda pg, cmd: cmd in pg.evaluate("__calls")
body = lambda pg: pg.inner_text("#ki-body")
with sync_playwright() as p:
    b = p.webkit.launch()
    # ---- the offer: wherever the person is, and never in the way ----
    pg = page(b); pg.wait_for_selector("#keyoffer:not([hidden])", timeout=5000)
    check("a new key is offered setup on the screen the person is on, without taking the focus", pg.locator("#keyoffer").is_visible() and "31 204 477" in pg.inner_text("#keyoffer") and "5.7.1" in pg.inner_text("#keyoffer") and pg.locator("#s-home.on").count() == 1 and pg.evaluate("document.activeElement === document.body"), pg.inner_text("body")[:300])
    check("looking at the key sent it no PIN and wrote nothing", not any(c.startswith("card_init") for c in pg.evaluate("__calls")))
    pg.locator("#keyoffer").screenshot(path=str(out / "offer.png"))
    pg.evaluate("window.__focus = false; window.dispatchEvent(new Event('blur'))"); pg.wait_for_timeout(100)
    check("it waits while the window is not the one in front", not pg.locator("#keyoffer").is_visible())
    pg.evaluate("window.__focus = true; window.dispatchEvent(new Event('focus'))"); pg.wait_for_timeout(100)
    check("and comes back with the window", pg.locator("#keyoffer").is_visible())
    pg.evaluate("show('settings')"); pg.wait_for_timeout(200); check("it follows the person to another screen", pg.locator("#keyoffer").is_visible())
    pg.click("#ko-later"); pg.wait_for_timeout(2400)
    check("Not now puts it away, and the next poll does not bring it back", not pg.locator("#keyoffer").is_visible())
    pg.evaluate("NEW.gone = true"); pg.wait_for_timeout(2300); pg.evaluate("NEW.gone = false"); pg.wait_for_timeout(4400)
    check("unplugging and plugging the key in again offers it again", pg.locator("#keyoffer").is_visible())

    # ---- the walk ----
    pg.click("#ko-go"); pg.wait_for_timeout(200)
    t = body(pg)
    check("step 1 says what the key is, checks its firmware, and lists what is already on it as left alone", pg.locator("#keyinit").is_visible() and "firmware 5.7.1" in t and "No security advisory" in t and "2 accounts" in t and t.count("left alone") == 4 and "Nothing is erased" in t, t)
    pg.locator(".ki-card").screenshot(path=str(out / "1-meet.png"))
    pg.click("[data-ki-go=pins]"); pg.wait_for_timeout(100)
    check("step 2 explains both PINs and what three wrong tries cost", "Three wrong tries" in body(pg) and "locks for good" in body(pg)); pg.locator(".ki-card").screenshot(path=str(out / "2-pins.png"))
    pg.click("[data-ki-go=record]"); pg.wait_for_timeout(100)
    check("step 3 will not go on until the PINs are said to be written down", pg.locator("#ki-next").is_disabled() and "Stonebook" in body(pg) and "31 204 477" in pg.inner_text(".ki-record"))
    pg.fill("#ki-name", "keyring"); check("the record card carries the name being typed", "keyring" in pg.inner_text(".ki-record"))
    pg.locator(".ki-card").screenshot(path=str(out / "3-record.png"))
    pg.click("#ki-written"); pg.click("#ki-next"); pg.wait_for_timeout(150)
    fill = lambda u1, u2, a1, a2: [pg.fill(i, v) for i, v in (("#ki-u1", u1), ("#ki-u2", u2), ("#ki-a1", a1), ("#ki-a2", a2))] and pg.wait_for_timeout(250)
    fill("123456", "123456", "12345678", "12345678")
    check("the factory PINs are refused as new PINs, in words, and the button stays shut", pg.locator("#ki-set").is_disabled() and "factory PIN" in pg.inner_text("#ki-u1-say") and "factory PIN" in pg.inner_text("#ki-a1-say"), body(pg))
    pg.locator(".ki-card").screenshot(path=str(out / "4-set-refused.png"))
    fill("493817", "493818", "60215934", "60215934"); check("a PIN typed two different ways is caught", pg.locator("#ki-set").is_disabled() and "Not the same" in pg.inner_text("#ki-u2-say"))
    fill("493817", "493817", "60215934", "60215934")
    pg.focus("#ki-a2"); pg.keyboard.press("Enter"); pg.wait_for_timeout(150)
    check("Enter in a PIN field never sends anything to the key", not called(pg, "card_init_pins"))
    pg.click("#ki-set"); pg.wait_for_timeout(300)
    check("the PINs are set, and the key is remembered as begun", pg.evaluate("NEW.user") == "493817" and pg.evaluate("NEW.admin") == "60215934" and "read them back" in body(pg).lower(), body(pg))
    pg.fill("#ki-ru", "493817"); pg.fill("#ki-ra", "60215943"); pg.click("#ki-check"); pg.wait_for_timeout(200)
    check("a slip when reading back is caught in the window and costs the key no try", not called(pg, "card_init_check") and "no try was used" in body(pg) and pg.evaluate("NEW.atries") == 3, body(pg))
    pg.locator(".ki-card").screenshot(path=str(out / "5-readback.png"))
    pg.fill("#ki-ru", "cccccbcgtrdjkbhivnnfgrlnhjrcdtbjvjtlhurkdhtd"); pg.fill("#ki-ra", "60215934"); pg.click("#ki-check"); pg.wait_for_timeout(150)
    check("a key typing a one-time code into the field is not sent on", not called(pg, "card_init_check") and "one-time code" in body(pg))
    pg.fill("#ki-ru", "493817"); pg.fill("#ki-ra", "60215934"); pg.click("#ki-check"); pg.wait_for_timeout(250)
    check("read back right, the key itself confirms both", called(pg, "card_init_check") and "Two switches" in body(pg))
    pg.locator(".ki-card").screenshot(path=str(out / "6-harden.png"))
    pg.click("#ki-apply"); pg.wait_for_timeout(2200)
    check("both switches are applied and the walk waits for the key to come back", pg.evaluate("__args.card_init_harden.sigPinEach") is True and pg.evaluate("__args.card_init_harden.otpOff") is True and "Make its signing key" in body(pg), body(pg))
    pg.locator(".ki-card").screenshot(path=str(out / "7-key.png"))
    pg.evaluate("NEW.notouch = true"); pg.click("#ki-make"); pg.wait_for_timeout(300)
    check("a touch that never came is said plainly, with the key already made", "not touched in time" in body(pg) and pg.evaluate("NEW.made") == K[2] and pg.locator("#ki-prove").is_visible())
    pg.click("#ki-prove"); pg.wait_for_timeout(150)
    check("the touch picture and words are on screen while the key waits", "Touch the gold contact" in body(pg) and pg.locator("#ki-art .g-pulse").count() == 1)
    pg.locator(".ki-card").screenshot(path=str(out / "8-touch.png"))
    pg.wait_for_timeout(700)
    check("the first signature checks out and the key is ready, by name", "keyring is ready" in body(pg) and pg.evaluate("store.key_ready['%s'].name" % K[2]) == "keyring" and pg.evaluate("ki.user === '' && ki.admin === ''"), body(pg))
    pg.locator(".ki-card").screenshot(path=str(out / "9-done.png"))
    check("no PIN was ever put in storage or on the clipboard path", "493817" not in pg.evaluate("JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)])"))
    pg.click("#ki-vault"); pg.wait_for_timeout(300)
    t = pg.inner_text("#setup-now")
    check("New vault knows the key by its name and asks for nothing more", pg.locator("#s-setup.on").count() == 1 and pg.input_value("#enrol-name") == "keyring" and "first signature was checked" in t and pg.locator("#enrol-admin").count() == 0 and pg.locator("#use-btn").is_visible(), t)
    pg.locator("#s-setup").screenshot(path=str(out / "10-setup.png"))
    pg.click("#use-btn"); pg.wait_for_timeout(200); check("and one click adds it", "keyring" in pg.inner_text("#setup-done"))
    pg.wait_for_timeout(2300); check("a key that is set up is not offered setup again", not pg.locator("#keyoffer").is_visible())

    # ---- a key that only looked new ----
    pg = page(b, new=dict(user="777777", admin="88888888")); pg.click("#ko-go"); pg.click("[data-ki-go=pins]"); pg.click("[data-ki-go=record]"); pg.click("#ki-written"); pg.click("#ki-next")
    fill("493817", "493817", "60215934", "60215934"); pg.click("#ki-set"); pg.wait_for_timeout(250)
    check("a key whose PIN was already changed stops the walk, says how many tries are left, and changes nothing", "already has a PIN" in body(pg) and "2 remain" in body(pg) and pg.evaluate("NEW.user") == "777777" and pg.evaluate("NEW.atries") == 3, body(pg))
    pg.locator(".ki-card").screenshot(path=str(out / "stop-not-factory.png"))
    pg.click("#ki-end"); check("and closes", not pg.locator("#keyinit").is_visible())

    # ---- the cable pulled between the two PINs ----
    pg = page(b, new=dict(fail_once=True)); pg.click("#ko-go"); pg.click("[data-ki-go=pins]"); pg.click("[data-ki-go=record]"); pg.click("#ki-written"); pg.click("#ki-next")
    fill("493817", "493817", "60215934", "60215934"); pg.click("#ki-set"); pg.wait_for_timeout(300)
    check("a failure after the key PIN was set says so and keeps what was typed", "already set to the one you typed" in body(pg) and pg.input_value("#ki-a1") == "60215934" and pg.evaluate("document.querySelector('#ki-u1').readOnly"), body(pg))
    pg.click("#ki-set"); pg.wait_for_timeout(300)
    check("pressing again carries on without offering the key its factory PIN a second time", pg.evaluate("__args.card_init_pins.userDone") is True and pg.evaluate("NEW.utries") == 3 and pg.evaluate("NEW.admin") == "60215934")

    # ---- begun here, finished later ----
    pg = page(b, new=dict(user="493817", admin="60215934", otp=False), store=dict(key_begun=[31204477])); pg.wait_for_selector("#keyoffer:not([hidden])", timeout=5000)
    check("a key whose PINs were set here earlier is offered finishing, not setup", "Finish setting it up" in pg.inner_text("#keyoffer"), pg.inner_text("#keyoffer"))
    pg.click("#ko-go"); pg.wait_for_timeout(150)
    check("and is asked for its PINs, never for the factory ones", "Enter the PINs you set" in body(pg) and not called(pg, "card_init_pins"))
    pg.fill("#ki-ru", "493817"); pg.fill("#ki-ra", "60215934"); pg.click("#ki-check"); pg.wait_for_timeout(250)
    check("then carries on from the switches, with the one already off shown as off", "Two switches" in body(pg) and "Already off" in body(pg) and pg.locator("#ki-otp").is_disabled(), body(pg))

    # ---- from New vault ----
    pg = page(b); pg.evaluate("show('setup')"); pg.wait_for_timeout(2500)
    t = pg.inner_text("#setup-now")
    check("New vault offers to set a new key up instead of sending the person to other software", "Set this key up" in t and "YubiKey Manager" not in t and pg.locator("#enrol-admin").count() == 0 and not pg.locator("#keyoffer").is_visible(), t)
    pg.locator("#setup-now").screenshot(path=str(out / "setup-new-key.png"))
    pg.click("#enrol-init"); pg.wait_for_timeout(150); check("and opens the same walk", pg.locator("#keyinit").is_visible() and "A new YubiKey 5C NFC" in body(pg))
    pg.click("#ki-close"); pg.wait_for_timeout(100)

    # ---- two keys in the computer at once ----
    other = dict(reader="Yubico YubiKey FIDO+CCID", serial=27446043, version="OpenPGP 3.4", pubkey=K[0], touch="FIXED", form=4, nfc=False, firmware="5.4.3")
    pg = page(b, cards=[other]); pg.evaluate("show('setup')"); pg.wait_for_timeout(2600)
    rows = pg.locator("#setup-now .enrol")
    check("with two keys plugged in, New vault shows both and says to choose", rows.count() == 2 and "2 keys are plugged in" in pg.inner_text("#setup-now") and "31204477" in rows.nth(0).inner_text() and "27446043" in rows.nth(1).inner_text(), pg.inner_text("#setup-now"))
    check("each is offered what suits it: the new one setup, the other its existing key under its own name", rows.nth(0).locator(".enrol-init").count() == 1 and rows.nth(1).locator(".use-btn").count() == 1 and rows.nth(1).locator(".enrol-name").input_value() == "laptop")
    pg.locator("#setup-now").screenshot(path=str(out / "setup-two-keys.png"))
    rows.nth(1).locator(".enrol-name").fill("desk"); pg.wait_for_timeout(2500)
    check("a name typed for the second key survives the poll", pg.locator("#setup-now .enrol").nth(1).locator(".enrol-name").input_value() == "desk")
    pg.locator("#setup-now .enrol").nth(1).locator(".use-btn").click(); pg.wait_for_timeout(200)
    check("the second can be added first, and the other stays on offer", "desk" in pg.inner_text("#setup-done") and pg.locator("#setup-now .enrol").count() == 1 and "31204477" in pg.inner_text("#setup-now"))

    # ---- don't ask again; and a PIN being typed elsewhere ----
    pg = page(b); pg.click("#ko-never"); pg.wait_for_timeout(2400)
    check("Don't ask again is remembered", pg.evaluate("__store.key_skip") == [31204477] and not pg.locator("#keyoffer").is_visible())
    pg = page(b, new=dict(firmware="5.4.3")); pg.click("#ko-go"); pg.wait_for_timeout(150)
    check("an older firmware's advisory is shown before anything is set", "YSA-2024-03" in body(pg), body(pg)[:400]); pg.locator(".ki-card").screenshot(path=str(out / "1-meet-advisory.png"))
    b.close()
print(f"{fails} failed"); sys.exit(1 if fails else 0)
