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
const store = () => ({ forms: {}, contacts: __contacts, vaults: [V].concat(__extra), settings: { network: V.network, custom: false, electrum: '', explorer: '', gap: 20, fiat: __state.fiat, prices: '', offline: !!window.__offline, drill_days: window.__drillDays === undefined ? 90 : window.__drillDays, cost_method: window.__method || 'utxo' }, seen: {}, last_psbt: null, last_message: null });
const H = {
  attach_fetch: (a) => { (window.__fetched = window.__fetched || []).push(a); if (window.__fetchFails) throw 'the computer that added this file has not finished backing it up yet'; window.__away = (window.__away || []).filter(h => h !== a.sha256); return true; },
  attach_have: (a) => a.hashes.map(h => !(window.__away || []).includes(h)),
  attach_thumb: () => 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7',
  attach_open: (a) => { window.__opened = a; return a.mime === 'application/octet-stream' ? 'shown in its folder: this kind of file is not opened from here' : 'opened'; },
  attach_remove: (a) => { V.files[a.key] = (V.files[a.key] || []).filter(e => e.sha256 !== a.sha256); return store(); },
  attach_paths: (a) => { window.__dropped = a; V.files = V.files || {}; const list = V.files[a.key] = V.files[a.key] || []; const errors = [];
    for (const p of a.paths) { if (/huge/.test(p)) { errors.push('huge.mov is larger than 25 MB'); continue; } const img = /[.](jpe?g|png|heic)$/i.test(p), nm = p.split('/').pop().replace(/[.]heic$/i, '.jpg');
      list.push({ name: nm, mime: img ? 'image/jpeg' : /pdf$/.test(p) ? 'application/pdf' : 'application/octet-stream', size: img ? 412000 : 88000, sha256: (list.length + 1).toString(16).padStart(64, 'a'), t: 1, meta: img ? (a.keepOriginal ? 'original' : 'stripped') : 'n/a', thumb: img ? 'b'.repeat(64) : undefined }); }
    return { store: store(), added: a.paths.length - errors.length, notes: [], errors }; },
  attach_bytes: (a) => { window.__pasted = a; V.files = V.files || {}; (V.files[a.key] = V.files[a.key] || []).push({ name: a.filename, mime: 'image/png', size: 20000, sha256: 'c'.repeat(64), t: 1, meta: 'stripped', thumb: 'd'.repeat(64) }); return { store: store(), added: 1, notes: [], errors: [] }; },
  location_status: () => 'off', location_preview: () => null,
  place_search: (a) => { (window.__asked = window.__asked || []).push(a.q); if (/zzzz/.test(a.q)) return []; return [{ label: '445 Upper Edward Street', detail: 'Spring Hill, Brisbane · Queensland 4000 · Australia', lat: -27.4615, lon: 153.0245 }, { label: '445 Upper Roma Street', detail: 'Brisbane City, Brisbane · Queensland 4000 · Australia', lat: -27.4655, lon: 153.0125 }]; },
  address_flow: (a) => { window.__flowAsked = a; return [{ txid: '%(t0)s', time: 1789627614, delta: 20000 }, { txid: '%(t1)s', time: 1789648528, delta: -20000 }, { txid: '%(t1)s', time: 1789648528, delta: 9430 }]; },
  report_pdf: (a) => { window.__report = a; return '/Users/p/Desktop/anytwokeys-demo-vault-report.pdf'; },
  record_pdf: (a) => { window.__recPdf = a; return '/Users/p/Desktop/anytwokeys-coin.pdf'; },
  tx_pdf: (a) => { window.__pdf = a; if (window.__pdfCancel) throw 'cancelled'; return '/Users/p/Desktop/anytwokeys-demo-vault-758adfd2.pdf'; },
  tx_map: (a) => { window.__mapAsked = a; return window.__noMap ? null : { png: 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7', metres_per_px: 4.0625, width: 640 }; },
  net_offline: (a) => { window.__offline = a.on; return store(); },
  vault_recovery: (a) => { const v = store().vaults.find(v => v.name === a.name) || {}; return v.ext || v.descriptor || ''; },
  descriptor_parse: (a) => { window.__parsedText = a.text; if (!/xpub/.test(a.text)) throw 'only fixed keys in this mock';
    const ks = [...a.text.matchAll(/\[([0-9a-f]{8})\/([^\]]+)\](xpub\w+)/g)].map(m => ({ fingerprint: m[1], path: 'm/' + m[2], xpub: m[3] }));
    return { threshold: 2, keys: ks.map(k => '[' + k.fingerprint + '/' + k.path.slice(2) + ']' + k.xpub), descriptor: a.text, address: 'bc1q3wtn9xskmmkp4m0zpagq6rngjujj0tfhvzt9xemlaanzgjul7rqq90hsk4', rotating: true, ext: ks, kind: 'sortedmulti' }; },
  vault_restore: (a) => { window.__restored = a; const v = { name: a.name, network: 'mainnet', threshold: 2, keys: a.names.map((n, i) => ({ name: n, serial: 0, pubkey: ['[dc567276/48h/0h/0h/2h]xpub6DiYrf', '[81ce8a63/48h/0h/0h/2h]xpub6E6Ckr', '[88725028/48h/0h/0h/2h]xpub6Dtep'][i] + 'AAAA' })),
    descriptor: a.text, ext: a.text, address: 'bc1q3wtn9xskmmkp4m0zpagq6rngjujj0tfhvzt9xemlaanzgjul7rqq90hsk4', created: 1789600000, labels: {}, notes: {}, files: {}, more: {}, seal: { shares: [], sealed: [] } };
    __extra.push(v); return v; },
  drill_every: (a) => { window.__drillDays = a.days; return store(); },
  vault_ext_rewrite: (a) => { window.__rewrite = a; const v = __extra.find(v => v.name === a.name); if (v) { v.ext = a.text; v.descriptor = a.text; } return store(); },
  cost_method_set: (a) => { window.__method = a.method; return store(); },
  vault_cost_method: (a) => { const v = store().vaults.find(v => v.name === a.name); window.__vaultMethod = a; V.cost_method = a.method; return store(); },
  location_now: () => { if (window.__locDenied) throw 'Location Services are off for Any Two Keys. Turn them on in System Settings › Privacy & Security › Location Services, then try again.'; return { lat: -27.4598, lon: 153.0301, acc: 30, name: 'Brisbane, Australia' }; },
  ctx_set: (a) => { window.__ctxSet = a; V.more = V.more || {}; const r = V.more['tx:' + a.txid] = V.more['tx:' + a.txid] || {}; if (a.remove) r.ctx = { removed: true, t: 9 }; else { r.ctx = Object.assign({ t: 9, how: 'noted', device: { name: 'Test Mac' } }, r.ctx || {}, { place: a.place, named: true }); delete r.ctx.geo; delete r.ctx.picked; if (a.lat !== null && a.lat !== undefined) { r.ctx.geo = { lat: a.lat, lon: a.lon, acc: 0 }; r.ctx.picked = true; } } return store(); },
  label_set: (a) => { V.labels = V.labels || {}; if (a.label) V.labels[a.key] = a.label; else delete V.labels[a.key]; return store(); },
  note_set: (a) => { window.__noteCalls = (window.__noteCalls || 0) + 1; V.notes = V.notes || {}; if (a.note.trim()) V.notes[a.key] = a.note; else delete V.notes[a.key]; return store(); },
  tx_detail: (a) => { window.__detailAsked = a; if (window.__detailDown) throw 'electrum: connection refused'; const spend = a.txid === '%(t1)s';
    return { txid: a.txid, wtxid: 'ff'.repeat(32), version: 2, locktime: 0, size: 380, vsize: 189, weight: 756, rbf: true, net: spend ? -5190 : 20000, fee: spend ? 190 : 642, fee_rate: spend ? 1.005 : 2.77, height: a.height, tip: 967685,
      block: a.height ? '00000000000000000001' + 'ab'.repeat(22) : null, time: a.height ? 1789648528 : null, proof: a.height ? (window.__badProof ? "the server's proof does NOT lead to that block's root" : 'proved') : 'unconfirmed', position: a.height ? 1757 : null, hex: '0200000001' + 'cd'.repeat(180),
      inputs: spend ? [{ outpoint: '%(t0)s:1', address: 'bc1qvaultaddr', sats: 20000, ours: 0, sequence: 4294967293, witness: 4, signers: ['laptop', 'off-site'] }] : [{ outpoint: 'ee'.repeat(32) + ':0', address: 'bc1psender', sats: 723914, ours: null, sequence: 4294967295, witness: 1, signers: [] }],
      outputs: spend ? [{ n: 0, address: 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4', sats: 5000, kind: 'P2WPKH', ours: null, peer: window.__peer ? ['savings vault', 3] : null }, { n: 1, address: 'bc1qvaultchange', sats: 14810, kind: 'P2WSH', ours: 2147483648 }] : [{ n: 0, address: 'bc1pother', sats: 703272, kind: 'P2TR', ours: null }, { n: 1, address: 'bc1qvaultaddr', sats: 20000, kind: 'P2WSH', ours: 0 }],
      peers: spend && window.__peer ? ['savings vault'] : [] }; },
  store_get: store, node_status: (a) => { (window.__nodeAsked = window.__nodeAsked || []).push(a.network); if (window.__nodeDown) throw 'electrum ssl://electrum.mineracks.com:50002: connection refused'; return { electrum: 'ssl://electrum.mineracks.com:50002', height: 967686, ms: 84, ours: true }; },
  contact_save: (a) => { if (!/^(bc1|tb1)/.test(a.address)) throw 'that is not a Bitcoin address'; const c = __contacts.find(c => c.address === a.address); if (c) c.name = a.name; else __contacts.push({ name: a.name, address: a.address, network: a.network, added: 1, used: 0, last_used: 0 }); return store(); },
  contact_delete: (a) => { __contacts = __contacts.filter(c => c.address !== a.address); return store(); }, contact_used: (a) => { __contacts.find(c => c.address === a.address).used += 1; return store(); },
  vault_receive_address: (a) => 'bc1qfreshaddressof' + a.name.replace(/[^a-z]/gi, '') + '0000000000000000000000007', device_locale: () => __state.os, cards_list: () => { throw 'no YubiKey found'; }, spend_ceremony: () => { throw 'no spend in progress'; }, fees_get: () => ({ slow: 1, medium: 2, fast: 3 }),
  settings_set: (a) => { if (a.fiat !== undefined) __state.fiat = a.fiat; return store(); },
  vault_addresses: (a) => [
    { chain: 'receive', index: 0, position: 0, address: 'bc1qexample', current: true, future: false },
    { chain: 'receive', index: 1, position: 1, address: 'bc1qnextone', current: false, future: true },
    { chain: 'change', index: 2147483648, position: 0, address: 'bc1qchangeone', current: true, future: false }],
  seal_read: () => (window.__seal || { cards: 0, foreign: [], found: [] }),
  seal_preview: (a) => { window.__previewed = a.found.length; return window.__cands || []; },
  vault_sync: (a) => ({ coins: [{ txid: '%(t0)s', vout: 0, value: 9430, height: 967374, index: 0 }], balance: a.name === 'other vault' ? 111 : 9430, height: 967685, rotating: !!window.__rotating, receive: 'bc1qexample', receive_index: 0, next_change: 0, gap: 20, servers: { electrum: 'ssl://electrum.mineracks.com:50002', explorer: 'https://mempool.mineracks.com' },
    history: [{ txid: '%(t0)s', height: 967374, index: 0 }, { txid: '%(t1)s', height: 967410, index: 0 }, { txid: '%(t2)s', height: 967497, index: 0 }, { txid: '%(t3)s', height: 0, index: 0 }] }),
  prices_series: (a) => { window.__seriesAsked = (window.__seriesAsked || []).concat([a.from + '..' + a.to]);
    const out = []; for (let t = Date.parse(a.from + 'T00:00:00Z'); t <= Date.parse(a.to + 'T00:00:00Z'); t += 86400000) { const d = new Date(t).toISOString().slice(0, 10); out.push([d, window.__holeDay === d ? null : { price: 110000, basis: 'daily ' + d }]); }
    return { every: 1, points: out }; },
  vault_activity: (a) => { __state.asked = a; if (__state.fail) throw 'electrum: connection refused'; const q = (p, b) => a.ccy ? { price: p, basis: b } : null; return { ccy: a.ccy, note: null, transfers: window.__xfers || {}, now: q(114209.6, 'daily 2026-09-19'), rows: [
    // one coin spent down: 20,000 in; spent, 14,810 change; spent, 9,430 change; and 1,000 not yet in a block
    { txid: '%(t0)s', time: 1789627614, net: 20000, fee: null, quote: q(107389.64, 'hourly 2026-09-17 06:00 UTC'), ins: [], outs: [[0, 20000]] },
    { txid: '%(t1)s', time: 1789648528, net: -5190, fee: 190, quote: q(107768.03, 'hourly 2026-09-17 12:00 UTC'), ins: [['%(t0)s:0', 20000]], outs: [[1, 14810]] },
    { txid: '%(t2)s', time: 1789699744, net: -5380, fee: 380, quote: null, ins: [['%(t1)s:1', 14810]], outs: [[1, 9430]] },
    { txid: '%(t3)s', time: null, net: 1000, fee: null, quote: null, ins: [], outs: [[0, 1000]] }] }; } };
window.__TAURI__ = { core: { invoke: async (cmd, args) => { (window.__n = window.__n || {})[cmd] = (window.__n[cmd] || 0) + 1; if (!H[cmd]) throw 'unmocked: ' + cmd; if (cmd === 'vault_sync' && window.__hold === (args || {}).name) await new Promise(r => window.__release = r);   // a slow server, on demand
  return H[cmd](args || {}); } }, event: { listen: async (name, fn) => { (window.__listeners = window.__listeners || {})[name] = fn; return () => {}; } } };
"""
fails = []
def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + (f" — {detail}" if detail and not ok else ""))
    if not ok: fails.append(name)
import sys as _s; _s.path.insert(0, str(pathlib.Path(__file__).parent)); import audit; audit.install(globals())
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
    pg.evaluate("(async () => { store = await window.__TAURI__.core.invoke('store_get'); vault = store.vaults.find(v => v.name === vault.name); })()"); pg.wait_for_timeout(200)   # what a label sync or a seal does: a fresh vault object
    rows = pg.locator("#activity tr"); rows.nth(2).click(); pg.wait_for_timeout(600)
    check("the block height survives the vault object being replaced: a confirmed transaction is never called unconfirmed", pg.evaluate("__detailAsked.height") == 967410 and "blocks deep" in pg.inner_text("#tx-status"), str(pg.evaluate("__detailAsked.height")))
    check("a click on an activity row opens that transaction, with the way back in the breadcrumb", pg.locator("#s-tx.on").count() == 1 and pg.evaluate("__detailAsked.txid") == T[1] and pg.inner_text("#crumbs").replace("\n", " ").split()[:5] == "demo vault › Activity ›".split(), pg.inner_text("#crumbs"))
    facts = pg.inner_text("#tx-facts")
    check("a spend: amount, value then and now, when, who it paid (from the address book), change, fee, and WHICH KEYS SIGNED", "−5,190" in facts and "then" in facts and "now" in facts and "hardware shop" in facts and "change #0" in facts and "190 sats" in facts and "laptop and off-site" in facts and "blocks deep" in facts, facts[:400])
    check("a transaction is Activity's: the sidebar lights Activity, and the title is the date and amount until there is a label", "Activity" in [e.inner_text().split("\n")[0] for e in pg.locator(".side .nav.on").all()] and "−5,190 sats" in pg.inner_text("#title"), pg.inner_text("#title"))
    check("blockchain data is folded away, with the proof in its heading", not pg.locator("#tx-chain-body").is_visible() and "the node proved it is in block" in pg.inner_text("#tx-proof"))
    pg.click("#tx-chain summary"); pg.wait_for_timeout(150); chain = pg.inner_text("#tx-chain-body")
    check("opened: ids, size, RBF, block and position, every input and output marked, the raw hex", all(x in chain for x in ["Witness id", "189 vB", "signalled", "1757", "spent", "P2WSH", "laptop, off-site"]) and pg.locator("#tx-hex").is_visible(), chain[:300])
    check("older and newer: the list's neighbours are one press away, and the ends are disabled", pg.locator("#tx-prev").is_enabled() and pg.locator("#tx-next").is_enabled() and "3 of 4" in pg.inner_text("#tx-pos"), pg.inner_text("#tx-pos"))
    pg.click("#tx-next"); pg.wait_for_timeout(500); check("Older goes down the list", pg.evaluate("__detailAsked.txid") == T[0] and "4 of 4" not in "" and "+20,000" in pg.inner_text("#tx-facts") and pg.locator("#tx-next").is_disabled(), pg.inner_text("#tx-pos"))
    pg.keyboard.press("ArrowLeft"); pg.wait_for_timeout(500); check("the left arrow key comes back up it", pg.evaluate("__detailAsked.txid") == T[1] and "−5,190" in pg.inner_text("#tx-facts"))
    pg.locator("#s-tx").screenshot(path=str(out / "tx-detail.png"))
    pg.fill("#tx-label", "laptop stand"); pg.keyboard.press("Enter"); pg.wait_for_timeout(400)
    check("a label becomes the title and the last breadcrumb", pg.inner_text("#title") == "laptop stand" and "laptop stand" in pg.inner_text("#crumbs") and "Saved" in pg.inner_text("#tx-saved"), pg.inner_text("#title"))
    pg.click("#tx-note"); pg.keyboard.type("Ordered 17 Sep, invoice 4471.\nAsk Sam about the warranty."); pg.wait_for_timeout(300)
    check("a note is not saved on every keystroke", (pg.evaluate("window.__noteCalls") or 0) == 0 and "Not saved yet" in pg.inner_text("#tx-saved"))
    pg.wait_for_timeout(1400)
    check("it saves itself a moment after you stop, line breaks and all", pg.evaluate("window.__noteCalls") == 1 and pg.evaluate("V.notes['tx:' + %s]" % json.dumps(T[1])).count("\n") == 1 and "Saved" in pg.inner_text("#tx-saved"))
    # ---- attachments: an OS drop arrives as a Tauri event carrying PATHS ----
    zone = pg.locator("#tx-addfiles")
    check("an empty record offers a place to drop files, tall and in words", "Drop files here" in zone.inner_text() and "choose files" in zone.inner_text() and "slim" not in zone.get_attribute("class") and zone.bounding_box()["height"] > 90 and not pg.locator("#tx-files").is_visible(), zone.inner_text())
    pg.locator("#tx-notes").screenshot(path=str(out / "tx-record-empty.png"))
    pg.evaluate("__listeners['tauri://drag-enter']({ payload: {} })"); pg.wait_for_timeout(150)
    check("dragging over the window lights the drop zone and says what letting go does", "over" in zone.get_attribute("class") and "Let go to attach" in zone.inner_text() and "to this transaction" in zone.inner_text(), zone.inner_text())
    pg.locator("#tx-notes").screenshot(path=str(out / "tx-record-over.png"))
    pg.evaluate("__listeners['tauri://drag-leave']({ payload: {} })"); pg.wait_for_timeout(100); check("and goes quiet again when the files are taken away", "over" not in zone.get_attribute("class") and "Drop files here" in zone.inner_text())
    pg.evaluate("__listeners['tauri://drag-enter']({ payload: {} })")
    pg.evaluate("__listeners['tauri://drag-drop']({ payload: { paths: ['/Users/p/Desktop/IMG_0412.HEIC', '/Users/p/Downloads/invoice-4471.pdf', '/Users/p/huge.mov', '/Users/p/tool.bin'] } })"); pg.wait_for_timeout(600)
    tiles_ = pg.locator("#tx-files .att")
    check("dropped files attach to THIS transaction; the oversize one is refused by name and the rest still go in", pg.evaluate("__dropped.key") == "tx:" + T[1] and tiles_.count() == 3 and "larger than 25 MB" in pg.inner_text("#toast") and "3 files attached" in pg.inner_text("#toast") and "over" not in zone.get_attribute("class"), pg.inner_text("#toast"))
    check("with files on the record the zone shrinks to one line and the heading counts them", "slim" in zone.get_attribute("class") and "Drop more files" in zone.inner_text() and zone.bounding_box()["height"] < 60 and pg.inner_text("#tx-files-sum").startswith("3 files · "), "%s | %s | %s" % (zone.get_attribute("class"), zone.inner_text(), zone.bounding_box()["height"]))
    check("a long file name keeps its ending", tiles_.nth(1).locator(".nm .ext").inner_text() == ".pdf")
    check("photos are re-saved without their data unless asked, and a picture shows its thumbnail", pg.evaluate("__dropped.keepOriginal") is False and tiles_.nth(0).locator("img").count() == 1 and "IMG_0412.jpg" in tiles_.nth(0).text_content() and "PDF" in tiles_.nth(1).text_content())
    tiles_.nth(1).click(); pg.wait_for_timeout(200); check("a click opens it with the system viewer", pg.evaluate("__opened.mime") == "application/pdf" and "opened" in pg.inner_text("#toast"))
    tiles_.nth(2).click(); pg.wait_for_timeout(200); check("a file of no viewable kind is shown in its folder, not opened", "not opened from here" in pg.inner_text("#toast"))
    pg.evaluate("window.__away = ['a'.repeat(63) + '1'.padStart(1, 'a')]; window.__away = [V.files['tx:' + %s][0].sha256]; renderFiles()" % json.dumps(T[1])); pg.wait_for_timeout(300)
    check("a file whose bytes are on another computer says so and offers to fetch it", "away" in tiles_.nth(0).get_attribute("class") and "click to fetch" in tiles_.nth(0).inner_text())
    pg.evaluate("window.__fetchFails = true; window.__opened = null"); tiles_.nth(0).click(); pg.wait_for_timeout(300)
    check("if the other computer has not finished backing it up, it says that and opens nothing", "not finished backing it up" in pg.inner_text("#toast") and pg.evaluate("__opened") is None and "away" in tiles_.nth(0).get_attribute("class"))
    pg.evaluate("window.__fetchFails = false"); tiles_.nth(0).click(); pg.wait_for_timeout(500)
    check("a click fetches it (and its thumbnail) from the backup, and it is here", pg.evaluate("__fetched.length") >= 2 and "away" not in tiles_.nth(0).get_attribute("class") and "on this computer now" in pg.inner_text("#toast"), pg.evaluate("JSON.stringify(__fetched)"))
    pg.evaluate("window.__away = []; renderFiles()"); pg.wait_for_timeout(300)
    tiles_.nth(2).hover(); pg.click("#tx-files [data-att-x='2']"); check("remove asks first", tiles_.count() == 3 and pg.inner_text("#tx-files [data-att-x='2']") == "Remove?")
    pg.click("#tx-files [data-att-x='2']"); pg.wait_for_timeout(300); check("and then removes", tiles_.count() == 2)
    pg.locator("#tx-notes").screenshot(path=str(out / "tx-attachments.png"))
    pg.click("#tx-strip"); check("turning the photo cleaning off says what that means", pg.get_attribute("#tx-strip", "aria-checked") == "false" and "location included" in pg.inner_text("#tx-strip-note"))
    pg.evaluate("__listeners['tauri://drag-drop']({ payload: { paths: ['/Users/p/as-taken.jpg'] } })"); pg.wait_for_timeout(400)
    check("and the next photo is kept as taken", pg.evaluate("__dropped.keepOriginal") is True and pg.evaluate("__dropped.paths.length") == 1)
    pg.click("#tx-strip"); pg.hover("#tx-files .att >> nth=2"); pg.click("#tx-files [data-att-x='2']"); pg.click("#tx-files [data-att-x='2']"); pg.wait_for_timeout(300)
    # ---- a picture from a camera (WebKit here has none: the devices are stood in for, the rest is the real code) ----
    pg.evaluate("""() => { const fake = () => { const c = document.createElement('canvas'); c.width = 640; c.height = 480; const x = c.getContext('2d'); x.fillStyle = '#b08420'; x.fillRect(0, 0, 640, 480); return c; };
      window.__camAsked = []; navigator.mediaDevices.getUserMedia = async (q) => { __camAsked.push(q.video.deviceId ? q.video.deviceId.exact : ''); const s = new MediaStream(); s.getVideoTracks = () => [{ getSettings: () => ({ deviceId: q.video.deviceId ? q.video.deviceId.exact : 'builtin' }), stop() {} }]; s.getTracks = () => s.getVideoTracks(); return s; };
      navigator.mediaDevices.enumerateDevices = async () => [{ kind: 'videoinput', deviceId: 'builtin', label: 'MacBook Pro Camera' }, { kind: 'audioinput', deviceId: 'mic', label: 'Microphone' }, { kind: 'videoinput', deviceId: 'phone', label: 'Piers’s iPhone Camera' }];
      const v = document.querySelector('#shot-video'); Object.defineProperty(v, 'videoWidth', { get: () => 640 }); Object.defineProperty(v, 'videoHeight', { get: () => 480 }); Object.defineProperty(v, 'srcObject', { set() {}, get() { return null; } });
      const draw = CanvasRenderingContext2D.prototype.drawImage; CanvasRenderingContext2D.prototype.drawImage = function (src, ...r) { return draw.call(this, src === v ? fake() : src, ...r); }; }""")
    before = tiles_.count(); pg.click("#tx-camera"); pg.wait_for_timeout(400)
    opts = pg.locator("#shot-pick .camopt")
    check("Take a picture draws a button per camera, and only for cameras", pg.locator("#shot-modal").is_visible() and opts.count() == 2 and "iPhone" in pg.inner_text("#shot-pick"))
    check("each button is a sketch of THAT kind of camera, and its name loses the word every one of them ends in",
          [opts.nth(i).get_attribute("data-kind") for i in range(2)] == ["mac", "iphone"] and opts.nth(0).inner_text() == "MacBook Pro" and opts.nth(1).locator("svg").count() == 1, [opts.nth(i).get_attribute("data-kind") for i in range(2)])
    check("the one in use is the one marked", [opts.nth(i).get_attribute("aria-checked") for i in range(2)] == ["true", "false"])
    pg.locator("#shot-modal .box").screenshot(path=str(out / "cameras.png"))
    opts.nth(1).click(); pg.wait_for_timeout(300)
    check("clicking another camera switches to it and marks it", pg.evaluate("__camAsked[__camAsked.length - 1]") == "phone" and opts.nth(1).get_attribute("aria-checked") == "true")
    at = pg.evaluate("txView.txid")
    pg.keyboard.press("ArrowLeft"); pg.wait_for_timeout(300)
    check("and the arrow keys move between them — without stepping to another transaction underneath", pg.evaluate("__camAsked[__camAsked.length - 1]") == "builtin" and pg.evaluate("txView.txid") == at)
    pg.evaluate("localStorage.setItem('a2k-camera', 'phone')"); pg.evaluate("shotClose()"); pg.click("#tx-camera"); pg.wait_for_timeout(400)
    check("the camera you chose last time is the one that opens", pg.evaluate("__camAsked[__camAsked.length - 1]") == "phone", pg.evaluate("JSON.stringify({asked: __camAsked, stored: localStorage.getItem('a2k-camera'), on: document.querySelector('#shot-modal').className})"))
    pg.evaluate("localStorage.setItem('a2k-camera', 'gone-since'); window.__camAsked = []"); pg.evaluate("shotClose()"); pg.click("#tx-camera"); pg.wait_for_timeout(400)
    check("a camera that has been unplugged since is not asked for", "gone-since" not in pg.evaluate("JSON.stringify(__camAsked)"), pg.evaluate("JSON.stringify(__camAsked)"))
    pg.evaluate("localStorage.removeItem('a2k-camera')")
    KINDS = {"FaceTime HD Camera": "mac", "Piers\u2019 iPhone Camera": "iphone", "Desk View Camera": "desk",
             "Studio Display Camera": "display", "Piers\u2019 iPad Camera": "ipad", "OBS Virtual Camera": "virtual",
             "Logitech BRIO": "webcam", "Elgato Facecam": "webcam", "USB Camera": "webcam",
             "IPEVO VZ-X": "doc", "IPEVO DO-CAM": "doc", "AVerVision U50": "doc",
             "IPEVO VZ-R HDMI/USB Document Camera": "doc",   # HDMI in the name of a document camera, not a capture stick
             "Elgato Cam Link 4K": "capture",
             "Canon EOS Webcam Utility": "slr",              # the word webcam in the name of something that is not one
             "Sony ILCE-7M3": "slr", "Logitech Rally Camera": "ptz", "Jabra PanaCast 50": "ptz",
             "GoPro HERO12 Black": "action",
             "Insta360 Link": "webcam"}                      # the one Insta360 that sits on a desk
    got = {n: pg.evaluate("(n) => camKind(n)", n) for n in KINDS}
    check("the kind is read from the name macOS gives the device, and the awkward names land where they belong",
          got == KINDS, " | ".join("%s: %s not %s" % (n, got[n], k) for n, k in KINDS.items() if got[n] != k))
    check("every kind has a sketch of its own, and no two are the same drawing",
          len({pg.evaluate("(k) => camArt(k)", k) for k in set(KINDS.values())}) == len(set(KINDS.values())))
    check("a name keeps what makes it itself, losing only a trailing 'Camera'",
          [pg.evaluate("(n) => camName(n, 0)", n) for n in ["FaceTime HD Camera", "IPEVO VZ-X Document Camera", "IPEVO DO-CAM"]]
          == ["FaceTime HD", "IPEVO VZ-X", "IPEVO DO-CAM"])
    check("the full name macOS gave the device is there to be read on hover, so a wrong sketch can be reported",
          pg.locator("#shot-pick .camopt").nth(0).get_attribute("title") == "MacBook Pro Camera")
    # every sketch side by side, for a person to check against the real things on their desk. It goes in a throwaway
    # element of its own: clobbering the live chooser left the modal in a state the checks below could not use.
    pg.evaluate("""(kinds) => { const d = document.createElement('div'); d.id = 'kind-sheet'; d.className = 'camrow';
        d.style.cssText = 'position:fixed;left:0;top:0;z-index:9999;background:var(--ground);padding:14px;width:660px';
        d.innerHTML = kinds.map(k => `<button type="button" class="camopt" data-kind="${k}" aria-checked="false">${camArt(k)}<span>${k}</span></button>`).join('');
        document.body.appendChild(d); }""",
        ["mac", "iphone", "desk", "ipad", "display", "webcam", "doc", "ptz", "slr", "action", "capture", "virtual"])
    pg.locator("#kind-sheet").screenshot(path=str(out / "camera-kinds.png"))
    pg.evaluate("document.querySelector('#kind-sheet').remove()")
    pg.click("#shot-take"); pg.wait_for_timeout(200)
    check("the first press holds the frame to be looked at; nothing is kept yet", pg.locator("#shot-still").is_visible() and "Keep it" in pg.inner_text("#shot-take") and pg.locator("#shot-again").is_visible() and tiles_.count() == before)
    pg.locator("#shot-modal .box").screenshot(path=str(out / "shot.png"))
    pg.evaluate("window.__pasted = null"); pg.click("#shot-take"); pg.wait_for_timeout(600)
    check("the second press keeps it with THIS transaction as a JPEG, cleaned like any photo", pg.evaluate("__pasted && __pasted.key") == "tx:" + T[1] and pg.evaluate("__pasted.filename").endswith(".jpg") and pg.evaluate("__pasted.keepOriginal") is False and pg.evaluate("atob(__pasted.base64).charCodeAt(0)") == 0xFF and not pg.locator("#shot-modal").is_visible() and tiles_.count() == before + 1)
    # ---- where and on what ----
    check("a transaction from before this was kept says so, and offers no removal", "Nothing was recorded" in pg.inner_text("#tx-ctx-dl") and not pg.locator("#tx-ctx-remove").is_visible())
    pg.evaluate("V.more = { ['tx:' + %s]: { ctx: { t: 1789648528, how: 'made', device: { name: 'Piers’s MacBook Pro', model: 'Mac16,5', os: 'macOS 27.0', app: '0.1.0' }, geo: { lat: -27.469, lon: 153.023, acc: 110 }, place: 'Brisbane, AU' } } }; vault = V; renderCtx()" % json.dumps(T[1])); pg.wait_for_timeout(200)
    ctxt = pg.inner_text("#tx-ctx")
    check("a spend made here says where, to what precision, and on which computer", "Where it was made" in ctxt and "Brisbane, AU" in ctxt and "-27.469, 153.023" in ctxt and "about 110 m" in ctxt and "MacBook Pro" in ctxt and "macOS 27.0" in ctxt, ctxt)
    check("the map is asked for by coordinates only, and carries a dot, a precision ring and the attribution", pg.evaluate("__mapAsked.lat") == -27.469 and pg.locator("#tx-map").is_visible() and pg.locator("#tx-map .pin").count() == 1 and "OpenStreetMap" in pg.inner_text("#tx-map") and 25 < pg.locator("#tx-map .zone").bounding_box()["width"] < 30, pg.locator("#tx-map .zone").bounding_box() if pg.locator("#tx-map .zone").count() else "no ring")
    pg.locator("#tx-ctx").screenshot(path=str(out / "tx-ctx.png"))
    pg.fill("#tx-place", "the office in Geneva"); pg.keyboard.press("Enter"); pg.wait_for_timeout(300)
    check("naming the place yourself replaces the coordinates", pg.evaluate("__ctxSet.place") == "the office in Geneva" and "as you named it" in pg.inner_text("#tx-ctx-dl") and "153.023" not in pg.inner_text("#tx-ctx-dl"))
    check("a place named by hand has no coordinates, so it offers to look one up instead of drawing a map", pg.locator("#tx-map img").count() == 0 and pg.locator("#ctx-find").is_visible())
    # ---- save as PDF ----
    pg.click("#tx-pdf"); pg.wait_for_timeout(400)
    check("Save as PDF asks for THIS transaction, at the height the screen knows and in the screen's currency, and says where it went",
          pg.evaluate("__pdf.txid") == T[1] and pg.evaluate("__pdf.height") == 967410 and pg.evaluate("__pdf.ccy") == "AUD" and "anytwokeys-demo-vault-758adfd2.pdf" in pg.inner_text("#toast") and pg.inner_text("#tx-pdf") == "Save as PDF" and not pg.is_disabled("#tx-pdf"),
          pg.evaluate("JSON.stringify(__pdf)"))
    pg.evaluate("window.__pdfCancel = true"); pg.click("#tx-pdf"); pg.wait_for_timeout(300)
    check("changing your mind about where to save it says nothing and leaves the button usable", "cancel" not in pg.inner_text("#toast").lower() and not pg.is_disabled("#tx-pdf"))
    pg.evaluate("window.__pdfCancel = false")

    # ---- search as you type ----
    pg.evaluate("window.__asked = []"); pg.click("#tx-place"); pg.fill("#tx-place", ""); pg.keyboard.type("445", delay=30); pg.wait_for_timeout(600)
    check("three characters are never asked about", pg.evaluate("__asked.length") == 0 and not pg.locator("#tx-place-list").is_visible())
    pg.keyboard.type(" Upper Edw", delay=40); pg.wait_for_timeout(700)
    check("typing is asked about ONCE, after it stops, not per keystroke", pg.evaluate("JSON.stringify(__asked)") == '["445 Upper Edw"]' and pg.locator("#tx-place-list li[role=option]").count() == 2 and "our own server only" in pg.inner_text("#tx-place-list"), pg.evaluate("JSON.stringify(__asked)"))
    pg.screenshot(path=str(out / "tx-place-search.png"))
    lb = pg.locator("#tx-place-list li[role=option]").nth(1).bounding_box(); check("the list is on top of what is below it, not cut off by the card", pg.evaluate("(p) => { const el = document.elementFromPoint(p.x, p.y); return !!(el && el.closest('#tx-place-list')); }", dict(x=lb["x"] + 20, y=lb["y"] + lb["height"] / 2)))
    pg.keyboard.press("ArrowDown"); pg.keyboard.press("ArrowDown"); pg.keyboard.press("ArrowUp"); pg.keyboard.press("Enter"); pg.wait_for_timeout(400)
    check("arrow keys and Enter choose an answer, which keeps its position so the record gets a map", pg.evaluate("__ctxSet.lat") == -27.4615 and "445 Upper Edward Street, Spring Hill, Brisbane, Queensland 4000, Australia" == pg.evaluate("__ctxSet.place") and "chose it from the search" in pg.inner_text("#tx-ctx-dl") and "· -27.46" in pg.inner_text("#tx-ctx-dl") and pg.input_value("#tx-place").endswith("Queensland 4000, Australia") and pg.locator("#tx-map").is_visible() and not pg.locator("#tx-place-list").is_visible(), pg.evaluate("JSON.stringify(__ctxSet)"))
    pg.click("#tx-place-form button[type=submit]"); pg.wait_for_timeout(300)
    check("pressing Save on the place you chose KEEPS the position it came with, and the map with it",
          pg.evaluate("__ctxSet.lat") == -27.4615 and "chose it from the search" in pg.inner_text("#tx-ctx-dl") and pg.locator("#tx-map img").count() == 1 and pg.locator("#ctx-find").count() == 0,
          pg.inner_text("#tx-ctx-dl")[:120])
    # the crosshair inside the place box: where this Mac is now
    here = pg.locator("#tx-place-here").bounding_box(); box = pg.locator("#tx-place").bounding_box()
    check("text in the place box stops short of the button, never running under it", pg.evaluate("parseFloat(getComputedStyle(document.querySelector('#tx-place')).paddingRight)") >= here["width"] + 6, pg.evaluate("getComputedStyle(document.querySelector('#tx-place')).paddingRight"))
    check("the current-location button sits inside the right-hand end of the place box", here and box and here["x"] > box["x"] + box["width"] - 50 and here["x"] + here["width"] <= box["x"] + box["width"] and box["y"] <= here["y"] and here["y"] + here["height"] <= box["y"] + box["height"], f"{here} in {box}")
    pg.locator("#tx-ctx-acts").screenshot(path=str(out / "tx-place-here.png"))
    pg.click("#tx-place-here"); pg.wait_for_timeout(400)
    check("one tap names where this Mac is and keeps its position for the map", pg.input_value("#tx-place") == "Brisbane, Australia" and pg.evaluate("__ctxSet.place") == "Brisbane, Australia" and pg.evaluate("__ctxSet.lat") == -27.4598, str(pg.evaluate("__ctxSet")))
    pg.evaluate("window.__locDenied = true"); pg.click("#tx-place-here"); pg.wait_for_timeout(300)
    check("with Location Services off, it says where to turn them on", "Privacy & Security" in pg.inner_text("#toast") and pg.get_attribute("#tx-place-here", "aria-busy") is None)
    pg.evaluate("window.__locDenied = false")
    pg.fill("#tx-place", "the office upstairs"); pg.click("#tx-place-form button[type=submit]"); pg.wait_for_timeout(300)
    check("but words you have actually changed are your own, and give the position up", pg.evaluate("__ctxSet.lat") is None and "as you named it" in pg.inner_text("#tx-ctx-dl"))
    pg.evaluate("document.querySelector('#tx-map').dataset.for = ''; window.__asked = []; V.more['tx:' + txView.txid] = { ctx: { t: 9, how: 'noted', named: true, place: 'The Astor Centre, 445 Upper Edward Street', device: { name: 'Test Mac' } } }; vault = V; renderCtx()"); pg.wait_for_timeout(300)
    check("a place you typed has no position, so it offers to look it up rather than show an empty corner", pg.locator("#ctx-find").is_visible() and "No position for this place" in pg.inner_text("#tx-map"))
    pg.click("#ctx-find"); pg.wait_for_timeout(600)
    check("finding it asks OUR server for that place and keeps the position it comes back with", pg.evaluate("__asked[0]") == "The Astor Centre, 445 Upper Edward Street" and pg.evaluate("__ctxSet.lat") == -27.4615 and pg.locator("#tx-map img").count() == 1 and "wrong one" in pg.inner_text("#toast"), "lat %s | imgs %d | %s" % (pg.evaluate("__ctxSet.lat"), pg.locator("#tx-map img").count(), pg.inner_text("#tx-map")[:80]))
    pg.evaluate("window.__noMap = true; document.querySelector('#tx-map').dataset.for = ''; V.more['tx:' + txView.txid] = { ctx: { t: 9, how: 'made', place: '445 Upper Edward Street', geo: { lat: -27.462, lon: 153.025, acc: 110 }, device: { name: 'Test Mac' } } }; vault = V; renderCtx()"); pg.wait_for_timeout(300)
    check("a place with a position but no map to be had says so, with the position, instead of showing nothing", pg.locator("#tx-map").is_visible() and "No map" in pg.inner_text("#tx-map") and "-27.46" in pg.inner_text("#tx-map"), pg.inner_text("#tx-map"))
    pg.evaluate("window.__noMap = false")
    pg.click("#tx-place"); pg.fill("#tx-place", ""); pg.keyboard.type("zzzz nowhere", delay=20); pg.wait_for_timeout(700)
    check("nothing found says so, and that Save still keeps your words", "Nothing found" in pg.inner_text("#tx-place-list"))
    pg.keyboard.press("Escape"); check("Escape closes the list and stays on the transaction", not pg.locator("#tx-place-list").is_visible() and pg.locator("#s-tx").is_visible())
    pg.fill("#tx-place", "the office in Geneva"); pg.keyboard.press("Enter"); pg.wait_for_timeout(300)
    check("Enter with nothing chosen keeps exactly the words and no position", pg.evaluate("__ctxSet.lat") is None and "as you named it" in pg.inner_text("#tx-ctx-dl") and pg.locator("#tx-map img").count() == 0 and pg.locator("#ctx-find").is_visible())
    pg.click("#tx-ctx-remove"); pg.wait_for_timeout(150); check("removing the place asks first", pg.evaluate("__ctxSet.remove") is False and "cannot be worked out again" in pg.inner_text("#tx-ctx"))
    pg.click("#tx-ctx-remove"); pg.wait_for_timeout(300); check("and then it is gone from the record, and says so", pg.evaluate("__ctxSet.remove") is True and "Removed from this record" in pg.inner_text("#tx-ctx-dl") and not pg.locator("#tx-ctx-remove").is_visible())
    pg.evaluate("V.more = { ['tx:' + %s]: { ctx: { t: 1789648528, how: 'seen', device: { name: 'Test Mac' } } } }; vault = V; renderCtx()" % json.dumps(T[1])); pg.wait_for_timeout(200)
    check("a deposit says this is only where the computer was, and that places are off", "first saw it" in pg.inner_text("#tx-ctx") and "wherever the sender was" in pg.inner_text("#tx-ctx-dl") and "places are off" in pg.inner_text("#tx-ctx-dl"))
    pg.evaluate("show('home')"); pg.wait_for_timeout(300); pg.evaluate("__listeners['tauri://drag-drop']({ payload: { paths: ['/x/y.pdf'] } })"); pg.wait_for_timeout(200)
    check("a drop with no record open attaches nothing and says why", "Open a transaction, an address or a coin first" in pg.inner_text("#toast") and pg.evaluate("__dropped.paths.length") == 1)
    pg.locator("#activity tr").nth(2).click(); pg.wait_for_timeout(600)
    check("coming back, the attachments are still on that transaction (two files and the photo from the camera)", pg.locator("#tx-files .att").count() == 3)
    pg.evaluate("window.__badProof = true"); pg.click("#crumbs [data-crumb=home]"); pg.wait_for_timeout(500)
    check("the vault's crumb walks back to its Home, where the row now shows its label", pg.locator("#s-home.on").count() == 1 and "laptop stand" in pg.inner_text("#activity"))
    pg.locator("#activity tr").nth(2).click(); pg.wait_for_timeout(500); pg.click("#crumbs [data-crumb=activity]"); pg.wait_for_timeout(500)
    check("and the Activity crumb walks back to Activity", pg.locator("#s-activity.on").count() == 1 and "laptop stand" in pg.inner_text("#act-all"))
    pg.evaluate("show('home')"); pg.wait_for_timeout(300)
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
    # everything the screen STATES about the vault — not a hover readout, which belongs to whatever the pointer is over
    stated = pg.evaluate("""() => { const c = document.querySelector('#s-home').cloneNode(true); c.querySelectorAll('.ctip').forEach(e => e.remove()); return c.innerText || c.textContent; }""")
    check("a sync that returns after the switch is dropped: the other vault keeps its own balance and no lookup is made for the wrong vault", pg.inner_text("#title") == "other vault" and asked == "other vault" and "111" in pg.inner_text("#balance") and "9,430" not in stated, f"title={pg.inner_text('#title')} asked={asked} balance={pg.inner_text('#balance')} stated={stated[:160]!r}")
    check("and the picture on Home is the one for the vault being shown, not the one left behind", pg.evaluate("document.querySelector('#home-summary').dataset.names") == "other vault", pg.evaluate("document.querySelector('#home-summary').dataset.names"))
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
    pg.evaluate("window.__nodeDown = true"); pg.click("#node"); pg.click("#net-recheck"); pg.wait_for_timeout(300)
    check("no answer: red, says so, and the reason is on hover", pg.get_attribute("#node", "data-state") == "offline" and "Offline" in pg.inner_text("#node-text") and "connection refused" in pg.get_attribute("#node", "title"))
    pg.evaluate("window.__nodeDown = false"); pg.click("#node"); pg.click("#net-recheck"); pg.wait_for_timeout(300)
    check("a click asks again", pg.get_attribute("#node", "data-state") == "online")
    # ---- the light pulses on traffic the app really made, and can be switched off ----
    pg.evaluate("netPing('the node')")
    check("the light pulses when a request actually goes out, and says what it was talking to",
          pg.get_attribute("#node", "data-busy") == "the node" and pg.get_attribute("#node", "data-last") == "the node")
    pg.wait_for_timeout(900)
    check("and it stops pulsing once the request is done", pg.get_attribute("#node", "data-busy") is None)
    pg.click("#node"); pg.wait_for_timeout(150)
    check("the light is where working offline is offered, not buried in settings",
          pg.locator("#net-menu").is_visible() and pg.inner_text("#net-toggle") == "Work offline" and pg.locator("#net-recheck").is_visible())
    asked = pg.evaluate("__nodeAsked.length")
    pg.click("#net-toggle"); pg.wait_for_timeout(400)
    check("turning it on says so plainly, in the light and in App settings",
          pg.get_attribute("#node", "data-state") == "paused" and "Offline · by choice" in pg.inner_text("#node-text") and pg.evaluate("__offline") is True)
    pg.evaluate("checkNode()"); pg.wait_for_timeout(300)
    check("and the app stops asking the node at all — offline is kept, not merely displayed", pg.evaluate("__nodeAsked.length") == asked)
    pg.evaluate("show('settings')"); pg.wait_for_timeout(200)
    check("the switch in App settings shows the same state", "on" in (pg.get_attribute("#offline-switch", "class") or ""))
    pg.locator("#net-menu").wait_for(state="hidden")
    pg.click("#node"); pg.wait_for_timeout(150)
    check("offline, the light offers the way back and stops offering to check", pg.inner_text("#net-toggle") == "Go back online" and not pg.locator("#net-recheck").is_visible())
    pg.click("#net-toggle"); pg.wait_for_timeout(500)
    check("and going back online asks the node again straight away",
          pg.evaluate("__offline") is False and pg.evaluate("__nodeAsked.length") > asked and pg.get_attribute("#node", "data-state") == "online")
    pg.evaluate("show('home')"); pg.wait_for_timeout(200)
    pg.locator(".side").screenshot(path=str(out / "sidebar.png"))
    check("every sidebar entry keeps its height when the list outgrows the window, and the whole of it can be reached",
          pg.evaluate("""() => { const s = document.querySelector('.side');
            const items = [...s.querySelectorAll('.nav, .sidegroup')];
            return items.every(n => n.hidden || n.getBoundingClientRect().height > 8) && (s.scrollHeight <= s.clientHeight || getComputedStyle(s).overflowY === 'auto'); }"""),
          pg.evaluate("JSON.stringify([...document.querySelectorAll('.side .sidegroup')].map(n => [n.textContent.slice(0,14), Math.round(n.getBoundingClientRect().height)]))"))

    # ---- the ledger: checked against figures worked out by hand, not by the code under test ----
    L1 = pg.evaluate("""buildLedger([{ name: 'A', rows: [{ txid: 'd1', time: 1000, net: 100000, quote: { price: 50000 } }] }])""")
    check("a deposit costs what it was worth when it arrived: 0.001 BTC at $50,000 is $50", abs(L1["timeline"][-1]["basis"] - 50) < 1e-9 and L1["timeline"][-1]["sats"] == 100000, L1["timeline"][-1])
    L2 = pg.evaluate("""buildLedger([{ name: 'A', rows: [{ txid: 'd1', time: 1000, net: 100000, quote: { price: 50000 } }, { txid: 's1', time: 2000, net: -50000, quote: { price: 60000 } }] }])""")
    check("spending half takes half the cost with it — average cost, whatever the price on the day it left",
          abs(L2["timeline"][-1]["basis"] - 25) < 1e-9 and L2["timeline"][-1]["sats"] == 50000, L2["timeline"][-1])
    L3 = pg.evaluate("""buildLedger([
        { name: 'A', rows: [{ txid: 'd1', time: 1000, net: 100000, quote: { price: 50000 } }, { txid: 'X', time: 3000, net: -60500, quote: { price: 55000 } }] },
        { name: 'B', rows: [{ txid: 'X', time: 3000, net: 60000, quote: { price: 55000 } }] }])""")
    check("a transfer between two of your vaults is RECOGNISED — the same transaction in both histories", L3["internal"] == 1)
    check("and the coins moved are NOT a disposal: only the 500-sat fee leaves your hands (a fee, which an accountant may treat otherwise)",
          L3["realised"]["X"]["sats"] == 500, str(L3["realised"]["X"]))
    # a vault's OWN screen must see it the same way: its ledger takes in every vault whose figures are loaded
    one = pg.evaluate("""() => {
      const keep = { facts: JSON.parse(JSON.stringify(txFacts)), vaults: store.vaults.slice(), memo: vaultLedger.memo };
      store.vaults = [{ name: 'A', network: 'mainnet', keys: [], threshold: 2 }, { name: 'B', network: 'mainnet', keys: [], threshold: 2 }];
      const rowsA = { d1: { txid: 'd1', time: 1000, net: 100000, quote: { price: 50000 }, ins: [], outs: [[0, 100000]] },
                      X:  { txid: 'X',  time: 3000, net: -60500, quote: { price: 55000 }, ins: [['d1:0', 100000]], outs: [[1, 39500]] } };
      const rowsB = { X: { txid: 'X', time: 3000, net: 60000, quote: { price: 55000 }, ins: [], outs: [[0, 60000]] } };
      for (const k of Object.keys(txFacts)) delete txFacts[k];
      txFacts['A'] = { rows: rowsA, ccy: 'AUD', now: { price: 60000 }, transfers: { X: ['B'] } };
      const alone = (() => { vaultLedger.memo = {}; const L = vaultLedger('A'); return L && L.realised.X ? L.realised.X.sats : null; })();
      txFacts['B'] = { rows: rowsB, ccy: 'AUD', now: { price: 60000 }, transfers: { X: ['A'] } };
      vaultLedger.memo = {};
      const L = vaultLedger('A'), LB = vaultLedger('B');
      const out = { aloneSats: alone, movedSats: L && L.realised.X ? L.realised.X.sats : null, basisB: LB ? LB.perVault.basis.B : null, lotsB: LB ? LB.coins['X:0'] : null };
      for (const k of Object.keys(txFacts)) delete txFacts[k];
      Object.assign(txFacts, keep.facts); store.vaults = keep.vaults; vaultLedger.memo = {};
      return out; }""")
    check("a vault's own screen counts only the fee as leaving, not the coins moved (seeing one vault, it counted all 60,500)",
          one["aloneSats"] == 60500 and one["movedSats"] == 500, str(one))
    check("and the coins arrive in the other vault with the cost and date they were bought at, not the price that day",
          abs(one["basisB"] - 30) < 0.01 and one["lotsB"] and abs(one["lotsB"][0]["cost"] - 30) < 0.01 and one["lotsB"][0]["t"] == 1000, str(one))
    check("and the coins keep their cost: B gets exactly $30.00 of basis, not 60,000 sats at that day's $55,000 ($33)",
          abs(L3["perVault"]["basis"]["B"] - 30) < 1e-9, L3["perVault"])
    check("only the fee is spent: combined holdings drop by 500 sats and combined cost by the fee's share, $0.25",
          L3["timeline"][-1]["sats"] == 99500 and abs(L3["timeline"][-1]["basis"] - 49.75) < 1e-9, L3["timeline"][-1])
    L4 = pg.evaluate("""buildLedger([{ name: 'B', rows: [{ txid: 'X', time: 3000, net: 60000, quote: { price: 55000 } }] }])""")
    check("seen from B alone, the same coins count as bought that day ($33) — which is why scope has to be said",
          abs(L4["timeline"][-1]["basis"] - 33) < 1e-9 and L4["internal"] == 0)
    L5 = pg.evaluate("""buildLedger([{ name: 'A', rows: [{ txid: 'd1', time: 1000, net: 5000, quote: null }] }])""")
    check("a deposit with no price is counted in the balance, and its cost is left OUT rather than guessed",
          L5["unpriced"] == 1 and L5["timeline"][-1]["basis"] == 0 and L5["timeline"][-1]["sats"] == 5000)

    # ---- realised gains: the SAME history under three methods, every figure worked out by hand ----
    H3 = """[{ name: 'A', rows: [
        { txid: 'd1', time: 1000, net: 100000, quote: { price: 10000 }, ins: [], outs: [[0, 100000]] },
        { txid: 'd2', time: 2000, net: 100000, quote: { price: 50000 }, ins: [], outs: [[0, 100000]] },
        { txid: 's1', time: 3000, net: -60000, quote: { price: 60000 }, ins: [['d2:0', 100000]], outs: [[1, 40000]] }] }]"""
    r = {m: pg.evaluate("buildLedger(%s, '%s')" % (H3, m)) for m in ("utxo", "fifo", "average")}
    g = {m: r[m]["realised"]["s1"] for m in r}
    check("UTXO: the coin the transaction ACTUALLY spent (bought at $50,000) — cost $30, worth $36 as it left, +$6",
          abs(g["utxo"]["cost"] - 30) < 1e-9 and abs(g["utxo"]["proceeds"] - 36) < 1e-9 and abs(g["utxo"]["gain"] - 6) < 1e-9 and g["utxo"]["from"] == 2000, g["utxo"])
    check("FIFO: the OLDEST coin first (bought at $10,000) — cost $6, +$30, dated from the first purchase",
          abs(g["fifo"]["cost"] - 6) < 1e-9 and abs(g["fifo"]["gain"] - 30) < 1e-9 and g["fifo"]["from"] == 1000, g["fifo"])
    check("average: one pool of $60 over 200,000 sats — cost $18, +$18",
          abs(g["average"]["cost"] - 18) < 1e-9 and abs(g["average"]["gain"] - 18) < 1e-9, g["average"])
    check("and what is still held carries the rest: UTXO $30 (the first coin $10 + change $20), FIFO $54, average $42",
          [round(r[m]["timeline"][-1]["basis"], 9) for m in ("utxo", "fifo", "average")] == [30, 54, 42], [r[m]["timeline"][-1]["basis"] for m in r])
    # a move between two of your vaults: not a disposal, and the coins keep their DATE as well as their cost
    MV = """[{ name: 'A', rows: [
        { txid: 'd1', time: 1000, net: 100000, quote: { price: 10000 }, ins: [], outs: [[0, 100000]] },
        { txid: 'X', time: 5000, net: -100000, quote: { price: 20000 }, ins: [['d1:0', 100000]], outs: [] }] },
      { name: 'B', rows: [{ txid: 'X', time: 5000, net: 99000, quote: { price: 20000 }, ins: [], outs: [[0, 99000]] }] }]"""
    mv = pg.evaluate("buildLedger(%s, 'utxo')" % MV)
    check("a move to another of your vaults carries 99% of the cost ($9.90) to it; only the 1,000-sat fee is realised ($0.20 worth, $0.10 cost)",
          mv["internal"] == 1 and abs(mv["perVault"]["basis"]["B"] - 9.9) < 1e-9 and abs(mv["realised"]["X"]["cost"] - 0.1) < 1e-9 and abs(mv["realised"]["X"]["proceeds"] - 0.2) < 1e-9, [mv["perVault"], mv["realised"].get("X")])
    later = pg.evaluate("""(() => { const L = buildLedger(%s.concat([]), 'utxo'); return L; })()""" % MV.replace("rows: [{ txid: 'X', time: 5000, net: 99000, quote: { price: 20000 }, ins: [], outs: [[0, 99000]] }]",
        "rows: [{ txid: 'X', time: 5000, net: 99000, quote: { price: 20000 }, ins: [], outs: [[0, 99000]] }, { txid: 'Y', time: 9000, net: -99000, quote: { price: 30000 }, ins: [['X:0', 99000]], outs: [] }]"))
    check("so when B later spends it, the holding runs from the ORIGINAL purchase in A, not from the day it moved",
          later["realised"]["Y"]["from"] == 1000 and abs(later["realised"]["Y"]["cost"] - 9.9) < 1e-9, later["realised"].get("Y"))
    noprice = pg.evaluate("""buildLedger([{ name: 'A', rows: [{ txid: 'd1', time: 1000, net: 5000, quote: null, ins: [], outs: [[0, 5000]] },
        { txid: 's', time: 2000, net: -5000, quote: { price: 50000 }, ins: [['d1:0', 5000]], outs: [] }] }], 'utxo')""")
    check("a coin whose purchase had no price: its disposal's cost and gain are UNKNOWN, never counted as zero",
          noprice["realised"]["s"]["cost"] is None and noprice["realised"]["s"]["gain"] is None and noprice["unpriced"] == 1, noprice["realised"]["s"])
    # ---- several transactions in ONE block: Piers's real round trip (19,639 out, 19,371 back, same block) ----
    RT = """[{ name: 'A', rows: [
        { txid: 'first', time: 1000, net: 19639, quote: { price: 100000 } },
        { txid: '0dedec3f', time: 2000, net: 19371, quote: { price: 110000 } },
        { txid: 'cb57ab55', time: 2000, net: -19639, quote: { price: 110000 } }] }]"""
    Lr = pg.evaluate("buildLedger(%s)" % RT)
    check("a coin sent out and its remainder back in the same block: the old coin is disposed of, the new one costs its own arrival — $21.3081, not an average with itself",
          abs(Lr["timeline"][-1]["basis"] - 19371 / 1e8 * 110000) < 1e-9 and Lr["timeline"][-1]["sats"] == 19371, Lr["timeline"][-1])
    check("inside that block the ledger never holds more than the vault did — no 39,010", max(e["sats"] for e in Lr["timeline"]) <= 19639, [e["sats"] for e in Lr["timeline"]])
    Sr = pg.evaluate("sumSeries(buildLedger(%s), 0, 3000, {}, null)" % RT)
    check("and the chart draws ONE point for that block, the state after it — no spike, no dip to nothing",
          [p for p in Sr["bal"] if p[0] == 2000] == [[2000, 19371]], Sr["bal"])
    Lc = pg.evaluate("""buildLedger([{ name: 'A', rows: [
        { txid: 'aa', time: 5000, net: -9000, quote: { price: 50000 } },
        { txid: 'bb', time: 5000, net: 10000, quote: { price: 50000 } }] }])""")
    check("a coin received and spent in the same block is spent AFTER it arrives — holdings never go below nothing",
          min(e["sats"] for e in Lc["timeline"]) >= 0 and Lc["timeline"][-1]["sats"] == 1000, [e["sats"] for e in Lc["timeline"]])
    # ---- the summary on Home ----
    pg.evaluate("show('home')"); pg.wait_for_timeout(1200)
    check("Home leads with stat tiles: what it holds, what it is worth, what it cost, and the gain not yet realised",
          [t.strip().lower() for t in pg.locator("#home-summary .kpi .k").all_inner_texts()] == ["holding", "worth now", "cost basis", "unrealised", "realised"],
          pg.locator("#home-summary .kpi .k").all_inner_texts())
    check("the gain is said with a sign and an arrow, never by colour alone", any(ch in pg.inner_text("#home-summary .kpi:nth-child(4) .v") for ch in "▲▼"))
    charts = pg.locator("#home-summary .chart")
    check("sats and money are on TWO charts — never one chart with two scales",
          charts.count() == 2 and "sats" in charts.nth(0).locator("h4").inner_text() and any(c in charts.nth(1).locator("h4").inner_text() for c in ("USD", "EUR", "AUD", "GBP")),
          [charts.nth(i).locator("h4").inner_text() for i in range(charts.count())])
    check("the one-series chart has no legend box; the two-series chart always has one",
          charts.nth(0).locator(".legend").count() == 0 and charts.nth(1).locator(".legend span").count() == 2)
    check("worth and cost basis are the two lines on the money chart", "Worth" in charts.nth(1).locator(".legend").inner_text() and "Cost basis" in charts.nth(1).locator(".legend").inner_text())
    check("every line ends in a labelled dot", charts.nth(1).locator("circle.dot").count() == 2 and charts.nth(1).locator("text.endlbl").count() == 2)
    # the crosshair: one readout for every series at the moment under the pointer
    hb = charts.nth(1).locator("rect.hit").bounding_box()
    pg.mouse.move(hb["x"] + hb["width"] * 0.6, hb["y"] + hb["height"] / 2); pg.wait_for_timeout(200)
    tip = charts.nth(1).locator(".ctip")
    check("hovering shows ONE readout listing every series, the value leading the name", tip.is_visible() and tip.locator(".r").count() == 2 and tip.locator(".r b").count() == 2, tip.inner_text() if tip.is_visible() else "hidden")
    pg.mouse.move(5, 5); pg.wait_for_timeout(150)
    charts.nth(0).locator("rect.hit").focus(); pg.keyboard.press("Home"); pg.wait_for_timeout(150)
    check("the same readout is reachable from the keyboard", charts.nth(0).locator(".ctip").is_visible())
    pg.keyboard.press("Tab")
    pg.click("#home-summary .linkish"); pg.wait_for_timeout(200)
    check("and every figure is in a table too, so nothing is reachable only by hovering", pg.locator("#home-summary .sum-table table tr").count() >= 2)
    check("the notes say what the figures rest on: which closing prices, and what 'cost' means here",
          "closing price" in pg.inner_text("#home-summary") and "a true cost only if it was a purchase" in pg.inner_text("#home-summary"))
    bad = pg.evaluate("""() => { const rows = [...document.querySelectorAll('#home-summary .sum-table tr')].slice(1);
        const num = (s) => parseFloat(String(s).replace(/[^0-9.]/g, ''));
        return rows.map(r => [...r.cells].map(c => c.textContent)).filter(c => { const bal = num(c[1]), worth = num(c[2]);
          const implied = bal > 0 ? worth / (bal / 1e8) : 0; return bal > 0 && (implied < 90000 || implied > 130000); }).map(c => c.join(' | ')); }""")
    check("the chart agrees with ITSELF: at every moment in the table, worth is the balance at a real price — never the balance before a spend",
          bad == [], bad)
    pg.locator("#home-summary").screenshot(path=str(out / "summary.png"))
    check("the whole range is ONE request to the price service, not one per day", len(pg.evaluate("window.__seriesAsked || []")) == 1, pg.evaluate("window.__seriesAsked"))
    check("and the note says worth is drawn from EVERY day's close", "every day\u2019s closing price" in pg.inner_text("#home-summary"))
    asked = len(pg.evaluate("window.__seriesAsked || []"))
    pg.click('#home-summary .seg button[data-range="90"]'); pg.wait_for_timeout(900)
    check("the time range scopes everything below it, and is remembered", pg.get_attribute('#home-summary .seg button[data-range="90"]', "aria-checked") == "true" and pg.evaluate("localStorage.getItem('a2k-sum-range')") == "90")
    pg.click('#home-summary .seg button[data-range="all"]'); pg.wait_for_timeout(900)
    check("a range already fetched this session is not asked for again", len(pg.evaluate("window.__seriesAsked || []")) - asked <= 1, pg.evaluate("window.__seriesAsked"))
    pg.evaluate("window.__holeDay = new Date(Date.now() - 2*86400000).toISOString().slice(0,10); for (const k in sumRuns) delete sumRuns[k]; renderHomeSummary()"); pg.wait_for_timeout(900)
    check("a day the source has no price for is left out and SAID to be, never filled", "no price at the source" in pg.inner_text("#home-summary"), pg.inner_text("#home-summary")[-300:])
    pg.evaluate("window.__holeDay = null; for (const k in sumRuns) delete sumRuns[k]")

    # ---- all vaults ----
    pg.evaluate("show('overview')"); pg.wait_for_timeout(1500)
    check("All vaults is a screen of its own, at the app tier, with the same view", pg.locator("#ov-summary .kpi").count() >= 1 and pg.locator("#ov-summary .chart").count() >= 1, pg.inner_text("#ov-summary")[:160])
    check("a test-network vault is NEVER added into a total of real coins, and the screen says which were left out",
          "a signet one" in pg.inner_text("#ov-note") and "must never be added to real ones" in pg.inner_text("#ov-note"), pg.inner_text("#ov-note"))
    check("and it counts across vaults", "across" in pg.inner_text("#ov-summary .kpi:first-child") or pg.locator("#ov-summary .vchip").count() >= 1)
    pg.locator("#s-overview").screenshot(path=str(out / "overview.png"))
    pg.emulate_media(color_scheme="dark"); pg.evaluate("renderOverview()"); pg.wait_for_timeout(900)
    check("dark mode draws with ITS OWN validated steps, not the light ones",
          pg.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--series-value').trim().toUpperCase()") == "#B08A26"
          and pg.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--series-cost').trim().toUpperCase()") == "#4A83C8")
    pg.locator("#s-overview").screenshot(path=str(out / "overview-dark.png"))
    pg.emulate_media(color_scheme="light"); pg.wait_for_timeout(200)
    pg.evaluate("show('home')"); pg.wait_for_timeout(400)

    # ---- a wallet of extended keys, pasted in: Piers's SeedSigner multisig ----
    DESC = 'wsh(sortedmulti(2,[dc567276/48h/0h/0h/2h]xpub6DiYrfRwNnjeX4vHsWMajJVFKrbEEnu8gAW9vDuQzgTWEsEHE16sGWeXXUV1LBWQE1yCTmeprSNcqZ3W74hqVdgDbtYHUv3eM4W2TEUhpan/<0;1>/*,[81ce8a63/48h/0h/0h/2h]xpub6E6CkrYs9KrKvmMYoZuY8r5a3MrhhXKw2XXbWsLpiA3pk8iPTrdpvBhrj47vQrCMnpUgyFTY9JXMfg2vXXSZRrVapmwy6qDdcEgnKMBWi3E/<0;1>/*,[88725028/48h/0h/0h/2h]xpub6DtepHbchFhqJJJyy8KTAKqxBadf1xqwqc7e9WS3nvwnQ3HSzHkTiaoTZoB1JDYKCZid3RaLoNoQ9F6TH287YcketYJRmK3NSKs39DhEBys/<0;1>/*))#ewpd0dfe'
    pg.evaluate("show('restore'); restoreTab('paste')"); pg.wait_for_timeout(300)
    pg.fill("#desc-in", DESC); pg.click("#desc-read"); pg.wait_for_timeout(500)
    card = pg.inner_text("#restore-card")
    check("an xpub multisig pasted in is READ — the failure Piers hit — as extended keys held by other signers",
          "over extended keys held by other signers" in card and "checksum ok" in card and "not a public key" not in card, card[:200])
    check("each key is shown by the fingerprint and path its device recognises, not as a card",
          all(fp in card for fp in ("dc567276", "81ce8a63", "88725028")) and card.count("signs on another device") == 3 and "plugged in" not in card)
    check("it is offered to watch, and says a spend is signed on its devices — nothing claims spending is not built",
          pg.inner_text("#restore-go") == "Watch this vault" and "signed on its devices" in card and "not built" not in card)
    check("the names offered are the keys' own fingerprints, not a card vault's laptop / keyring / off-site",
          pg.input_value('[data-kname="0"]') == "key 1 · dc567276")
    pg.fill("#restore-name", "seedsigner demo"); pg.click("#restore-go"); pg.wait_for_timeout(800)
    check("what reaches the restore is the descriptor AS WRITTEN — capitals kept, since base58 is case-sensitive",
          "xpub6DiYrfRwNnjeX4vHsWMajJVFKrbEEnu8gAW9vDuQzgTWEsEHE16sGWeXXUV1LBWQE1yCTmeprSNcqZ3W74hqVdgDbtYHUv3eM4W2TEUhpan" in pg.evaluate("__restored.text"), pg.evaluate("__restored.text")[:80])
    pg.evaluate("vault = store.vaults.find(v => v.name === 'seedsigner demo'); show('keys')"); pg.wait_for_timeout(600)
    check("on Keys, each is a key on another device — fingerprint and path — with no card, no seal slot and no plug-in state",
          pg.locator("#keys-list .keyrow").count() == 3 and pg.inner_text("#keys-list").count("signs on another device") == 3 and "plugged in" not in pg.inner_text("#keys-list") and "dc567276" in pg.inner_text("#keys-list"),
          pg.inner_text("#keys-list")[:200])
    check("the seal note says how THIS vault comes back: seed words plus the descriptor", "seed words" in pg.inner_text("#seal-note") and "descriptor" in pg.inner_text("#seal-note"))
    check("and the YubiKey firmware / end-of-life card is not shown for keys that are not YubiKeys", not pg.locator("#key-life").is_visible())
    check("and on Keys the line is still blurred until revealed — the rule that now serves both",
          "blur" in pg.evaluate("getComputedStyle(document.querySelector('#keys-desc')).filter"))
    kd = pg.inner_text("#keys-desc-ext") if pg.locator("#keys-desc-ext").is_visible() else ""
    check("the recovery line on Keys is described as what it is — a standard wallet descriptor — with no word of seals, cc= or PINs",
          "wallet descriptor" in kd and "seed words" in kd and not pg.locator("#keys-desc-cards").is_visible(), kd[:160])
    pg.evaluate("show('send')"); pg.wait_for_timeout(500)
    check("Send can build a spend from this vault of devices, and says it is signed by scanning codes — nothing about cards or 'not built'",
          pg.locator("#ext-send").is_visible() and "scanning the code" in pg.inner_text("#ext-send") and "PIN" not in pg.inner_text("#ext-send") and "not built" not in pg.inner_text("#ext-send") and not pg.locator("#build-btn").is_disabled(), pg.inner_text("#ext-send"))
    pg.evaluate("renderVaultSettings.shown = null; show('vault')"); pg.wait_for_timeout(400)
    real = pg.evaluate("vault.ext")
    # what the person SEES, not what the text is: the first version scrambled it but left it unblurred (Piers, 21:27)
    blur = lambda sel: pg.evaluate("(s) => getComputedStyle(document.querySelector(s)).filter", sel)
    check("Vault settings keeps the recovery line HIDDEN until asked, as Keys does — scrambled AND blurred, the same look",
          pg.inner_text("#vs-desc") != real and "blur" in blur("#vs-desc") and pg.locator("#vs-desc-reveal").is_visible(), blur("#vs-desc"))
    pg.click("#vs-desc-reveal"); pg.wait_for_timeout(300)
    check("and shows it plainly when asked", pg.inner_text("#vs-desc") == real and "blur" not in blur("#vs-desc"), blur("#vs-desc"))
    check("an xpub vault is offered a way to take its other wallet's wording", pg.locator("#vs-rewrite").is_visible())
    pg.fill("#vs-rewrite-in", real); pg.click("#vs-rewrite-go"); pg.wait_for_timeout(400)
    check("which goes to the app to be checked as the same wallet before it replaces anything", pg.evaluate("__rewrite.text") == real and pg.evaluate("__rewrite.name") == "seedsigner demo")
    check("Vault settings does not offer a card drill for a vault with no cards, and says how to check it instead",
          not pg.locator("#vs-drill").is_visible() and "load its seed words and confirm it shows the fingerprint" in pg.inner_text("#vs-drill-what"))
    pg.evaluate("vault = store.vaults[0]; show('home')"); pg.wait_for_timeout(400)
    check("and a card vault is untouched by all of this: its Send still builds", (pg.evaluate("show('send')") or True) and not pg.locator("#build-btn").is_disabled() and not pg.locator("#ext-send").is_visible())
    pg.evaluate("show('home')"); pg.wait_for_timeout(300)

    # ---- realised, on screen: the mock vault's coin spent down twice ----
    pg.evaluate("show('home')"); pg.wait_for_timeout(1200)
    L = pg.evaluate("vaultLedger(vault.name)")
    # by hand: the 20,000-sat coin cost 20,000 x 107,389.64 / 1e8 = 21.477928. The first spend sends 5,190 of it away
    # (5,000 + 190 fee) at 107,768.03: cost 5190/20000 x 21.477928, worth 5190 x 107,768.03 / 1e8.
    c1, p1 = 5190 / 20000 * (20000 * 107389.64 / 1e8), 5190 * 107768.03 / 1e8
    r1 = L["realised"][T[1]]
    check("the first spend is realised against the coin it actually spent — cost and worth to the cent of the hand figure",
          abs(r1["cost"] - c1) < 1e-9 and abs(r1["proceeds"] - p1) < 1e-9 and abs(r1["gain"] - (p1 - c1)) < 1e-9 and L["method"] == "utxo", r1)
    check("the second spend had no price on its day: its gain is unknown, and it is NOT counted as zero", L["realised"][T[2]]["gain"] is None)
    pg.evaluate("openTx(%r)" % T[1]); pg.wait_for_timeout(1200)
    dl = pg.inner_text("#tx-dl")
    check("the transaction screen shows what the spend realised, what the coins had cost, how long they were held, and by which method",
          "realised" in dl.lower() and "had cost" in dl and "held" in dl and "the coins this transaction actually spent" in dl, dl[-300:])
    pg.evaluate("openTx(%r)" % T[0]); pg.wait_for_timeout(1200)
    check("a deposit shows its cost: what it was worth as it arrived", "cost" in pg.inner_text("#tx-dl").lower() and "worth as it arrived" in pg.inner_text("#tx-dl"), pg.inner_text("#tx-dl")[-200:])
    pg.evaluate("show('activity')"); pg.wait_for_timeout(900)
    check("Activity carries the realised figure under each spend it is known for", "realised +" in pg.inner_text("#act-all") or "realised \u2212" in pg.inner_text("#act-all"), pg.inner_text("#act-all")[:300])
    pg.evaluate("show('settings')"); pg.wait_for_timeout(300)
    check("App settings: the method, defaulting to the coins actually spent, and says where each is accepted",
          pg.input_value("#cost-method") == "utxo" and "ATO" in pg.inner_text("#cost-method-note"))
    pg.select_option("#cost-method", "average"); pg.wait_for_timeout(400)
    check("average cost is offered only for reference, and says plainly where it is NOT accepted",
          pg.evaluate("__method") == "average" and "NOT generally accepted" in pg.inner_text("#cost-method-note") and "warn" in (pg.get_attribute("#cost-method-note", "class") or ""))
    pg.select_option("#cost-method", "fifo"); pg.wait_for_timeout(400)
    check("choosing a method re-works every realised figure", pg.evaluate("vaultLedger(vault.name).method") == "fifo")
    pg.select_option("#cost-method", "utxo"); pg.wait_for_timeout(300)
    pg.evaluate("show('vault')"); pg.wait_for_timeout(300)
    check("Vault settings follows the app's method unless told otherwise, and says an override is for a different taxpayer",
          pg.input_value("#vs-method") == "" and "different taxpayer" in pg.inner_text("#vs-method-note"))
    pg.select_option("#vs-method", "fifo"); pg.wait_for_timeout(500)
    check("an override is saved against that vault, and warns that one person cannot pick methods per wallet to choose the result",
          pg.evaluate("__vaultMethod.method") == "fifo" and "cannot use different methods for different wallets" in pg.inner_text("#vs-method-note"))
    pg.select_option("#vs-method", ""); pg.wait_for_timeout(400)
    pg.evaluate("show('home')"); pg.wait_for_timeout(400)

    # ---- Activity: the whole history, with room ----
    pg.evaluate("show('activity')"); pg.wait_for_timeout(500)
    home_rows = pg.locator("#activity tr[data-txid]").count()
    check("Activity holds every transaction, and Home keeps only a recent few with a way through to the rest",
          pg.locator("#act-all tr[data-txid]").count() >= home_rows and "transaction" in pg.inner_text("#act-count") and pg.locator("#activity-more-row").count() == 1,
          "%s in Activity, %s on Home" % (pg.locator("#act-all tr[data-txid]").count(), home_rows))
    check("the amounts reach the Activity table too, not only Home's", "sats" in pg.inner_text("#act-all"), pg.inner_text("#act-all")[:90])
    pg.fill("#act-find", "zzzznothing"); pg.wait_for_timeout(250)
    check("finding nothing says so, and says a label or a note counts as well as an id", pg.locator("#act-all tr[data-txid]").count() == 0 and "label or a note" in pg.inner_text("#act-foot"))
    pg.fill("#act-find", T[0][:8]); pg.wait_for_timeout(250)
    check("and an id finds its transaction", pg.locator("#act-all tr[data-txid]").count() == 1)
    pg.fill("#act-find", ""); pg.wait_for_timeout(250)

    # ---- Addresses: its own screen, and a record per address ----
    pg.evaluate("show('addresses')"); pg.wait_for_timeout(600)
    check("a vault with ONE fixed address has no list, and the screen says that instead of sitting empty",
          pg.locator("#addr-fixed").is_visible() and "single fixed address" in pg.inner_text("#addr-fixed") and not pg.locator("#addr-card").is_visible())
    pg.evaluate("window.__rotating = true; vault._rotating = true; show('addresses')"); pg.wait_for_timeout(600)
    check("Addresses is a screen of its own, both chains, including ones never used yet",
          pg.locator("#addr-recv tbody tr").count() >= 2 and pg.locator("#addr-chg tbody tr").count() >= 1 and "unused" in pg.inner_text("#addr-recv"))
    pg.locator("#addr-recv tbody tr").first.click(); pg.wait_for_timeout(400)
    check("clicking an address opens its own record, and says why labelling an unused one is the point",
          pg.locator("#addr-detail").is_visible() and "never been used" in pg.inner_text("#addr-detail") and "before you hand it out" in pg.inner_text("#addr-detail"))
    check("the record card itself has MOVED to the address — one card, not a copy", pg.evaluate("document.querySelector('#addr-detail').contains(document.querySelector('#tx-notes'))"))
    pg.fill("#tx-label", "for the plumber"); pg.evaluate("document.querySelector('#tx-label').blur()"); pg.wait_for_timeout(400)
    check("a label typed there is saved against the ADDRESS, not a transaction",
          pg.evaluate("JSON.stringify(Object.keys(V.labels))").count("addr:") == 1 and pg.evaluate("V.labels['addr:bc1qnextone']") == "for the plumber",
          pg.evaluate("JSON.stringify(V.labels)"))
    pg.fill("#tx-note", "he is coming on Tuesday"); pg.evaluate("document.querySelector('#tx-note').blur()"); pg.wait_for_timeout(400)
    check("and so is a note", pg.evaluate("V.notes['addr:bc1qnextone']") == "he is coming on Tuesday", pg.evaluate("JSON.stringify(V.notes)"))
    check("the drop zone follows the record, so files land on the address too", pg.evaluate("onRec()") is True)

    # ---- Coins ----
    pg.evaluate("show('coins')"); pg.wait_for_timeout(700)
    check("Coins lists one row per unspent output, with what it is worth", pg.locator("#coin-rows tr[data-coin]").count() == 1 and "9,430 sats" in pg.inner_text("#coin-rows"))
    check("and the address the coin sits on, worked out from its index", "bc1qexample" in pg.inner_text("#coin-rows tr[data-coin]"), pg.inner_text("#coin-rows"))
    pg.locator("#coin-rows tr[data-coin]").first.click(); pg.wait_for_timeout(400)
    check("clicking a coin opens its record, keyed on the output itself",
          pg.locator("#coins-detail").is_visible() and pg.evaluate("recView.key") == "output:%s:0" % T[0] and pg.evaluate("document.querySelector('#coins-detail').contains(document.querySelector('#tx-notes'))"),
          pg.evaluate("recView.key"))
    pg.fill("#tx-note", "change from the car"); pg.evaluate("document.querySelector('#tx-note').blur()"); pg.wait_for_timeout(400)
    check("a note on a coin is kept against that one coin", pg.evaluate("V.notes['output:%s:0']" % T[0]) == "change from the car")
    check("while a coin's record is open the card is NOT back home", pg.evaluate("document.querySelector('#tx-notes').parentElement === recHome") is False)
    # ---- a coin is a page: the list steps aside, crumbs say where, figures and a strip, a PDF, and links onward ----
    check("the coin opens as a page: the list steps aside", not pg.locator("#coin-rows").is_visible())
    cr = pg.inner_text("#crumbs")
    check("the crumbs read vault › Coins › this coin", "Coins" in cr and ("%s…:0" % T[0][:8]) in cr and pg.locator("#crumbs [data-crumb=coins]").count() == 1, cr)
    dd = pg.inner_text("#coins-detail")
    ddl = dd.lower()   # the labels are uppercased by CSS
    check("what it cost, what it is worth, and how long it has been held", "cost basis" in ddl and "worth now" in ddl and "held since" in ddl, dd[:300])
    pg.wait_for_timeout(600)
    check("a strip across the top: its worth against its cost, over time", pg.locator("#rec-chart svg").count() >= 1 or "nothing to draw" in pg.inner_text("#rec-chart"), pg.inner_text("#rec-chart")[:120])
    check("with one coin, ‹ › have nowhere to go", pg.locator("#coins-detail [data-rec-step]:not([disabled])").count() == 0)
    pg.evaluate("recPageOpen('coin', recPage.id)"); pg.wait_for_timeout(300)   # what ‹ › does: the page rebuilt in place
    check("rebuilding the page (as ‹ › does) keeps the record card, with its note", pg.evaluate("!!document.querySelector('#coins-detail #tx-notes')") and pg.input_value("#tx-note") == "change from the car")
    pg.click("#rec-pdf"); pg.wait_for_timeout(400)
    rp = pg.evaluate("window.__recPdf") or {}
    check("Save as PDF sends the page's own figures, keyed on the coin", rp.get("key") == "output:%s:0" % T[0] and any(k == "Cost basis" for k, _ in rp.get("facts", [])) and "Saved" in pg.inner_text("#toast"), json.dumps(rp)[:300])
    pg.locator("#coins-detail [data-go-addr]").first.click(); pg.wait_for_timeout(700)
    check("the coin's address is a link to that address's own page", pg.locator("#s-addresses.on").count() == 1 and pg.locator("#addr-detail").is_visible() and "Addresses" in pg.inner_text("#crumbs"), pg.inner_text("#crumbs"))
    pg.wait_for_timeout(500)
    check("an address's page also draws its balance in sats over time, from its own transactions", "Balance, in sats" in pg.inner_text("#rec-bal") and pg.locator("#rec-bal svg").count() >= 1 and pg.evaluate("window.__flowAsked && window.__flowAsked.history.length") >= 1, pg.inner_text("#rec-bal")[:160])
    pg.click("#rec-pdf"); pg.wait_for_timeout(400)
    check("and its PDF carries that balance chart", any(c["title"] == "Balance, in sats" for c in (pg.evaluate("window.__recPdf") or {}).get("charts", [])), str([c.get("title") for c in (pg.evaluate("window.__recPdf") or {}).get("charts", [])]))
    pg.locator("#addr-detail [data-go-coin]").first.click(); pg.wait_for_timeout(700)
    check("and the address links back to the coin it holds", pg.locator("#s-coins.on").count() == 1 and pg.locator("#coins-detail").is_visible())
    pg.click("#crumbs [data-crumb=coins]"); pg.wait_for_timeout(400)
    check("Coins in the crumbs goes back to the list", pg.locator("#coin-rows").is_visible() and not pg.locator("#coins-detail").is_visible())
    pg.locator("#coin-rows tr[data-coin]").first.click(); pg.wait_for_timeout(400)
    pg.evaluate("show('home')"); pg.wait_for_timeout(400)
    check("leaving the record screens puts the one card back where the transaction screen expects it",
          pg.evaluate("document.querySelector('#tx-notes').parentElement === recHome"))
    pg.evaluate("show('coins')"); pg.wait_for_timeout(400)
    check("Coins can be put away for people who do not want it, and the tab goes",
          pg.evaluate("localStorage.setItem('a2k-coins-tab','off'); paintCoinsTab(); document.querySelector('#nav-coins').hidden") is True)
    pg.evaluate("localStorage.setItem('a2k-coins-tab','on'); paintCoinsTab()")

    # ---- Vault settings, and the recovery drill ----
    pg.evaluate("show('vault')"); pg.wait_for_timeout(400)
    check("Vault settings says what this vault IS, and which network it is on",
          "2 of 3" in pg.inner_text("#vs-facts") and pg.inner_text("#vs-facts").count("key") >= 1 and "network" in pg.inner_text("#vs-facts").lower(), pg.inner_text("#vs-facts")[:160])
    check("a vault that has never been drilled says so, and says what that means",
          "never been drilled" in pg.inner_text("#vs-drill-when") and "believe rather than something you have seen" in pg.inner_text("#vs-drill-when"))
    check("and the drill promises to spend nothing", "Nothing is spent" in pg.inner_text("#vs-drill-what"))
    pg.evaluate("window.__seal = { cards: 0, foreign: [], found: [] }; window.__cands = []")
    pg.click("#vs-drill"); pg.wait_for_timeout(400)
    check("with no key plugged in the drill says so rather than claiming anything", "No key plugged in" in pg.inner_text("#vs-drill-out"))
    pg.evaluate("""window.__seal = { cards: 2, foreign: [], found: [
        { pubkey: 'a', serial: 11, entry: { id: 'v1', k: 2, x: 1, locked: false } },
        { pubkey: 'b', serial: 12, entry: { id: 'v1', k: 2, x: 2, locked: false } }] };
      window.__cands = [{ id: 'v1', have: 2, locked: 0, need: 2, known_as: vault.name, address: vault.address,
        line: 'wsh(...)', record: { name: vault.name, threshold: vault.threshold, pubkeys: ['a','b','c'], key_names: ['laptop','keyring','off-site'], network: vault.network, chain_code: null } }]""")
    pg.click("#vs-drill"); pg.wait_for_timeout(600)
    check("a drill that rebuilds THIS vault from the cards says so, naming the keys it used",
          "Recovered" in pg.inner_text("#vs-drill-out") and "serial 11" in pg.inner_text("#vs-drill-out") and "same first address" in pg.inner_text("#vs-drill-out"), pg.inner_text("#vs-drill-out")[:200])
    check("it says plainly which keys it did NOT try, so a success is not read as more than it is",
          "not tried" in pg.inner_text("#vs-drill-out") and "says nothing about the" in pg.inner_text("#vs-drill-out"))
    check("and the date of the drill is written down with the vault, as a date that can be COMPARED, shown in words",
          __import__("re").match(r"^\d{4}-\d{2}-\d{2} ", pg.evaluate("V.notes['vault:drill']")) is not None and "Last drill" in pg.inner_text("#vs-drill-when"), pg.evaluate("V.notes['vault:drill']"))
    check("a fresh drill counts as today and says when the next is due", "today" in pg.inner_text("#vs-drill-when") and "due in 90 days" in pg.inner_text("#vs-drill-when"), pg.inner_text("#vs-drill-when"))
    # ---- the reminder ----
    pg.evaluate("V.notes['vault:drill'] = '2020-01-01 \u00b7 keys 11, 12'; paintDrillNudge.hidden = null; show('vault')"); pg.wait_for_timeout(300)
    check("a drill long past its interval is called out as overdue, not merely reported",
          "past the 90 days" in pg.inner_text("#vs-drill-when") and "warn" in (pg.get_attribute("#vs-drill-when", "class") or ""), pg.inner_text("#vs-drill-when"))
    pg.evaluate("show('home')"); pg.wait_for_timeout(500)
    check("and Home carries a quiet line about it, with a way straight to the drill",
          pg.locator("#drill-nudge").is_visible() and "days since this vault was proved recoverable" in pg.inner_text("#drill-nudge") and pg.locator("#drill-nudge-go").is_visible())
    check("the line says WHY, which is that a key fails without saying so", "die, be wiped or lock itself out" in pg.inner_text("#drill-nudge"))
    pg.click("#drill-nudge-hide"); pg.wait_for_timeout(200)
    check("it can be put aside, and it never blocks anything", not pg.locator("#drill-nudge").is_visible())
    pg.evaluate("delete V.notes['vault:drill']; paintDrillNudge.hidden = null; paintDrillNudge()"); pg.wait_for_timeout(200)
    check("a vault that has NEVER been drilled is nudged too, in its own words",
          pg.locator("#drill-nudge").is_visible() and "never been proved recoverable" in pg.inner_text("#drill-nudge"))
    pg.evaluate("show('settings')"); pg.wait_for_timeout(300)
    check("the interval defaults to 90 days and can be changed, up to once a year or never",
          pg.input_value("#drill-every") == "90" and [o.strip() for o in pg.locator("#drill-every option").all_inner_texts()] == ["every 30 days", "every 90 days", "every 6 months", "once a year", "never ask"])
    pg.select_option("#drill-every", "0"); pg.wait_for_timeout(300)
    pg.evaluate("paintDrillNudge.hidden = null; show('home')"); pg.wait_for_timeout(400)
    check("asking never to be reminded is honoured", pg.evaluate("__drillDays") == 0 and not pg.locator("#drill-nudge").is_visible())
    pg.evaluate("window.__drillDays = 90; store.settings.drill_days = 90")
    # put the drill back as the checks below expect to find it: this block borrowed the record to test the reminder
    pg.evaluate("V.notes['vault:drill'] = new Date().toISOString().slice(0,10) + ' \u00b7 keys 11, 12'; paintDrillNudge.hidden = null")
    pg.evaluate("show('vault')"); pg.wait_for_timeout(300)
    check("so the next time the screen is opened it says when it was last proved", "Last drill" in pg.inner_text("#vs-drill-when"))
    pg.evaluate("window.__cands[0].address = 'bc1qSOMETHINGELSE'; window.__cands[0].known_as = null")
    pg.click("#vs-drill"); pg.wait_for_timeout(600)
    check("a rebuild that does NOT derive this vault's address is a failure, however well it read the cards",
          "not this vault" in pg.inner_text("#vs-drill-out"), pg.inner_text("#vs-drill-out")[:160])
    pg.locator("#s-vault").screenshot(path=str(out / "vault-settings.png"))
    pg.evaluate("show('home')"); pg.wait_for_timeout(300)
    check("labels live with the vault they belong to, on its Home", pg.locator("#s-home #labels-export").count() == 1 and pg.locator("#s-settings #labels-export").count() == 0)
    # ---- Settings ----
    pg.evaluate("__state.fail = false; show('settings')"); pg.wait_for_timeout(300)
    check("Settings: automatic names the currency and why", "Automatic: AUD" in pg.locator("#fiat-pick option").first.inner_text() and "region is AU" in pg.inner_text("#fiat-note"), pg.inner_text("#fiat-note"))
    pg.select_option("#fiat-pick", "EUR"); pg.wait_for_timeout(300); pg.evaluate("show('home')"); pg.wait_for_timeout(600)
    check("choosing EUR asks for EUR and draws euros", pg.evaluate("__state.asked.ccy") == "EUR" and ("€" in pg.inner_text("#activity") or "EUR" in pg.inner_text("#activity")), pg.inner_text("#activity")[:120])
    pg.evaluate("show('settings')"); pg.select_option("#fiat-pick", ""); pg.wait_for_timeout(300); pg.evaluate("show('home')"); pg.wait_for_timeout(600)
    check("None: no money anywhere, sats lead again", pg.evaluate("__state.asked.ccy") == "" and "EUR" not in pg.inner_text("#s-home") and "$" not in pg.inner_text("#s-home") and "9,430" in pg.inner_text("#balance"))
    # ---- everything cross-links; a transfer between two of this person's vaults is said, and leads to the other end ----
    pg.evaluate("""() => { __extra = [Object.assign({}, V, { name: 'savings vault' })]; window.__peer = true; window.__xfers = { '%s': ['savings vault'] }; }""" % T[1])
    pg.evaluate("(async () => { store = await window.__TAURI__.core.invoke('store_get'); vault = store.vaults[0]; vault._syncedAt = 0; show('home'); })()"); pg.wait_for_timeout(900)
    check("an activity row of a transfer says so, with the other vault's name", "savings vault" in pg.inner_text("#activity") and pg.locator("#activity [data-go-vault-tx='savings vault']").count() == 1, pg.inner_text("#activity")[:300])
    pg.evaluate("openTx('%s')" % T[1]); pg.wait_for_timeout(700)
    dl = pg.inner_text("#tx-dl")
    check("the transaction says it is a transfer to the other vault, as a link", "Transfer" in dl.replace("TRANSFER", "Transfer") and pg.locator("#tx-dl a[data-go-vault-tx='savings vault']").count() >= 1, dl[:200])
    check("and says plainly that moving your own coins is not a disposal, only the fee", "not a disposal" in dl and "190 sat fee leaves your hands" in dl, dl[:400])
    check("this vault's own address on it is a link to that address's page", pg.locator("#tx-dl a[data-go-addr='bc1qvaultchange']").count() == 1)
    pg.locator("#tx-dl a[data-go-vault-tx='savings vault']").first.click(); pg.wait_for_timeout(900)
    check("following it opens the same transaction in the other vault, lit in the menu", pg.evaluate("vault.name") == "savings vault" and pg.locator("#s-tx.on").count() == 1 and pg.evaluate("txView.txid") == "%s" % T[1] and pg.locator("#vault-tabs .nav.on").inner_text().startswith("savings vault") and "savings vault" in pg.inner_text("#crumbs"), pg.inner_text("#crumbs"))
    # ---- a report of the whole vault ----
    # (the Settings checks above left the currency on None: back to Australian dollars, as this vault's owner uses)
    pg.evaluate("(async () => { __state.fiat = 'AUD'; store = await window.__TAURI__.core.invoke('store_get'); vault = store.vaults.find(v => v.name !== 'savings vault'); await sync(); show('home'); })()"); pg.wait_for_timeout(1500)
    pg.click("#s-home [data-vault-report]"); pg.wait_for_timeout(900)
    rp = pg.evaluate("window.__report") or {}
    flat = json.dumps(rp)
    heads = [b.get("text") for b in rp.get("blocks", []) if b.get("kind") == "heading"]
    check("Home has Save as PDF: a report of the whole vault, titled with its name", rp.get("title") == pg.evaluate("vault.name") and "Saved" in pg.inner_text("#toast"), str(rp.get("title")))
    check("with its figures: holding, worth, cost basis (and the method), unrealised, realised", all(k in flat for k in ("Holding", "Worth now", "Cost basis", "Unrealised", "Realised", "The coins actually spent")), flat[:300])
    check("the two charts from Home", [b["chart"]["title"] for b in rp.get("blocks", []) if b.get("kind") == "chart"][:2] == ["Balance, in sats", "Worth and cost basis, in AUD"], str([b["chart"]["title"] for b in rp.get("blocks", []) if b.get("kind") == "chart"]))
    check("its keys, its coins and every transaction", "Keys" in heads and any(h.startswith("Coins held") for h in heads) and any(h.startswith("Transactions (") for h in heads), str(heads))
    check("a transfer between vaults is said on its line", "transfer to savings vault" in flat)
    check("and never the recovery line: that is the recovery sheet's, kept apart", "wsh(" not in flat and "sortedmulti" not in flat and "recovery sheet" in flat)
    pg.evaluate("(async () => { __state.fiat = ''; store = await window.__TAURI__.core.invoke('store_get'); vault = store.vaults.find(v => v.name !== 'savings vault'); await sync(); })()"); pg.wait_for_timeout(1200)   # as the Settings checks left it
    pg.evaluate("""() => { window.__peer = false; window.__xfers = {}; __extra = []; }""")
    pg.evaluate("(async () => { store = await window.__TAURI__.core.invoke('store_get'); vault = store.vaults[0]; show('home'); })()"); pg.wait_for_timeout(600)

    # ---- performance: what is known is shown, not fetched again ----
    n = lambda c: pg.evaluate(f"(window.__n || {{}})['{c}'] || 0")
    pg.evaluate("show('home')"); pg.wait_for_timeout(600)          # a sync just happened (or happens now)
    s0, a0 = n("vault_sync"), n("vault_activity")
    pg.evaluate("show('send')"); pg.wait_for_timeout(300); pg.evaluate("show('home')"); pg.wait_for_timeout(400)
    check("back to Home within half a minute: no sync, no activity fetch", n("vault_sync") == s0 and n("vault_activity") == a0, f"{n('vault_sync') - s0} syncs")
    check("and the balance is drawn from what is known", "9,430" in pg.inner_text("#balance"))
    pg.evaluate("(async () => { store = await window.__TAURI__.core.invoke('store_get'); vault = store.vaults[0]; show('coins'); })()"); pg.wait_for_timeout(500)
    check("a store re-read (as after any label or note) keeps the vault's coins: Coins does not sync again", n("vault_sync") == s0 and pg.evaluate("!!vault._coins"), f"{n('vault_sync') - s0} syncs")
    pg.evaluate("window.__listeners['vault-changed']({ payload: vault.name }); show('send')"); pg.wait_for_timeout(200); pg.evaluate("show('home')"); pg.wait_for_timeout(500)
    check("the watcher saw a change while away: the next Home syncs", n("vault_sync") == s0 + 1, f"{n('vault_sync') - s0} syncs")
    s1 = n("vault_sync"); pg.evaluate("vault._syncedAt = Date.now() - 31000; show('home')"); pg.wait_for_timeout(500)
    check("after half a minute Home syncs again", n("vault_sync") == s1 + 1)
    s2, a2 = n("vault_sync"), n("vault_activity")
    pg.evaluate("window.__listeners['prices-filled']({ payload: vault.name })"); pg.wait_for_timeout(400)
    check("prices arriving in the background redraw the money column, without asking the chain again", n("vault_activity") == a2 + 1 and n("vault_sync") == s2)
    pg.evaluate("window.__listeners['prices-filled']({ payload: 'some other vault' })"); pg.wait_for_timeout(300)
    check("and another vault's prices do not touch this screen", n("vault_activity") == a2 + 1)
    check("no page errors", not pg.errs, "; ".join(pg.errs)); pg.context.close()
    pg = page(b, "en-AU", "Australia/Brisbane", net="signet"); 
    check("a signet vault asks about the SIGNET server", 'signet' in pg.evaluate("window.__nodeAsked || []"))
    check("signet: dates and amounts, never a value", pg.evaluate("__state.asked.ccy") == "" and "+20,000 sats" in pg.inner_text("#activity") and "$" not in pg.inner_text("#s-home"))
    b.close()
audit.finish()
sys.exit(1 if fails else 0)
