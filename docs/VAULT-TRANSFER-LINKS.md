# Transfers between watched vaults

## Exit transaction evidence (tier 2)

Checked the upstream Spark checkout at `0b3a32a05c9ac06cc411683551dd1f1bde9d0caa`, the revision named in the build brief.

- `protos/spark.proto:1099`: `Transfer` fields 1–14 contain identities, value, status, times, leaves, invoice, network, receivers and senders. No exit transaction id.
- `protos/spark.proto:1220`: `QueryTransfersResponse` returns these transfers; the app's `query_all_transfers` read path has no separate cooperative-exit result.
- `protos/spark.proto:1386`: `CooperativeExitRequest.exit_txid` is field 3 of the **write request**. It is not a field of the returned `Transfer` or `CooperativeExitResponse`.
- The operator stores the exit txid internally (`spark/so/ent/cooperativeexit`); that does not make it accessible to this designated-reader API.

Consequently no tier-2 automatic exit match is implemented. The app must not mistake a leaf/refund transaction for the final exit payment. An exit with no shared public identifier can only be proposed from amount/time evidence and confirmed by the person (tier 3). A future read API exposing a verifiable exit id can extend this index without changing user answers.

This is a limit of the pinned reader protocol, not a claim that Spark's owner SDK or another service cannot obtain exit metadata.
