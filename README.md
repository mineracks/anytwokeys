# Any Two Keys

A Bitcoin vault signed by your YubiKeys. **Experimental research software** — not audited, not a product; test on signet, never savings. The write-up, the threat model and what is and is not proven are at [anytwokeys.com](https://anytwokeys.com).

Each YubiKey 5 (firmware 5.2.3+) generates a secp256k1 key in its OpenPGP signature slot that never leaves the card, with the touch policy locked to FIXED. The vault is a native-segwit k-of-n over those keys (`wsh(sortedmulti(k, pub…))`), or `wpkh(pub)` for one key. Any k cards, PIN + touch each, sign a spend. Recovery is any k cards plus the descriptor line — or any k *sealed* cards alone, see [docs/SEAL.md](docs/SEAL.md) and `tools/seal-recover.py`, a standard-library Python tool that rebuilds a vault from the shares without this app.

## Get it

- **macOS (Apple silicon):** the notarised DMG and its SHA-256 are on the [releases page](https://github.com/mineracks/anytwokeys/releases). Check the hash before you open it.
- **iPhone, iPad, Android:** builds reach testers by invitation through TestFlight and Google Play internal testing — write to info@mineracks.com. The YubiKey plugs into the USB-C port; NFC is not used.
- **Windows, Linux:** build from source (below); not yet packaged or tested by us.

## What is where

- `src-tauri/src/cards.rs` — the OpenPGP APDUs over PC/SC (select, verify, generate, touch policy, sign a digest)
- `src-tauri/src/wallet.rs` — descriptors, the vault store, building/signing/finalising spends (`bitcoin`, `miniscript`)
- `src-tauri/src/chain.rs` — Electrum (coins, history, broadcast) and an esplora-style explorer (fees)
- `src-tauri/src/seal.rs`, `docs/SEAL.md` — the seal: a Shamir share of the vault kept on each key, behind its PIN
- `src-tauri/src/labels.rs`, `src-tauri/src/sync.rs` — labels and profile backed up encrypted on a VSS host (LDK Versioned Storage Service, default `vss.mineracks.com`). The key is HKDF over the vault's seal shares, so any k keys with their PINs bring the labels back with the vault and the host holds ciphertext it cannot open or tie to an address. Profile: [vss.mineracks.com/labels](https://vss.mineracks.com/labels)
- `src-tauri/src/nkfw.rs` — Nitrokey 3 firmware updates, in the app
- `src/` — the UI (plain HTML and JS, no framework); `src-tauri/gen/` — the Xcode and Android projects, kept because they carry hand-made changes
- `docs/` — formats and design notes; `test/` — browser-driven UI checks (Playwright) and the PDF check; `tools/` — the seal recovery tool and test-vector fixtures

## Build

Tauri 2: Rust stable, Node 20+, and for the card driver a PC/SC stack (macOS has one; Linux needs `pcscd` and `libpcsclite-dev`).

```
npm install
npm run dev      # develop
npm run build    # .app + .dmg under src-tauri/target/release/bundle
cd src-tauri && cargo test
```

Phone builds use `tauri ios build` and `tauri android build`; the release-build traps we hit (and vendored fixes for `swift-rs` and `tao` under `src-tauri/vendor/`) are documented in the comments of `src-tauri/Cargo.toml` and `.cargo/config.toml`.

## Defaults you may want to change

Settings → network. Mainnet points at `electrum.mineracks.com` and `mempool.mineracks.com`; signet at Blockstream's public signet Electrum and `signet.mineracks.com`. Any Electrum server and any esplora-style explorer work; so does a node of your own.

## Status

Research prototype: one tester, a handful of transactions, no third-party review. Read [what is and isn't proven](https://anytwokeys.com/#status) before trusting it with anything. Bug reports and review are welcome as issues here.

Not affiliated with Yubico or Nitrokey. YubiKey is a trademark of Yubico AB. [MIT](LICENSE).
