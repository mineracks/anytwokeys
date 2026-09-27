# Exchange accounts

An exchange account is a vault read from the statements the exchange gives: New vault › **From an exchange** › choose
the files. Two exports are known by their header:

- **Amber** — one CSV. An id on every row, local time (the offset is read from the header's own `(+10)`), no
  running balance: what is held is the rows' sum, and the preview says so. Rows that never settled are not rows.
- **Bitaroo** — a zip per financial year holding an AUD statement and a BTC statement; choose every year at once.
  UTC, a running balance, no ids. The two statements pair a buy by its second; a fee row folds into the row it was
  charged on (Buy Fee, Deposit Fee, Withdrawal Fee); a row's id is the hash of its own columns. The running balance
  is checked row by row through every file — to the sat for coins, to the cent for money, since Bitaroo rounds each
  fee to the cent on the statement while its ledger runs finer — and the result is shown, never assumed.

Every figure is read from the file's decimal, never a float. A buy's **real price** — money paid plus the fee, over
the coins received — becomes that row's quote when the app's currency is the statement's, so the cost basis of coins
bought at the exchange is what was paid, not the feed's price for the day. A withdrawal is proposed against the
vault deposit that matches it by amount within three days, like any other move between the person's own wallets.

A later export: Vault settings › **Add a newer export** — the same preview, headed by how many rows are new; rows
already here are left alone. Nothing in an exchange vault can be spent.

Rust: `exchange.rs` (`parse`, `preview`, `merge`, `activity_rows`, `real_price`; a small zip reader on flate2);
commands `exchange_pick_preview` / `exchange_import`. Design and review: `roadmap/anytwokeys/EXCHANGES-AND-ENTITIES-SPEC.md`
in the infrastructure repo.

## Another exchange's CSV

For an exchange this app does not know by header: New vault › From an exchange › **Another exchange's CSV**. The
files' header comes back as a row per column, each with a guessed role in the house segments — Date, Kind, Coins,
Money, Fee (money), Fee (coins), Id, or not read — for the person to correct; then the exchange's name, the
currency, whether times are UTC or local, and whether coins are written as BTC or sats. *Read it* gives the same
preview as a known exchange's. Dates are read in any of the ways exports write them (ISO, day/month/year, the long
form, seconds since 1970); the kind is read from its column's words (buy, sell, withdraw, deposit, receive…) or,
without one, from the signs of the amounts; unsigned amounts take the sign their kind implies. The mapping is kept
on the account, so a later export from Vault settings is read by it without asking again. `exchange::raw`,
`exchange::mapped`, `Mapping`; commands `exchange_pick_raw` / `exchange_map_preview`.
