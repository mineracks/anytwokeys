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
If the object holds anything that does not start with the magic below, the app leaves it alone.

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

## What a finder of one card learns

That card's own public key (always readable from an OpenPGP card) and a share that is random noise. Not the
other keys, not the addresses, not the balance. A finder of k cards learns the descriptor — what the vault
holds and has done — but cannot spend without the PINs and a touch on each key.

## Recovery without the app

    gpg-connect-agent "scd apdu 00 A4 04 00 06 D2 76 00 01 24 01" "scd apdu 00 CA 7F 21 00" /bye
    tools/seal-recover.py container <hex of the answer, without the trailing 9000>
    tools/seal-recover.py combine 1:<share> 3:<share>
