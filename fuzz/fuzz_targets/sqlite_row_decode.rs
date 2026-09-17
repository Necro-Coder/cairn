//! Feeds arbitrary bytes to the decoder that opens a sealed column of a row.
//!
//! The second parser whose input somebody with the file gets to write, and the one with more to
//! find than the header: the header is a fixed layout with no length fields, and this walks a
//! blob whose length came from the same untrusted place as its contents. A column of thirty nine
//! bytes, a column of nothing, a nonce that runs off the end of the buffer — every one of those
//! is a slice somebody has to have got right.
//!
//! What is under test is not that decoding succeeds. Almost nothing here will decrypt, and that
//! is the expected answer. What is under test is that every input returns: an error for the ones
//! that are not a value this vault wrote, and a plaintext for the ones that are, and never a
//! panic, an index out of bounds or a subtraction that wrapped.
//!
//! The row identity is varied along with the bytes. A decoder that ignored the associated data
//! would pass a fuzzer that only ever asked it about one row, and ignoring the associated data is
//! precisely the bug that would let a ciphertext be moved from one row to another.
#![no_main]

use cairn_db::codec::{FieldCodec, RowKey};
use cairn_domain::Rev;
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

/// The key the target decodes against.
///
/// A fixed one, and it is not a secret: nothing here is protecting anything. What matters is that
/// the same bytes mean the same thing on every run, because a fuzzer that finds a crash with a
/// key it generated has found a crash nobody can reproduce.
const KEY: [u8; 32] = [7; 32];

/// The identifier of the key, likewise fixed.
const KEY_ID: [u8; 16] = [9; 16];

fuzz_target!(|data: &[u8]| {
    // The first bytes steer the row this value claims to belong to, and the rest are the value.
    // Taken from the same input so the fuzzer can move both together; a target whose shape is
    // fixed in the harness only ever explores one dimension of it.
    let (steering, stored) = data.split_at(data.len().min(18));

    let mut row_id = [0_u8; 16];
    for (slot, byte) in row_id.iter_mut().zip(steering.iter()) {
        *slot = *byte;
    }

    let rev = Rev::from_number(u64::from(steering.get(16).copied().unwrap_or(0)));
    let table = match steering.get(17).copied().unwrap_or(0) % 4 {
        0 => "vault_entries",
        1 => "habits",
        2 => "settings",
        _ => "transactions",
    };
    let column = match steering.first().copied().unwrap_or(0) % 3 {
        0 => "title",
        1 => "notes",
        _ => "value",
    };

    let key = cairn_crypto::DataKey::from_bytes(KEY);
    let codec = FieldCodec::new(&key, KEY_ID);
    let row = RowKey {
        table,
        row_id: Uuid::from_bytes(row_id),
        rev,
    };

    // The answer is almost always an error, and an error is a pass. The one thing this must not
    // do is fail to answer.
    let _ = codec.open(row, column, stored);
    let _ = codec.open_text(row, column, stored);
});
