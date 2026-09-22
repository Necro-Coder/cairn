//! The only way the vault can be searched: what is searchable about every entry, held in memory
//! while the vault is open.
//!
//! Every readable value in that module is encrypted, so SQL cannot compare them, cannot sort them
//! and cannot match a prefix. What it can do is hand over the ciphertext, and the key that opens
//! it is already in this process for as long as the vault is unlocked. So the searchable values
//! are opened once when the vault opens, kept in a list, and searched by walking it.
//!
//! Walking a list is the point rather than a compromise. A search index written to disk is a
//! second copy of every title, and a copy that is not encrypted is exactly the thing the vault
//! exists to prevent; a copy that is encrypted cannot be searched, which puts it back here. A few
//! thousand entries is a few hundred kilobytes and a comparison that finishes before the next
//! frame, and the ceiling in [`vault::MAX_TITLES`] is what keeps that sentence true for a file
//! this program did not write.
//!
//! Three fields are indexed and the choice of which is the whole design. Title, user name and
//! address, because what somebody remembers about an account is often not what they called it but
//! which address they signed up with. The notes are left out, and that is where the line is: they
//! are the longest and the most revealing thing an entry holds, and keeping every one of them
//! open so that a search could look inside them would multiply what a dump of this process shows.
//! The password and the value of a custom field are not indexed for reasons that need no sentence.
//!
//! What matters as much as the search is the clearing. The index is plaintext, so it lives and
//! dies with the unlock: it is emptied when the vault closes, and emptying it overwrites the
//! characters rather than dropping the pointers.

use cairn_domain::vault::MAX_URL_CHARS;
use rusqlite::Connection;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::codec::FieldCodec;
use crate::error::DbError;
use crate::repositories::vault;

/// The most results one page holds.
///
/// Fifty. A list somebody reads, not a list somebody scrolls: a search that answers with four
/// thousand rows has not narrowed anything, and the cost of drawing them is paid on the thread
/// that draws. What was missing was a way to ask for the next fifty, which is what `page` is.
pub const MAX_RESULTS: usize = 50;

/// One entry as the index knows it: everything that can be searched, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Searchable {
    /// The entry it describes.
    pub id: Uuid,
    /// What the entry is called.
    pub title: Zeroizing<String>,
    /// The user name, if it has one.
    pub username: Option<Zeroizing<String>>,
    /// Every address of the entry. Empty for a note.
    pub urls: Vec<Zeroizing<String>>,
}

/// Which field answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matched {
    /// The title did.
    Title,
    /// The user name did.
    Username,
    /// One of the addresses did.
    Url,
}

/// What matched, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The entry that matched.
    pub id: Uuid,
    /// Its title, which is what the list draws.
    pub title: Zeroizing<String>,
    /// Which of the three fields the needle was found in, for the interface to say so.
    pub matched: Matched,
}

/// One page of an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Results {
    /// The matches on the page that was asked for.
    pub hits: Vec<Match>,
    /// How many matched in total, across every page.
    pub total: usize,
    /// `false` if the index does not hold every entry in the file.
    pub complete: bool,
}

/// Everything searchable about every live entry, in the clear, for as long as the vault is open.
///
/// Holds the folded form beside the original: folding on every keystroke of every search would be
/// the same work repeated, and the folded form is no more revealing than what it came from. All
/// of it clears itself when the index is emptied.
#[derive(Debug)]
pub struct SearchIndex {
    entries: Vec<Indexed>,
    complete: bool,
}

/// One row of the index: what it holds, and the form it is compared in.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Indexed {
    id: Uuid,
    title: Zeroizing<String>,
    folded_title: Zeroizing<String>,
    folded_username: Option<Zeroizing<String>>,
    folded_urls: Vec<Zeroizing<String>>,
}

impl Indexed {
    /// Folds one entry into the form the index compares.
    fn of(entry: Searchable) -> Self {
        Self {
            id: entry.id,
            folded_title: Zeroizing::new(fold(&entry.title)),
            folded_username: entry
                .username
                .as_deref()
                .map(|name| Zeroizing::new(fold(name))),
            folded_urls: entry
                .urls
                .iter()
                .map(|url| Zeroizing::new(fold(url)))
                .collect(),
            title: entry.title,
        }
    }

