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
const survey = () => NEW.gone || NEW.pubkey ? [] : [{ reader: NEW.reader || 'Yubico YubiKey OTP+FIDO+CCID', serial: NEW.serial, firmware: NEW.firmware, form: NEW.form || 3, nfc: NEW.nfc == null ? true : NEW.nfc, maker: NEW.maker || 'yubico', device: NEW.device || 'yubikey', pubkey: null, state: NEW.state || 'fresh', why: [],
  pgp: { tries: [NEW.utries, 0, NEW.atries], sig_pin_each: NEW.sig, slots: [false, false, false], fingerprints: false, sig_count: 0, name: false, url: false, login: false, cert: 0, private: false, kdf: false, unread: [] },
  piv: { keys: 0, certs: 0, pin_default: true }, oath: { password: false, accounts: 2 }, otp: NEW.otp ? { slot1: true, slot2: false } : null, otp_usb: NEW.otp, fido: true, config_locked: false, pin_complexity: false }];
const judge = (p, admin) => !p ? null : p.length < (admin ? 8 : 6) ? 'needs at least ' + (admin ? 8 : 6) + ' characters' : (p === '123456' || p === '12345678') ? 'That is the factory PIN every YubiKey ships with.' : null;
const H = {
  lock_status: () => ({ sealed: true, unlocked: true, finished: true, touch: true, idle_minutes: 0, with_screen: true, cards: [], moved: null }), lock_open_mac: () => H.lock_status(),
  cards_list: () => (NEW.gone ? [] : [{ reader: NEW.reader || 'Yubico YubiKey OTP+FIDO+CCID', serial: NEW.serial, version: 'OpenPGP 3.4', pubkey: NEW.pubkey || null, touch: NEW.pubkey ? 'FIXED' : 'OFF', form: NEW.form || 3, nfc: NEW.nfc == null ? true : NEW.nfc, maker: NEW.maker || 'yubico', device: NEW.device || 'yubikey', firmware: NEW.firmware }]).concat(__cards),
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
  card_init_prove: async (a) => { if (NEW.notouch) { NEW.notouch = false; throw 'the key was not touched in time'; } await new Promise(r => setTimeout(r, 500)); if (a.userPin !== NEW.user) throw 'wrong PIN'; NEW.pubkey = a.pubkey; __store.key_begun = []; __store.key_ready = { [a.pubkey]: { name: a.name, serial: a.serial, at: 1791000000, offkey: !!a.offkey } }; return store(); },
  key_setup_skip: (a) => { __store.key_skip = [a.serial]; return store(); },
  card_init_resume_check: (a) => NEW.user === '123456' ? 'fresh' : 'begun',
  seed_wordlist: () => ['abandon', 'ability', 'able', 'art', 'cat', 'dog', 'egg', 'fig', 'gum', 'hat', 'ice', 'zoo'],
  seed_check: (a) => { const n = a.words.trim().split(/\s+/).length; if (n !== 24 && n !== 12) throw 'check the seed words and their checksum'; return n; },
  key_setup_restart: (a) => { __store.key_begun = []; return store(); },
  secret_preview: (a) => { const n = a.source.kind === 'app' ? (a.source.words || 24) : 24, W = Array.from({ length: n - 1 }, () => 'abandon').concat([n === 12 ? 'about' : 'art']).join(' ');
    const typed = (a.source.text || '').trim().split(/\s+/).filter(Boolean).length;
    if (a.source.kind === 'words' && [11, 14, 17, 20, 23].includes(typed)) return { words: null, pubkey: '', endings: typed === 11 ? Array.from({ length: 128 }, (_, i) => 'w' + i) : ['art', 'cat', 'dog', 'egg', 'fig', 'gum', 'hat', 'ice'] };
    if (a.source.kind === 'words' && ![12, 15, 18, 21, 24].includes(typed)) throw 'a phrase is 12, 15, 18, 21 or 24 words; that is ' + typed;
    if (a.source.kind === 'dice' && a.source.text.replace(/\s/g, '').length < 99) throw '42 rolls carry fewer than 256 bits; 99 or more are needed';
    return { words: a.source.kind === 'words' ? null : W, pubkey: '%(k2)s', endings: null }; },
  card_init_key_from: (a) => { if (a.adminPin !== NEW.admin) throw 'admin PIN: wrong PIN'; NEW.made = '%(k2)s'; __store.key_offkey = true; return NEW.made; },
  card_import: (a) => { if (!/^(\w+ ){23}\w+$/.test(a.words.trim())) throw 'a key is 24 words'; if (a.adminPin !== '87654321') throw 'wrong PIN, 2 tries left'; return a.expect; },
  nk_firmware: (a) => ({ uuid: 'ABCD', version: NEW.firmware || '1.9.0', variant: 'lpc55', latest_known: '1.9.1', latest_known_as_of: '9 October 2026', behind: (NEW.firmware || '1.9.0') !== '1.9.1', flashable: !NEW.nrf }),
  nk_firmware_update: async (a) => { const emit = (p) => (window.__listeners['nk-firmware'] || []).forEach(f => f({ payload: p }));
    emit({ phase: 'fetch', pct: 5, text: 'Downloading firmware 1.9.1' }); await new Promise(r => setTimeout(r, 150));
    emit({ phase: 'touch', pct: 10, text: 'Touch the key when it blinks white' }); await new Promise(r => setTimeout(r, 250));
    emit({ phase: 'flash', pct: 50, text: 'Writing 1.9.1: 250 of 497 KB' }); await new Promise(r => setTimeout(r, 150));
    if (NEW.fw_fail) throw 'the key was not touched in time';
    NEW.firmware = '1.9.1'; return { uuid: 'ABCD', version: '1.9.1', variant: 'lpc55', latest_known: '1.9.1', latest_known_as_of: '9 October 2026', behind: false, flashable: true }; },
};
window.__listeners = {};
window.__TAURI__ = { core: { invoke: async (cmd, args) => { __calls.push(cmd); __all.push([cmd, args]); __args[cmd] = args; if (!H[cmd]) throw 'unmocked: ' + cmd; return H[cmd](args || {}); } }, event: { listen: async (name, cb) => { (window.__listeners[name] = window.__listeners[name] || []).push(cb); return () => { window.__listeners[name] = (window.__listeners[name] || []).filter(f => f !== cb); }; } } };
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
    check("both switches are applied and the walk waits for the key to come back", pg.evaluate("__args.card_init_harden.sigPinEach") is True and pg.evaluate("__args.card_init_harden.otpOff") is True and "One secret. Two ways to make it." in body(pg), body(pg))
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

    # ---- a key remembered as "begun" but reset since (factory PINs again) starts over, not at the read-back ----
    pg = page(b, store=dict(key_begun=[31204477])); pg.wait_for_selector("#keyoffer:not([hidden])", timeout=5000)
    check("the offer for a begun key says so", "not finished" in pg.inner_text("#keyoffer"))
    pg.click("#ko-go"); pg.wait_for_timeout(400); t = body(pg)
    check("but a key that answers its factory PIN is taken as fresh: the walk starts from the first step and the memory is cleared", "A new" in t and "Set this key up" in t and pg.evaluate("__store.key_begun") == [] and "card_init_resume_check" in pg.evaluate("__calls"), t)

    # ---- don't ask again; and a PIN being typed elsewhere ----
    pg = page(b); pg.click("#ko-never"); pg.wait_for_timeout(2400)
    check("Don't ask again is remembered", pg.evaluate("__store.key_skip") == [31204477] and not pg.locator("#keyoffer").is_visible())
    pg = page(b, new=dict(firmware="5.4.3")); pg.click("#ko-go"); pg.wait_for_timeout(150)
    check("an older firmware's advisory is shown before anything is set", "YSA-2024-03" in body(pg), body(pg)[:400]); pg.locator(".ki-card").screenshot(path=str(out / "1-meet-advisory.png"))

    # ---- a Nitrokey 3: known by what the card answered (its maker in the AID), never by the reader's name ----
    pg = page(b, new=dict(reader="8352 17074", form=14, nfc=False, maker="nitrokey", device="nitrokey3", firmware="", otp=False)); pg.wait_for_selector("#keyoffer:not([hidden])", timeout=5000)
    t = pg.inner_text("#keyoffer")
    check("a new Nitrokey 3 is offered setup under its own name, though macOS names its reader 8352 17074", "A new Nitrokey 3" in t and "YubiKey" not in t, t)
    check("and is drawn as a Nitrokey, not as a YubiKey shape", pg.locator("#keyoffer .ko-art .c-body").count() >= 1 and pg.locator("#keyoffer .ko-art circle[stroke='var(--gold)']").count() == 0)
    pg.click("#ko-go"); pg.wait_for_timeout(200); t = body(pg)
    pg.wait_for_timeout(300); t = body(pg)
    check("its first step never calls it a YubiKey, reads its firmware over USB, and offers the newer release before any PIN is set", "Nitrokey 3" in t and "YubiKey" not in t and "Firmware 1.9.0; 1.9.1 is out" in t and "before the PINs are set" in t and pg.locator("#ki-fw-go").count() == 1 and "none on a Nitrokey" in t, t)
    check("the firmware check asked nothing of the network: the comparison is against the version built in", "nk_firmware" in pg.evaluate("__calls") and "nk_firmware_update" not in pg.evaluate("__calls"))
    pg.locator(".ki-card").screenshot(path=str(out / "1-meet-nitrokey.png"))
    pg.click("#ki-fw-go"); pg.wait_for_timeout(250); t = body(pg)
    check("the update step shows the white-blink touch instruction and the LED language, with a progress bar", ("Press the key" in t or "Touch the key" in t) and "white" in t and "red" in t and pg.locator(".ki-bar").count() == 1, t)
    pg.locator(".ki-card").screenshot(path=str(out / "1b-firmware-touch.png"))
    pg.wait_for_timeout(600); t = body(pg)
    check("when the key reports the new version the walk says so and continues from the start", "1.9.1 is on the key" in t and pg.locator("#ki-fw-done").count() == 1, t)
    pg.click("#ki-fw-done"); pg.wait_for_timeout(500); t = body(pg)
    check("back at the first step the key is on the newest release known, with nothing more offered", "the newest release known" in t and pg.locator("#ki-fw-go").count() == 0, t)
    # a failed update is said plainly, with a way back and a retry, and costs nothing
    pg = page(b, new=dict(reader="8352 17074", form=14, nfc=False, maker="nitrokey", device="nitrokey3", firmware="", otp=False, fw_fail=True)); pg.wait_for_selector("#keyoffer:not([hidden])", timeout=5000)
    pg.click("#ko-go"); pg.wait_for_timeout(400); pg.click("#ki-fw-go"); pg.wait_for_timeout(900); t = body(pg)
    check("an update that fails says why, that nothing is lost, and offers to try again", "did not go through" in t and "not touched in time" in t and "Nothing on the key is lost" in t and pg.locator("#ki-fw-retry").count() == 1, t)
    pg.click("[data-ki-go=meet]"); pg.wait_for_timeout(400); t = body(pg)
    check("Back returns to the first step with the offer still there", "1.9.1 is out" in t, t)
    pg = page(b, new=dict(reader="8352 17074", form=14, nfc=False, maker="nitrokey", device="nitrokey3", firmware="", otp=False)); pg.wait_for_selector("#keyoffer:not([hidden])", timeout=5000); pg.click("#ko-go"); pg.wait_for_timeout(300)
    # every step's picture is the Nitrokey's own: no gold contact anywhere in the walk
    gold, rings = 0, 0
    for step, prep in (("set", ""), ("readback", ""), ("key", "ki.phase='touch'; kiPaint()"), ("firmware", "ki.fw={phase:'touch',pct:10,text:'t'}; kiPaint()"), ("done", "")):
        pg.evaluate(f"kiGo('{step}'); {prep}"); pg.wait_for_timeout(120)
        gold += pg.locator("#ki-art .g-gold, #ki-art circle[stroke='var(--gold)']").count(); rings += pg.locator("#ki-art .g-ring, #ki-art .c-body").count()
        pg.locator(".ki-card").screenshot(path=str(out / f"nitro-{step}.png"))
    check("every step of the Nitrokey walk draws the Nitrokey, never a YubiKey's gold contact", gold == 0 and rings >= 5, f"gold marks: {gold}, nitrokey marks: {rings}")
    pg.evaluate("kiGo('harden')"); pg.wait_for_timeout(100); t = body(pg)
    check("the one-time-code switch is explained away and cannot be pressed", "no one-time-code keyboard" in t and pg.locator("#ki-otp").is_disabled(), t)
    pg.evaluate("kiGo('key'); ki.phase = 'touch'; kiPaint()"); pg.wait_for_timeout(100); t = body(pg)
    check("the first signature asks for a press on the top of the key, not a gold contact", "Press the top of the key" in t and "gold" not in t, t)
    pg.locator(".ki-card").screenshot(path=str(out / "7-key-touch-nitrokey.png"))
    pg.evaluate("kiClose(); show('setup')"); pg.wait_for_timeout(2500); t = pg.inner_text("#setup-now")
    check("New vault offers to set the Nitrokey up, by its own name", "Nitrokey 3" in t and "Set this key up" in t and "YubiKey" not in t, t)

    # ---- a secret made outside the key: shown and confirmed written BEFORE the key is written; Entropia's 23 words ----
    pg = page(b, new=dict(user="493817", admin="60215934", utries=3, atries=3, sig=True, otp=False))
    pg.wait_for_selector("#keyoffer:not([hidden])", timeout=5000); pg.click("#ko-go"); pg.wait_for_timeout(200)
    pg.evaluate("ki.user = '493817'; ki.admin = '60215934'; kiGo('key')"); pg.wait_for_timeout(100); t = body(pg)
    check("the signing-key step is two cards, each with its own picture and claim, and says what making it outside costs", "One secret" in t and "Only in your key" in t and "A key. And a backup" in t and "existed outside the key once" in t and pg.locator(".ki-way").count() == 2 and pg.locator("#ki-make-out").count() == 1 and pg.locator(".ki-way svg").count() >= 4 and "your touch" in t, t)
    pg.locator(".ki-card").screenshot(path=str(out / "7-key-choice.png"))
    pg.click("#ki-make-out"); pg.wait_for_timeout(100); t = body(pg)
    check("the source step lists the app, drawn words (Entropia), dice and hex", "Entropia" in t and "rolled a die" in t and "64 hex" in t and "Let this app make it" in t, t)
    pg.locator(".ki-card").screenshot(path=str(out / "7a-source.png")); pg.locator("#ki-art").screenshot(path=str(out / "7a-source-art.png"))
    pg.click("#ki-src-next"); pg.wait_for_timeout(300); t = body(pg)
    check("the words are 24 hidden cells in the house grid, none shown yet, and the page cannot be called written until every word has been seen", "Write these 24 words down" in t and pg.locator(".ki-words button").count() == 24 and pg.locator(".ki-words button.on").count() == 0 and pg.locator("#ki-src-written").is_disabled() and "24 not yet shown" in t and "not a wallet seed" in t and K[2][:16] in t, t)
    check("looking at the words wrote nothing to the key", "card_init_key_from" not in pg.evaluate("__calls"))
    pg.click("#ki-word-next"); pg.wait_for_timeout(80); t = body(pg)
    check("the words can go onto metal instead of paper", pg.locator("#ki-metal").count() == 1)
    check("stepping shows one word at a time", pg.locator(".ki-words button.on").count() == 1 and "Word 1 of 24" in t and pg.inner_text(".ki-words button.on") .strip().endswith("abandon") and pg.locator(".ki-words button:not(.on) b").first.inner_text().startswith("\u2022"), t)
    pg.locator(".ki-card").screenshot(path=str(out / "7b-words.png"))
    for i in range(24): pg.click("#ki-word-next"); pg.wait_for_timeout(30)
    check("after every word has been shown the confirmation opens; the last step hides the word again", not pg.locator("#ki-src-written").is_disabled() and pg.locator(".ki-words button.on").count() == 0, body(pg))
    pg.click("#ki-src-written"); pg.click("#ki-write"); pg.wait_for_timeout(900); t = body(pg)
    check("writing it in uses the very words shown, then proves the key with a touch, and the key is remembered as made outside", pg.evaluate("__args.card_init_key_from.source.kind") == "words" and "ready" in t and pg.evaluate("__store.key_ready[Object.keys(__store.key_ready)[0]].offkey") is True and "24 words on your page are its backup" in t, t)
    pg.locator(".ki-card").screenshot(path=str(out / "8-done-offkey.png"))
    # Entropia: 23 drawn words complete to 24 with a choice of eight
    pg = page(b, new=dict(user="493817", admin="60215934", sig=True, otp=False)); pg.wait_for_selector("#keyoffer:not([hidden])", timeout=5000); pg.click("#ko-go"); pg.wait_for_timeout(200)
    pg.evaluate("ki.user = '493817'; ki.admin = '60215934'; kiGo('key')"); pg.click("#ki-make-out"); pg.wait_for_timeout(100)
    pg.click("[data-src=words]"); pg.wait_for_timeout(200)
    check("typing words uses the house grid: numbered hidden boxes with the list behind them, every BIP39 length, and a one-word-short switch for the tablets", pg.locator("#ki-seed .seed-grid input").count() == 24 and pg.locator("#ki-seed .seed-len .opt").count() == 5 and pg.locator("#ki-short").count() == 1 and pg.locator("#ki-src-text").count() == 0)
    pg.click("#ki-short"); pg.wait_for_timeout(250)
    check("one word short of 24 makes 23 boxes", pg.locator("#ki-seed .seed-grid input").count() == 23)
    pg.locator("#ki-seed .seed-grid input").first.focus(); pg.evaluate("navigator.clipboard = null"); pg.evaluate("(() => { const boxes = [...document.querySelectorAll('#ki-seed .seed-grid input')]; boxes.forEach(b => { b.value = 'abandon'; b.dispatchEvent(new Event('blur')); }); })()"); pg.wait_for_timeout(300)
    check("the count says the 23 are on the list and the 24th comes next", "the 24th is chosen next" in pg.inner_text("#ki-seed .seed-count"), pg.inner_text("#ki-seed .seed-count"))
    pg.locator(".ki-card").screenshot(path=str(out / "7a-source-grid.png"))
    pg.click("#ki-src-next"); pg.wait_for_timeout(300); t = body(pg)
    check("23 drawn words bring up their eight valid endings to pick from", "pick one" in t and pg.locator("[data-ending]").count() == 8, t)
    pg.locator(".ki-card").screenshot(path=str(out / "7c-endings.png"))
    pg.click("[data-ending=art]"); pg.wait_for_timeout(700); t = body(pg)
    check("with the ending chosen the typed words are taken as the key, and are not shown again", "The words you typed carry this key" in t and pg.locator(".ki-words").count() == 0 and not pg.locator("#ki-write").is_disabled(), t)
    pg = page(b, new=dict(user="493817", admin="60215934", sig=True, otp=False)); pg.wait_for_selector("#keyoffer:not([hidden])", timeout=5000); pg.click("#ko-go"); pg.wait_for_timeout(200)
    pg.evaluate("ki.user = '493817'; ki.admin = '60215934'; kiGo('key')"); pg.click("#ki-make-out"); pg.click("[data-src=dice]"); pg.wait_for_timeout(100)
    check("dice get a pad: six faces, a delete, the rolls shown in tens and counted against 99", pg.locator("#ki-dice .dice-key").count() == 7 and "0 of 99" in pg.inner_text("#ki-dice-count") and pg.locator("#ki-src-text").count() == 0)
    for i in range(42): pg.click(f"#ki-dice .dice-key[data-roll='{i % 6 + 1}']")
    check("rolls appear as they are entered, in tens, with the faces as dice and a bar filling toward 99", "42 of 99" in pg.inner_text("#ki-dice-count") and pg.locator("#ki-dice-rolls span").count() == 5 and pg.locator("#ki-dice .die-pip").count() == 21 and 41 < float(pg.evaluate("parseFloat($('#ki-dice-bar').style.width)")) < 43)
    pg.locator(".ki-card").screenshot(path=str(out / "7d-dice.png"))
    pg.click("#ki-src-next"); pg.wait_for_timeout(300); t = body(pg)
    check("too few dice rolls are refused with the count", "fewer than 256 bits" in t and "99" in t, t)
    pg.click("[data-src=hex]"); pg.wait_for_timeout(100); pg.fill("#ki-hex", "ab" * 16); pg.wait_for_timeout(80)
    check("hex gets a single mono field that counts toward a phrase length", "32 characters: 12 words" in pg.inner_text("#ki-hex-say") and pg.locator("#ki-src-text").count() == 0, pg.inner_text("#ki-hex-say"))
    pg.click("[data-src=app]"); pg.wait_for_timeout(100)
    check("the app's word-count picker offers every BIP39 length, all inside it", pg.locator("#ki-app-len .opt").count() == 5 and pg.locator("#ki-app-len .opt.on").count() == 1 and pg.locator("#ki-app-len .opt.on").inner_text() == "24")
    pg.click("#ki-app-len .opt[data-n='12']"); pg.click("#ki-src-next"); pg.wait_for_timeout(300); t = body(pg)
    check("the app can make 12 words too, and the write-down screen counts them", "Write these 12 words down" in t and pg.locator(".ki-words button").count() == 12 and pg.evaluate("__args.secret_preview.source.words") == 12, t)
    # ---- the vault's Backup card: a key made outside its card can be written to a spare ----
    pg = page(b, store=dict(key_ready={K[0]: dict(name="laptop", serial=1, at=1791000000, offkey=True)}), cards=[dict(reader="Yubico YubiKey FIDO+CCID", serial=99, version="OpenPGP 3.4", pubkey=None, touch="OFF", form=3, nfc=True, maker="yubico", device="yubikey", firmware="5.7.1")])
    pg.evaluate("vault = store.vaults[0]; renderVaultSettings()"); pg.wait_for_timeout(200); t = pg.inner_text("#vs-backup")
    check("Vault settings shows each card key's backup state: made outside (words) or inside (no copy)", "laptop" in t and "outside" in t and "words are its backup" in t and "off-site" in t and "no copy exists" in t and pg.locator("#vs-backup .bk-open").count() == 1, t)
    check("every screen still sits inside the content column: no section has fallen out of it", pg.evaluate("[...document.querySelectorAll('section.screen')].every(s => s.parentElement.classList.contains('content'))") and pg.evaluate("document.querySelectorAll('section.screen').length") >= 20, pg.evaluate("[...document.querySelectorAll('section.screen')].filter(s => !s.parentElement.classList.contains('content')).map(s => s.id).join(',')"))
    check("the Backup card lives on the vault's own Backup screen, with the recovery line, the drill, and shares and metal beside it", pg.locator("#s-backup #vs-backup").count() == 1 and pg.locator("#s-backup #keys-desc").count() == 1 and pg.locator("#s-backup #vs-drill").count() == 1 and pg.locator("#s-backup #vs-shares-metal").count() == 1 and pg.locator(".nav[data-go=backup]").count() == 1 and pg.locator("#s-keys [data-go=backup]").count() == 1 and pg.locator("#s-vault #vs-backup").count() == 0)
    pg.evaluate("show('backup'); $('#vs-backup .bk-open').click()"); pg.wait_for_timeout(150); t = pg.inner_text("#vs-backup")
    offered = pg.evaluate("[...$('#vs-backup .bk-card').options].map(o => o.textContent)")
    check("writing to a spare asks for the words, the other card (only ones with an empty slot are offered) and its admin PIN, and offers metal for the words", "checked against this key" in t and any("serial 99" in o for o in offered) and not any("27446043" in o for o in offered) and pg.locator("#vs-backup .bk-admin").count() == 1 and pg.locator("#vs-backup .bk-metal").count() == 1, f"{t} offered={offered}")
    pg.locator("#vs-backup").screenshot(path=str(out / "vs-backup.png"))
    check("the Backup card types the words in the house grid too", pg.locator("#vs-backup .seed-grid input").count() == 24 and pg.locator("#vs-backup textarea").count() == 0)
    pg.evaluate("[...document.querySelectorAll('#vs-backup .seed-grid input')].forEach((b, i) => { b.value = i < 23 ? 'abandon' : 'art'; }); $('#vs-backup .bk-admin').value = '87654321'; $('#vs-backup .bk-card').value = 'Yubico YubiKey FIDO+CCID'; $('#vs-backup .bk-go').click()"); pg.wait_for_timeout(300)
    check("the import is asked with the vault's own public key to check the words against", pg.evaluate("__args.card_import.expect") == K[0] and pg.evaluate("__args.card_import.reader") == "Yubico YubiKey FIDO+CCID")
    # ---- a key's card offers to reset its signing part, naming what that costs ----
    pg = page(b, cards=[dict(reader="Yubico YubiKey FIDO+CCID", serial=1, version="OpenPGP 3.4", pubkey=K[0], touch="FIXED", form=4, nfc=False, maker="yubico", device="yubikey", firmware="5.4.3")])
    pg.evaluate("vault = store.vaults[0]; show('keys'); openKeyCard(%r)" % K[0]); pg.wait_for_timeout(300); t = pg.inner_text("#key-detail")
    check("the key's card is its own element, outside the hidden list, and visible", pg.evaluate("!$('#keys-main').contains($('#key-detail')) && $('#key-detail').offsetHeight > 100"))
    check("a plugged-in vault key's card ends with a reset, listing the vaults that lose it, their quorum and network, and asking the admin PIN", "reset this key" in t.lower() and "Household" in t and "mainnet" in t and "any 2 of 2" in t and "loses this cosigner for good" in t and pg.locator("#key-reset-pin").count() == 1 and pg.locator("#key-reset").count() == 1, t)
    pg.screenshot(path=str(out / "key-reset.png"), full_page=True)
    pg = page(b); pg.evaluate("vault = store.vaults[0]; show('keys'); openKeyCard(%r)" % K[1]); pg.wait_for_timeout(300); t = pg.inner_text("#key-detail")
    check("a key that is not plugged in shows the warning but no way to proceed", "reset this key" in t.lower() and "Plug the key in to go on" in t and pg.locator("#key-reset").count() == 0, t)
    pg = page(b, store=dict(key_ready={K[0]: dict(name="laptop", serial=1, at=1791000000, offkey=True)}), cards=[dict(reader="Yubico YubiKey FIDO+CCID", serial=99, version="OpenPGP 3.4", pubkey=None, touch="OFF", form=3, nfc=True, maker="yubico", device="yubikey", firmware="5.7.1")])
    pg.evaluate("vault = store.vaults[0]; show('backup'); $('#vs-backup .bk-open').click()"); pg.wait_for_timeout(150)
    pg.evaluate("[...document.querySelectorAll('#vs-backup .seed-grid input')].forEach((b, i) => { b.value = i < 23 ? 'abandon' : 'art'; }); $('#vs-backup .bk-admin').value = '87654321'; $('#vs-backup .bk-card').value = 'Yubico YubiKey FIDO+CCID'; $('#vs-backup .bk-go').click()"); pg.wait_for_timeout(300)
    check("the import is asked with the vault's own public key to check the words against (again, from the Backup screen)", pg.evaluate("__args.card_import.expect") == K[0])
    check("afterwards the words and PIN are gone from the screen", pg.locator("#vs-backup .seed-grid input").count() == 0 or all(v == "" for v in pg.evaluate("[...document.querySelectorAll('#vs-backup .seed-grid input')].map(i => i.value)")))
    b.close()
print(f"{fails} failed"); sys.exit(1 if fails else 0)
