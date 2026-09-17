# 0009 — What the database encrypts, column by column

Date: 2026-09-17 · Status: accepted

## Context

Cairn encrypts at rest twice. SQLCipher encrypts the whole file, including the table names, the indexes and the write-ahead log; on top of that, individual columns are sealed with XChaCha20-Poly1305 and associated data that binds each ciphertext to its table, its row, its column and the revision it was written at.

The second layer is not redundant. Whole-file encryption answers "somebody took the disk". It answers nothing about somebody who has the file and wants to rearrange it: move the ciphertext of one row into another, or put back the value a field held two revisions ago. The associated data is what turns both of those into a value that does not decrypt instead of a value that decrypts into a lie.

The second layer also has a cost that the first does not, and the cost is what this decision is about. A sealed column cannot be compared, sorted, filtered or indexed by the database, because the database cannot read it. Every query that would have touched it becomes a full decrypt of the table in Rust.

So the question is not "should everything be sealed". Everything is already encrypted. The question is which columns pay the second cost, and the answer is different per module because what counts as content is different per module.

## Decision

Sealed with per-record AEAD: anything a person composed as prose, and anything that is a secret in itself.

- Habits: `habits.notes`, `habit_entries.note`, `habit_pauses.reason`.
- Vault: `vault_entries.title`, `.username`, `.password`, `.notes`; `vault_folders.name`; `vault_urls.value`; `vault_fields.label` and `.value`; `vault_tags.name`; `vault_password_history.password`.
- Finances: `transactions.note`, `budgets.note`.
- Settings: `settings.value`.

Left to SQLCipher alone: identifiers, parent pointers, positions, flags, kinds, quantities, currencies, moments, civil days, colours and icons — and, deliberately, the names in habits and finances: `habit_areas.name`, `habits.name`, `accounts.name`, `categories.name`.

The vault is the exception and takes the opposite rule. There, almost everything a person reads is sealed, including the title of an entry.

## Alternatives considered (and why not)

**Seal every column everywhere.** The consistent answer, and the one that makes the module list above unnecessary. It also makes every list a full table decrypt: no ordering in SQL, no paging by keyset, no heatmap of a year inside its budget, no monthly report that is two sums. For habits and finances those are the operations the screens are made of, and doing them in Rust over a decrypted copy of the table means the whole table is in memory in the clear anyway — which is the property sealing was supposed to protect.

**Seal nothing, and rely on SQLCipher.** Cheap, fast, and it loses the property that makes a synchronised file safe to accept: without associated data, a ciphertext moved between rows still verifies, and a rollback to an earlier revision is undetectable.

**A blind index — an HMAC of the normalised token — so sealed columns can still be searched.** This is the standard answer and it stays on the table for the vault if the in-memory title index stops fitting. It is not free: an HMAC over a normalised token leaks equality, so two entries with the same title are visibly the same, and a dictionary of likely titles can be tried offline against the index. For a few thousand entries the index in memory is simpler and leaks nothing, so the blind index is the fallback and not the first move.

**Sealing the names in habits and finances too.** The tempting half measure, and the reason it is written down here rather than decided quietly: "tomar la medicación de las ocho" as the name of a habit says as much as any note attached to it, and "Hipoteca" as the name of an account says something about the person. Under this decision those are protected by the file's own encryption and by nothing else. That is a real reduction against an attacker who has write access to the file and is trying to learn what the names are by rearranging rows — an attack the note is protected from and the name is not. It was taken with open eyes because sealing the name is what removes ordering, paging and grouping from the three screens those tables exist for.

## Consequences (good and bad)

Good: habits and finances keep their queries in SQL. A year of a habit heatmap, a page of a ledger and a month of totals are index scans, and the budgets in the phase document are reachable.

Good: the vault leaks nothing about its contents to anyone reading the file, structure aside. What is visible in the clear inside the decrypted database is: how many entries there are, how they are grouped, which ones are favourites, and when each was last used.

Bad: the vault cannot be searched or sorted in SQL by anything a person reads. Search is a list of titles decrypted into memory when the vault unlocks and emptied when it locks, which puts a ceiling on how many entries can be searched and makes the unlock proportional to the size of the vault. The ceiling is a constant in the code and the number is measured rather than assumed.

Bad: two rules for two modules is a thing to remember. The mitigation is that each table's sealed columns are declared once, as a `SealedColumns` constant beside its repository, and the sealing code refuses a partial set — so a row is sealed whole or the write fails.

Bad: the names in habits and finances are a known, accepted gap. If it is ever closed, it is a migration with data in it, which is the expensive kind.
