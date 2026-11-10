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

## The node's balance is the authority; the gap is booked

A ledger built from what the node's bookkeeper saw cannot see money that left **without** it: channels closed by someone
else holding the keys, coins swept by another instance of the wallet, a database rolled back so the node forgets a spend.
So on every read, and on every import that carries a balance (`listfunds`, `walletbalance`, `listchannels`), the node's
reported balance — on-chain + channels + pending — is taken as the truth and the ledger is **reconciled** to it
(`node::reconcile`): if the rows do not sum to what the node holds, one row of kind **unaccounted** (layer `node`) is
booked for the difference, dated at the read (or at the newest event of an import), with a memo saying why. A later read
that explains the gap removes the row; a wider gap books a wider row. Nothing the node said is altered to fit.

Retiring a node with **"It holds nothing now"** does the same by the person's word: the balance becomes zero and the gap
is booked today with the reason given. That is the close-out for a node that was emptied by an attacker.

### Exact to the millisatoshi

Lightning amounts are millisatoshi. A ledger of rows each rounded to whole sats is out by up to a satoshi a row, and
on a node with hundreds of routing fees that rounding would show up as "unaccounted" money. So every row carries the
exact figure where the source gave it (`Event.msat`: invoices, payments, forwards, bookkeeper events, a channel's
`final_to_us_msat`), and sums — a day's routing, a merged fee, the reconciliation itself — are taken in msat and rounded
once. A row under one satoshi (a routing fee of 313 msat; the fraction a closing transaction cannot pay and rounds away)
is shown as 0.313 sats, not dropped and not shown as 0. A gap with a sub-satoshi part says the exact msat in its memo.
A ledger that says 0 msat unaccounted is exactly reconciled; any other figure is money whose path is not yet understood.

### The chain over the node's word

A restored Core Lightning copy lists outputs it no longer has and channels whose closing it never booked. On every read
and every import of a file that carries `listfunds` and `listclosedchannels`, the chain is asked about each output the
node calls unspent and each closed channel with money on our side (`node::cln_chain_truth`): spent? by which
transaction, in which block, to which addresses? A spent output becomes a **withdrawal** row named by the spending
transaction; a close the bookkeeper never saw becomes a **close** row for our side's final balance, with the outputs
under **To**. A close whose money came home — some output of it spent by a transaction this wallet made, even one the
bookkeeper booked only by its fee (a sweep is an internal move, not income) — is not a loss and gets no row. A close
counts as already booked only by a deposit or withdrawal; a fee row naming the same transaction is the node's view of
it, not the chain's. A transaction the bookkeeper named that is on no server's chain is a **phantom**: kept, counted as
zero, said so.

An LND node read live goes through the same three chain steps as a file imported from a backup of one: a close whose
output came home through a transaction the ledger holds, a channel the node abandoned that still sits on the chain,
and the wallet's own change listed as paid away (`lnd_returned`, `lnd_abandoned`, `lnd_own_change`, run in `node_read`
from the node's own `closedchannels` and `listchaintxns` answers). A node that is still answering but was emptied by
someone holding its keys is read with the same detail as a restored copy of one.

Bookkeeper fee rows are named by their transaction (`onchain_fee|txid|account`), never by the time of the read: a
restored copy and the live node must give the same row the same id, and equal fee or close rows of one transaction
collapse to one after every merge (`node::dedupe_fee_rows`, earliest kept, destinations taken from either).

## Where the money went

Every row that left on the chain or closed a channel carries the transaction's outputs — address and sats — read from
the transaction itself (`node_destinations`, through the chain cache; LND's `output_details` when it gives them). For a
channel the node's wallet never saw closed (a cooperative close signed by someone else), a **close** row is made from
`closedchannels` with the closing transaction, its outputs and the block's time, and a memo saying none of the outputs
paid this node's wallet. The transaction sheet shows them under **To**: that is the evidence for the books and for a
report to police or an exchange.

What this cannot give: the per-channel balance the moment before a close that happened after the backup was taken. The
backup knows the channel's state at backup time; the closing transaction knows where the money went; the difference
between them is what left. If a reading of the channel just before the close is wanted, the backup has to be restored
and `listchannels` read **before** it syncs.

## Stolen and lost coins in the books

A row that left a vault has a **What was this?** picker in its transaction sheet (also asked at Send). Two kinds are for
incidents: **Stolen** and **Lost**. Both are a disposal for nothing — proceeds nil, the loss is the whole cost of the
coins under the vault's cost method — and are shown as a capital loss. In Australia that is CGT event C1 (loss or
destruction of a CGT asset), claimed in the year the loss is discovered, with the evidence kept: the row, the
transaction, the addresses the coins went to, the report made. The unaccounted row of a drained node takes the same
picker, so a whole incident can be booked as one loss once the node is retired as empty.

A whole incident can also be booked from one file: a BIP 329 labels file (Vault settings → *Import labels*) may carry
`"disposal": "stolen"` (or sold, spent, gift, self, lost) on a `tx` line beside, or instead of, its label, and the export
writes the field back out. Other BIP 329 readers ignore it.

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
  "listchannels": $(lncli listchannels), "closedchannels": $(lncli closedchannels), "pendingchannels": $(lncli pendingchannels),
  "walletbalance": $(lncli walletbalance), "channelbalance": $(lncli channelbalance) }
```

Include the balance keys (`walletbalance` / `channelbalance` for LND, `listfunds` for Core Lightning) when the file is a
read of a node **after** an incident: they are what the ledger is reconciled against.

**Restoring a node from a backup to read it: keep it off the network.** An old channel state that reaches a peer is
penalised. Give the restored machine no route except to its bitcoind (the daemons need the chain to start), read it
through the hypervisor's guest agent, and destroy it afterwards. The mineracks infrastructure repo has the scripts for
the October 2026 restores.

Why this exists: in the October 2026 intrusion the attacker drained three nodes and wiped their histories; the credentials
the app held stopped working, and the ledgers it had read were the only copy left on this side. The app now keeps them
on purpose, says when they are stale, and takes a backup's word to fill the gap.
