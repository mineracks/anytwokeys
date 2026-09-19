#!/usr/bin/env python3
"""Home screen in WebKit against a mocked backend: currency worked out from the device (real browser locale and
   timezone), dates / amounts / values in the activity list, the sats-or-money balance, the Settings selector.
   usage: <python with playwright> test/ui-home.py <outdir>      exits non-zero on any failed check"""
import json, sys, pathlib
from playwright.sync_api import sync_playwright
here = pathlib.Path(__file__).resolve().parent; src = here.parent / "src"
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
K = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe", "038410f240613b33fe73d968f331ca82eb1fbd018c6189f89f8c5f9b9fb26f6a47"]
vault = lambda net: dict(name="demo vault", network=net, threshold=2, keys=[dict(name=n, serial=i + 1, pubkey=k) for i, (n, k) in enumerate(zip(["laptop", "keyring", "off-site"], K))], descriptor="wsh(…)", address="bc1qexample", created=0, chain_code=None, labels={}, seal=dict(shares=[], sealed=[]))
T = ["758adfd2a98e" + "0" * 52, "bcd3fb3da2e6" + "1" * 52, "7dbcc8c5894e" + "2" * 52, "aaaa11112222" + "3" * 52]
MOCK = """
window.__state = { fiat: %(fiat)s, asked: null, fail: false, os: %(os)s };
const V = %(vault)s; window.V = V;
window.__contacts = []; window.__extra = [];
const store = () => ({ forms: {}, contacts: __contacts, vaults: [V].concat(__extra), settings: { network: V.network, custom: false, electrum: '', explorer: '', gap: 20, fiat: __state.fiat, prices: '' }, seen: {}, last_psbt: null, last_message: null });
const H = {
  label_set: (a) => { V.labels = V.labels || {}; if (a.label) V.labels[a.key] = a.label; else delete V.labels[a.key]; return store(); },
  note_set: (a) => { window.__noteCalls = (window.__noteCalls || 0) + 1; V.notes = V.notes || {}; if (a.note.trim()) V.notes[a.key] = a.note; else delete V.notes[a.key]; return store(); },
  tx_detail: (a) => { window.__detailAsked = a; if (window.__detailDown) throw 'electrum: connection refused'; const spend = a.txid === '%(t1)s';
    return { txid: a.txid, wtxid: 'ff'.repeat(32), version: 2, locktime: 0, size: 380, vsize: 189, weight: 756, rbf: true, net: spend ? -5190 : 20000, fee: spend ? 190 : 642, fee_rate: spend ? 1.005 : 2.77, height: a.height, tip: 967685,
      block: a.height ? '00000000000000000001' + 'ab'.repeat(22) : null, time: a.height ? 1789648528 : null, proof: a.height ? (window.__badProof ? "the server's proof does NOT lead to that block's root" : 'proved') : 'unconfirmed', position: a.height ? 1757 : null, hex: '0200000001' + 'cd'.repeat(180),
      inputs: spend ? [{ outpoint: '%(t0)s:1', address: 'bc1qvaultaddr', sats: 20000, ours: 0, sequence: 4294967293, witness: 4, signers: ['laptop', 'off-site'] }] : [{ outpoint: 'ee'.repeat(32) + ':0', address: 'bc1psender', sats: 723914, ours: null, sequence: 4294967295, witness: 1, signers: [] }],
      outputs: spend ? [{ n: 0, address: 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4', sats: 5000, kind: 'P2WPKH', ours: null }, { n: 1, address: 'bc1qvaultchange', sats: 14810, kind: 'P2WSH', ours: 2147483648 }] : [{ n: 0, address: 'bc1pother', sats: 703272, kind: 'P2TR', ours: null }, { n: 1, address: 'bc1qvaultaddr', sats: 20000, kind: 'P2WSH', ours: 0 }] }; },
  store_get: store, node_status: (a) => { (window.__nodeAsked = window.__nodeAsked || []).push(a.network); if (window.__nodeDown) throw 'electrum ssl://electrum.mineracks.com:50002: connection refused'; return { electrum: 'ssl://electrum.mineracks.com:50002', height: 967686, ms: 84, ours: true }; },
  contact_save: (a) => { if (!/^(bc1|tb1)/.test(a.address)) throw 'that is not a Bitcoin address'; const c = __contacts.find(c => c.address === a.address); if (c) c.name = a.name; else __contacts.push({ name: a.name, address: a.address, network: a.network, added: 1, used: 0, last_used: 0 }); return store(); },
  contact_delete: (a) => { __contacts = __contacts.filter(c => c.address !== a.address); return store(); }, contact_used: (a) => { __contacts.find(c => c.address === a.address).used += 1; return store(); },
  vault_receive_address: (a) => 'bc1qfreshaddressof' + a.name.replace(/[^a-z]/gi, '') + '0000000000000000000000007', device_locale: () => __state.os, cards_list: () => { throw 'no YubiKey found'; }, spend_ceremony: () => { throw 'no spend in progress'; }, fees_get: () => ({ slow: 1, medium: 2, fast: 3 }),
  settings_set: (a) => { if (a.fiat !== undefined) __state.fiat = a.fiat; return store(); },
  vault_sync: (a) => ({ coins: [{ txid: '%(t0)s', vout: 0, value: 9430, height: 967374 }], balance: a.name === 'other vault' ? 111 : 9430, height: 967685, rotating: false, receive: 'bc1qexample', receive_index: 0, next_change: 0, gap: 20, servers: { electrum: 'ssl://electrum.mineracks.com:50002', explorer: 'https://mempool.mineracks.com' },
    history: [{ txid: '%(t0)s', height: 967374, index: 0 }, { txid: '%(t1)s', height: 967410, index: 0 }, { txid: '%(t2)s', height: 967497, index: 0 }, { txid: '%(t3)s', height: 0, index: 0 }] }),
  vault_activity: (a) => { __state.asked = a; if (__state.fail) throw 'electrum: connection refused'; const q = (p, b) => a.ccy ? { price: p, basis: b } : null; return { ccy: a.ccy, note: null, now: q(114209.6, 'daily 2026-09-19'), rows: [
    { txid: '%(t0)s', time: 1789627614, net: 20000, fee: null, quote: q(107389.64, 'hourly 2026-09-17 06:00 UTC') }, { txid: '%(t1)s', time: 1789648528, net: -5190, fee: 190, quote: q(107768.03, 'hourly 2026-09-17 12:00 UTC') },
    { txid: '%(t2)s', time: 1789699744, net: -5380, fee: 380, quote: null }, { txid: '%(t3)s', time: null, net: 1000, fee: null, quote: null }] }; } };
window.__TAURI__ = { core: { invoke: async (cmd, args) => { if (!H[cmd]) throw 'unmocked: ' + cmd; if (cmd === 'vault_sync' && window.__hold === (args || {}).name) await new Promise(r => window.__release = r);   // a slow server, on demand
  return H[cmd](args || {}); } }, event: { listen: async (name, fn) => { (window.__listeners = window.__listeners || {})[name] = fn; return () => {}; } } };
"""
fails = []
def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + (f" — {detail}" if detail and not ok else ""))
    if not ok: fails.append(name)
