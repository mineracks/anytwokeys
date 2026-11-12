# Provenance: who can link a coin to you

KYC is one answer to that question, so a coin's **status** is a small scale, not a yes or no: **KYC** (a regulated
party holds your identity against it), **Known** (a person or a service knows it is yours, with no regulated record),
**Private** (nothing in your records links it to you), **Unclassified** (you have not said yet — a work queue, never a
fourth kind). A **method** says how the sats arrived (exchange, ATM, peer to peer, mined, earned, gift, inheritance,
coinjoin, Lightning channel, swap, from another of your vaults); a **counterparty** says who was on the other side.
*What was this?* on a spend classifies how sats left; this classifies how they arrived.

Everything is answered from your own records: the marks you set, the vault's default, and the statements of the
exchanges and custodians you imported. No chain-analysis service is asked, and no coin is ever called clean or dirty.

## Where you say it

- **Receive → Who is paying?** marks the address before the coin exists — the cheapest moment. Every coin that lands
  there takes the mark unless the coin itself is marked.
- **A coin's page** and **an address's page** (Coins, Addresses) each carry the same picker: status chips, how it
  arrived, who. A mark on the coin comes before its address's, which comes before its transaction's.
- **A transaction's sheet → Who can link it** marks the transaction: on a deposit, every coin it brought in; on a
  payment, the coins it spent (paid to an exchange, they are KYC from then on).
- **Vault settings → Who can link this vault's coins to you**: a default for coins nothing else classifies, and a
  policy for when a build would mix statuses — *Warn* (the default), *Refuse* (a private-only vault), *Say nothing*.
- A transaction one of your exchange vaults also records (a transfer link) is KYC with that exchange as counterparty
  without a tap; one a custodial statement records is Known.

## How it spreads

Worst wins: KYC over Known over Private. Unclassified never wins and Private never spreads — it is the absence of a
link. Two ways:

- **Forward.** Change, and a move to another of your vaults, inherit the worst status of the coins spent; a payment
  to an address you marked as an exchange's marks the coins it spent.
- **Across a cluster.** Coins spent together were controlled together, and when every input of a transaction was
  yours, its outputs are linkable to them. A status of Known or better flows through the whole cluster, backwards in
  time too: deposit one change output to an exchange and its siblings become linkable. A coin whose status came this
  way is shown **linked**, and its trail says through what.

The trail is on every coin (hover the chip; the coin's page lists it): *set by you on this coin* · *from its address*
· *change: inherits from the coins spent on …* · *KYC since 9 Nov 2026: paid to …, an address you marked as Kraken's*
· *linked: … is KYC, and the chain shows the two were controlled together*. A status can change without you touching
the coin, so the trail is not optional.

## Where you see it

- **Coins**: a *Who knows* column and filter, a *Who knows* grouping; the selection bar says the mix (*2 KYC and 1
  Private — spending these together links them*).
- **Send, Combine, Split**, before Build: what spending the chosen coins together links, what the change inherits,
  a destination you marked and the sibling coins still in the vault that paying it would link, and the vault's policy.
  *Refuse* stops the build. A split says the status does not change.
- **Home, All vaults, each entity**: *Who can link these coins to you* — a donut and a table of coins, sats, worth
  and share by status, for the unspent coins of the vaults in view.

## In the labels file

BIP 329 export carries `"provenance": {"status", "method", "counterparty", "note"}` on `output`, `addr` and `tx`
lines, next to `"disposal"`; a line with a mark and no label is written too. Import reads it back (an object, or just
the status as a string), and files a coin's record under `output:<txid:vout>`, the key the coin page reads. Other
BIP 329 readers ignore the field.

Rust: `provenance.rs` (the engine, `derive`, with its tests on a hand-built graph); commands `provenance_all`
(the graph from every chain vault's cached transactions, the marks from every vault's records, the statement rule from
the transfer links), `provenance_set`, `vault_provenance`. The design: `roadmap/anytwokeys/PROVENANCE-SPEC.md` in the
infrastructure repo.
