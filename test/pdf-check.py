#!/usr/bin/env python3
"""Open the PDFs this app wrote with a library that is not ours (pypdf) and check they say what they should.
   usage: <python with pypdf> test/pdf-check.py [dir]     default dir: $TMPDIR/a2k-pdf     exits non-zero on any failure"""
import sys, os, pathlib, re
from pypdf import PdfReader
d = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.environ.get("TMPDIR", "/tmp"), "a2k-pdf"))
files = sorted(d.glob("*.pdf"))
fails = 0
def check(name, ok, detail=""):
    global fails
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else f" — {detail}"))
    if not ok: fails += 1
check(f"there are PDFs to read in {d}", bool(files), "run: cargo test a_document_carries -- --nocapture")
for f in files:
    r = PdfReader(str(f))
    text = "\n".join(p.extract_text() or "" for p in r.pages)
    imgs = [im for p in r.pages for im in p.images]
    check(f"{f.name}: another library opens it and finds pages", len(r.pages) >= 1, f"{len(r.pages)} pages")
    check(f"{f.name}: its words come back out", "Any Two Keys" in text, text[:120])
    if f.name == "doc.pdf":
        check("doc.pdf: the title, a fact, a note over two lines and a caption are all there",
              all(s in text for s in ["Lets check if labelling still works", "+9,171 sats", "hourly 2026-09-18 08:00 UTC", "Ask Sam about the warranty.", "IMG_0412.jpg", "receipt.jpg"]),
              repr(text[:400]))
        check("doc.pdf: both pictures are in the file and are real images", len(imgs) == 2 and all(im.image.size == (400, 300) for im in imgs), f"{len(imgs)} images")
        draws = [float(x) for x in re.findall(r"q (\d+\.\d) 0 0 \d+\.\d", "\n".join(p.get_contents().get_data().decode("latin-1") for p in r.pages))]
        check("doc.pdf: the wide picture is drawn wider than the narrow one", len(draws) == 2 and draws[0] > draws[1] + 100, draws)
        check("doc.pdf: a page names only the pictures it draws", all(len(p.get("/Resources", {}).get("/XObject", {})) <= 1 for p in r.pages), [len(p.get("/Resources", {}).get("/XObject", {})) for p in r.pages])
    elif f.name == "sheet.pdf":
        check("sheet.pdf: the money, the counterparty by name, the keys that signed and the place all print",
              all(s in text for s in ["Lets check if labelling still works", "5,190 sats", "8.54 AUD", "hardware shop", "laptop and off-site", "445 Upper Edward Street", "Ask Sam about the warranty.", "invoice-4471.pdf"]),
              repr(text[:600]))
        check("sheet.pdf: the chain's own facts are on it, with the id and the proof",
              "abababababababababababababababababababababababababababababababab" in text.replace("\n", "") and "proved" in text and "position 1,757" in text, repr(text[-600:]))
        flat = " ".join(text.split())
        check("sheet.pdf: a long place wraps onto another line instead of running off the paper",
              "445 Upper Edward Street, Spring Hill, Brisbane, Queensland 4000, Australia" in flat and max((len(l) for l in text.splitlines()), default=0) < 130,
              f"longest line {max((len(l) for l in text.splitlines()), default=0)}")
    else:
        check(f"{f.name}: a transaction sheet names its vault, its id and how inclusion was shown",
              "printed 20" in text and re.search(r"Transaction\s+[0-9a-f]{64}", text.replace("\n", " ")) is not None and ("proved" in text or "not in a block yet" in text),
              repr(text[:300]))
print(f"{fails} failed")
sys.exit(1 if fails else 0)