def page(b, locale, zone, net="mainnet", fiat="auto", os=None):
    ctx = b.new_context(locale=locale, timezone_id=zone, viewport=dict(width=1300, height=900)); pg = ctx.new_page(); pg.errs = []
    pg.on("pageerror", lambda e: pg.errs.append(str(e)))
    pg.add_init_script(MOCK % dict(vault=json.dumps(vault(net)), fiat=json.dumps(fiat), os=json.dumps(os or dict(region=None, currency=None)), t0=T[0], t1=T[1], t2=T[2], t3=T[3]))
    pg.goto((src / "index.html").as_uri()); pg.wait_for_timeout(900); return pg
with sync_playwright() as p:
    b = p.webkit.launch()
    # ---- the device decides the currency: OS region first, then the language list, then the clock ----
    for os_, locale, zone, want, why in [(dict(region="AU", currency=None), "en-US", "America/New_York", "AUD", "region is AU"),          # an Australian abroad with a US-English Mac
                                          (dict(region="AU", currency="USD"), "en-AU", "Australia/Brisbane", "USD", "currency is set to USD"),  # they overrode it in System Settings
                                          (dict(region="KE", currency=None), "en-GB", "Africa/Nairobi", "USD", "no price for the currency of KE"),
                                          (None, "en-AU", "Australia/Brisbane", "AUD", "language region is AU"), (None, "en-US", "Australia/Brisbane", "AUD", "clock is set to Australia/Brisbane"),
                                          (None, "en-US", "America/New_York", "USD", "US"), (None, "en-GB", "Europe/London", "GBP", "language region is GB"), (None, "de-DE", "Europe/Berlin", "EUR", "DE"), (None, "ja-JP", "Asia/Tokyo", "JPY", "")]:
        pg = page(b, locale, zone, os=os_); got = pg.evaluate("deviceCurrency()")
        check(f"OS {os_ and os_['region']}/{os_ and os_['currency']} · {locale} · {zone} -> {want}", got["ccy"] == want and why in got["why"] and pg.evaluate("__state.asked && __state.asked.ccy") == want, json.dumps(got)); pg.context.close()
    pg = page(b, "en-US", "Etc/UTC")
    for langs, zone, want in [(["fr-CH"], "Europe/Zurich", "CHF"), (["en"], "Pacific/Auckland", "NZD"), (["en"], "Etc/UTC", "USD"), (["en-US"], "Europe/Paris", "EUR"), (["es-AR", "en"], "America/Argentina/Buenos_Aires", "ARS")]:
        got = pg.evaluate("([l, z]) => deviceCurrency({ region: null, currency: null }, l, z)", [langs, zone]); check(f"no OS answer · {langs} · {zone} -> {want}", got["ccy"] == want, json.dumps(got))
    pg.context.close()
    # ---- the activity list and the balance, as an Australian sees them ----
    pg = page(b, "en-AU", "Australia/Brisbane", os=dict(region="AU", currency=None)); rows = pg.locator("#activity tr")
    check("newest first, four rows", rows.count() == 4 and "pending" in rows.nth(0).inner_text())
    r = rows.nth(3).inner_text()
    check("a deposit: local date and time, +sats in green, its value then", "17 Sep" in r and "2026" in r and "4:46" in r and "+20,000 sats" in r and "$21.48 then" in r and "in" in rows.nth(3).locator(".amt").get_attribute("class"), r)
    check("the quote's basis is on hover", "hourly 2026-09-17 06:00 UTC" in rows.nth(3).locator(".amt").get_attribute("title") and "ECB" in rows.nth(3).locator(".amt").get_attribute("title"))
    r = rows.nth(2).inner_text(); check("a spend: minus sign, value then, and the fee", "−5,190 sats" in r and "$5.59 then" in r and "fee 190" in r, r)
    r = rows.nth(1).inner_text(); check("a price that could not be fetched is blank, not zero", "−5,380 sats" in r and "fee 380" in r and "$" not in r, r)
    r = rows.nth(0).inner_text(); check("an unconfirmed one is valued at today's price and says so", "+1,000 sats" in r and "now" in r and "pending" in r, r)
    check("balance leads with sats, money beneath", "9,430" in pg.inner_text("#balance") and "$10.77" in pg.inner_text("#balance-fiat"), pg.inner_text("#balance-fiat"))
    pg.locator("#s-home").screenshot(path=str(out / "home-fiat.png"))
    pg.click("#balance"); check("one click: money leads, sats beneath", "$10.77" in pg.inner_text("#balance") and "AUD" in pg.inner_text("#balance") and "9,430 sats" in pg.inner_text("#balance-fiat"), pg.inner_text("#balance"))
    pg.evaluate("sync()"); pg.wait_for_timeout(500); check("the choice survives a sync", "$10.77" in pg.inner_text("#balance"))
    pg.evaluate("__state.fail = true; sync()"); pg.wait_for_timeout(500)
    check("if dates and amounts cannot be fetched: dashes and the reason, and the balance still shows", "—" in rows.nth(3).inner_text() and "connection refused" in pg.inner_text("#activity-note") and "9,430" in pg.inner_text("#balance"), pg.inner_text("#activity-note"))
    # ---- one transaction: click in, read it, write about it, walk back ----
    pg.evaluate("__contacts.push({ name: 'hardware shop', address: 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4', network: 'mainnet', added: 1, used: 1, last_used: 1 }); sync()"); pg.wait_for_timeout(700)
    rows = pg.locator("#activity tr"); rows.nth(2).click(); pg.wait_for_timeout(600)
    check("a click on an activity row opens that transaction, with the way back in the breadcrumb", pg.locator("#s-tx.on").count() == 1 and pg.evaluate("__detailAsked.txid") == T[1] and pg.inner_text("#crumbs").replace("\n", " ").split()[:5] == "demo vault › Activity ›".split(), pg.inner_text("#crumbs"))
    facts = pg.inner_text("#tx-facts")
    check("a spend: amount, value then and now, when, who it paid (from the address book), change, fee, and WHICH KEYS SIGNED", "−5,190" in facts and "then" in facts and "now" in facts and "hardware shop" in facts and "change #0" in facts and "190 sats" in facts and "laptop and off-site" in facts and "blocks deep" in facts, facts[:400])
    check("the sidebar still says Home, and the title is the date and amount until there is a label", "Home" in [e.inner_text().split("\n")[0] for e in pg.locator(".side .nav.on").all()] and "−5,190 sats" in pg.inner_text("#title"), pg.inner_text("#title"))
    check("blockchain data is folded away, with the proof in its heading", not pg.locator("#tx-chain-body").is_visible() and "the node proved it is in block" in pg.inner_text("#tx-proof"))
    pg.click("#tx-chain summary"); pg.wait_for_timeout(150); chain = pg.inner_text("#tx-chain-body")
    check("opened: ids, size, RBF, block and position, every input and output marked, the raw hex", all(x in chain for x in ["Witness id", "189 vB", "signalled", "1757", "spent", "P2WSH", "laptop, off-site"]) and pg.locator("#tx-hex").is_visible(), chain[:300])
    pg.locator("#s-tx").screenshot(path=str(out / "tx-detail.png"))
    pg.fill("#tx-label", "laptop stand"); pg.keyboard.press("Enter"); pg.wait_for_timeout(400)
    check("a label becomes the title and the last breadcrumb", pg.inner_text("#title") == "laptop stand" and "laptop stand" in pg.inner_text("#crumbs") and "Saved" in pg.inner_text("#tx-saved"), pg.inner_text("#title"))
    pg.click("#tx-note"); pg.keyboard.type("Ordered 17 Sep, invoice 4471.\nAsk Sam about the warranty."); pg.wait_for_timeout(300)
    check("a note is not saved on every keystroke", (pg.evaluate("window.__noteCalls") or 0) == 0 and "not saved yet" in pg.inner_text("#tx-saved"))
    pg.wait_for_timeout(1400)
    check("it saves itself a moment after you stop, line breaks and all", pg.evaluate("window.__noteCalls") == 1 and pg.evaluate("V.notes['tx:' + %s]" % json.dumps(T[1])).count("\n") == 1 and "Saved" in pg.inner_text("#tx-saved"))
    pg.evaluate("window.__badProof = true"); pg.click("#crumbs [data-crumb=home]"); pg.wait_for_timeout(500)
    check("the breadcrumb walks back to Home, where the row now shows its label", pg.locator("#s-home.on").count() == 1 and "laptop stand" in pg.inner_text("#activity"))
    pg.locator("#activity tr").nth(3).click(); pg.wait_for_timeout(600)
    facts = pg.inner_text("#tx-facts")
    check("a deposit reads the other way round, and a proof that does not check out is said in red", "+20,000" in facts and "outside this vault" in facts and "receive #0" in facts and "paid by the sender" in facts and "does NOT lead" in pg.inner_text("#tx-proof") and "warn" in pg.get_attribute("#tx-proof", "class"), facts[:200])
    check("reopening shows the note that belongs to THIS transaction (none), not the last one's", pg.input_value("#tx-note") == "" and pg.input_value("#tx-label") == "")
    pg.evaluate("window.__badProof = false; window.__detailDown = true"); pg.keyboard.press("Escape"); pg.wait_for_timeout(300); pg.locator("#activity tr").nth(2).click(); pg.wait_for_timeout(600)
    check("Escape goes back; with the server down the facts from the list still show, the note is still there, and the failure is named", "−5,190" in pg.inner_text("#tx-facts") and "connection refused" in pg.inner_text("#tx-dl") and "invoice 4471" in pg.input_value("#tx-note"))
    pg.evaluate("window.__detailDown = false; __contacts.length = 0; show('home')"); pg.wait_for_timeout(500)
    # ---- switching vaults while a sync is in flight: the late answer belongs to the vault that was left ----
    pg.evaluate("""() => { __state.fail = false; store.vaults.push(Object.assign({}, store.vaults[0], { name: 'other vault', network: 'signet' })); window.__hold = 'demo vault'; sync(); vault = store.vaults[1]; show('home'); }""")
    pg.wait_for_timeout(300)
    check("the slow sync really is still in flight", pg.evaluate("typeof window.__release") == "function")
    pg.evaluate("window.__hold = null; window.__release()"); pg.wait_for_timeout(600)
    asked = pg.evaluate("__state.asked && __state.asked.name")
    check("a sync that returns after the switch is dropped: the other vault keeps its own balance and no lookup is made for the wrong vault", pg.inner_text("#title") == "other vault" and asked == "other vault" and "111" in pg.inner_text("#balance") and "9,430" not in pg.inner_text("#s-home"), f"title={pg.inner_text('#title')} asked={asked} balance={pg.inner_text('#balance')}")
    pg.evaluate("vault = store.vaults[0]; show('home')"); pg.wait_for_timeout(600)
    # ---- the address book on Send ----
    A = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
    pg.evaluate("""() => { __extra = [Object.assign({}, V, { name: 'savings vault', chain_code: 'ab'.repeat(32) }), Object.assign({}, V, { name: 'a signet one', network: 'signet' })]; }""")
    pg.evaluate("""() => { __extra = [Object.assign({}, V, { name: 'savings vault', chain_code: 'ab'.repeat(32) }), Object.assign({}, V, { name: 'a signet one', network: 'signet' })]; }""")
    pg.evaluate("(async () => { store = await window.__TAURI__.core.invoke('store_get'); vault = store.vaults[0]; show('send'); })()"); pg.wait_for_timeout(500)
    pg.fill("#send-to", A); pg.wait_for_timeout(100)
    check("a pasted address that is not in the book offers to be saved", pg.locator("#save-addr").is_visible())
    pg.click("#save-addr"); pg.keyboard.type("exchange deposit"); pg.keyboard.press("Enter"); pg.wait_for_timeout(300)
    check("named and saved from the keyboard; the line now says who it is", "exchange deposit" in pg.inner_text("#to-who") and "not yet paid" in pg.inner_text("#to-who").lower() and pg.evaluate("__contacts.length") == 1, pg.inner_text("#to-who"))
    pg.fill("#send-to", ""); pg.click("#book-to"); pg.wait_for_timeout(200)
    book = pg.inner_text("#book")
    check("the book lists own vaults on THIS network and the saved address", "savings vault" in book and "a signet one" not in book and "demo vault" not in book and "exchange deposit" in book and "next unused address" in book, book)
    pg.click("#book [data-addr]"); pg.wait_for_timeout(150)
    check("picking an entry fills the field, closes the book and moves to the amount", pg.input_value("#send-to") == A and not pg.locator("#book").is_visible() and pg.evaluate("document.activeElement.id") == "send-amount")
    pg.click("#book-to"); pg.click("#book [data-own]"); pg.wait_for_timeout(200)
    check("an own vault gives a FRESH address, and says so", pg.input_value("#send-to").startswith("bc1qfreshaddressofsavingsvault") and "has not used before" in pg.inner_text("#to-who"), pg.inner_text("#to-who"))
    pg.evaluate("__contacts[0].used = 3"); pg.fill("#send-to", A); pg.wait_for_timeout(100)
    check("a reused address says how often, and what that costs", "Paid 3 times before" in pg.inner_text("#to-who") and "link these payments" in pg.inner_text("#to-who"))
    pg.click("#book-to"); pg.wait_for_timeout(150); pg.locator("#book").screenshot(path=str(out / "address-book.png"))
    pg.click("#book [data-del]"); pg.wait_for_timeout(150)
    check("Remove asks first", pg.evaluate("__contacts.length") == 1 and pg.inner_text("#book [data-del]") == "Yes, remove")
    pg.click("#book [data-del]"); pg.wait_for_timeout(300)
    check("and then removes", pg.evaluate("__contacts.length") == 0 and "None yet" in pg.inner_text("#book"))
    pg.keyboard.press("Escape"); check("Escape closes the book", not pg.locator("#book").is_visible())
    pg.evaluate("show('home')"); pg.wait_for_timeout(500)
    # ---- money moved while the window is open: a notice in the window, as well as the system one ----
    pg.evaluate("""() => { __listeners['vault-event']({ payload: { vault: 'savings vault', kind: 'in', title: 'savings vault: 20,000 sats arrived', body: '758adfd2…a98e00 · waiting for a confirmation', txid: 'x' } });
                           __listeners['vault-event']({ payload: { vault: 'demo vault', kind: 'out', title: 'demo vault: 5,190 sats sent', body: 'bcd3fb3d…391e11 · in the mempool', txid: 'y' } }); }""")
    pg.wait_for_timeout(400); cards_ = pg.locator("#notices .notice-card")
    check("an arrival and a spend each get a notice in the window, stacked", cards_.count() == 2 and "20,000 sats arrived" in cards_.nth(0).inner_text() and "in" in cards_.nth(0).get_attribute("class") and "5,190 sats sent" in cards_.nth(1).inner_text())
    nb = cards_.nth(0).bounding_box(); check("top right, clear of the header and of every button row", nb["y"] >= 90 and nb["x"] + nb["width"] <= 1300 and nb["x"] > 800, str(nb))
    pg.locator("#notices").screenshot(path=str(out / "notices.png"))
    cards_.nth(1).locator(".nx").click(); check("× dismisses one", pg.locator("#notices .notice-card").count() == 1)
    pg.locator("#notices .notice-card").first.click(); pg.wait_for_timeout(500)
    check("clicking a notice opens THAT vault", pg.inner_text("#title") == "savings vault" and pg.locator("#notices .notice-card").count() == 0)
    pg.evaluate("vault = store.vaults[0]; show('home')"); pg.wait_for_timeout(500)
    # ---- whose screens are these, and is the node there? ----
    side = pg.inner_text(".side")
    check("the sidebar heads the vault's screens with the vault's name, and keeps the app's own things apart", pg.inner_text("#side-vault").lower() == "demo vault" and "App settings" in side and pg.evaluate("() => { const s = [...document.querySelectorAll('.side .nav[data-go]')].map(n => n.dataset.go); return s.indexOf('settings') === s.length - 1 && !!document.querySelector('.sidefoot .nav[data-go=settings]'); }"), side[:200])
    check("the light is green on a fresh answer and names the node", pg.get_attribute("#node", "data-state") == "online" and pg.inner_text("#node-text") == "Online · mineracks node" and "block 967,686" in pg.get_attribute("#node", "title") and "84 ms" in pg.get_attribute("#node", "title"), pg.inner_text("#node-text"))
    nb = pg.locator("#node").bounding_box(); check("bottom left of the window", nb["x"] < 60 and nb["y"] > 700, str(nb))
    pg.evaluate("window.__nodeDown = true"); pg.click("#node"); pg.wait_for_timeout(300)
    check("no answer: red, says so, and the reason is on hover", pg.get_attribute("#node", "data-state") == "offline" and "Offline" in pg.inner_text("#node-text") and "connection refused" in pg.get_attribute("#node", "title"))
    pg.evaluate("window.__nodeDown = false"); pg.click("#node"); pg.wait_for_timeout(300)
    check("a click asks again", pg.get_attribute("#node", "data-state") == "online")
    pg.locator(".side").screenshot(path=str(out / "sidebar.png"))
    check("labels live with the vault they belong to, on its Home", pg.locator("#s-home #labels-export").count() == 1 and pg.locator("#s-settings #labels-export").count() == 0)
    # ---- Settings ----
    pg.evaluate("__state.fail = false; show('settings')"); pg.wait_for_timeout(300)
    check("Settings: automatic names the currency and why", "Automatic: AUD" in pg.locator("#fiat-pick option").first.inner_text() and "region is AU" in pg.inner_text("#fiat-note"), pg.inner_text("#fiat-note"))
    pg.select_option("#fiat-pick", "EUR"); pg.wait_for_timeout(300); pg.evaluate("show('home')"); pg.wait_for_timeout(600)
    check("choosing EUR asks for EUR and draws euros", pg.evaluate("__state.asked.ccy") == "EUR" and ("€" in pg.inner_text("#activity") or "EUR" in pg.inner_text("#activity")), pg.inner_text("#activity")[:120])
    pg.evaluate("show('settings')"); pg.select_option("#fiat-pick", ""); pg.wait_for_timeout(300); pg.evaluate("show('home')"); pg.wait_for_timeout(600)
    check("None: no money anywhere, sats lead again", pg.evaluate("__state.asked.ccy") == "" and "EUR" not in pg.inner_text("#s-home") and "$" not in pg.inner_text("#s-home") and "9,430" in pg.inner_text("#balance"))
    check("no page errors", not pg.errs, "; ".join(pg.errs)); pg.context.close()
    pg = page(b, "en-AU", "Australia/Brisbane", net="signet"); 
    check("a signet vault asks about the SIGNET server", 'signet' in pg.evaluate("window.__nodeAsked || []"))
    check("signet: dates and amounts, never a value", pg.evaluate("__state.asked.ccy") == "" and "+20,000 sats" in pg.inner_text("#activity") and "$" not in pg.inner_text("#s-home"))
    b.close()
sys.exit(1 if fails else 0)