    /// Which field the needle is in, at the highest priority that answers.
    ///
    /// Title first, then user name, then address, so an entry that matches in two places appears
    /// once and appears where somebody would look for it.
    fn matching(&self, needle: &str) -> Option<Matched> {
        if self.folded_title.contains(needle) {
            return Some(Matched::Title);
        }
        if self
            .folded_username
            .as_deref()
            .is_some_and(|name| name.contains(needle))
        {
            return Some(Matched::Username);
        }
        self.folded_urls
            .iter()
            .any(|url| url.contains(needle))
            .then_some(Matched::Url)
    }
}

impl Default for SearchIndex {
    fn default() -> Self {
        Self::empty()
    }
}

impl SearchIndex {
    /// An index holding nothing, which is what a locked vault has.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            entries: Vec::new(),
            complete: true,
        }
    }

    /// Opens everything searchable and keeps it. Run once, on the unlock.
    ///
    /// Replaces whatever the index held before, clearing it first, so rebuilding is also a way of
    /// emptying.
    ///
    /// # Errors
    ///
    /// [`DbError::Sealed`] if a value does not open, [`DbError::Sqlite`] if a read fails. Either
    /// way the index is left empty rather than half filled, because a half filled index is a
    /// search that quietly does not find things.
    pub fn build(
        &mut self,
        connection: &Connection,
        codec: &FieldCodec<'_>,
    ) -> Result<(), DbError> {
        self.clear();

        let found = vault::searchable(connection, codec)?;
        self.entries = found.entries.into_iter().map(Indexed::of).collect();
        self.complete = found.complete;

        Ok(())
    }

    /// Empties the index, overwriting everything it held.
    ///
    /// What the lock calls. [`Zeroizing`] clears each string as it is dropped, and the vector is
    /// replaced rather than truncated so its buffer is released with it.
    pub fn clear(&mut self) {
        self.entries = Vec::new();
        self.complete = true;
    }

    /// How many entries are held.
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

    /// One page of the entries matching what was typed.
    ///
    /// `page` is zero based. A page past the end is an empty page, not an error.
    ///
    /// Case and accents are ignored on both sides: somebody typing `hacienda` finds `Hacienda`,
    /// and somebody typing `telefono` finds `Teléfono`, which is the whole reason this is a
    /// comparison in Rust and not a `LIKE` in SQL. It is a substring match and nothing more: no
    /// edit distance, no fuzzy score, no threshold, because every one of those turns "why did
    /// that not come up" into a question with no answer.
    ///
    /// An empty needle matches nothing rather than everything. The question "show me the entries
    /// matching nothing in particular" is a list, not a search, and the list has its own command.
    #[must_use]
    pub fn search(&self, needle: &str, page: u16) -> Results {
        let trimmed = needle.trim();
        // Bounded before it is folded. Nothing in the index is longer than the longest address,
        // so a needle past that cannot match anything, and folding a megabyte somebody pasted
        // into the search box on every keystroke is a frozen window written in one line.
        if trimmed.is_empty() || trimmed.chars().count() > MAX_URL_CHARS {
            return Results {
                hits: Vec::new(),
                total: 0,
                complete: self.complete,
            };
        }

        let needle = Zeroizing::new(fold(trimmed));
        let mut found: Vec<(Matched, &Indexed)> = self
            .entries
            .iter()
            .filter_map(|entry| {
                entry
                    .matching(&needle)
                    .map(|where_it_is| (where_it_is, entry))
            })
            .collect();

        // Stable, and by where the needle was found first. Sorting by anything else, or not
        // sorting at all, would make page two a different set rather than the continuation of
        // page one.
        found.sort_by_key(|(where_it_is, _entry)| priority(*where_it_is));

        let total = found.len();
        let from = usize::from(page).saturating_mul(MAX_RESULTS);
        let hits = found
            .into_iter()
            .skip(from)
            .take(MAX_RESULTS)
            .map(|(matched, entry)| Match {
                id: entry.id,
                title: entry.title.clone(),
                matched,
            })
            .collect();

        Results {
            hits,
            total,
            complete: self.complete,
        }
    }

    /// Adds, replaces or removes one entry without rebuilding the whole index.
    ///
    /// What a write calls, so that creating an entry does not cost a full decrypt of the file.
    /// `None` removes it, which is what throwing something in the bin does.
    ///
    /// Replacing leaves the entry where it was and adding puts it at the end, so the order a
    /// search pages through does not reshuffle itself because somebody saved a form.
    pub fn upsert(&mut self, id: Uuid, entry: Option<Searchable>) {
        let at = self.entries.iter().position(|held| held.id == id);

        match (at, entry) {
            (Some(at), Some(entry)) => {
                if let Some(held) = self.entries.get_mut(at) {
                    *held = Indexed::of(entry);
                }
            }
            (Some(at), None) => {
                self.entries.remove(at);
            }
            (None, Some(entry)) => self.entries.push(Indexed::of(entry)),
            (None, None) => {}
        }
    }
}

