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
const V = %(vault)s;
const store = () => ({ forms: {}, vaults: [V], settings: { network: V.network, custom: false, electrum: '', explorer: '', gap: 20, fiat: __state.fiat, prices: '' }, seen: {}, last_psbt: null, last_message: null });
const H = { store_get: store, device_locale: () => __state.os, cards_list: () => { throw 'no YubiKey found'; }, spend_ceremony: () => { throw 'no spend in progress'; }, fees_get: () => ({ slow: 1, medium: 2, fast: 3 }),
  settings_set: (a) => { if (a.fiat !== undefined) __state.fiat = a.fiat; return store(); },
  vault_sync: () => ({ coins: [{ txid: '%(t0)s', vout: 0, value: 9430, height: 967374 }], balance: 9430, height: 967685, rotating: false, receive: 'bc1qexample', receive_index: 0, next_change: 0, gap: 20, servers: { electrum: 'ssl://electrum.mineracks.com:50002', explorer: 'https://mempool.mineracks.com' },
    history: [{ txid: '%(t0)s', height: 967374, index: 0 }, { txid: '%(t1)s', height: 967410, index: 0 }, { txid: '%(t2)s', height: 967497, index: 0 }, { txid: '%(t3)s', height: 0, index: 0 }] }),
  vault_activity: (a) => { __state.asked = a; if (__state.fail) throw 'electrum: connection refused'; const q = (p, b) => a.ccy ? { price: p, basis: b } : null; return { ccy: a.ccy, note: null, now: q(114209.6, 'daily 2026-09-19'), rows: [
    { txid: '%(t0)s', time: 1789627614, net: 20000, fee: null, quote: q(107389.64, 'hourly 2026-09-17 06:00 UTC') }, { txid: '%(t1)s', time: 1789648528, net: -5190, fee: 190, quote: q(107768.03, 'hourly 2026-09-17 12:00 UTC') },
    { txid: '%(t2)s', time: 1789699744, net: -5380, fee: 380, quote: null }, { txid: '%(t3)s', time: null, net: 1000, fee: null, quote: null }] }; } };
window.__TAURI__ = { core: { invoke: async (cmd, args) => { if (!H[cmd]) throw 'unmocked: ' + cmd; return H[cmd](args || {}); } }, event: { listen: async () => () => {} } };
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
    # ---- Settings ----
    pg.evaluate("__state.fail = false; show('settings')"); pg.wait_for_timeout(300)
    check("Settings: automatic names the currency and why", "Automatic: AUD" in pg.locator("#fiat-pick option").first.inner_text() and "region is AU" in pg.inner_text("#fiat-note"), pg.inner_text("#fiat-note"))
    pg.select_option("#fiat-pick", "EUR"); pg.wait_for_timeout(300); pg.evaluate("show('home')"); pg.wait_for_timeout(600)
    check("choosing EUR asks for EUR and draws euros", pg.evaluate("__state.asked.ccy") == "EUR" and ("€" in pg.inner_text("#activity") or "EUR" in pg.inner_text("#activity")), pg.inner_text("#activity")[:120])
    pg.evaluate("show('settings')"); pg.select_option("#fiat-pick", ""); pg.wait_for_timeout(300); pg.evaluate("show('home')"); pg.wait_for_timeout(600)
    check("None: no money anywhere, sats lead again", pg.evaluate("__state.asked.ccy") == "" and "EUR" not in pg.inner_text("#s-home") and "$" not in pg.inner_text("#s-home") and "9,430" in pg.inner_text("#balance"))
    check("no page errors", not pg.errs, "; ".join(pg.errs)); pg.context.close()
    pg = page(b, "en-AU", "Australia/Brisbane", net="signet"); 
    check("signet: dates and amounts, never a value", pg.evaluate("__state.asked.ccy") == "" and "+20,000 sats" in pg.inner_text("#activity") and "$" not in pg.inner_text("#s-home"))
    b.close()
sys.exit(1 if fails else 0)
