# A node's ledger: what is kept, what happens when the node is gone, and how to feed it from a backup

A Core Lightning or LND node is read as a vault (`src-tauri/src/node.rs`). What the vault keeps is the node's **ledger**:
every event that moved money, oldest first, in one shape for both kinds of node — on-chain deposits and withdrawals,
Lightning received and paid, routing income folded to one row per day, channel opens and closes as moves within the
node. It is kept **inside the encrypted profile store**, not in the chain cache (the SQLite file holds only confirmed
on-chain transactions, which the chain can always give back). LND is read incrementally from saved marks; Core Lightning's
bookkeeper gives the whole income view each time.

## The ledger never shrinks

What a node once said about money that moved is evidence. So:

- A **whole re-read** (Core Lightning) replaces each row the node reports now and **keeps** every row it no longer
  reports — a database rolled back, a history wiped. `node::keep_and_replace`.
- An **incremental read** (LND) adds what is new. `node::merge_events`.
- A node that **cannot be read** — credential revoked, certificate changed, box rebuilt or gone — does not blank the
  screen. The vault draws its ledger as it was last read and says so: *"did not answer · showing its ledger as last
  read, 18 October 2026 · <the reason>"*. Nothing is written on a failed read. Only a node that never gave a ledger
  shows the error itself.
- A node that answers under a **different identity** is refused ("a different identity answered"): a rebuilt node is a
  new vault, and the old vault stays as the record of the old node.

## Retiring a node

Vault settings → *The node* → **Retire this node — keep its ledger**, with a reason that is kept with the record. A retired
node is not asked on refresh; its ledger is read-only and every link it made to the other vaults stays. The sealed
credential is not touched, so if the same node comes back under the same identity, **Read it again** carries on from
where it left off. Forgetting the vault is the only thing that removes the ledger.

## Adding a ledger from a file

Vault settings → *The node* → **Add its ledger from a file…**. For a node that is gone, a **backup restored off the
network** can still answer its own CLI; save those answers as one JSON file and import it. Rows already here are replaced
by the file's reading of them, rows the file does not hold are kept, and the file must be from this node (one that names
another identity is refused; one without an identity is taken on trust). The parsers are the same ones the live read
uses (`node::cln_income_events`, `node::lnd_events_from`), so a file and a live read give the same rows.

Core Lightning — the output of `bkpr-listincome` as it is, optionally with `getinfo` beside it:

```sh
lightning-cli bkpr-listincome consolidate_fees=true > income.json      # {"income_events": [...]}
# optional identity check: add "getinfo": {"id": "<node id>"} to that object
```

LND — one object keyed by the lncli command names (any subset; `r_hash` as lncli prints it, hex, or as REST sends it,
base64):

```sh
{ "getinfo": $(lncli getinfo), "listchaintxns": $(lncli listchaintxns),
  "listinvoices": $(lncli listinvoices --max_invoices 100000), "listpayments": $(lncli listpayments --max_payments 100000),
  "fwdinghistory": $(lncli fwdinghistory --start_time 1 --max_events 50000),
  "listchannels": $(lncli listchannels), "closedchannels": $(lncli closedchannels), "pendingchannels": $(lncli pendingchannels) }
```

**Restoring a node from a backup to read it: keep it off the network.** An old channel state that reaches a peer is
penalised. Give the restored machine no route except to its bitcoind (the daemons need the chain to start), read it
through the hypervisor's guest agent, and destroy it afterwards. The mineracks infrastructure repo has the scripts for
the October 2026 restores.

Why this exists: in the October 2026 intrusion the attacker drained three nodes and wiped their histories; the credentials
the app held stopped working, and the ledgers it had read were the only copy left on this side. The app now keeps them
on purpose, says when they are stale, and takes a backup's word to fill the gap.
