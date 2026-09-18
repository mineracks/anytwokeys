#!/usr/bin/env python3
"""Rebuild an Any Two Keys vault from the shares sealed on its cards — without the app.

Standard library only. The format is described in docs/SEAL.md.

  Read a card's container (no PIN needed), e.g. with GnuPG's scdaemon:
      gpg-connect-agent "scd apdu 00 A4 04 00 06 D2 76 00 01 24 01" "scd apdu 00 CA 7F 21 00" /bye
  or pass the hex of the whole container; this tool lists the entries in it:
      seal-recover.py container <hex>
  Then combine k shares of the same vault id:
      seal-recover.py combine <x>:<sharehex> <x>:<sharehex> [...]
"""
import hashlib, sys

# GF(2^8) over x^8+x^4+x^3+x+1 by log/antilog tables, generator 3
EXP, LOG = [0] * 510, [0] * 256
_v = 1
for _i in range(255):
    EXP[_i], LOG[_v] = _v, _i
    _v ^= (_v << 1) ^ (0x11B if _v & 0x80 else 0)   # multiply by 3 = x·2 + x
    _v &= 0xFF
for _i in range(255, 510):
    EXP[_i] = EXP[_i - 255]

def mul(a, b): return 0 if a == 0 or b == 0 else EXP[LOG[a] + LOG[b]]
def div(a, b): return 0 if a == 0 else EXP[LOG[a] + 255 - LOG[b]]

def interpolate(points, at=0):
    """points: [(x, bytes)] → the polynomial's value at `at` for every byte."""
    out = bytearray(len(points[0][1]))
    for i, (xi, si) in enumerate(points):
        w = 1
        for j, (xj, _) in enumerate(points):
            if i != j: w = mul(w, div(at ^ xj, xi ^ xj))
        for p, s in enumerate(si): out[p] ^= mul(w, s)
    return bytes(out)

def split(secret, k, n, random):
    assert len(random) == (k - 1) * len(secret)
    shares = []
    for x in range(1, n + 1):
        sh = bytearray()
        for j, s in enumerate(secret):
            y = 0
            for c in range(k - 1, 0, -1): y = mul(y ^ random[(c - 1) * len(secret) + j], x)
            sh.append(y ^ s)
        shares.append(bytes(sh))
    return shares

def decode(b):
    body, chk = b[:-4], b[-4:]
    if len(b) < 9 or hashlib.sha256(body).digest()[:4] != chk or body[0] != 1:
        raise SystemExit("these shares do not rebuild a record: wrong cards together, or too few")
    net, k, n, rot = ("mainnet", "signet")[body[1]], body[2], body[3], body[4] & 1
    i = 5
    keys = [body[i + 33 * m:i + 33 * m + 33].hex() for m in range(n)]; i += 33 * n
    cc = None
    if rot: cc = body[i:i + 32].hex(); i += 32
    texts = []
    for _ in range(n + 1):
        l = body[i]; texts.append(body[i + 1:i + 1 + l].decode("utf-8", "replace")); i += 1 + l
    return dict(network=net, threshold=k, keys=keys, chain_code=cc, name=texts[0], key_names=texts[1:])

def container(b):
    if not b: return []
    if b[:4] != b"A2K\x01": raise SystemExit("not an Any Two Keys container")
    i, out = 4, []
    while i < len(b):
        ident, k, x, ln = b[i:i + 4].hex(), b[i + 4], b[i + 5], int.from_bytes(b[i + 6:i + 8], "big")
        out.append((ident, k, x, b[i + 8:i + 8 + ln])); i += 8 + ln
    return out

def vector():
    """The frozen test vector the Rust build checks itself against (src-tauri/src/seal.rs)."""
    keys = ["03375055e53d2e4acd7245ef554e054fffe4bae7385adbb019afdd924337b2e6f2", "032c2bc9f764a1265f0fd69c0bcb90b4429007f975858fc8b4df65cd0211ffb8fe",
            "038410f240613b33fe73d968f331ca82eb1a2b43ddb2ad6c4a27b2f6d0c1e2a3b4"]
    body = bytes([1, 1, 2, 3, 1]) + b"".join(bytes.fromhex(k) for k in keys) + bytes.fromhex("ab" * 32)
    for t in ["fämily vault", "key 0", "key 1", "key 2"]:
        e = t.encode(); body += bytes([len(e)]) + e
    secret = body + hashlib.sha256(body).digest()[:4]
    rnd = bytes(hashlib.sha256(i.to_bytes(4, "big")).digest()[0] for i in range(len(secret)))
    shares = split(secret, 2, 3, rnd)
    for a in range(3):
        for b in range(a + 1, 3):
            assert interpolate([(a + 1, shares[a]), (b + 1, shares[b])]) == secret
    return hashlib.sha256(b"".join(shares)).hexdigest()

if __name__ == "__main__":
    a = sys.argv[1:]
    if a[:1] == ["vector"]: print(vector())
    elif a[:1] == ["container"] and len(a) == 2:
        for ident, k, x, share in container(bytes.fromhex(a[1])): print(f"vault {ident}: share {x}, any {k} rebuild it\n  {x}:{share.hex()}")
    elif a[:1] == ["combine"] and len(a) >= 2:
        pts = [(int(s.split(":")[0]), bytes.fromhex(s.split(":")[1])) for s in a[1:]]
        r = decode(interpolate(pts))
        line = f"wsh(sortedmulti({r['threshold']},{','.join(r['keys'])}))" + (f" cc={r['chain_code']}" if r["chain_code"] else "")
        print(f"{r['name']} · {r['network']} · {r['threshold']} of {len(r['keys'])} · keys: {', '.join(r['key_names'])}\n{line}")
        print("Paste that line into Any Two Keys › Restore. Without a cc= part it is a standard descriptor any wallet reads (the checksum is optional).")
    else: print(__doc__)
