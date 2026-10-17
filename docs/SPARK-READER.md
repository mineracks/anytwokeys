# Spark reader — hardware test handoff

The implementation is in this repository. This is ready for hardware acceptance testing, not a claim that Wallet of Satoshi enrolment has already been proved on your wallet.

## Start the new build

Quit any running copy of Any Two Keys, then open **Any Two Keys.app** beside this guide. The build keeps the existing app identity and uses the same local vault store.

In the app, choose **New vault → Restore → From a Spark wallet** (or the existing Restore entry), then expand the seed-word section. Set the app to mainnet; authenticated Spark support in this build is mainnet only.

## Enrol a wallet you control

1. Give the vault a name and enter its seed words in the masked boxes.
2. Choose **Gate this wallet with a YubiKey**. Discovery signs in to Spark as each candidate owner. It does not change wallet settings. The default range is Spark accounts 0–5; progress identifies the current account. Empty accounts can take longer because their transfer history is also checked.
3. Select the discovered identity whose balance/history you recognise. Compare it with the wallet you control. If you know its owner public key or Spark address, enter it under **Account discovery options** to constrain the search. Do not enrol an unrelated identity simply because it appears.
4. Select your YubiKey and refresh the list if needed. An existing secp256k1 key must have touch locked to **FIXED**. To create a dedicated key, select an empty card slot, open **Create a reader key in an empty slot**, enter its admin PIN and generate. This flow refuses an occupied signature slot, including a key of another algorithm.
5. Confirm the wallet selection. Privacy stays as it is unless you tick **Also make the wallet private**. If a different designated reader already exists, its replacement requires a separate acknowledgement; this may remove an existing app/service's reader access.
6. Enter the reader PIN and choose **Register reader — PIN + touch**. Touch when the key flashes. The app authenticates the card, updates the setting, reads the setting back, and checks that the card session can see the wallet's known funds/history before saving the local vault.
7. Compare **both the balance and Activity** with your wallet. Open a transfer: it should show Spark identity keys and status, rather than an on-chain transaction lookup. Only completed transfers enter cost-basis calculations.

If nothing matches, the result does not prove the wallet is empty. The owning identity or derivation remains unresolved. The advanced options support bounded ranges of direct Spark accounts and the separately labelled Blitz child-wallet scheme. Blitz is not assumed to be Wallet of Satoshi's scheme. Cancel clears pending owner sessions; entering words again starts another scan.

## Check session behaviour

- Refresh after successful enrolment: a live cached session should avoid another PIN/touch.
- Unplug the card while the session is live: refresh may still work, by design.
- Lock the app or fully quit/reopen it, then refresh: the reader needs its registered card and PIN/touch again. Its seed is not needed.
- A different card must not unlock the reader. A wrong PIN must show an error without an automatic PIN retry. Do not deliberately exhaust the card's retry counter.
- Cancelling an unlock must leave the reader marked locked, without replacing a previously known balance with an anonymous zero.
- A failed operator/session is discarded. Retrying unlock selects the next configured operator and requires a fresh session; authenticated reads never silently fall back to anonymous requests.

A wallet that becomes empty and one whose reader access has been revoked can both return no visible records. Spark does not make that distinction available through these read responses; the app explains the ambiguity.

## What is saved

The vault stores the wallet identity/address, reader public key, card serial and authenticated-reader flag. Seed words, derived private keys, PINs and session tokens are not written to the vault store. Rust-owned secret buffers use cleanup guards; JavaScript and third-party library copies mean memory erasure is best effort, not a promise that a computer which saw a seed could not retain it.

Discovery's temporary owner credentials remain in memory for at most ten minutes (or their earlier server expiry). They are dropped on cancellation, enrolment completion or app lock. Reader tokens are held only in memory until expiry/lock/exit. A request already sent before locking may finish, but that session cannot issue another request afterwards.

Only freshly generated on-card keys have known non-exportable provenance in this flow. An existing key could have been imported or used by another vault. The designated key's permissions on this Spark wallet are enforced by Spark's operators; it does not contain this wallet's spending keys.

## Validation and remaining proof

- **104 Rust tests passed** (23 opt-in tests skipped), including protocol fixtures, session isolation/expiry, malformed responses, derivation fixtures and history pagination.
- **WebKit interface tests passed** for discovery/cancel, cleared inputs, confirmation gates, privacy preservation, cached sessions, wrong PIN, reader selection and Spark transfer details.
- **Live authentication passed**, plus an authenticated balance request against Breez's Spark operator using a new disposable, unfunded software key. This proves protocol interoperability only.
- The real YubiKey signature, funded-wallet positive control, and Wallet of Satoshi's acceptance of the setting still require your test.

If an enrolment request times out after reaching Spark, the app explains that the setting may already have changed. Discover again to inspect it before retrying. Please report the visible error and where it occurred; do not send seed words, PINs or session credentials.
