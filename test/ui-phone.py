#!/usr/bin/env python3
"""Every screen of Any Two Keys at iPhone width (430 px, WebKit), against ui-home.py's mocked backend: document width, elements past the
right edge, the spacing audit, and a full-height screenshot of each. usage: <python with playwright> test/ui-phone.py <outdir> [screen ...]   -- exits non-zero when any screen is wider than the phone
   VNAME=... sets the vault name (a long one is the hard case for the header)"""
import json, re, sys, pathlib
from playwright.sync_api import sync_playwright
here = pathlib.Path(__file__).resolve().parent; root = here.parent; src = root / "src"; test = here
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
only = sys.argv[2:]
uh = (test / "ui-home.py").read_text()
MOCK = re.search(r'MOCK = """(.*?)"""', uh, re.S).group(1)
K = eval(re.search(r'^K = (\[.*?\])$', uh, re.M | re.S).group(1))
T = ["758adfd2a98e" + "0" * 52, "bcd3fb3da2e6" + "1" * 52, "7dbcc8c5894e" + "2" * 52, "aaaa11112222" + "3" * 52]
ns = {}; exec(re.search(r'^vault = lambda.*$', uh, re.M).group(0), {"K": K}, ns); vault = ns["vault"]
EXTRA = """
H.platform = () => 'ios'; H.attach_view = () => ({ mime: 'image/gif', base64: 'R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7' });
H.card_tap = () => { throw 'cancelled'; };
"""
W = 430
with sync_playwright() as p:
    b = p.webkit.launch()
    ctx = b.new_context(locale="en-AU", timezone_id="Australia/Brisbane", viewport=dict(width=W, height=932), device_scale_factor=2, is_mobile=True, has_touch=True)
    pg = ctx.new_page(); errs = []; pg.on("pageerror", lambda e: errs.append(str(e)))
    pg.add_init_script(MOCK % dict(vault=json.dumps(dict(vault("mainnet"), name=__import__("os").environ.get("VNAME", "demo vault"))), fiat=json.dumps("auto"), os=json.dumps(dict(region="AU", currency=None)), t0=T[0], t1=T[1], t2=T[2], t3=T[3]) + EXTRA)
    pg.add_init_script((test / "layout-audit.js").read_text())
    pg.goto((src / "index.html").as_uri()); pg.wait_for_timeout(1200)
    screens = pg.evaluate("[...document.querySelectorAll('.screen')].map(s => s.id.replace(/^s-/, ''))")
    goes = pg.evaluate("[...new Set([...document.querySelectorAll('[data-go]')].map(e => e.dataset.go))]")
    print("screens:", " ".join(screens)); print("nav targets:", " ".join(goes))
    MEASURE = """(W) => {
      const on = document.querySelector('.screen.on'); const name = (e) => { let s = e.tagName.toLowerCase(); if (e.id) s += '#' + e.id; else if (typeof e.className === 'string' && e.className.trim()) s += '.' + e.className.trim().split(/\\s+/).slice(0, 2).join('.'); const t = (e.textContent || '').trim().replace(/\\s+/g, ' ').slice(0, 28); return t ? s + ' "' + t + '"' : s; };
      const over = []; const seen = new Set();
      for (const e of [document.querySelector('.top'), on].filter(Boolean).flatMap(r => [r, ...r.querySelectorAll('*')])) {
        const cs = getComputedStyle(e); if (cs.display === 'none' || cs.visibility === 'hidden') continue; const r = e.getBoundingClientRect(); if (r.width === 0) continue;
        if (r.right > W + 1 || r.left < -1) { const k = name(e); if (!seen.has(k)) { seen.add(k); over.push({ el: k, left: Math.round(r.left), right: Math.round(r.right) }); } }
      }
      // the widest leaf is the cause; its ancestors just carry it
      const leaves = over.filter(o => !over.some(p => p !== o && o.el !== p.el && false));
      const bodyW = document.body.scrollWidth;
      return { body: bodyW, doc: document.documentElement.scrollWidth, content: (document.querySelector('.content') || {}).scrollWidth, top: (document.querySelector('.top') || {}).scrollWidth, over: over.slice(0, 12), audit: (window.__layoutAudit ? window.__layoutAudit() : []).slice(0, 8), h1: (document.querySelector('.top h1') || {}).textContent, crumbs: (document.querySelector('#crumbs') || {}).innerText };
    }"""
    report = {}
    pg.add_style_tag(content=".main,.content{overflow:visible!important;height:auto!important;max-height:none!important} body{height:auto!important;overflow:visible!important}")
    for s in (only or goes + ['tx', 'entity', 'coin', 'drawer', 'welcome']):
        try:
            if s == 'tx': pg.evaluate("(t) => openTx(t)", T[1])
            elif s == 'addresses': pg.evaluate("() => { window.__rotating = true; return sync(); }"); pg.wait_for_timeout(800); pg.evaluate("show('addresses')")
            elif s == 'entity': pg.evaluate("() => { store.entities = [{ id: 'e1', name: 'mineracks', kind: 'company', note: 'mineracks pty ltd', added: 1 }]; store.vaults.forEach(v => v.entity = 'e1'); store.icons = { 'entity:e1': 'img:' + 'a'.repeat(64) }; openEntity('e1'); }")
            elif s == 'activity': pg.evaluate("(t) => { window.__xfers = { [t]: ['Demo - Mixed YubiKey and SeedSigner Vault'] }; return sync(); }", T[1]); pg.wait_for_timeout(800); pg.evaluate("show('activity')")
            elif s == 'drawer': pg.evaluate("show('home'); document.body.classList.add('drawer')"); pg.wait_for_timeout(400)
            elif s == 'welcome': pg.evaluate("showWelcome()")
            elif s == 'coin': pg.evaluate("() => { show('coins'); }"); pg.wait_for_timeout(500); pg.evaluate("() => { const tr = document.querySelector('#coin-rows tr[data-op]'); if (tr) tr.click(); }")
            else: pg.evaluate("(s) => { try { show(s); } catch (e) { return String(e); } }", s)
            pg.wait_for_timeout(700)
        except Exception as e: print("show", s, "failed:", e); continue
        m = pg.evaluate(MEASURE, W); report[s] = m
        flag = " <-- WIDE" if (m["doc"] or 0) > W or (m["body"] or 0) > W or (m["content"] or 0) > W or (m["top"] or 0) > W else ""
        print(f"\n== {s}: body {m['body']} doc {m['doc']} content {m['content']} top {m['top']}{flag}  h1={m['h1']!r}")
        for o in m["over"]: print("   over:", o["el"], o["left"], o["right"])
        for a in m["audit"]: print("   audit:", a["kind"], "|", a["a"], "|", a["b"], a["px"])
        pg.screenshot(path=str(out / f"{s}.png"), full_page=True)
    (out / "report.json").write_text(json.dumps(report, indent=1))
    print("\npage errors:", errs[:5])
    b.close()
    wide = [s for s, m in report.items() if max(m.get("body") or 0, m.get("doc") or 0, m.get("content") or 0, m.get("top") or 0) > W]
    print("wider than the phone:", wide or "none")
    sys.exit(1 if wide or errs else 0)
