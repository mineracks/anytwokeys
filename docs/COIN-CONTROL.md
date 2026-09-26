# Coin control: send, combine, split

Tick coins on **Coins**. A bar at the foot of the list says what they add up to and offers three things:

- **Send these** — Send opens with those coins ticked under Advanced, *Send all* on, and the Amount field holding what
  the recipient will get: the coins less the fee. A line at the top names the coins; *Change selected coins* goes back.
- **Combine coins** (two or more) — one coin, of everything less the fee, to the vault's next fresh receive address.
- **Split coin** (exactly one) — two to ten coins to fresh receive addresses. *Equal after the fee* divides what is left
  after the fee; when it does not divide, the remainder is spread one sat at a time so no two coins differ by more than
  one. *Amounts I choose* takes an amount for every coin but the last, which is the remainder. Every coin must clear
  546 sats.

Combine and Split are their own screen: the figures first (selected amount, remaining in this vault, fee), what goes
in, what comes out, the fee level, and every destination address in full with a Copy control. Build hands the
transaction to Sign like any other; after broadcast the transaction is labelled *Combined N coins into one* or *Split
one coin into N*, and each new coin *Combined from N coins* or *Split from one coin of X sats*.

Words the screens keep to: spending coins together publicly links them; a split's coins still share a visible history;
the amount returned keeps its cost basis and the fee is recorded separately; a YubiKey has no screen, so the card and
the review are where the destinations are checked. A vault of extended keys has every output described in the PSBT, so
a signer with a screen shows them as the vault's own.

Rust: `wallet::plan_own` / `wallet::build_own`; commands `spend_preview_own` / `spend_build_own`. The design and its
review history: `roadmap/anytwokeys/COIN-CONTROL-SPEC.md` in the infrastructure repo.
