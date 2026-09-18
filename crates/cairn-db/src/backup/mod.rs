//! Exporting the whole vault to one encrypted file, and reading one back.
//!
//! The cryptography of the file lives in `cairn-crypto`: the sixty-four byte header, the
//! key derivation and the chunk framing that makes truncation, reordering and a single
//! changed byte all fail. What lives here is everything that needs a database or a disk.
//!
//! [`compress`] is the zstd layer and the two limits that stop a decompression bomb.
//! [`format`] is the record stream, one line of JSON per row, and the limits that bound a
//! hostile one. [`schema`] is the list of tables a backup carries, checked against the real
//! schema by a test so the two cannot drift. [`tables`] turns rows into records and back,
//! decrypting every sealed value on the way out and sealing it again on the way in.
//! [`base64`] is the encoding blobs travel in, written here rather than taken from a crate.
//!
//! Three properties hold across all of it, and each one is a decision rather than an
//! accident.
//!
//! Nothing ever holds the whole file. Rows are paged out of SQLite, compressed, chunked,
//! sealed and written; on the way back they are read, opened, decompressed and split into
//! lines a buffer at a time. The memory an export or an import needs is a constant, which
//! is what decides whether this works on a phone at all.
//!
//! Nothing hostile reaches the live database. An import writes a staging database of its
//! own, complete and verified, and only then is the caller asked whether to replace
//! anything.
//!
//! An import may say four things and no more: this is not a Cairn backup, this version
//! cannot be read, the password is wrong, and the file is damaged or incomplete. Never
//! which byte, never which check. A reader that says where it stopped tells whoever is
//! editing the file how close they got.

pub mod base64;
pub mod compress;
pub mod export;
pub mod format;
pub mod history;
pub mod import;
pub mod plaintext;
pub mod schema;
pub mod swap;
pub mod tables;
pub mod verify;
