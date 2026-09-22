//! The only way the vault can be searched: a list of titles held in memory while it is open.
//!
//! Every title in that module is encrypted, so SQL cannot compare them, cannot sort them and
//! cannot match a prefix. What it can do is hand over the ciphertext, and the key that opens it
//! is already in this process for as long as the vault is unlocked. So the titles are opened
//! once when the vault opens, kept in a list, and searched by walking it.
//!
//! Walking a list is the point rather than a compromise. A search index written to disk is a
//! second copy of every title, and a copy that is not encrypted is exactly the thing the vault
//! exists to prevent; a copy that is encrypted cannot be searched, which puts it back here. A
//! few thousand titles is a few hundred kilobytes and a comparison that finishes before the next
//! frame, and the ceiling in [`vault::MAX_TITLES`] is what keeps that sentence true for a file
//! this program did not write.
//!
//! What matters as much as the search is the clearing. The index is plaintext, so it lives and
//! dies with the unlock: it is emptied when the vault closes, and emptying it overwrites the
//! characters rather than dropping the pointers.

use rusqlite::Connection;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::codec::FieldCodec;
use crate::error::DbError;
use crate::repositories::vault;

/// The most results one search hands back.
///
/// Fifty. A list somebody reads, not a list somebody scrolls: a search that answers with four
/// thousand rows has not narrowed anything, and the cost of drawing them is paid on the thread
/// that draws.
pub const MAX_RESULTS: usize = 50;

/// One title, opened, beside the entry it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The entry the title belongs to.
    pub id: Uuid,
    /// The title itself, which clears itself when it is dropped.
    pub title: Zeroizing<String>,
}

/// The titles of every live entry, in the clear, for as long as the vault is open.
///
/// Holds the folded form beside the original: folding on every keystroke of every search would
/// be the same work repeated, and the folded form is no more revealing than the title it came
/// from. Both clear themselves when the index is emptied.
#[derive(Debug)]
pub struct TitleIndex {
    entries: Vec<Indexed>,
    complete: bool,
}

/// One row of the index.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Indexed {
    id: Uuid,
    title: Zeroizing<String>,
    folded: Zeroizing<String>,
}

impl Default for TitleIndex {
    fn default() -> Self {
        Self::empty()
    }
}

impl TitleIndex {
    /// An index holding nothing, which is what a locked vault has.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            entries: Vec::new(),
            complete: true,
        }
    }

    /// Opens every live title and keeps it.
    ///
    /// Run once, on the unlock, before anything asks to search. Replaces whatever the index held
    /// before, clearing it first, so rebuilding is also a way of emptying.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Sealed`] if a title does not open and [`DbError::Sqlite`] if the read
    /// fails. Either way the index is left empty rather than half filled, because a half filled
    /// index is a search that quietly does not find things.
    pub fn build(
        &mut self,
        connection: &Connection,
        codec: &FieldCodec<'_>,
    ) -> Result<(), DbError> {
        self.clear();

        let titles = vault::titles(connection, codec)?;
        self.entries = titles
            .entries
            .into_iter()
            .map(|(id, title)| Indexed {
                id,
                folded: Zeroizing::new(fold(&title)),
                title,
            })
            .collect();
        self.complete = titles.complete;

        Ok(())
    }

    /// Empties the index, overwriting every title it held.
    ///
    /// What the lock calls. [`Zeroizing`] clears each string as it is dropped, and the vector is
    /// replaced rather than truncated so its buffer is released with it.
    pub fn clear(&mut self) {
        self.entries = Vec::new();
        self.complete = true;
    }

    /// How many titles are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the index holds every live entry, or stopped at the ceiling.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// The entries whose title contains what was typed, at most [`MAX_RESULTS`] of them.
    ///
    /// Case and accents are ignored on both sides: somebody typing `hacienda` finds `Hacienda`,
    /// and somebody typing `telefono` finds `Teléfono`, which is the whole reason this is a
    /// comparison in Rust and not a `LIKE` in SQL.
    ///
    /// An empty needle matches nothing rather than everything. The question "show me the entries
    /// matching nothing in particular" is a list, not a search, and the list has its own command.
    #[must_use]
    pub fn matches(&self, needle: &str) -> Vec<Match> {
        let needle = Zeroizing::new(fold(needle.trim()));
        if needle.is_empty() {
            return Vec::new();
        }

        self.entries
            .iter()
            .filter(|entry| entry.folded.contains(needle.as_str()))
            .take(MAX_RESULTS)
            .map(|entry| Match {
                id: entry.id,
                title: entry.title.clone(),
            })
            .collect()
    }
}