/// Which group a match belongs to, lowest first.
const fn priority(matched: Matched) -> u8 {
    match matched {
        Matched::Title => 0,
        Matched::Username => 1,
        Matched::Url => 2,
    }
}

/// The form two values are compared in.
///
/// Lower case, and with the accents of the Latin alphabet removed. Deliberately not a full
/// Unicode normalisation: that needs a table this workspace would have to take a dependency for,
/// and the alphabet this application is written for is covered by the letters listed here. A
/// value in a script this misses is still found by typing it as it is written.
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
    use cairn_domain::vault::EntryKind;
    use uuid::Uuid;
    use zeroize::Zeroizing;

    use super::{MAX_RESULTS, Matched, SearchIndex, Searchable, fold};
    use crate::codec::FieldCodec;
    use crate::device::DeviceId;
    use crate::error::DbError;
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

    /// What one entry of these tests is made of.
    #[derive(Clone, Copy)]
    struct Written<'a> {
        title: &'a str,
        username: Option<&'a str>,
        notes: Option<&'a str>,
        urls: &'a [&'a str],
    }

    impl<'a> Written<'a> {
        const fn titled(title: &'a str) -> Self {
            Self {
                title,
                username: None,
                notes: None,
                urls: &[],
            }
        }
    }

    /// Writes one entry with everything the index reads, and some of what it must not.
    fn write(database: &Database, vault: &UnlockedVault, step: u64, entry: Written<'_>) -> Uuid {
        database
            .with(|connection| {
                let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
                let written = vault::create_entry(
                    connection,
                    &codec,
                    DeviceId::from_bytes([7; 16]),
                    at(step),
                    NOW_US,
                    NewEntry {
                        title: entry.title,
                        username: entry.username,
                        password: Some("contraseña-secreta"),
                        notes: entry.notes,
                        folder_id: None,
                        favorite: false,
                    },
                    EntryKind::Account,
                )?;
                if !entry.urls.is_empty() {
                    vault::replace_urls(
                        connection,
                        &codec,
                        DeviceId::from_bytes([7; 16]),
                        at(step + 1),
                        NOW_US,
                        written.id,
                        entry.urls,
                    )?;
                }

                Ok(written.id)
            })
            .expect("the entry is written")
    }

    /// Builds the index against an open database, the way the unlock does.
    fn build(index: &mut SearchIndex, database: &Database, vault: &UnlockedVault) {
        database
            .with(|connection| {
                let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
                index.build(connection, &codec)
            })
            .expect("the index is built");
    }

    /// An index over the entries given, built through the database the way the unlock does.
    fn an_index(
        label: &str,
        entries: &[Written<'_>],
    ) -> (Scratch, UnlockedVault, Database, SearchIndex) {
        let scratch = Scratch::new(label);
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        for (number, entry) in entries.iter().enumerate() {
            write(
                &database,
                &vault,
                u64::try_from(number).unwrap_or(0) * 2 + 1,
                *entry,
            );
        }

        let mut index = SearchIndex::empty();
        build(&mut index, &database, &vault);

        (scratch, vault, database, index)
    }

    #[test]
    fn a_title_a_user_name_and_an_address_all_answer_and_say_which_did() {
        let (_scratch, _vault, database, index) = an_index(
            "index-three-fields",
            &[
                Written::titled("Nómina"),
                Written {
                    title: "Correo",
                    username: Some("juan@correo.es"),
                    notes: None,
                    urls: &[],
                },
                Written {
                    title: "Entidad",
                    username: None,
                    notes: None,
                    urls: &["https://banco.es/login"],
                },
            ],
        );

        assert_eq!(index.search("nomina", 0).total, 1);
        assert_eq!(
            index
                .search("nomina", 0)
                .hits
                .first()
                .map(|hit| hit.matched),
            Some(Matched::Title)
        );
        assert_eq!(
            index.search("HACIENDA", 0).total,
            0,
            "something matched a word no entry carries"
        );
        assert_eq!(
            index.search("juan@", 0).hits.first().map(|hit| hit.matched),
            Some(Matched::Username)
        );
        assert_eq!(
            index
                .search("banco.es", 0)
                .hits
                .first()
                .map(|hit| hit.matched),
            Some(Matched::Url)
        );

        database.close().expect("the connection closes");
    }

    #[test]
    fn case_and_accents_are_ignored_on_both_sides() {
        let (_scratch, _vault, database, index) =
            an_index("index-accents", &[Written::titled("Teléfono de casa")]);

        assert_eq!(index.search("telefono", 0).total, 1);
        assert_eq!(index.search("TELÉFONO", 0).total, 1);
        assert_eq!(index.search("fono de", 0).total, 1);
        assert_eq!(index.search("banco", 0).total, 0);

        database.close().expect("the connection closes");
    }

    #[test]
    fn an_entry_that_matches_twice_appears_once_and_at_the_higher_priority() {
        let (_scratch, _vault, database, index) = an_index(
            "index-priority",
            &[Written {
                title: "Hacienda",
                username: Some("hacienda@correo.es"),
                notes: None,
                urls: &["https://hacienda.example"],
            }],
        );

        let found = index.search("hacienda", 0);
        assert_eq!(found.total, 1, "one entry answered three times");
        assert_eq!(
            found.hits.first().map(|hit| hit.matched),
            Some(Matched::Title)
        );

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_notes_are_not_searched() {
        // The line the questionnaire drew. Indexing them would mean every note of every entry is
        // open in this process from the unlock until the lock.
        let (_scratch, _vault, database, index) = an_index(
            "index-notes",
            &[Written {
                title: "Un sitio",
                username: None,
                notes: Some("la aguja está aquí dentro"),
                urls: &[],
            }],
        );

        assert_eq!(index.search("aguja", 0).total, 0);
        assert_eq!(
            index.search("contraseña-secreta", 0).total,
            0,
            "the password is in the index"
        );
        assert_eq!(index.search("sitio", 0).total, 1);

        database.close().expect("the connection closes");
    }

    #[test]
    fn an_empty_or_enormous_needle_matches_nothing_rather_than_everything() {
        let (_scratch, _vault, database, index) =
            an_index("index-needle", &[Written::titled("Banco Santander")]);

        assert_eq!(index.search("", 0).total, 0);
        assert_eq!(index.search("   ", 0).total, 0);

        let enormous: String = std::iter::repeat_n('a', 10_000).collect();
        assert_eq!(index.search(&enormous, 0).total, 0);

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_pages_of_one_search_are_the_list_split_up_and_not_three_different_lists() {
        let entries: Vec<String> = (0..120).map(|number| format!("Cuenta {number}")).collect();
        let written: Vec<Written<'_>> = entries
            .iter()
            .map(|title| Written::titled(title.as_str()))
            .collect();
        let (_scratch, _vault, database, index) = an_index("index-pages", &written);

        let pages: Vec<Vec<Uuid>> = (0..3_u16)
            .map(|page| {
                index
                    .search("cuenta", page)
                    .hits
                    .iter()
                    .map(|hit| hit.id)
                    .collect()
            })
            .collect();

        assert_eq!(
            pages.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![MAX_RESULTS, MAX_RESULTS, 20]
        );
        assert_eq!(index.search("cuenta", 0).total, 120);

        let walked: Vec<Uuid> = pages.concat();
        let mut unique = walked.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 120, "a page repeated or skipped something");

        // The concatenation of the pages is the order a single unpaged list would have had.
        let straight: Vec<Uuid> = index
            .search("cuenta", 0)
            .hits
            .iter()
            .map(|hit| hit.id)
            .collect();
        assert_eq!(
            walked.get(..MAX_RESULTS).unwrap_or_default(),
            straight.as_slice()
        );

        let past_the_end = index.search("cuenta", 9);
        assert!(past_the_end.hits.is_empty());
        assert_eq!(past_the_end.total, 120);

        database.close().expect("the connection closes");
    }

    #[test]
    fn something_in_the_bin_is_not_in_the_index() {
        let scratch = Scratch::new("index-bin");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let id = write(&database, &vault, 1, Written::titled("Banco Santander"));

        let mut index = SearchIndex::empty();
        build(&mut index, &database, &vault);
        assert_eq!(index.search("banco", 0).total, 1);

        database
            .with(|connection| {
                let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
                vault::set_trashed(connection, &codec, at(50), NOW_US + 1, id, true)
            })
            .expect("the entry goes in the bin");

        build(&mut index, &database, &vault);
        assert_eq!(index.search("banco", 0).total, 0);

        database.close().expect("the connection closes");
    }

    #[test]
    fn one_entry_can_be_added_replaced_or_taken_out_without_rebuilding_the_rest() {
        let (_scratch, _vault, database, mut index) = an_index(
            "index-upsert",
            &[Written::titled("Banco"), Written::titled("Correo")],
        );
        let id = index
            .search("banco", 0)
            .hits
            .first()
            .map(|hit| hit.id)
            .expect("one match");

        index.upsert(
            id,
            Some(Searchable {
                id,
                title: Zeroizing::new("Caja de ahorros".to_owned()),
                username: None,
                urls: Vec::new(),
            }),
        );
        assert_eq!(index.search("banco", 0).total, 0);
        assert_eq!(index.search("ahorros", 0).total, 1);
        assert_eq!(
            index.len(),
            2,
            "replacing one entry changed how many there are"
        );

        index.upsert(id, None);
        assert_eq!(index.search("ahorros", 0).total, 0);
        assert_eq!(index.len(), 1);

        let fresh = Uuid::from_bytes([8; 16]);
        index.upsert(
            fresh,
            Some(Searchable {
                id: fresh,
                title: Zeroizing::new("Nueva".to_owned()),
                username: None,
                urls: Vec::new(),
            }),
        );
        assert_eq!(index.search("nueva", 0).total, 1);

        database.close().expect("the connection closes");
    }

    /// The property the whole design rests on: nothing decrypted outlives the unlock.
    #[test]
    fn the_index_is_emptied_when_the_vault_is_locked() {
        let (_scratch, _vault, database, mut index) =
            an_index("index-lock", &[Written::titled("Banco Santander")]);
        assert_eq!(index.search("banco", 0).total, 1);

        index.clear();

        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert_eq!(
            index.search("banco", 0).total,
            0,
            "a title survived the lock and is still findable"
        );

        database.close().expect("the connection closes");
    }

    #[test]
    fn an_entry_with_nothing_but_a_title_is_still_indexed() {
        let (_scratch, _vault, database, index) =
            an_index("index-bare", &[Written::titled("Sólo un nombre")]);

        assert_eq!(index.search("nombre", 0).total, 1);
        assert_eq!(index.len(), 1);

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_value_that_does_not_open_leaves_the_index_empty_rather_than_half_built() {
        let scratch = Scratch::new("index-damaged");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        write(&database, &vault, 1, Written::titled("Banco"));
        write(&database, &vault, 3, Written::titled("Correo"));

        // One row's ciphertext replaced with something that is not ours. A half filled index is a
        // search that quietly does not find things, which is worse than one that says so.
        database
            .with(|connection| {
                connection.execute(
                    "UPDATE vault_entries SET title = x'00112233' WHERE rowid = 1",
                    [],
                )?;
                Ok(())
            })
            .expect("the row can be damaged");

        let mut index = SearchIndex::empty();
        let built = database.with(|connection| {
            let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
            index.build(connection, &codec)
        });

        assert!(matches!(built, Err(DbError::Sealed(_))));
        assert!(index.is_empty(), "the index was left half built");

        database.close().expect("the connection closes");
    }

    #[test]
    #[ignore = "writes a hundred thousand encrypted rows, which is minutes rather than seconds; run it on demand"]
    fn a_file_with_more_entries_than_the_ceiling_says_the_index_is_not_all_of_them() {
        // The ceiling is what stops the cost and the memory of an unlock being decided by the
        // size of the file rather than by this program. A search over an index that silently
        // holds nine tenths of the entries is worse than one that says so.
        let scratch = Scratch::new("index-ceiling");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);

        database
            .with(|connection| {
                let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
                for number in 0..=u64::try_from(vault::MAX_TITLES).unwrap_or(0) {
                    vault::create_entry(
                        connection,
                        &codec,
                        DeviceId::from_bytes([7; 16]),
                        at(number + 1),
                        NOW_US,
                        NewEntry {
                            title: "Una cuenta",
                            username: None,
                            password: None,
                            notes: None,
                            folder_id: None,
                            favorite: false,
                        },
                        EntryKind::Account,
                    )?;
                }
                Ok(())
            })
            .expect("the entries are written");

        let mut index = SearchIndex::empty();
        build(&mut index, &database, &vault);

        assert_eq!(index.len(), vault::MAX_TITLES);
        assert!(!index.is_complete());
        assert!(!index.search("cuenta", 0).complete);

        database.close().expect("the connection closes");
    }

    #[test]
    fn folding_leaves_letters_this_alphabet_does_not_use_alone() {
        assert_eq!(fold("Teléfono"), "telefono");
        assert_eq!(fold("Ñandú"), "ñandu");
        assert_eq!(fold("ÇEDILLA"), "cedilla");
    }
}
