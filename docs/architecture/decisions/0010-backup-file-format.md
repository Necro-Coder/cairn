# 0010 — A backup is one file, framed in chunks, with nothing written that a reader has to believe

Date: 2026-09-17 · Status: accepted

## Context

A backup is the file somebody reaches for on the worst day they have with this application: the disk failed, the laptop was stolen, the password was changed and then forgotten, the database will not open. It is read years after it was written, on a machine that may be a different one, by a build that may be several versions newer. Nothing else this application produces has that shape.

That gives it three properties nothing else here has to have. It is read by a program that does not trust it, because a file on a USB stick is a file anybody could have edited. It has to keep working across versions, because a format that only the build which wrote it can read is a format that fails exactly when it is needed. And it can be large — the ceiling in this design is 512 MiB — on a device with a few hundred megabytes of memory to spare.

The application already has an encrypted database. The obvious answer is to copy that file, and it is wrong for a reason worth writing down: a copy of `cairn.db` is openable only with the key hierarchy on the machine that made it, which means the one scenario where a backup matters most — this machine is gone — is the one it does not cover.

## Decision

**A backup is a single file with a 64-byte cleartext header and a body of 64 KiB chunks.**

The header holds the magic `CAIRNBAK`, a format version, a compression code, the Argon2id salt and parameters, a 16-byte nonce base and the chunk length, and the rest is reserved and must be zero. It is in the clear because a reader has to know how to derive the key before it can decrypt anything, and it is authenticated because every chunk names the whole 64 bytes in its associated data. Editing one byte of it does not produce a different reading; it produces a file where the first chunk fails its tag.

**Each chunk is sealed with XChaCha20-Poly1305 and its nonce is recomputed, never stored.** The nonce is the 16-byte base from the header followed by the chunk index as a big-endian `u64`. A nonce read out of a file is a nonce an attacker chooses; a recomputed one means reordering, repeating or dropping a chunk produces the wrong nonce and fails the tag, before the index in the associated data even gets a chance to.

**A chunk on disk is ciphertext followed by its 16-byte tag, with no length prefix.** Nothing is written that a reader has to believe before it can check anything. A reader knows a chunk has ended because the file has, and the associated data of every chunk carries a "last chunk" flag, so truncation exactly on a chunk boundary fails too: the last complete chunk says it is not the last one and there is nothing behind it.

**The whole record stream is compressed with zstd once, before it is chunked**, rather than each chunk being compressed on its own. Chunk-by-chunk compression throws away almost all of the ratio, which is the only reason the dependency is accepted at all.

**Decompression is bounded while it runs**, not afterwards: an absolute ceiling of 1 GiB produced, and a maximum expansion ratio of 100 to 1 that starts being enforced after the first mebibyte of output. Small files legitimately compress far better than that at the start, so demanding the ratio from the first byte would refuse honest files; after a mebibyte, 100 to 1 is not data.

**The body is one record per line.** A manifest line with the schema version and the table list, then a header line per table, then one line of JSON per row. Every type refuses unknown fields, and each of the limits — line length, field length, nesting depth, fields per record, rows per table, bytes in the file — is checked while reading rather than after.

**Nothing ever holds the whole file.** Rows are paged out of SQLite, serialised, compressed, chunked, sealed and written; on the way back the file is read a buffer at a time through a reader that decrypts chunk by chunk, the decompressor pulls from that reader, and lines come out one at a time. This is the property that decides whether a restore works on a phone.

**An encrypted column travels as its plaintext**, and is sealed again under the receiving vault's key on the way in. It has to: a stored ciphertext is authenticated against its table, its row, its column, its revision and the identifier of the key that sealed it, and none of that survives a move into another database.

**An export is not finished until it has been read back.** It writes to a temporary name beside the destination, flushes, renames, and then opens the finished file and decrypts it from the first byte to the last using the password rather than the key still in memory.

**A reader says four things about a file and no more**: this is not a Cairn backup, this version cannot be read, the password does not open it, and it is damaged or incomplete.

**One frozen file is committed to the repository**, with a test that opens it and a README saying that regenerating it destroys the only compatibility proof the project has.

## Alternatives considered, and why not

**Copying the SQLCipher database file.** Simplest by a distance, and it fails the main case. That file opens only with the key hierarchy of the machine that wrote it, so it is useless on a new machine, which is exactly when a backup is wanted. It also carries `sync_state`, which is about the device rather than about the person, and restoring it on another machine would make a later merge skip records that machine has never seen — data loss that looks like nothing at all.

**Storing the 24-byte nonce in front of each chunk.** The usual shape, and it hands the attacker a value the reader uses. Recomputing it costs nothing, saves 24 bytes per chunk, and turns "reorder two chunks" from something the format has to detect into something that cannot be expressed.

**A length prefix per chunk.** It would make the reader simpler. It would also be the one number in the file that is read and acted on before anything has been authenticated, which is where length-prefix parsers go wrong. Without it there is nothing to lie about.

**One JSON document for the whole backup.** Much easier to write and read, and it forces the whole thing into memory at both ends. At 512 MiB that is not a slow restore, it is a restore that does not happen on a phone.

**A binary format of our own instead of lines of JSON.** It would save some space over an already-compressed stream, and it would still need documenting, fuzzing and a compatibility fixture. The saving does not pay for the work or for the extra ways a hand-rolled parser goes wrong.

**Carrying encrypted columns as opaque ciphertext and re-sealing nothing.** It sounds stronger, and it does not work: the associated data binds each value to a row identity and a key that the receiving database does not have. It would produce a backup whose passwords never open again. The real consequence of the choice that does work is the uncomfortable one, and it is stated in the user documentation in plain words: inside a decrypted backup the passwords are in the clear, and the whole protection of the file is Argon2id over the password it was made with.

**Checking the decompression limits at the end.** A limit checked after the fact is a limit that has already cost what it was there to save.

**Trusting an export because the writer did not report an error.** Rejected on principle. A backup nobody has ever read is not a backup, and the day it is needed is the worst possible day to find out. The verification pass derives the key again from the finished file's own header rather than reusing what is in memory, so what it checks is the file rather than the intention.

**Saying which check failed.** A reader that reports "the tag failed on chunk 37" tells whoever is editing the file how close they got. Four answers, and never which byte.

## Consequences

**Good.** A backup opens with a password and nothing else. No machine, no header file, no key hierarchy, no installation of this application beyond one that can read the format.

**Good.** Cutting the file, moving a chunk, repeating one, or changing a single byte anywhere — header, ciphertext or tag — all fail, and they fail before anything has been handed to a parser.

**Good.** Memory is flat regardless of file size, at both ends.

**Good.** The parameters that decide how expensive the password is to attack live in the file. They can be raised as devices get faster without stranding a single backup that already exists, and the frozen fixture is the test that says so.

**Bad.** The format is now a public contract. Changing it means a version bump, a second fixture, and code that reads both. That cost is real and it is the point: the alternative is a backup that a newer build cannot read.

**Bad.** zstd is a binding to a C library, so it cannot live in `cairn-crypto` without ending that crate's ability to be checked by Miri. It lives in `cairn-db` instead, which means the compression layer and the encryption layer are in different crates, and the file format is described in two places rather than one.

**Bad.** A decrypted backup has one layer of protection, not two. Somebody who exports to a shared folder with a weak password has weakened their password vault to that one password, and no amount of care inside this application changes it.

**Neutral.** The header is in the clear, so anybody who finds the file can tell it is a Cairn backup and can read the Argon2id parameters it was made with. Both are unavoidable — a reader needs them — and neither says anything about the contents.
