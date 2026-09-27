# What the paste box reads

New vault › Paste the line takes more than a descriptor. Everything is read on this computer and turned into the
wallet descriptor it means before it is checked like any other:

- **A wallet descriptor** with `<0;1>/*` on each key, in any of these scripts: `wsh(sortedmulti…)`, `wsh(multi…)`,
  `sh(wsh(sortedmulti…))` and `sh(wsh(multi…))` (nested segwit), `sh(sortedmulti…)` and `sh(multi…)` (legacy,
  BIP45 style), `wpkh`, `sh(wpkh)`, `pkh`, and `tr` by its key path (a single key, no script tree). Addresses,
  balances, history, coins and receive addresses follow the script; a PSBT for a nested script carries its redeem
  script, and one for a legacy script carries every input's whole previous transaction, as those signers require.
- **A Coldcard multisig file** — also what Sparrow, Keeper and Nunchuk export under that name: `Policy: 2 of 3`,
  `Format: P2WSH` (or `P2SH-P2WSH`, `P2SH`), a `Derivation:` for the keys after it, one `fingerprint: xpub` per key.
- **A Sparrow or Specter JSON** with a `"descriptor"` in it.
- **Two descriptors on two lines**, receive (`…/0/*`) and change (`…/1/*`) of one wallet, as Bitcoin Core lists them.
- **One key with its origin**, `[fingerprint/path]xpub…`: the purpose in the path says the script — 84 wpkh, 49
  sh(wpkh), 44 pkh, 86 tr. A 48 or 45 path is one key of a multisig and is refused with a pointer to the whole file.
- **SLIP-132 keys** (zpub, Zpub, ypub, Ypub, and the testnet forms) anywhere in the above, written back as the plain
  xpub or tpub they are; a checksum that no longer fits is dropped rather than kept wrong.

BSMS files have their own button. Bare `xpub…` with no origin is offered as a watched wallet.

Fee estimates size an input by its script (`wallet::in_vbytes_for`). Rust: `wallet::normalise_import`, `ext_parse`
(now with `script`), `script_kind`, `legacy`, `attach_prev_txs`; `lib::legacy_prev_txs` on Send.
