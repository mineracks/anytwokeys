#!/usr/bin/env python3
"""The Metal panel against a mocked backend: share plates from the vault's settings (3 shares, two sides each),
   the pen-up trace arming the engrave button, and "Engrave anything" from Settings. The real engraving engine
   (vendor/composer/composer.wasm) runs in the page, so the previews and stroke counts are the real ones.
   usage: <python with playwright> test/ui-metal.py <outdir>      exits non-zero on any failed check"""
import http.server, json, pathlib, sys, threading
from playwright.sync_api import sync_playwright

here = pathlib.Path(__file__).resolve().parent; src = here.parent / "src"
# reuse ui-home's mock backend (everything up to its browser session), then add the engraver
home = (here / "ui-home.py").read_text().split("with sync_playwright() as p:")[0]
exec(compile(home, str(here / "ui-home.py"), "exec"))
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)

EXTRA = r"""
const PAY = (seed) => Array.from({ length: 410 }, (_, i) => (i * 7 + seed) & 255);
Object.assign(H, {
  vault_shares_plates: (a) => { window.__platesAsked = a; return Array.from({ length: a.n }, (_, i) => ({ index: i + 1, set: 'c9ad277c', ur_type: 'envelope', payload: PAY(i + 3), ur: 'ur:envelope/mock' + i })); },
  engraver_find: () => window.__noEngraver ? [] : [{ path: '/dev/cu.usbserial-A100U6EB', serial: 'A100U6EB', product: 'FT232R USB UART' }],
  engraver_jog: () => 'moved to (5.0, 5.0) mm and back to (0.0, 0.0) mm',
  engraver_cancel: () => { window.__cancelled = true; return null; },
  engraver_run: (a) => new Promise((ok) => { window.__runs = (window.__runs || []).concat([{ plate: a.plate, dry: a.dry, n: a.commands.length / 10 }]);
    const total = a.commands.length / 10; let done = 0;
    const tick = () => { done = Math.min(total, done + Math.ceil(total / 4)); const fn = (window.__listeners || {})['engrave-progress']; if (fn) fn({ payload: { done, total, dry: a.dry } });
      if (done < total) setTimeout(tick, 60); else setTimeout(() => ok(a.dry ? `traced ${total} commands with the pen up` : `engraved ${total} commands`), 60); };
    setTimeout(tick, 60); }),
});
"""

class Quiet(http.server.SimpleHTTPRequestHandler):
    protocol_version = "HTTP/1.0"
    def log_message(self, *a): pass
    def end_headers(self):
        if self.path.endswith(".wasm"): self.send_header("Content-Type", "application/wasm")
        super().end_headers()

def serve():
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), lambda *a, **k: Quiet(*a, directory=str(src), **k))
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd.server_address[1]

def mpage(b, port):
    ctx = b.new_context(locale="en-AU", timezone_id="Australia/Brisbane", viewport=dict(width=1300, height=1000)); pg = ctx.new_page(); pg.errs = []
    pg.on("pageerror", lambda e: pg.errs.append(str(e)))
    pg.add_init_script(MOCK % dict(vault=json.dumps(vault("mainnet")), fiat=json.dumps("auto"), os=json.dumps(dict(region=None, currency=None)), t0=T[0], t1=T[1], t2=T[2], t3=T[3]) + EXTRA)
    pg.goto(f"http://127.0.0.1:{port}/index.html", wait_until="domcontentloaded"); pg.wait_for_timeout(1200); return pg

