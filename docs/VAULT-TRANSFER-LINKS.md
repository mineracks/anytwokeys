# Transfers between watched vaults

## Exit transaction evidence (tier 2)

Checked the upstream Spark checkout at `0b3a32a05c9ac06cc411683551dd1f1bde9d0caa`, the revision named in the build brief.

- `protos/spark.proto:1099`: `Transfer` fields 1–14 contain identities, value, status, times, leaves, invoice, network, receivers and senders. No exit transaction id.
- `protos/spark.proto:1220`: `QueryTransfersResponse` returns these transfers; the app's `query_all_transfers` read path has no separate cooperative-exit result.
- `protos/spark.proto:1386`: `CooperativeExitRequest.exit_txid` is field 3 of the **write request**. It is not a field of the returned `Transfer` or `CooperativeExitResponse`.
- The operator stores the exit txid internally (`spark/so/ent/cooperativeexit`); that does not make it accessible to this designated-reader API.

Consequently no tier-2 automatic exit match is implemented. The app must not mistake a leaf/refund transaction for the final exit payment. An exit with no shared public identifier can only be proposed from amount/time evidence and confirmed by the person (tier 3). A future read API exposing a verifiable exit id can extend this index without changing user answers.

This is a limit of the pinned reader protocol, not a claim that Spark's owner SDK or another service cannot obtain exit metadata.

## Implementation

`src-tauri/src/transfers.rs` owns the evidence index. `vault_activity` feeds it settled, signed amount/time facts; balance reads do not touch it. `transfer_links` returns the prepared view; `transfer_answer` accepts a currently valid suggestion, with a second uniqueness check in Rust.

Tier 1 matches the Spark counterparty identity on the same network. A self-transfer has zero net effect. Reciprocal links and both histories are required before distinct records are grouped in the ledger. Existing shared on-chain transaction ids continue to work.

The encrypted `chain.db` has a `transfer_cache` table containing derived snapshots. The live index includes per-vault fingerprints, certain-link caches and per-pair candidate caches. Fingerprints include ids, signed amounts, timestamps, counterparty identity and already-known transfer evidence; prices are excluded. Unchanged activity does not rebuild links or write this cache. Changed histories only rerun candidate searches for their vault pairs. A roster change rechecks identity matches; forgotten metadata is retained to explain old links. Locking clears the in-memory index. Losing this cache loses tier-1 forgotten-vault explanations until evidence is available again; it does not lose user answers.

`Store.transfer_answers` holds Yes/No answers plus the reviewed pair. Keys hash both vault identities, network and row ids, independent of display names. No answers are never proposed again. Yes answers apply only while the same signed amounts still exist; a missing/changed record cannot silently inherit an old confirmation. Forgetting the peer removes the effective link, leaves a note, and restores ordinary acquisition/disposal treatment. Confirmed-answer notes survive cache loss because the reviewed pair is in the store. Existing label synchronisation does not synchronise these local answers.

## Suggestions and limits

Amount/time matching is an amount-and-time ordered bucket search, not a cartesian row scan. Only pairs involving Spark are considered; the existing chain-to-chain path remains authoritative.

| Pair | Maximum difference | Arrival relative to outflow |
| --- | ---: | --- |
| Spark ↔ Spark, no shared identity proof | 0 sats | −1 hour to +1 hour |
| Spark ↔ Muun | 1,000 sats | −1 hour to +24 hours, because Muun's observed leg is on-chain |
| Spark ↔ another on-chain vault | 10,000 sats | −1 hour to +24 hours |

These are proposal limits, not fee estimates or proof that a payment was Lightning. Timestamp-less/pending records, self-transfers, already-linked records and different networks are excluded. Both ends must have exactly one remaining candidate to offer Yes. Ambiguity is listed without a Yes button; rejecting a false candidate may make another pair unique. Split payments are out of scope.

Activity has a Possible transfers box, with Yes, that's mine / No. Check other vaults reads available histories without requesting secrets. A locked Spark reader must be opened by the person's existing refresh/PIN/touch flow. No seed entry is needed. Old cached facts are not a claim that every wallet has been refreshed.

## Accounting and presentation

`keepActivity` preserves transfer metadata for background loads. Both activity rows and details show the link; a different peer record id is used when navigating a confirmed pair. Linked history is followed transitively, so A → B → C retains A's original basis. Home summaries and reports use that full history but scope holdings, basis and realised totals to the selected vault(s). Source timestamps order joined events so a slightly earlier receiving clock does not move them before the source acquisition. Source prices value the fee. Background price updates refresh loaded peer facts too.

FIFO and average keep their existing methods. Under the coin-identification method, actual chain inputs/outputs remain authoritative; Spark supplies no lineage, so off-chain account entries are consumed oldest first, proportionally within an entry that carries several lots. This convention is stated in Home's notes. It is not a claim to identify individual Spark leaves. Unused parts retain their basis. Moving the principal produces no disposal; the difference between outgoing and incoming amounts does. Merely proposing a pair never changes arithmetic.

There is currently no answer-management/undo screen. Don't confirm a suggestion without checking both records. If an answer needs correction, implement a small reviewed edit through `wallet::update`, preserving the pair key, rather than deleting the cache (which deliberately does not remove answers).

## Validation and live acceptance

Automated coverage: Rust matching tests (identity/network/self, fingerprint gate, both-sided uniqueness, time/amount boundaries, later history, forgetting/rename, changed amounts, answer serialisation); `test/ledger-transfers.cjs` (all methods, bridge directions, fee-only disposal, scope, multi-hop, clock skew, no mutation, partial lots); `test/ui-transfers.cjs` (proposal/confirmation/denial, ambiguity, Activity/detail/Home); existing WebKit Home, Spark, lock, seal, sign and signer suites.

Live acceptance remains a separate check by Piers: open the new build, refresh/unlock both Wallet of Satoshi and b Wallet Spending, check the known move in both Activity lists, and compare their Home cost basis. Direct matching Spark identities should label the records automatically. If the payment went through Lightning and has different identifiers, inspect Possible transfers and confirm only the matching pair. Verify only the fee is counted as a disposal, then reopen and check the result persists. No new payment is required to test the historical move.
