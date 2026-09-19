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
import hashlib, hmac, sys

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
        out.append((ident, k & 0x7F, x, b[i + 8:i + 8 + ln], bool(k & 0x80))); i += 8 + ln
    return out

def _prf(key, label, ident, k, x, nonce, tail):
    return hmac.new(key, label + ident + bytes([k, x]) + nonce + tail, hashlib.sha256).digest()

def lock_share(key, ident, k, x, share, nonce):
    ct = bytes(d ^ s for c in range(0, len(share), 32) for d, s in zip(share[c:c + 32], _prf(key, b"a2k-seal-v2-stream", ident, k, x, nonce, (c // 32).to_bytes(4, "big"))))
    return nonce + ct + _prf(key, b"a2k-seal-v2-tag", ident, k, x, nonce, ct)[:8]

def unlock_share(key, ident, k, x, body):
    """A locked entry (bit 7 of its k byte) is nonce[8] + ciphertext + tag[8]; `key` is the 32 bytes after "A2K\x02" in the
    card's private object 0103, which the card gives up only after its PIN (VERIFY with P2 = 82)."""
    nonce, ct, tag = body[:8], body[8:-8], body[-8:]
    if not hmac.compare_digest(_prf(key, b"a2k-seal-v2-tag", ident, k, x, nonce, ct)[:8], tag):
        raise SystemExit("that share was not locked by that key's secret")
    return bytes(d ^ s for c in range(0, len(ct), 32) for d, s in zip(ct[c:c + 32], _prf(key, b"a2k-seal-v2-stream", ident, k, x, nonce, (c // 32).to_bytes(4, "big"))))

def lock_vector():
    share = bytes((i * 7 % 251) for i in range(171))
    body = lock_share(bytes([7]) * 32, bytes.fromhex("deadbeef"), 2, 3, share, bytes(range(1, 9)))
    assert unlock_share(bytes([7]) * 32, bytes.fromhex("deadbeef"), 2, 3, body) == share
    return hashlib.sha256(body).hexdigest()

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
        for ident, k, x, share, locked in container(bytes.fromhex(a[1])):
            print(f"vault {ident}: share {x}, any {k} rebuild it" + (" · LOCKED: open it first with  unlock <0103 secret hex> " + f"{ident} {k} {x} {share.hex()}" if locked else f"\n  {x}:{share.hex()}"))
    elif a[:1] == ["unlock"] and len(a) == 6:
        print(f"{a[4]}:" + unlock_share(bytes.fromhex(a[1])[-32:], bytes.fromhex(a[2]), int(a[3]), int(a[4]), bytes.fromhex(a[5])).hex())
    elif a[:1] == ["lock-vector"]: print(lock_vector())
    elif a[:1] == ["combine"] and len(a) >= 2:
        pts = [(int(s.split(":")[0]), bytes.fromhex(s.split(":")[1])) for s in a[1:]]
        r = decode(interpolate(pts))
        line = f"wsh(sortedmulti({r['threshold']},{','.join(r['keys'])}))" + (f" cc={r['chain_code']}" if r["chain_code"] else "")
        print(f"{r['name']} · {r['network']} · {r['threshold']} of {len(r['keys'])} · keys: {', '.join(r['key_names'])}\n{line}")
        print("Paste that line into Any Two Keys › Restore. Without a cc= part it is a standard descriptor any wallet reads (the checksum is optional).")
    else: print(__doc__)
