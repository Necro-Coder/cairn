# Frozen backup fixtures

## Do not regenerate these files

`v1-schema-4.cairn` was written once, by a build that no longer exists, and is never written again. It is the only proof this project has that a backup made by an older version of Cairn can still be opened by a newer one. Every other test of the backup format exports and imports inside a single process with a single build, which shows that the two halves of the code agree with each other and shows nothing whatsoever about the bytes on disk.

If `tests/backup_compatibility.rs` fails, the file is not the thing that is wrong. A change to the sixty-four byte header, to the chunk framing, to the record stream, to the compression settings or to the list of tables a backup carries has made last year's file unreadable, and the fix belongs in the reader. Rewriting the fixture makes the test pass and makes it worthless in the same commit, and nobody finds out until somebody needs a restore.

The test that writes it is marked `#[ignore]` so it cannot run as part of an ordinary `cargo test`. Running it deliberately is the only way to overwrite the file.

## When a new fixture is correct

Exactly one case: a deliberate bump of `BACKUP_FORMAT_VERSION` or of `RECORD_VERSION`, made on purpose and written down in a decision record. Then a second fixture is **added** beside this one, with its own version in the name, and the tests for this one still have to pass. The whole point of a format version is that the old readers keep working; a project that replaces its fixture on every bump has version numbers and no compatibility.

## What is in the file

Six rows across six tables, plus the ten tables a backup carries that happen to be empty. A null in a column where a null means something different from an absent value, text that is not ASCII, and a row that points at another row so that the write order matters. It is small on purpose: the fixture is about the shape of the file, not about volume.

The password is in the test source, and it is not a secret. It protects six invented rows in a file published in a public repository. A fixture whose password lived somewhere else would be a fixture nobody could open.

The Argon2id parameters in its header are the lowest the code accepts, which is lower than what a real vault is created with. That is also deliberate: it keeps the test fast, and it is what demonstrates that the parameters are read out of the file rather than assumed, so the defaults can be raised without stranding anybody's old backups.
