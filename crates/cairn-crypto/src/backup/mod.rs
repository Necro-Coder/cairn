//! The cryptography of an exported backup: its header, its key and its framing.
//!
//! A backup is the only artefact this project produces that leaves the machine it was made
//! on, and everything about the design follows from that one fact. There is no attempt
//! counter to slow an attacker down, no operating system keychain in the way, no lock
//! screen: whoever has the file has it for as long as they like. The only thing between
//! them and the content is Argon2id over the password somebody chose, and the public
//! documentation says so in those words rather than implying more.
//!
//! What lives here is the part with no input or output in it: [`header`] parses and writes
//! the sixty-four bytes at the front, and [`chunk`] seals and opens the body. Compression
//! and the record format live in the storage crate, next to the database they describe,
//! and the reason is not tidiness: the compression library is a binding to a C library, and
//! linking it into this crate would end the property that this crate can be checked under
//! Miri. A crate that cannot be checked under Miri is a crate whose `forbid(unsafe_code)`
//! covers only the code, not what the code calls.
//!
//! The key does not hang off the vault. [`crate::export_key`] takes a key encryption key
//! derived with Argon2id over the salt and parameters in the file's own header, so the file
//! opens with the password and nothing else. Decision record 0011 has the argument.

pub mod chunk;
pub mod header;