port = serve()
with sync_playwright() as p:
    try: b = p.webkit.launch()
    except Exception: b = p.chromium.launch()
    pg = mpage(b, port)
    # ---- share plates from the vault's settings ----
    pg.evaluate("renderVaultSettings.shown = null; show('vault')"); pg.wait_for_timeout(400)
    check("the shares block offers metal", pg.locator("#vs-shares-metal").is_visible())
    pg.click("#vs-shares-metal"); pg.wait_for_timeout(3500)
    check("the Metal panel opened", pg.locator("#metal").is_visible() and not pg.errs, "; ".join(pg.errs))
    chips = pg.locator("#metal-chips .vchip").count()
    check("three shares became six plate sides", chips == 6, f"{chips} chips")
    check("the first side is selected and previewed by the engine", pg.locator("#metal-chips .vchip[aria-pressed='true']").inner_text().startswith("Share 1 of 3 · side A") and pg.locator("#metal-preview svg path").count() >= 1)
    facts = pg.inner_text("#metal-facts")
    check("facts name the plate and the strokes", "SH-03" in facts and "strokes" in facts, facts)
    check("the engraver was found on its port", "usbserial-A100U6EB" in pg.inner_text("#metal-engraver"))
    check("engrave is disabled until a trace has been run", pg.locator("#metal-go").is_disabled() and pg.locator("#metal-trace").is_enabled())
    pg.locator("#metal").screenshot(path=str(out / "metal-shares.png")); pg.screenshot(path=str(out / "vault-screen.png"), full_page=True)
    pg.click("#metal-trace"); pg.wait_for_timeout(800)
    runs = pg.evaluate("window.__runs")
    check("the trace went to the engraver pen-up on a Large plate", runs and runs[0]["dry"] is True and runs[0]["plate"] == 2 and runs[0]["n"] > 1000, json.dumps(runs))
    check("after the trace the engrave button is armed", pg.locator("#metal-go").is_enabled() and "✓" in pg.locator("#metal-chips .vchip").first.inner_text())
    say = pg.inner_text("#metal-say"); check("and the panel says so", "pen up" in say and "armed" in say, say)
    pg.locator("#metal").screenshot(path=str(out / "metal-traced.png"))
    pg.click("#metal-chips .vchip >> nth=1"); pg.wait_for_timeout(300)
    check("another side is not armed by the first side's trace", pg.locator("#metal-go").is_disabled())
    # the text side's preview is text, not a code: more strokes than the QR side's
    pg.locator("#metal").screenshot(path=str(out / "metal-side-b.png"))
    # ---- no engraver plugged in ----
    pg.evaluate("window.__noEngraver = true"); pg.click("#metal-engraver #metal-again"); pg.wait_for_timeout(400)
    check("without an engraver the row says what to plug in, and nothing is enabled", "No engraver" in pg.inner_text("#metal-engraver") and pg.locator("#metal-trace").is_disabled())
    pg.evaluate("window.__noEngraver = false")
    # ---- engrave anything, from settings ----
    pg.evaluate("show('settings')"); pg.wait_for_timeout(300)
    check("settings has the card", pg.locator("#any-card").is_visible())
    pg.locator("#any-card").screenshot(path=str(out / "any-card.png")); pg.screenshot(path=str(out / "settings-screen.png"), full_page=True)
    pg.fill("#any-text", "family vault\nkeep dry"); pg.click("#any-size .vchip[data-pt='18']"); pg.click("#any-go"); pg.wait_for_timeout(1500)
    check("a two-line label laid out on an SH-02", pg.locator("#metal").is_visible() and "SH-02" in pg.inner_text("#metal-facts") and pg.locator("#metal-chips .vchip").count() == 1, pg.inner_text("#metal-facts"))
    pg.locator("#metal").screenshot(path=str(out / "metal-any.png"))
    # ---- an SVG drawing ----
    pg.evaluate("show('settings')"); pg.wait_for_timeout(200)
    pg.fill("#any-text", ""); pg.locator("#any-plate button[data-plate='2']").click()
    svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60"><rect x="5" y="5" width="90" height="50"/><circle cx="50" cy="30" r="20"/><text>logo</text></svg>'
    pg.set_input_files("#any-svg input", dict(name="logo.svg", mimeType="image/svg+xml", buffer=svg.encode()))
    pg.wait_for_timeout(500)
    check("the drawing was read and fitted", "logo.svg" in pg.inner_text("#any-svg") and "2 paths" in pg.inner_text("#any-svg"), pg.inner_text("#any-svg"))
    pg.click("#any-go"); pg.wait_for_timeout(1500)
    facts = pg.inner_text("#metal-facts")
    check("line art laid out on an SH-03, inside the margins", "SH-03" in facts and not pg.locator("#metal-facts .warn").count(), facts)
    pg.locator("#metal").screenshot(path=str(out / "metal-svg.png")); pg.screenshot(path=str(out / "settings-svg-screen.png"), full_page=True)
    check("no page errors", not pg.errs, "; ".join(pg.errs))
    b.close()
print(f"\n{len(fails)} failed" if fails else "\nall ok"); sys.exit(1 if fails else 0)