/// The form two titles are compared in.
///
/// Lower case, and with the accents of the Latin alphabet removed. Deliberately not a full
/// Unicode normalisation: that needs a table this workspace would have to take a dependency for,
/// and the alphabet this application is written for is covered by the letters listed here. A
/// title in a script this misses is still found by typing it as it is written.
fn fold(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| match character {
            'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};
    use cairn_domain::Hlc;

    use super::{MAX_RESULTS, TitleIndex, fold};
    use crate::codec::FieldCodec;
    use crate::device::DeviceId;
    use crate::migrations;
    use crate::open::Database;
    use crate::repositories::vault::{self, NewEntry};
    use crate::test_support::Scratch;

    const NOW_US: i64 = 1_700_000_000_000_000;

    fn an_open_vault() -> UnlockedVault {
        let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");
        vault
    }

    fn a_database(scratch: &Scratch, vault: &UnlockedVault) -> Database {
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        database
    }

    fn at(step: u64) -> Hlc {
        Hlc::new(step, 0, [1; 6])
    }

    /// Writes one entry with nothing in it but a title, which is all the index reads.
    fn write(database: &Database, vault: &UnlockedVault, step: u64, title: &str) {
        database
            .with(|connection| {
                let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
                vault::create_entry(
                    connection,
                    &codec,
                    DeviceId::from_bytes([7; 16]),
                    at(step),
                    NOW_US,
                    NewEntry {
                        title,
                        username: None,
                        password: None,
                        notes: None,
                        folder_id: None,
                        favorite: false,
                    },
                    cairn_domain::vault::EntryKind::Account,
                )
                .map(|_written| ())
            })
            .expect("the entry is written");
    }

    /// Builds the index against an open database, the way the unlock does.
    fn build(index: &mut TitleIndex, database: &Database, vault: &UnlockedVault) {
        database
            .with(|connection| {
                let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
                index.build(connection, &codec)
            })
            .expect("the index is built");
    }

    #[test]
    fn the_index_finds_a_title_sql_could_not_have_compared() {
        let scratch = Scratch::new("index-finds");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        write(&database, &vault, 1, "Banco Santander");
        write(&database, &vault, 2, "Correo del trabajo");

        let mut index = TitleIndex::empty();
        build(&mut index, &database, &vault);

        assert_eq!(index.len(), 2);
        assert!(index.is_complete());

        let found = index.matches("banco");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found.first().map(|hit| hit.title.as_str()),
            Some("Banco Santander")
        );
    }

    #[test]
    fn accents_and_case_are_ignored_on_both_sides() {
        let scratch = Scratch::new("index-accents");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        write(&database, &vault, 1, "Teléfono de casa");

        let mut index = TitleIndex::empty();
        build(&mut index, &database, &vault);

        assert_eq!(index.matches("telefono").len(), 1);
        assert_eq!(index.matches("TELÉFONO").len(), 1);
        assert_eq!(index.matches("fono de").len(), 1);
        assert!(index.matches("banco").is_empty());
    }

    /// The property the whole design rests on: nothing decrypted outlives the unlock.
    #[test]
    fn the_index_is_emptied_when_the_vault_is_locked() {
        let scratch = Scratch::new("index-lock");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        write(&database, &vault, 1, "Banco Santander");

        let mut index = TitleIndex::empty();
        build(&mut index, &database, &vault);
        assert_eq!(index.matches("banco").len(), 1);

        // What closing the vault does.
        index.clear();

        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert!(
            index.matches("banco").is_empty(),
            "a title survived the lock and is still findable"
        );
    }

    #[test]
    fn a_deleted_entry_leaves_the_index_on_the_next_build() {
        let scratch = Scratch::new("index-deleted");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        write(&database, &vault, 1, "Banco Santander");

        let mut index = TitleIndex::empty();
        build(&mut index, &database, &vault);
        let found = index.matches("banco");
        let id = found.first().map(|hit| hit.id).expect("one match");

        database
            .with(|connection| vault::delete_entry(connection, at(2), NOW_US, id))
            .expect("the entry is deleted");

        build(&mut index, &database, &vault);

        assert!(
            index.is_empty(),
            "a tombstone is still in the index somebody searches"
        );
    }

    #[test]
    fn an_empty_search_matches_nothing_rather_than_everything() {
        let scratch = Scratch::new("index-empty-needle");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        write(&database, &vault, 1, "Banco Santander");

        let mut index = TitleIndex::empty();
        build(&mut index, &database, &vault);

        assert!(index.matches("").is_empty());
        assert!(index.matches("   ").is_empty());
    }

    #[test]
    fn a_search_that_matches_everything_still_stops_at_the_ceiling() {
        let scratch = Scratch::new("index-ceiling");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        for number in 0..(MAX_RESULTS + 5) {
            write(
                &database,
                &vault,
                number as u64 + 1,
                &format!("Cuenta {number}"),
            );
        }

        let mut index = TitleIndex::empty();
        build(&mut index, &database, &vault);

        assert_eq!(index.len(), MAX_RESULTS + 5);
        assert_eq!(index.matches("cuenta").len(), MAX_RESULTS);
    }

    #[test]
    fn folding_leaves_letters_this_alphabet_does_not_use_alone() {
        assert_eq!(fold("Teléfono"), "telefono");
        assert_eq!(fold("Ñandú"), "ñandu");
        assert_eq!(fold("ÇEDILLA"), "cedilla");
    }
}
