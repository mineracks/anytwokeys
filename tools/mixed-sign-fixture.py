#!/usr/bin/env python3
"""Sign the app's mixed-vault spend as a SeedSigner and a card would, with embit — the library SeedSigner signs with.
   usage: cargo test emit_mixed_spend -- --ignored --nocapture | sed -n 's/^PSBT //p' | <embit python> tools/mixed-sign-fixture.py
   Writes src-tauri/tests/fixtures/mixed-signed.json. Every key here is a throwaway made from a public phrase."""
import json, hashlib, sys, pathlib
from embit import bip32, ec
from embit.psbt import PSBT
from embit.script import Witness, Script

b64 = sys.stdin.read().strip()
unsigned = PSBT.from_string(b64)
root = bip32.HDKey.from_seed(hashlib.sha256(b"anytwokeys test one").digest())
assert root.my_fingerprint.hex() == "c9ad277c", root.my_fingerprint.hex()

# what a SeedSigner checks before it signs: an input it can find its key in, and an output it can prove is change
mine_in = [any(d.fingerprint == root.my_fingerprint for d in i.bip32_derivations.values()) for i in unsigned.inputs]
change = [o for o in unsigned.outputs if any(d.fingerprint == root.my_fingerprint and d.derivation[-2] == 1 for d in o.bip32_derivations.values())]
assert all(mine_in), "the device would not find its key"
assert len(change) == 1, "the device would not recognise change"

# what the SeedSigner ITSELF does, not a charitable version of it: psbt_parser.py collects a path string for EVERY
# derivation on a change output, and psbt_views.py:123 reads the FIRST one two levels from its end. A key with no
# derivation gives "m", and "m".split("/")[-2] raised IndexError on Piers's bench (2026-09-23) before the device
# could show the spend. The checks above never caught it: they short-circuit on the fingerprint, and the device does not.
for o in unsigned.outputs:
    paths = [bip32.path_to_str(d.derivation) for d in o.bip32_derivations.values()]
    if not paths: continue
    assert paths[0].split("/")[-2] in ("0", "1"), f"the device reads {paths[0]!r} two levels from its end and crashes"

def minimal(signed):
    # SeedSigner hands back a trimmed PSBT: the transaction and its signatures, nothing else
    t = PSBT(signed.tx)
    for i, inp in enumerate(signed.inputs): t.inputs[i].partial_sigs = dict(inp.partial_sigs)
    return t

dev = PSBT.from_string(b64); n = dev.sign_with(root); assert n == 1, n
card_key = ec.PrivateKey(hashlib.sha256(b"card one").digest())
card = PSBT.from_string(b64); n = card.sign_with(card_key); assert n == 1, n

# finish it here too, independently of the app: witness = <empty> <sigs in the script's key order> <script>
ws = unsigned.inputs[0].witness_script
data = ws.data; keys = []; i = 1
while data[i] == 33: keys.append(data[i + 1:i + 34]); i += 34
sigs = {}
for p in (dev, card):
    for pub, sig in p.inputs[0].partial_sigs.items(): sigs[pub.sec()] = sig
tx = unsigned.tx
tx.vin[0].witness = Witness([b""] + [sigs[k] for k in keys if k in sigs] + [ws.data])
out = {"unsigned": b64, "device_minimal": minimal(dev).to_string(), "card_one": minimal(card).to_string(), "final_tx": tx.serialize().hex()}
f = pathlib.Path(__file__).resolve().parent.parent / "src-tauri/tests/fixtures/mixed-signed.json"
f.write_text(json.dumps(out, indent=1) + "\n"); print("wrote", f, "·", len(sigs), "signatures")
