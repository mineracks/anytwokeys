# Entities, and the three sections

**Entities** are the legal owners — a person, a company, a trust, a fund. Each vault belongs to one (`Vault.entity`),
set from Vault settings as chips or from the Entities list, where vaults with no owner are listed with the same
chips. An entity's card shows its vaults, its keys and where they are kept, and *Over time* for exactly its vaults:
the All vaults summary filtered to the owner, so holding, worth, cost basis and gains come out per entity — what each
return is about. All vaults groups its chips by owner. `Store.entities`; `entity_save` / `entity_delete` /
`vault_entity_set`. Entities are not gated.

**Keys**, **Locations** and **Entities** are each a section under ANY TWO KEYS in the sidebar: a list of rows, and a
card reached by clicking one, with crumbs back. Keys lists every key across every vault and opens the same card the
vault's Keys screen opens; the crumb and the way back say where it was opened from. Locations lists a row per place;
the card is its own screen; *Not yet placed* stays on the list. Keys and Locations wait for the same proof of
presence (Touch ID, or a plugged-in key signing a digest), drawn by one gate, because together they are the map to
the custody.

**Icons.** An entity, a place or a key wears a mark: a house glyph, or a picture. Click the mark on a card to change
it; a picture is chosen in Rust, sealed through the attachment store (metadata stripped, a small copy made) and kept
by the small copy's hash (`Store.icons`, `icon_set` / `icon_pick`). The mark shows on list rows, cards, and the
owner's chips on All vaults.
