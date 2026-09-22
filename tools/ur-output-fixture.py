#!/usr/bin/env python3
"""Cross-check fixture for UR descriptors: a throwaway 2-of-3 encoded as ur:crypto-output by `urtypes` (the encoder
Krux uses), Sparrow-style — each key with its origin, no child path. Plus the official vectors of BCR-2020-010 and
BCR-2023-010, fetched from BlockchainCommons/Research.
   usage: <python with embit and urtypes> tools/ur-output-fixture.py <vectors.json> > src-tauri/tests/fixtures/ur-output.json"""
import json, hashlib, sys
from embit import bip32
from urtypes.crypto import HDKey, Keypath, PathComponent, MultiKey, Output
from urtypes.crypto.output import SCRIPT_EXPRESSION_TAG_MAP as T

def key(word):
    root = bip32.HDKey.from_seed(hashlib.sha256(word.encode()).digest())
    acct = root.derive("m/48h/0h/0h/2h")
    xpub = acct.to_public()
    origin = Keypath([PathComponent(48, True), PathComponent(0, True), PathComponent(0, True), PathComponent(2, True)], root.my_fingerprint, 4)
    return HDKey({"key": xpub.key.sec(), "chain_code": xpub.chain_code, "origin": origin, "parent_fingerprint": xpub.fingerprint}), root.my_fingerprint.hex()

keys = [key(w) for w in ("anytwokeys test one", "anytwokeys test two", "anytwokeys test three")]
out = Output([T[401], T[407]], MultiKey(2, [], [k for k, _ in keys]))
official = json.load(open(sys.argv[1]))
print(json.dumps({
    "sparrow_style": {"cbor": out.to_cbor().hex(), "urtypes_descriptor": out.descriptor(), "fingerprints": [f for _, f in keys]},
    "official": official,
}, indent=1))
