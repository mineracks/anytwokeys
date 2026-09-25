# Silent payments on a watched wallet

A wallet imported from its extended key can also receive **silent payments** (BIP 352): the sender derives each
output's taproot key from their own inputs and the wallet's two public keys, so nothing derived from the xpub ever
matches those outputs. A plain watch of such a wallet is short by every silent payment it has received.

This is a *watch-only* feature. Any Two Keys' own vaults do not send or receive silent payments (that decision stands:
`roadmap/anytwokeys/SILENT-PAYMENTS.md` in the infrastructure repo). Here the app only **finds** what someone else's
wallet received, so the figures and the cost basis are whole.

## What it needs, and what it keeps

- The wallet's **scan private key** (`m/352'/0'/0'/1'/0`) and **spend public key** (`m/352'/0'/0'/0'/0`), both
  from the seed words. The words are entered once, in the vault's settings, in the masked boxes; the app proves they
  are this wallet's by deriving the BIP 84 account key and matching it to the key the vault watches, then keeps:
  - the scan key, in its own sealed file (`sp-<id>.key`, under the store key, never in the store the window sees);
  - the spend *public* key and the scan *public* key, in the vault (`Watch.sp`), which together are the `sp1…` address;
  - what the scan finds: output key, height, outpoint, value, and whether it carried the change label.
- The scan key can read every payment ever made to the wallet and cannot spend one. It is a privacy secret, not a
  spending secret; it is never exported, printed or included in a recovery line.
- The sealed store is required. Without it the app refuses to keep the key.

## How it finds them

Per block, from the wallet's birthday (a date the person gives; earlier is safe, only slower):

1. `GET /compute-index/{height}` from the BlindBit oracle (`sp.mineracks.com`, ours; configurable in App settings)
   gives, for each transaction that pays to taproot, the pre-multiplied `input_hash·A` point and 8-byte prefixes of
   its output keys.
2. `ecdh = b_scan · tweak`, `t_k = hash_BIP0352/SharedSecret(ecdh ‖ k)`, `P_k = B_spend + t_k·G`, for k = 0, 1, …
   while something matches; each candidate is also checked with the change label (`m = 0`). Comparing
   `x(P_k + L)` covers the negated case the BIP asks for, since outputs are x-only.
3. A prefix hit fetches `GET /utxos/{height}` and is confirmed on the whole key, with its outpoint and value.

Six blocks are fetched at once; the arithmetic is negligible beside the round trip. Blocks are read in chunks so the
saved "read to" height only ever covers blocks fully read: a scan that stops (oracle down, app locked) resumes from
there on the next refresh. Every client fetches every block, so the oracle learns nothing about any wallet.

Found outputs become ordinary scripts for the existing Electrum path: balance, spent-or-not, history, cost basis. They
are indexed under scheme 7 (`SP_SCHEME`) and read as "silent payment #n" on screen.

## Proof

- `cargo test sp::` runs every receiving vector in BIP 352's `send_and_receive_test_vectors.json` (fetched into
  `test/fixtures/`, 27 usable cases; the one that exceeds K_max is skipped as the BIP intends): addresses, output
  keys, private tweaks and labels all match.
- The oracle's live shape was checked at height 900000 before the client was written.
- Positive control on a real wallet: pending — a wallet that has received a silent payment, watched here, must show it.

## Not done

- Restoring a watched wallet from seed words in one step (deriving the BIP 84 xpub and the silent payment keys
  together). Today: import the xpub, then add silent payments in Vault settings.
- Labels other than change (`m = 0`). A wallet that hands out labelled addresses would need its labels listed.
- Reading blocks without the oracle (full block download). `sp::input_key`/`input_tweak` exist for it; nothing calls
  them outside the tests.
