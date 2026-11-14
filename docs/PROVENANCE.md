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
- Under a mark on a coin or a transaction, one tap — *Use this for every coin at bc1q… — 4 there now, and every one
  still to come* — copies it onto the address, so an address that is paid again and again is answered once. On a
  payment the same line offers the address paid to.
- **Vault settings → Who can link this vault's coins to you**: a default for coins nothing else classifies, and a
  policy for when a build would mix statuses — *Warn* (the default), *Refuse* (a private-only vault), *Say nothing*.
- A transaction one of your exchange vaults also records (a transfer link) is KYC with that exchange as counterparty
  without a tap; one a custodial statement records is Known.
- **The counterparty register** is every name the app already knows — the exchanges and custodians it tracks, and
  every name you have given with a mark. Each *Who was on the other side* box offers them as you type; a name the app
  knows as an exchange, given with no method, sets the method to *Bought on an exchange*.
- **Sort them ›** on the *Unclassified* row of any donut (Home, All vaults, an entity) opens the bulk flow for the
  unclassified coins of the vaults in view: *From what you called them* (a coin's or its address's label names someone
  the register knows, or says how it arrived — *salary*, *gift*, *mined*, *coinjoin*, *Bisq* … — one tap marks the
  group); *Several at one address* (open the address and say it once); *The rest, in one go* (tick, pick a status or a
  method and a name, and *Mark the ticked coins* — optionally onto each coin's address too, so coins still to come
  there take it). One write, across vaults.
- **Freeze** on a coin's page (BIP 329 `spendable: false`): a frozen coin stays in the holding and in every view, and
  is left out of Send, Combine and Split until thawed; naming it by hand is refused with the reason. A small,
  unexplained, unasked-for receipt is offered *Freeze it as dust* — spent with your other coins it would link them to
  whoever sent it. Coins has a *Frozen* filter.

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
  *Refuse* stops the build. A split says the status does not change. When a spend would mix, Send offers the unmixed
  spend in one tap — *Use only private coins · 2 coins, fee +310 sats*, *Use only KYC coins*, or *only coins already
  linked to you* — each with the fee difference from a real preview. Combine says how many separate holders the chain
  shows the coins as now, and that spent together they become one.
- **Home, All vaults, each entity**: *Who can link these coins to you* — a donut and a table of coins, sats, worth
  and share by status, for the unspent coins of the vaults in view. A vault that is not coins on the chain is one
  bucket, listed under the table with its reason: an exchange balance is KYC (the exchange holds your identity), a
  custodian's statement and a Spark wallet are Known (the custodian, or the Spark service provider, sees every
  payment), a Lightning node is Known (its public key is a public identity), a Liquid wallet is unclassified until
  you say. The vault's default in Vault settings comes before each of these rules. *Show it over time* under the
  table draws sats held by status across the whole history: each coin counted from the day it came to the day it
  left, by what your records say about it now — knowledge of who can link a coin applies to its whole life.

## In the labels file

BIP 329 export carries `"provenance": {"status", "method", "counterparty", "note"}` on `output`, `addr` and `tx`
lines, next to `"disposal"`, and `"spendable": false` on a frozen coin's `output` line (BIP 329's own field, so Sparrow
and the others leave it alone too; import honours it, and an explicit `true` thaws); a line with a mark and no label is
written too. Import reads it back (an object, or just
the status as a string), and files a coin's record under `output:<txid:vout>`, the key the coin page reads. Other
BIP 329 readers ignore the field.

Rust: `provenance.rs` (the engine, `derive`, with its tests on a hand-built graph); commands `provenance_all`
(the graph from every chain vault's cached transactions, the marks from every vault's records, the statement rule from
the transfer links), `provenance_set`, `provenance_set_many` (the bulk flow's one write), `vault_provenance`,
`coin_freeze`. The derived view carries `spent_at`, `frozen` and `addr` so the screens can draw time and group by
address without the vault loaded. The design: `roadmap/anytwokeys/PROVENANCE-SPEC.md` in the
infrastructure repo.
