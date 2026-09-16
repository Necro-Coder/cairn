# Frozen vaults

Two vaults, written once and never regenerated. Every future version of this code has to keep opening them.

The header is a file format, and a file format is a promise. Every other test in this crate writes something and reads it back, which proves that a build agrees with itself. None of them would notice a change that made a new build disagree with the one somebody already has a vault from. These two would.

## What is here

| Directory | Argon2id parameters | Why it exists |
| --- | --- | --- |
| `default-parameters/` | m = 64 MiB, t = 3, p = 1 | What a vault created today is given. |
| `lowered-parameters/` | m = 32 MiB, t = 4, p = 1 | A second, different set, so that a build which only ever handled the defaults correctly would still be caught. |

Each directory holds a `vault.header` of exactly 168 bytes and a `record.sealed` of 77 bytes: a 24 byte nonce, the ciphertext, and a 16 byte tag.

## The password

    una contrasena de ejemplo para el fixture

It is written here on purpose. There is nothing to protect: the record inside says `media hora de lectura antes de dormir`, which somebody made up while writing this file. No real vault, no real password and no real data is involved.

## Reading them

`crates/cairn-crypto/tests/fixtures.rs` opens both, decrypts the record in each, and checks that the parameters, the creation time and the text are what they were when the files were written. The associated data of the record is spelled out field by field in that file rather than derived, because a future version has to produce the same associated data from the same fields and the only way to check that is to state them somewhere other than in the code that builds them.

## Regenerating them

    cargo test -p cairn-crypto --test fixtures -- --ignored

Do not run this to make a failing test pass. If a fixture stops opening, the question is what changed about the format, and overwriting the file answers it by deleting the evidence. Regenerating is a deliberate decision that belongs in a pull request explaining why the format moved and what happens to vaults that already exist.
