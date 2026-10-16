#!/usr/bin/env python3
"""Market context, in WebKit against a mocked backend: the percentile on the Coins line, the "Market context" table on a
   deposit and a spend, the same table under the money on Send, the Purchase context card on Home (two percentiles, the
   three views, selection, date buttons, how it is calculated), the "Received highest" way of picking coins, and what
   is shown when the service is away or the vault is on signet. Reuses the Home test's mock, with the cycle commands added.
   usage: <python with playwright> test/ui-cycle.py <outdir>      exits non-zero on any failed check"""
import json, re, sys, pathlib
from playwright.sync_api import sync_playwright
here = pathlib.Path(__file__).resolve().parent; src = here.parent / "src"
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
home = (here / "ui-home.py").read_text()
K = json.loads(re.search(r'^K = (\[.*?\])$', home, re.M).group(1))
T = ["758adfd2a98e" + "0" * 52, "bcd3fb3da2e6" + "1" * 52, "7dbcc8c5894e" + "2" * 52, "aaaa11112222" + "3" * 52]
vault = lambda net: dict(name="demo vault", network=net, threshold=2, keys=[dict(name=n, serial=i + 1, pubkey=k) for i, (n, k) in enumerate(zip(["laptop", "keyring", "off-site"], K))], descriptor="wsh(…)", address="bc1qexample", created=0, chain_code=None, labels={}, notes={}, files={}, more={}, seal=dict(shares=[], sealed=[]))
MOCK = re.search(r'^MOCK = """(.*?)^"""', home, re.M | re.S).group(1)
# the cycle answers: the deposit at the 30th percentile (1.4×), the first spend at the 85th (2.1×), the second at the
# 95th (2.6×), anything happening now at the 85th (1.9×); the market line sits at the 50th the whole way
MOCK += """
window.__cycleAsked = [];
const MULT = { 1789627614: 1.4, 1789648528: 2.1, 1789699744: 2.6 };
const mark = (t, m) => ({ t, price: 70000 * m, wma: 70000, mult: m, pct: m < 1 ? 0.1 : m < 1.5 ? 0.3 : m < 2.5 ? 0.85 : 0.95 });
Object.assign(H, {
  cycle_marks: (a) => { window.__cycleAsked.push(a.times); if (window.__cycleDown) throw 'the price service could not be reached';
    return a.times.map(t => { const m = MULT[t] || (t > 1790000000 ? 1.9 : 1.4); return mark(t, m); }); },
  cycle_now: () => { window.__nowAsked = (window.__nowAsked || 0) + 1; if (window.__cycleDown) throw 'offline'; return mark(Math.floor(Date.now() / 1000) - (window.__stale ? 5 * 86400 : 0), 1.9); },
  cycle_curve: () => { const weeks = []; for (let t = 1789627614 - 60 * 86400; t <= Date.now() / 1000; t += 7 * 86400) weeks.push({ day: new Date(t * 1000).toISOString().slice(0, 10), t, close: 112000, wma: 70000, mult: 1.6, pct: 0.5 }); return { weeks, ranks: [], since: '2014-06-12' }; },
});
"""
fails = []
def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + (f" — {detail}" if detail and not ok else ""))
    if not ok: fails.append(name)
def page(b, net="mainnet", width=1300):
    ctx = b.new_context(locale="en-AU", timezone_id="Australia/Brisbane", viewport=dict(width=width, height=900)); pg = ctx.new_page(); pg.errs = []
    pg.on("pageerror", lambda e: pg.errs.append(str(e)))
    pg.add_init_script(MOCK % dict(vault=json.dumps(vault(net)), fiat=json.dumps("auto"), os=json.dumps(dict(region="AU", currency=None)), t0=T[0], t1=T[1], t2=T[2], t3=T[3]))
    pg.goto((src / "index.html").as_uri()); pg.wait_for_timeout(1200); return pg
