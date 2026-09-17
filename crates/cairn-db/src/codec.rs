//! Encrypting one field of one row, and the rule that a row is sealed whole or not at all.
//!
//! SQLCipher already encrypts the file. This is the second layer, and it exists for something
//! the first one cannot do: it binds a ciphertext to the row it belongs to. Whole-file
//! encryption protects the file against somebody who does not have the key, and offers nothing
//! at all against somebody who has the file and wants to rearrange it. The associated data is
//! what makes copying the ciphertext of one row over another, or putting back the value a
//! field had two revisions ago, fail to decrypt instead of succeeding quietly.
//!
//! Two decisions here are worth reading for.
//!
//! The revision is inside the associated data. Without it, an attacker with write access could
//! reinstate an old ciphertext into the same row and the same column and the tag would still
//! verify, which is a rollback nobody can detect. With it, the tag is bound to the revision
//! the value was written at, so the old ciphertext only verifies at the old revision, and the
//! row does not carry that revision any more.
//!
//! A row is sealed whole. [`FieldCodec::seal_row`] wants every encrypted column of the table
//! and refuses a partial set. That is not tidiness: sealing three of a table's four encrypted
//! columns at a new revision leaves the fourth authenticated under the old one, and the next
//! read of it fails with an error that says a value did not decrypt and nothing about why.

use cairn_crypto::{Aad, DataKey, FreshNonce, ID_LEN, Sealed, open, seal};
use cairn_domain::Rev;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::DbError;

/// The version of the record layout that goes into the associated data.
///
/// One rather than the header's format version, because they are different things that may
/// move independently: the header describes how a vault is opened, and this describes how a
/// stored value is bound to its row. Sharing a number would tie one to the other for no reason.
pub const RECORD_FORMAT_VERSION: u16 = 1;

/// The identity of one row at one revision.
///
/// Everything that goes into the associated data except the column and the key identifier: the
/// column changes per value, and the key identifier belongs to the open vault rather than to
/// the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowKey<'a> {
    /// The table, as the schema names it.
    pub table: &'a str,
    /// The identifier of the row.
    pub row_id: Uuid,
    /// The revision the value is being written at, or was written at.
    pub rev: Rev,
}

/// The encrypted columns of one table, in the order the schema declares them.
///
/// A named type rather than a slice passed around, so that a table's list is written once and
/// every sealer for that table is checked against the same thing. A second, slightly different
/// list somewhere else is how half a row ends up sealed under the wrong revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealedColumns(&'static [&'static str]);

impl SealedColumns {
    /// Declares the encrypted columns of a table.
    #[must_use]
    pub const fn new(columns: &'static [&'static str]) -> Self {
        Self(columns)
    }

    /// The columns, in order.
    #[must_use]
    pub const fn names(&self) -> &'static [&'static str] {
        self.0
    }

    /// How many there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the table has no encrypted columns at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Seals and opens stored values for as long as the vault is open.
///
/// Borrows the key rather than holding one. The key lives in exactly one place in the process
/// and is read through a closure that cannot outlive its lock, so a codec is built inside that
/// closure, used, and dropped with it.
#[derive(Debug)]
pub struct FieldCodec<'a> {
    key: &'a DataKey,
    key_id: [u8; ID_LEN],
}

impl<'a> FieldCodec<'a> {
    /// Builds a codec around the data key of an open vault and the identifier of that key.
    #[must_use]
    pub const fn new(key: &'a DataKey, key_id: [u8; ID_LEN]) -> Self {
        Self { key, key_id }
    }

