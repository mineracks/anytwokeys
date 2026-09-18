# Any Two Keys

A Bitcoin vault signed by your YubiKeys. **Experimental research software** — not audited, not a product; test on signet, never savings.

Each YubiKey 5 (firmware 5.2.3+) generates a secp256k1 key in its OpenPGP signature slot that never leaves the card, with the touch policy locked to FIXED. The vault is a native-segwit k-of-n over those keys (`wsh(sortedmulti(k, pub…))`), or `wpkh(pub)` for one key. Any k cards, PIN + touch each, sign a spend. Recovery is any k cards plus the descriptor line.

- Desktop app: Tauri 2 (Rust core, plain HTML UI). macOS first; iOS, Windows, Linux to follow.
- `src-tauri/src/cards.rs` — the OpenPGP APDUs over PC/SC (select, verify, generate, touch policy, sign a digest)
- `src-tauri/src/wallet.rs` — descriptors, the vault store, building/signing/finalising spends (`bitcoin`, `miniscript`)
- `src-tauri/src/chain.rs` — Electrum (coins, history, broadcast) and an esplora-style explorer (fees)
- `src/` — the UI

```
npm install
npm run dev      # develop
npm run build    # .app + .dmg under src-tauri/target/release/bundle
```

Not affiliated with Yubico. YubiKey is a trademark of Yubico AB. MIT.
