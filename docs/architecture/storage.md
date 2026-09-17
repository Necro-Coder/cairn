# Storage

How Cairn keeps data on disk: which files exist, what is encrypted by what, and what somebody who takes the disk can see.

This page is about the file. For the key hierarchy that produces the keys named here, read [cryptography](cryptography.md). For what each table holds, read [the data model](data-model.md).

## The files

Everything lives in one directory, chosen by the platform and never written into the repository. On Windows it is under `%LOCALAPPDATA%`; the profile inside it can be changed with the `CAIRN_PROFILE` environment variable, whose value is validated as a single path segment so that it cannot escape the directory.

| File | What it is | Encrypted by |
| --- | --- | --- |
| `cairn.header` | The vault header: the Argon2id parameters, the salts, the key identifier, and the data key wrapped under the key-encryption key | Itself. The wrapped key is an AEAD ciphertext whose associated data is the rest of the header, so a header edited by hand no longer unwraps |
| `cairn.device` | The identifier this installation stamps into every row it writes | Sealed under the data key |
| `cairn.db` | The database | SQLCipher, under `k_db` |
| `cairn.db-wal`, `cairn.db-shm` | SQLite's write-ahead log and its shared-memory index | SQLCipher encrypts the log as well. This is the reason for whole-file encryption rather than column encryption alone: without it, every value ever written would be readable in the log |

A backup taken before a migration is written beside them with `VACUUM INTO`, which produces a file encrypted with the same key rather than a plaintext copy.

## Two layers, and what each one is for

**SQLCipher over the whole file.** AES-256 in CBC with a per-page HMAC, keyed by `k_db`, which is derived from the data key by HKDF-SHA-512 with its own info string. It covers the pages, the schema, the indexes and the journal. Its answer is to a stolen disk: without the key the file is indistinguishable from noise, and not even the names of the tables are visible.

**XChaCha20-Poly1305 per record.** Applied to individual columns, keyed by the data key, with a fresh random 24-byte nonce on every write. Its answer is to somebody who has the file and can write to it. The associated data of each value is

```
format_version || table || row_id || column || rev || key_id
```

so a ciphertext copied from one row into another fails to verify, a value from an earlier revision put back into the same row and column fails to verify, and a value from a different vault fails to verify. Each of those has a test.

A row is sealed whole or not at all: the sealing function is given every encrypted column of its table and refuses a partial set. Sealing three of four columns at a new revision would leave the fourth authenticated under the old one, and the next read of it would fail with an error that says a value did not decrypt and nothing about why.

Which columns carry the second layer, and why the answer differs between the vault and the other modules, is [ADR 0009](decisions/0009-what-the-database-encrypts.md).

## The PRAGMAs, in order

Order matters here, and getting it wrong is silent rather than loud.

1. `PRAGMA key` — first, before any other statement. The key goes in as `x'…'`, which tells SQLCipher these are the key bytes rather than a passphrase: Argon2id has already run, at parameters this project chose, and a second derivation on top would buy nothing while obscuring which one actually protects the file.
2. `PRAGMA cipher_memory_security = ON` and `PRAGMA cipher_page_size = 4096` — the cipher settings, before a page is read. Afterwards the file has been interpreted and changing how it is interpreted is too late.
3. `PRAGMA foreign_keys = ON` — there are no foreign keys in the schema, for reasons [the data model](data-model.md) explains, but the setting is on so that adding one later behaves as written rather than as a comment.
4. `PRAGMA journal_mode = WAL`, `PRAGMA synchronous = NORMAL`, `PRAGMA busy_timeout` — durability without a full rewrite per commit, and a bounded wait rather than an immediate failure when the lock is held.
5. `PRAGMA temp_store = MEMORY` — no temporary table and no sort buffer reaches the disk in the clear.
6. `PRAGMA mmap_size = 0` — SQLCipher and memory-mapped reads are incompatible. Written explicitly, and asserted in a test, so that nobody turns it on later as an optimisation and gets plaintext pages in the page cache.
7. `PRAGMA trusted_schema = OFF` and `PRAGMA cache_size` — the schema is not allowed to invoke functions, and the cache is expressed in kibibytes so that it means the same amount of memory whatever the page size is.

The journal mode is read back rather than trusted: a file on a network share stays in its old mode without saying so.

Opening a connection reads no page, so it succeeds against any file at all. A wrong key is therefore found deliberately, by reading the schema immediately after the settings are applied, instead of by whichever query happens to run first — which could be minutes later and somewhere that reports it as something else.

The connection is a single one behind a mutex. A pool with a local SQLite file buys nothing and multiplies the places a connection could still be alive, with the key inside it, at the moment the vault locks.

## What locking does

Closing the vault, whether the person asked or the inactivity timer did, does all of this in one place and in this order:

1. The in-memory index of vault titles is emptied, and the strings are overwritten rather than dropped. It is the only plaintext the application keeps for longer than one call.
2. The connection is closed, explicitly. `Storage::close` consumes the value, so there is nothing left afterwards that could still be used.
3. The keys are dropped, which is what clears them.

The index is cleared before the connection is closed and whether or not the close succeeds. A database that refuses to let go, because something still holds a statement, is not a reason to leave every title of the vault readable in the process.

## What is visible in the cold file

With the key: everything, subject to the per-record layer above.

Without the key: the size of the file, and its timestamps. The header is a fixed-size structure whose fields are the Argon2id parameters, two salts, a key identifier and a wrapped key — no name, no path, no count, nothing about the person. The database is indistinguishable from random data.

What the file's size leaks is the order of magnitude of how much is in it, and nothing finer. Cairn does not pad, because padding a local database to a fixed size costs the size of the padding on every write and answers a question — "roughly how many records does this person have" — that the existence of the application already mostly answers.

## What is visible with the key but without the second layer

This is the interesting case, because it is what a synchronised peer sees and what somebody who compromised the master password sees before they open anything.

Inside the decrypted database, in the clear: every identifier, every `created_at` and `updated_at`, every `device_id`, every `deleted` flag, every clock reading and revision, the structure of the folder tree, which entry each URL and field belongs to, positions, colours, icons, flags, quantities, currencies and days. In habits and finances, also the names: of an area, of a habit, of an account, of a category.

Sealed, and not readable without also opening each value: every note, every password, every user name, every custom field, every URL, and in the vault, every title, folder name and tag name.

So a reader of the decrypted database learns how many passwords there are, how they are organised, which are favourites and when each was last used — and not one of them.
