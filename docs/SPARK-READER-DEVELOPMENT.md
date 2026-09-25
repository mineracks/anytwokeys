# Spark reader implementation references

Protocol/SDK/operator code reviewed at `buildonspark/spark` commit `0b3a32a05c9ac06cc411683551dd1f1bde9d0caa`:

- `protos/spark_authn.proto`: challenge and verification messages.
- `sdks/js/packages/spark-sdk/src/services/connection/connection.ts`: SHA256 of re-encoded Challenge, identity ECDSA DER signature, returned expiration and Bearer metadata.
- `spark/so/grpc/spark_authn_server.go`: accepts secp256k1 ECDSA and Schnorr, checks identity, challenge version, freshness and nonce.
- `spark/so/handler/wallet_setting_handler.go`: master identity read access; settings updates target the authenticated identity itself.
- `spark/so/authz/authz.go`, `handler/base_transfer_handler.go`, `handler/coop_exit_handler.go`, `rpcpolicy/policy.go`: owner checks on fund-moving paths. Owner enforcement depends on operator configuration; no claim is made about all deployed or future operators.

Blitz derivation is based on `BlitzWallet/spark-recover` commit `3126b04ff7522558438ecaa5080d978cef04a8f9`, `src/functions/recovery/derivedWallets.js`. It takes the first 16 bytes of a derived private key as entropy for a new mnemonic, then opens that mnemonic's Spark account 1. Golden vectors in its test file are checked here. No evidence establishes that Wallet of Satoshi uses those namespaces.

Tests:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --offline --lib -- --test-threads=1
node test/ui-spark.cjs /tmp/anytwokeys-spark-ui
cargo test --manifest-path src-tauri/Cargo.toml --offline --lib live_ephemeral_authentication_smoke -- --ignored --nocapture --test-threads=1
```

The UI test needs Playwright (via module resolution or `NODE_PATH`) and its WebKit browser. An existing browser launcher can be selected with `A2K_WEBKIT`. The ignored live test generates an ephemeral software key and performs login plus a read, with no wallet settings update or funding. Hardware tests remain manual.

Sessions are operator-bound and stored by network + owner identity + reader public key + card serial. A runtime generation counter invalidates even sessions held by in-flight readers when the app locks. Discovery has a separate cancellation counter. Credentials are not serialized. PIN failures are not automatically retried.

Enrolment is intentionally not a distributed transaction: a remote setting may succeed before a local save or verification fails. The error makes this explicit; rediscovery reads the current setting for a recoverable retry. Existing designated readers are never silently overwritten.
