"""Spacing audit hooked into a UI harness: every check the harness makes also measures the page it is on.
   install(globals()) in a harness; report() at its end prints what was found (and returns the count)."""
import pathlib
JS = (pathlib.Path(__file__).parent / "layout-audit.js").read_text()
found = {}   # (kind, a, b) -> (px, where)
took = []
last = {}

def install(g):
    import os
    real = g["check"]
    global _real; _real = real
    if os.environ.get("A2K_NO_AUDIT"): return
    def check(name, ok, detail=""):
        pg = g.get("pg")
        try:
            if pg is not None and not pg.is_closed():
                where = pg.evaluate("(() => { const m = document.querySelector('.modal.on'); const s = document.querySelector('.screen.on'); return (m ? 'modal ' + m.id : 'screen ' + (s ? s.id : '?')); })()")
                # measured when what is on screen has changed shape, not at every check: a full measuring pass right
                # before a harness's next click made the camera checks flaky (6 of 6 clean without it, 7 of 10 failing with it)
                shape = pg.evaluate("document.querySelectorAll('.screen.on *, .modal.on *').length") if where else 0
                if (where, shape) == last.get("at"): return real(name, ok, detail)
                last["at"] = (where, shape)
                import time as _t; t0 = _t.time()
                res = pg.evaluate(JS + "; window.__layoutAudit()")
                took.append(_t.time() - t0)
                for v in res:
                    found.setdefault((v["kind"], v["a"], v["b"]), (v["px"], where))
        except Exception:
            pass
        return real(name, ok, detail)
    g["check"] = check

def finish():
    n = report()
    _real("every screen reached keeps its spacing (test/layout-audit.js)", n == 0, f"{n} problem(s) listed above")

def report():
    if took: print(f"layout audit: {len(took)} runs, longest {max(took)*1000:.0f} ms, total {sum(took):.1f} s")
    if not found: print("layout: no spacing problems found"); return 0
    print(f"layout: {len(found)} spacing problem(s)")
    for (kind, a, b), (px, where) in sorted(found.items(), key=lambda x: x[1][1]):
        print(f"  [{where}] {kind} ({px}px): {a}" + (f"  ↔  {b}" if b else ""))
    return len(found)
