//! Associated data: the bytes that are authenticated but not encrypted.
//!
//! Without it, an attacker who can write to the database file can copy the ciphertext of
//! one row over another, or put back an old version of a field, and the tag still
//! verifies. Nothing about the ciphertext says where it came from, so nothing about it can
//! say that it is in the wrong place. Binding the row identity into the tag is what makes
//! those edits fail to decrypt, and it is not optional.
//!
//! The one way to get this wrong is ambiguous serialisation. Joining the fields with a
//! separator means a table called `notes/2` and a table called `notes` with a row id
//! beginning `2` can produce the same bytes, and two different records that produce the
//! same associated data are two records whose ciphertexts can be swapped. So every
//! variable length field is written with its length in front of it, and no separator is
//! used anywhere.

use crate::error::CryptoError;

/// Longest a table or column name may be, in bytes.
///
/// These names come from this codebase, never from a person, so the limit is not about
/// hostile input. It is about keeping the encoding total: a length that cannot overflow a
/// `u32` is a length whose prefix is always exactly four bytes, and a parser with no
/// unbounded case is a parser with nowhere for a mistake to hide.
pub const MAX_NAME_LEN: usize = 64;

/// Length of a row identifier or a key identifier, in bytes.
///
/// Both are version four UUIDs. This crate takes them as plain arrays so that it does not
/// have to agree with the rest of the workspace about which UUID library to use; sixteen
/// bytes is the wire format either way.
pub const ID_LEN: usize = 16;

/// The authenticated context of one encrypted value.
///
/// Built through a constructor that demands every field, so a caller cannot leave one out
/// and end up with associated data that binds less than it should. The bytes are not a
/// secret: they are the table, the column and the identifiers, all of which are visible in
/// the schema. What they buy is that the ciphertext cannot be moved anywhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aad(Vec<u8>);

impl Aad {
    /// Builds the associated data for one field of one row.
    ///
    /// The layout is the one the project fixed for records: format version, table name,
    /// row identifier, column name, revision, key identifier. Every variable length field
    /// is preceded by its length as four little endian bytes, which is what makes two
    /// different sets of fields impossible to encode the same way.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::FieldTooLong`] if the table or column name is longer than
    /// [`MAX_NAME_LEN`]. That is a mistake in this codebase rather than something a person
    /// can cause, and it fails loudly rather than truncating, because a truncated name is
    /// a name that two different columns can share.
    pub fn record(
        format_version: u16,
        table: &str,
        row_id: &[u8; ID_LEN],
        column: &str,
        rev: u64,
        key_id: &[u8; ID_LEN],
    ) -> Result<Self, CryptoError> {
        check_name("table", table)?;
        check_name("column", column)?;

        let mut bytes = Vec::with_capacity(
            size_of::<u16>()
                + size_of::<u32>()
                + table.len()
                + ID_LEN
                + size_of::<u32>()
                + column.len()
                + size_of::<u64>()
                + ID_LEN,
        );

        bytes.extend_from_slice(&format_version.to_le_bytes());
        push_with_length(&mut bytes, table.as_bytes());
        bytes.extend_from_slice(row_id);
        push_with_length(&mut bytes, column.as_bytes());
        bytes.extend_from_slice(&rev.to_le_bytes());
        bytes.extend_from_slice(key_id);

        Ok(Self(bytes))
    }

    /// The encoded bytes, as the cipher wants them.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Rejects a name that is too long to encode.
fn check_name(field: &'static str, value: &str) -> Result<(), CryptoError> {
    if value.len() > MAX_NAME_LEN {
        return Err(CryptoError::FieldTooLong {
            field,
            len: value.len(),
            max: MAX_NAME_LEN,
        });
    }
    Ok(())
}

/// Appends a length prefix and then the bytes.
///
/// The cast cannot lose information because the caller has already rejected anything
/// longer than [`MAX_NAME_LEN`], which is far below what a `u32` holds. The assertion says
/// so in a way that survives somebody raising that limit without reading this far.
fn push_with_length(buffer: &mut Vec<u8>, value: &[u8]) {
    debug_assert!(value.len() <= MAX_NAME_LEN, "length prefix would truncate");
    let length = u32::try_from(value.len()).unwrap_or(u32::MAX);
    buffer.extend_from_slice(&length.to_le_bytes());
    buffer.extend_from_slice(value);
}

#[cfg(test)]
mod tests {
    use super::{Aad, ID_LEN, MAX_NAME_LEN};
    use crate::error::CryptoError;