    /// Encrypts one value, binding it to its row, its column and its revision.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Sealed`] if the associated data cannot be built, which means a table
    /// or column name in this codebase is longer than the format allows, or if the value is
    /// larger than one record may be.
    pub fn seal(
        &self,
        row: RowKey<'_>,
        column: &str,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, DbError> {
        let aad = self.associated_data(row, column)?;
        let sealed = seal(self.key, FreshNonce::generate()?, &aad, plaintext)?;

        Ok(sealed.to_bytes())
    }

    /// Encrypts a value that may be absent.
    ///
    /// An absent value stores as `NULL` and an empty one stores as the encryption of zero
    /// bytes. They are different things and the schema keeps them different: a note that was
    /// never written and a note somebody cleared are not the same fact about a row.
    ///
    /// # Errors
    ///
    /// The same as [`FieldCodec::seal`].
    pub fn seal_optional(
        &self,
        row: RowKey<'_>,
        column: &str,
        plaintext: Option<&[u8]>,
    ) -> Result<Option<Vec<u8>>, DbError> {
        plaintext
            .map(|value| self.seal(row, column, value))
            .transpose()
    }

    /// Decrypts one value, checking that it belongs where it was found.
    ///
    /// The plaintext comes back in a buffer that clears itself when it is dropped, because
    /// what this decrypts is a password as often as it is a note.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Sealed`] if the value does not decrypt here. A wrong key, a
    /// ciphertext moved from another row, a value put back from an earlier revision and a
    /// flipped bit all arrive as the same error, carrying nothing that tells them apart.
    pub fn open(
        &self,
        row: RowKey<'_>,
        column: &str,
        stored: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, DbError> {
        let aad = self.associated_data(row, column)?;
        let sealed = Sealed::from_bytes(stored)?;

        Ok(open(self.key, &sealed, &aad)?)
    }

    /// Decrypts one value and reads it as text.
    ///
    /// # Errors
    ///
    /// The same as [`FieldCodec::open`], and [`DbError::Sealed`] again if what came back is not
    /// UTF-8. Bytes that decrypted but are not text mean the row was written by something that
    /// is not this program, which is the same class of problem and gets the same answer.
    pub fn open_text(
        &self,
        row: RowKey<'_>,
        column: &str,
        stored: &[u8],
    ) -> Result<Zeroizing<String>, DbError> {
        let bytes = self.open(row, column, stored)?;
        let text = core::str::from_utf8(&bytes)
            .map_err(|_not_text| DbError::Sealed(cairn_crypto::CryptoError::Open))?;

        Ok(Zeroizing::new(text.to_owned()))
    }

    /// Seals every encrypted column of a row at one revision, refusing anything less.
    ///
    /// The values arrive named rather than positional, and come back in the order the table
    /// declares. Naming them is what lets the check below be about the set of columns rather
    /// than about how many there happened to be, and ordering the answer is what lets the
    /// caller bind them straight into a statement.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::RowColumns`] if the number offered is not the number the table has,
    /// [`DbError::UnknownColumn`] if one of them is not a column of this table or is named
    /// twice, and whatever [`FieldCodec::seal`] returns.
    pub fn seal_row(
        &self,
        row: RowKey<'_>,
        columns: SealedColumns,
        values: &[(&str, Option<&[u8]>)],
    ) -> Result<Vec<Option<Vec<u8>>>, DbError> {
        if values.len() != columns.len() {
            return Err(DbError::RowColumns {
                found: values.len(),
                expected: columns.len(),
            });
        }

        let mut sealed = Vec::with_capacity(columns.len());
        for name in columns.names() {
            // Looked up by name rather than by position, and exactly once. A duplicate in the
            // offered values would otherwise satisfy the count while leaving another column
            // unwritten, which is the partial row this function exists to refuse.
            let mut found = values.iter().filter(|(offered, _)| offered == name);
            let matched = found.next();
            if found.next().is_some() {
                return Err(DbError::ColumnNotOfferedOnce { column: name });
            }
            let Some((_, value)) = matched else {
                return Err(DbError::ColumnNotOfferedOnce { column: name });
            };

            sealed.push(self.seal_optional(row, name, *value)?);
        }

        Ok(sealed)
    }

    /// The associated data for one value: format, table, row, column, revision and key.
    fn associated_data(&self, row: RowKey<'_>, column: &str) -> Result<Aad, DbError> {
        Ok(Aad::record(
            RECORD_FORMAT_VERSION,
            row.table,
            row.row_id.as_bytes(),
            column,
            row.rev.as_number(),
            &self.key_id,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};
    use cairn_domain::Rev;
    use uuid::Uuid;

    use super::{FieldCodec, RowKey, SealedColumns};
    use crate::error::DbError;

    /// Not a real password: a phrase invented for the test, at the cheapest parameters the
    /// cryptographic crate accepts, so the suite spends its time on the codec.
    fn an_open_vault() -> UnlockedVault {
        let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");
        vault
    }

    fn a_row(id: Uuid, rev: u64) -> RowKey<'static> {
        let rev = Rev::from_number(rev);
        RowKey {
            table: "habits",
            row_id: id,
            rev,
        }
    }

    #[test]
    fn what_goes_in_comes_back_out() {
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let row = a_row(Uuid::from_u128(1), 0);

        let stored = codec.seal(row, "notes", b"tomar la medicacion").unwrap();
        let read = codec.open(row, "notes", &stored).unwrap();

        assert_eq!(read.as_slice(), b"tomar la medicacion");
    }

    #[test]
    fn two_writes_of_the_same_value_produce_different_bytes() {
        // A fresh nonce every time. Identical ciphertexts would say that two rows hold the
        // same value, which is exactly the thing an encrypted column is for hiding.
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let row = a_row(Uuid::from_u128(1), 0);

        let first = codec.seal(row, "notes", b"same").unwrap();
        let second = codec.seal(row, "notes", b"same").unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn a_ciphertext_moved_to_another_row_does_not_decrypt() {
        // The whole reason the second layer exists. Whole file encryption is no defence at
        // all against somebody who has the file and rearranges it.
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let source = a_row(Uuid::from_u128(1), 0);
        let destination = a_row(Uuid::from_u128(2), 0);

        let stored = codec.seal(source, "notes", b"the secret").unwrap();

        assert!(matches!(
            codec.open(destination, "notes", &stored),
            Err(DbError::Sealed(_))
        ));
    }

    #[test]
    fn a_ciphertext_moved_to_another_column_does_not_decrypt() {
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let row = a_row(Uuid::from_u128(1), 0);

        let stored = codec.seal(row, "notes", b"the secret").unwrap();

        assert!(matches!(
            codec.open(row, "name", &stored),
            Err(DbError::Sealed(_))
        ));
    }

    #[test]
    fn a_ciphertext_moved_to_another_table_does_not_decrypt() {
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let id = Uuid::from_u128(1);

        let stored = codec.seal(a_row(id, 0), "notes", b"the secret").unwrap();
        let elsewhere = RowKey {
            table: "vault_entries",
            row_id: id,
            rev: Rev::FIRST,
        };

        assert!(matches!(
            codec.open(elsewhere, "notes", &stored),
            Err(DbError::Sealed(_))
        ));
    }

    #[test]
    fn raising_the_revision_invalidates_the_value_written_at_the_old_one() {
        // The anti-rollback property. Somebody with write access to the file can put back the
        // ciphertext a column had two revisions ago; binding the revision into the tag is what
        // makes that fail rather than succeed silently.
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let id = Uuid::from_u128(1);

        let old = codec.seal(a_row(id, 4), "notes", b"the old value").unwrap();

        assert!(matches!(
            codec.open(a_row(id, 5), "notes", &old),
            Err(DbError::Sealed(_))
        ));
        assert!(codec.open(a_row(id, 4), "notes", &old).is_ok());
    }

    #[test]
    fn another_vault_cannot_read_it() {
        let first = an_open_vault();
        let second = an_open_vault();
        let row = a_row(Uuid::from_u128(1), 0);

        let stored = FieldCodec::new(first.data_key(), *first.key_id())
            .seal(row, "notes", b"the secret")
            .unwrap();

        assert!(matches!(
            FieldCodec::new(second.data_key(), *second.key_id()).open(row, "notes", &stored),
            Err(DbError::Sealed(_))
        ));
    }

    #[test]
    fn an_absent_value_and_an_empty_one_are_different_things() {
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let row = a_row(Uuid::from_u128(1), 0);

        assert_eq!(codec.seal_optional(row, "notes", None).unwrap(), None);

        let empty = codec
            .seal_optional(row, "notes", Some(b""))
            .unwrap()
            .expect("an empty value is stored, not dropped");
        assert!(codec.open(row, "notes", &empty).unwrap().is_empty());
    }

    #[test]
    fn a_blob_too_short_to_be_a_sealed_value_is_refused_before_any_key_is_used() {
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let row = a_row(Uuid::from_u128(1), 0);

        for length in [0_usize, 1, 23, 24, 39] {
            assert!(
                matches!(
                    codec.open(row, "notes", &vec![0_u8; length]),
                    Err(DbError::Sealed(_))
                ),
                "a {length} byte blob was accepted"
            );
        }
    }

    #[test]
    fn a_row_is_sealed_whole_or_not_at_all() {
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let row = a_row(Uuid::from_u128(1), 1);
        let columns = SealedColumns::new(&["notes", "reason"]);

        let sealed = codec
            .seal_row(
                row,
                columns,
                &[("notes", Some(b"a note".as_slice())), ("reason", None)],
            )
            .expect("every column was offered");
        assert_eq!(sealed.len(), 2);
        assert!(sealed[0].is_some());
        assert!(sealed[1].is_none());

        let partial = codec.seal_row(row, columns, &[("notes", Some(b"a note".as_slice()))]);
        assert!(matches!(
            partial,
            Err(DbError::RowColumns {
                found: 1,
                expected: 2
            })
        ));
    }

    #[test]
    fn the_same_column_twice_does_not_satisfy_the_count() {
        // The failure the count alone would miss: two values for one column and none for the
        // other, which is the partial row wearing the right number of fields.
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let row = a_row(Uuid::from_u128(1), 1);
        let columns = SealedColumns::new(&["notes", "reason"]);

        let duplicated = codec.seal_row(
            row,
            columns,
            &[
                ("notes", Some(b"one".as_slice())),
                ("notes", Some(b"two".as_slice())),
            ],
        );

        assert!(matches!(
            duplicated,
            Err(DbError::ColumnNotOfferedOnce { column: "notes" })
        ));
    }

    #[test]
    fn a_sealed_row_opens_column_by_column_in_the_order_the_table_declares() {
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let row = a_row(Uuid::from_u128(7), 3);
        let columns = SealedColumns::new(&["notes", "reason"]);

        let sealed = codec
            .seal_row(
                row,
                columns,
                &[
                    ("reason", Some(b"because".as_slice())),
                    ("notes", Some(b"a note".as_slice())),
                ],
            )
            .expect("every column was offered");

        let notes = sealed[0].as_ref().expect("notes was written");
        let reason = sealed[1].as_ref().expect("reason was written");

        assert_eq!(
            codec.open(row, "notes", notes).unwrap().as_slice(),
            b"a note"
        );
        assert_eq!(
            codec.open(row, "reason", reason).unwrap().as_slice(),
            b"because"
        );
    }

    #[test]
    fn text_that_is_not_utf8_is_refused_rather_than_replaced() {
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let row = a_row(Uuid::from_u128(1), 0);

        let stored = codec.seal(row, "notes", &[0xff, 0xfe]).unwrap();

        assert!(matches!(
            codec.open_text(row, "notes", &stored),
            Err(DbError::Sealed(_))
        ));
    }

    #[test]
    fn text_survives_a_round_trip_including_characters_outside_ascii() {
        let vault = an_open_vault();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let row = a_row(Uuid::from_u128(1), 0);

        for value in ["", "medicación", "日本語", "a\u{0}b", &"z".repeat(10_000)] {
            let stored = codec.seal(row, "notes", value.as_bytes()).unwrap();
            assert_eq!(
                codec.open_text(row, "notes", &stored).unwrap().as_str(),
                value
            );
        }
    }
}
