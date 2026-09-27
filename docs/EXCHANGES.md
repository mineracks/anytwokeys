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