    const ROW: [u8; ID_LEN] = [0x11; ID_LEN];
    const OTHER_ROW: [u8; ID_LEN] = [0x22; ID_LEN];
    const KEY_ID: [u8; ID_LEN] = [0x33; ID_LEN];
    const OTHER_KEY_ID: [u8; ID_LEN] = [0x44; ID_LEN];

    fn sample() -> Aad {
        Aad::record(1, "credentials", &ROW, "password", 7, &KEY_ID).unwrap()
    }

    #[test]
    fn the_same_fields_always_encode_the_same_way() {
        assert_eq!(sample(), sample());
    }

    #[test]
    fn changing_any_single_field_changes_the_encoding() {
        let baseline = sample();
        let variants = [
            Aad::record(2, "credentials", &ROW, "password", 7, &KEY_ID).unwrap(),
            Aad::record(1, "finances", &ROW, "password", 7, &KEY_ID).unwrap(),
            Aad::record(1, "credentials", &OTHER_ROW, "password", 7, &KEY_ID).unwrap(),
            Aad::record(1, "credentials", &ROW, "username", 7, &KEY_ID).unwrap(),
            Aad::record(1, "credentials", &ROW, "password", 8, &KEY_ID).unwrap(),
            Aad::record(1, "credentials", &ROW, "password", 7, &OTHER_KEY_ID).unwrap(),
        ];

        for variant in variants {
            assert_ne!(
                baseline, variant,
                "two records with different fields encoded to the same associated data"
            );
        }
    }

    #[test]
    fn a_name_cannot_be_confused_with_a_longer_one_split_differently() {
        // The failure a separator based encoding has. With a delimiter of any kind, a
        // table whose name ends where a column name begins can be rearranged to produce
        // the same bytes. With a length in front of each field, it cannot.
        let split_one = Aad::record(1, "ab", &ROW, "c", 0, &KEY_ID).unwrap();
        let split_two = Aad::record(1, "a", &ROW, "bc", 0, &KEY_ID).unwrap();
        assert_ne!(split_one, split_two);
    }

    #[test]
    fn a_name_containing_common_separators_is_still_unambiguous() {
        let with_slash = Aad::record(1, "notes/2", &ROW, "body", 0, &KEY_ID).unwrap();
        let with_null = Aad::record(1, "notes\u{0}2", &ROW, "body", 0, &KEY_ID).unwrap();
        let plain = Aad::record(1, "notes", &ROW, "2body", 0, &KEY_ID).unwrap();

        assert_ne!(with_slash, with_null);
        assert_ne!(with_slash, plain);
        assert_ne!(with_null, plain);
    }

    #[test]
    fn a_name_of_exactly_the_limit_is_accepted() {
        let name = "n".repeat(MAX_NAME_LEN);
        assert!(Aad::record(1, &name, &ROW, "body", 0, &KEY_ID).is_ok());
        assert!(Aad::record(1, "notes", &ROW, &name, 0, &KEY_ID).is_ok());
    }

    #[test]
    fn a_name_one_byte_over_the_limit_is_rejected() {
        let name = "n".repeat(MAX_NAME_LEN + 1);

        let table = Aad::record(1, &name, &ROW, "body", 0, &KEY_ID);
        assert!(matches!(
            table,
            Err(CryptoError::FieldTooLong { field: "table", .. })
        ));

        let column = Aad::record(1, "notes", &ROW, &name, 0, &KEY_ID);
        assert!(matches!(
            column,
            Err(CryptoError::FieldTooLong {
                field: "column",
                ..
            })
        ));
    }

    #[test]
    fn an_empty_name_is_accepted_and_still_distinguishable() {
        let empty_table = Aad::record(1, "", &ROW, "body", 0, &KEY_ID).unwrap();
        let empty_column = Aad::record(1, "body", &ROW, "", 0, &KEY_ID).unwrap();
        assert_ne!(empty_table, empty_column);
    }

    #[test]
    fn the_encoding_has_the_length_the_layout_says() {
        let encoded = sample();
        let expected = 2 + 4 + "credentials".len() + ID_LEN + 4 + "password".len() + 8 + ID_LEN;
        assert_eq!(encoded.as_bytes().len(), expected);
    }
}
