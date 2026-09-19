# The seal — a vault's recovery record, kept on its own keys

Format version 1. This document, `tools/seal-recover.py` (standard-library Python) and the tests in
`src-tauri/src/seal.rs` describe the same thing; the Rust and Python implementations are checked against
each other by a frozen vector (`tools/seal-vector.sha256`). Nothing here needs this app to survive.

## Why

A multisig vault cannot be rebuilt from k of its n keys alone: the script needs **all n** public keys, and a
card can only tell you its own. The usual answer is a paper descriptor. The seal puts that record on the
cards instead, split so that the same k cards that can spend can also rebuild — and fewer learn nothing.

## Where

OpenPGP applet, data object `7F21` (cardholder certificate), first instance. On a YubiKey 5 it holds 2,048
bytes, is **readable without a PIN** and writable with the admin PIN (PW3). The signing key is never touched.
If the object holds anything that does not start with the magic below, the app leaves it alone. The object itself is
readable without a PIN; what keeps a share private is that it is LOCKED (below).

    container = "A2K" 0x01 , entry*
    entry     = id[4] , k[1] , x[1] , len[2, big-endian] , share[len]

`id` names the vault: the first 4 bytes of `SHA256("a2k-seal|<network>|<k>|<pubkeys, lower-case hex, sorted,
comma-joined>|<chain code hex or empty>")`. A card in several vaults carries one entry per vault.

## The record (the secret that is split)

    0x01                       version
    network                    0 mainnet · 1 signet
    k , n                      threshold, number of keys
    flags                      bit 0: a 32-byte chain code follows the keys (rotating addresses)
    n × pubkey[33]             compressed secp256k1, in the vault's key order; key i belongs to share x = i + 1
    [chain code[32]]
    name                       len[1] , UTF-8 (≤ 40 bytes)
    n × key name               len[1] , UTF-8 (≤ 24 bytes)
    SHA256(all of the above)[0..4]

The descriptor is `wsh(sortedmulti(k, pubkeys…))` (or `wpkh(key)` for a single key), plus ` cc=<chain code>`
for a rotating vault — the same recovery line the app prints.

## The split

Shamir's scheme over GF(2^8) with the AES polynomial `x^8+x^4+x^3+x+1`, applied to each byte of the record
independently. For byte `s` the polynomial is `s + c1·x + … + c(k-1)·x^(k-1)` with uniformly random `c`;
share `x` (1…n) is its value at `x`. Any k shares give the record by Lagrange interpolation at 0; the same
interpolation at another `x` re-makes a lost card's share. The 4-byte checksum tells a true rebuild from a
wrong set of shares. k−1 shares are statistically independent of the record.

The shares of a vault are made **once** and kept in the app's store, so cards sealed on different days agree.
A computer that does not have them (a vault restored from paper) adopts them from any k sealed cards.

## Locked shares (format unchanged, entries flagged)

A share that anyone holding the card could read was a privacy hole: two found keys would show what the vault holds. Since
2026-09-19 every share is written **locked**, and the app offers to lock shares written before that.

- The card keeps a 32-byte secret in OpenPGP private object **`0103`**: `"A2K" 0x02`, then 32 random bytes. The card gives
  that object up only after the **user PIN** (`VERIFY`, `P2 = 82`) and lets it be written under the same PIN. One secret per
  card serves every vault on it. Anything else found in `0103` is left alone.
- A locked entry sets **bit 7 of its `k` byte**. Its body is `nonce[8] , ciphertext , tag[8]`.
- `PRF(label, tail) = HMAC-SHA256(secret, label ‖ id[4] ‖ k ‖ x ‖ nonce ‖ tail)`, with the unflagged `k`.
  Keystream block `c` is `PRF("a2k-seal-v2-stream", c as 4 bytes big-endian)`; `tag = PRF("a2k-seal-v2-tag", ciphertext)[0..8]`.
- Frozen by `tools/seal-lock-vector.sha256`, which the Rust tests and `seal-recover.py lock-vector` both reproduce.

So recovery asks each key for its PIN — the same PIN it signs with, with the same three tries. Nothing is readable from
keys someone merely holds.

    gpg-connect-agent "scd apdu 00 A4 04 00 06 D2 76 00 01 24 01" "scd apdu 00 20 00 82 06 <PIN as hex>" "scd apdu 00 CA 01 03 00" /bye
    tools/seal-recover.py unlock <hex of 0103> <vault id> <k> <x> <entry body hex>      → prints  x:<share>

## What a finder of one card learns

That card's own public key (always readable from an OpenPGP card), the 4-byte vault id, and ciphertext. Not the other
keys, not the addresses, not the balance. A finder of k cards learns no more than that, unless they also have the PINs —
and with k cards and their PINs they could spend anyway. (A share written before locking existed is readable on sight
until it is locked: the app says which keys those are.)

## Recovery without the app

    gpg-connect-agent "scd apdu 00 A4 04 00 06 D2 76 00 01 24 01" "scd apdu 00 CA 7F 21 00" /bye
    tools/seal-recover.py container <hex of the answer, without the trailing 9000>
    tools/seal-recover.py combine 1:<share> 3:<share>