with sync_playwright() as p:
    b = p.webkit.launch()
    pg = page(b); pg.wait_for_timeout(800)
    # ---- Home: the portfolio tiles as they were, then the Purchase context card ----
    tiles = [t.strip().lower() for t in pg.locator("#home-summary .kpi .k").all_inner_texts()]
    check("the portfolio row is untouched: holding, worth now, cost basis, unrealised, realised", tiles == ["holding", "worth now", "cost basis", "unrealised", "realised"], str(tiles))
    pc = pg.locator("#home-summary .pc"); check("one Purchase context card, after the charts", pc.count() == 1 and pg.locator("#home-summary .chart").count() >= 3)
    st = [s.inner_text().replace("\n", " | ") for s in pc.locator(".pc-stat").all()]
    check("'Coins received' leads with the sats-weighted percentile, weighted by sats, with the multiple beside it", st[0].startswith("Coins received") and "33rd" in st[0] and "percentile" in st[0] and "weighted by sats" in st[0] and "2 arrivals" in st[0] and "1.4× the 200-week average" in st[0], st[0])
    check("'Market now' is the percentile now with the multiple beside it", st[1].startswith("Market now") and "85th" in st[1] and "1.9× the 200-week average" in st[1], st[1])
    check("one batch of marks was asked for, not one request per dot", len(pg.evaluate("__cycleAsked")) <= 2, str(pg.evaluate("__cycleAsked")))
    # the timeline is the default view: a market line, arrivals filled, departures hollow, the newest arrival selected
    check("Over time is the default view, with the market line and every point on it", pc.locator("[data-view=time][aria-checked=true]").count() == 1 and pc.locator(".mkt-line").count() == 1 and pc.locator("circle.buy").count() == 2 and pc.locator("circle.sell").count() == 2, f"line {pc.locator('.mkt-line').count()} buy {pc.locator('circle.buy').count()} sell {pc.locator('circle.sell').count()}")
    sel = pc.locator(".pc-sel").inner_text().replace("\n", " | ")
    check("the newest arrival is selected and read out: date, sats, that it is counted as a purchase, percentile, multiple", "1,000 sats" in sel and "counted as a purchase" in sel and "85th percentile" in sel and "1.9× the 200-week average" in sel, sel)
    check("a button per arrival and departure, departures marked, the selected one pressed", pc.locator(".pc-pick").count() == 4 and pc.locator(".pc-pick.left").count() == 2 and pc.locator(".pc-pick[aria-pressed=true]").count() == 1, str(pc.locator(".pc-pick").all_inner_texts()))
    pc.locator(".pc-pick").nth(0).click(); pg.wait_for_timeout(150); sel = pc.locator(".pc-sel").inner_text().replace("\n", " | ")
    check("choosing the first date selects the 20,000-sat deposit at the 30th percentile", "20,000 sats" in sel and "30th percentile" in sel and "1.4×" in sel, sel)
    pc.locator("circle.sell").first.click(); pg.wait_for_timeout(150); sel = pc.locator(".pc-sel").inner_text().replace("\n", " | ")
    check("clicking a hollow dot selects that departure", "Left the vault" in sel and ("85th" in sel or "95th" in sel), sel)
    check("every dot is reachable by keyboard, with its words on it", pc.locator("circle.pt[tabindex='0']").count() == 4 and "percentile" in (pc.locator("circle.pt").first.get_attribute("aria-label") or ""))
    pc.screenshot(path=str(out / "cycle-home-time.png"))
    pc.locator("[data-view=dist]").click(); pg.wait_for_timeout(200)
    check("Distribution: four shaded quarters, 'Historically lower/higher', and now in its own lane above the plot", pc.locator(".band").count() == 4 and "Historically lower" in pc.inner_text() and "Historically higher" in pc.inner_text() and "Now · 85th" in pc.inner_text(), pc.locator(".pc-chart").inner_text())
    pc.screenshot(path=str(out / "cycle-home-dist.png"))
    pc.locator("[data-view=price]").click(); pg.wait_for_timeout(200)
    sel = pc.locator(".pc-sel").inner_text().replace("\n", " | ")
    check("Price & 200WMA: the price line, the dashed average, now on the average, and the readout in dollars", pc.locator(".px-line").count() == 1 and pc.locator(".wma-line").count() == 1 and pc.locator(".nowmark").count() == 1 and "Spot " in sel and "70,000" in sel, sel)
    pc.screenshot(path=str(out / "cycle-home-price.png"))
    pc.locator(".pc-how summary").click(); pg.wait_for_timeout(150); how = pc.locator(".pc-howbody").inner_text()
    check("'How this is calculated' says: USD over the 200-week average; ranked against weeks known on that day; weighted by sats; the steady weekly buy in percentiles; what a lower percentile does and does not mean; arrivals counted as purchases", all(x in how for x in ["known on that day", "weighted by sats", "steady weekly buy", "50th percentile", "33rd", "not that the coins were cheaper", "counted as a purchase", "not what to do"]), how)
    # ---- a deposit and a spend: the money as before, the market context as its own row after it ----
    rows = pg.locator("#activity tr"); rows.nth(3).click(); pg.wait_for_timeout(700)
    facts = pg.inner_text("#tx-facts"); fl = facts.lower()
    check("a deposit keeps its Cost line and gains a 'Market context' table: When received against Now, percentile and multiple", "its cost, if it was bought" in facts and "market context" in fl and "when received" in fl and "historical percentile" in fl and "30th" in facts and "85th" in facts and "1.4×" in facts and "1.9×" in facts, facts)
    pg.locator("#s-tx .card").first.screenshot(path=str(out / "cycle-tx-deposit.png"))
    pg.evaluate("show('home')"); pg.wait_for_timeout(300); pg.locator("#activity tr").nth(2).click(); pg.wait_for_timeout(700)
    facts = pg.inner_text("#tx-facts"); fl = facts.lower()
    check("a spend keeps its Realised line as it was, then a table: When received against When it left", "they had cost $5.57" in facts and "by the coins this transaction actually spent" in facts and "when it left" in fl and "30th" in facts and "85th" in facts and "1.4×" in facts and "2.1×" in facts, facts)
    pg.locator("#s-tx .card").first.screenshot(path=str(out / "cycle-tx-spend.png"))
    # ---- Coins ----
    pg.evaluate("show('send')"); pg.wait_for_timeout(400); pg.evaluate("document.querySelector('#coins-adv').open = true"); pg.wait_for_timeout(300)
    ct = pg.inner_text("#coin-list")
    check("the coin says what percentile it was received at, with the strip", "received at the 30th percentile" in ct and pg.locator("#coin-list .cyc i").count() == 4 and pg.evaluate("[...document.querySelectorAll('#coin-list .cyc i')].findIndex(i => i.classList.contains('on'))") == 1, ct)
    check("the strip reads the same in words", "30th percentile" in (pg.get_attribute("#coin-list .cyc", "aria-label") or "") and "1.4× the 200-week average" in (pg.get_attribute("#coin-list .cyc", "aria-label") or ""))
    check("'Received highest' is one of the ways to pick", pg.locator("#coin-plans [data-plan=dear]").count() == 1 and "Received highest" in pg.inner_text("#coin-plans"))
    pg.fill("#send-to", "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"); pg.fill("#send-amount", "5000"); pg.wait_for_timeout(900)
    pg.click("#coin-plans [data-plan=dear]"); pg.wait_for_timeout(400)
    check("picking by 'Received highest' ticks the coin and says why", pg.locator("#coin-list input:checked").count() == 1 and "highest historical percentile" in pg.inner_text(".toast"), pg.inner_text(".toast"))
    # ---- Send: the money rows as they were, then the table ----
    pg.wait_for_timeout(600); rv = pg.inner_text("#review"); rl = rv.lower()
    check("the review keeps They cost / Worth now / Gain, then the table: When received against Now", "They cost" in rv and "Worth now" in rv and "Gain if this is a disposal" in rv and "when received" in rl and "historical percentile" in rl and "30th" in rv and "85th" in rv and "1.9×" in rv and "not what to do" in rv, rv)
    pg.locator("#s-send").screenshot(path=str(out / "cycle-send.png"))
    # ---- a steady buyer: twenty arrivals get a stepper, not twenty buttons ----
    pg.evaluate("""() => { const d = document.createElement('div'); d.id = 'stp'; document.body.appendChild(d); const tl = []; let s = 0;
      for (let i = 0; i < 20; i++) { const t = 1789627614 - (20 - i) * 14 * 86400; tl.push({ t, txid: 'x' + i, before: { sats: s }, sats: s + 1000, internal: false }); s += 1000; }
      cycleSummary({ timeline: tl }, d, () => true); }"""); pg.wait_for_timeout(900)
    stp = pg.locator("#stp")
    check("with twenty arrivals the list is a stepper with a count, the newest selected", stp.locator(".pc-pick").count() == 0 or stp.locator("[data-step]").count() == 2, str(stp.locator(".pc-pick").count()))
    check("the count reads 20 of 20 and 'Newer' is disabled at the end", "20 of 20" in stp.inner_text() and stp.locator("[data-step='1']").is_disabled(), stp.locator(".pc-list").inner_text())
    stp.locator("[data-step='-1']").click(); pg.wait_for_timeout(150)
    check("Older steps back one", "19 of 20" in stp.inner_text(), stp.locator(".pc-list").inner_text())
    check("no page errors", not pg.errs, "; ".join(pg.errs))
    pg.context.close()
    # ---- a narrow window: the card still fits, nothing overflows sideways ----
    pg = page(b, width=760); pg.wait_for_timeout(900)
    over = pg.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
    check("at 760 px the Home does not scroll sideways and the card is drawn", over <= 0 and pg.locator("#home-summary .pc .pc-chart svg").count() == 1, f"overflow {over}")
    pg.locator("#home-summary .pc").screenshot(path=str(out / "cycle-home-760.png")); pg.context.close()
    # ---- the price service has no price for this hour: the newest close stands in, dated ----
    pg = page(b); pg.evaluate("window.__stale = true; cycleNowMemo = null"); pg.evaluate("show('send')"); pg.wait_for_timeout(300)
    pg.fill("#send-to", "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"); pg.fill("#send-amount", "5000"); pg.wait_for_timeout(1200)
    rv = pg.inner_text("#review"); check("a stale 'now' says which day it is as of", "as of" in rv, rv); pg.context.close()
    # ---- the service is away: every screen still draws, the context says there is none, nothing errors ----
    pg = page(b); pg.evaluate("window.__cycleDown = true; cycleMemo.clear(); cycleNowMemo = null; cycleCurveP = null; renderHomeSummary()"); pg.wait_for_timeout(1000)
    hs = pg.inner_text("#home-summary")
    check("Home without the service: the tiles stand, the card says there is no price", "WORTH NOW" in hs.upper() and "$11.27" in hs and "no price or no average" in hs, hs[:500])
    pg.evaluate("show('send')"); pg.wait_for_timeout(300); pg.fill("#send-to", "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"); pg.fill("#send-amount", "5000"); pg.wait_for_timeout(1000)
    rv = pg.inner_text("#review"); check("Send without the service: the money rows stand, the table shows dashes", "They cost" in rv and "$5.57" in rv and "1.9×" not in rv and "—" in rv, rv)
    check("no page errors with the service away", not pg.errs, "; ".join(pg.errs)); pg.context.close()
    # ---- test coins have no price, so nothing of this is said ----
    pg = page(b, "signet"); pg.evaluate("show('send')"); pg.wait_for_timeout(300); pg.evaluate("document.querySelector('#coins-adv').open = true"); pg.wait_for_timeout(300)
    check("on signet no percentile is shown and no marks are asked for", "received at" not in pg.inner_text("#coin-list") and pg.evaluate("__cycleAsked.length") == 0 and pg.locator("#home-summary .pc").count() == 0, pg.inner_text("#coin-list"))
    pg.context.close(); b.close()
print(f"\n{len(fails)} failed" if fails else "\nall checks passed"); sys.exit(1 if fails else 0)
