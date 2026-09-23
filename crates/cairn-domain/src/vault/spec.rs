//! What an entry may hold, checked once, before anything touches the database.
//!
//! The shape of this module is decided by one thing: whoever is filling a form wants to be told
//! about the four things that are wrong at the same time, not four times about one thing each. So
//! nothing here stops at the first problem. [`EntryDraft::validate`] walks every field, collects
//! every complaint, and answers with all of them or with a value that says the draft is sound.
//!
//! That value is [`ValidEntry`], and it exists so the check happens exactly once. It wraps a
//! draft nobody outside this module can put inside it, which means a repository that takes one
//! does not re-measure anything: holding a `ValidEntry` is holding something already judged. A
//! second check further down would not be defence in depth, it would be a second opinion, and the
//! day the two disagree is the day a row is written that no reader believes.
//!
//! What is deliberately not here: any notion of what a URL is. See [`EntryDraft::validate`].

use uuid::Uuid;

/// What an entry is, which is the one thing about it the database may read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A site, a user name and a password.
    Account,
    /// Text and nothing else. The same row with the account half left empty.
    Note,
}

/// What a custom field holds, and how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Shown as it is.
    Text,
    /// Hidden until somebody asks for it, like the password.
    Secret,
}

/// The longest a title may be, in characters.
pub const MAX_TITLE_CHARS: usize = 256;

/// The longest a user name may be, in characters.
pub const MAX_USERNAME_CHARS: usize = 256;

/// The longest a password may be, in bytes.
///
/// Bytes rather than characters, unlike the two above, and the difference is not an oversight. A
/// title is read, so it is measured by what somebody sees; a password is stored and compared, so
/// it is measured by what it costs. A kibibyte is far past any passphrase and far short of what
/// a paste could put in the box.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// The longest the notes of an entry may be, in bytes.
pub const MAX_NOTES_BYTES: usize = 64 * 1024;

/// The most custom fields one entry may have.
pub const MAX_FIELDS: usize = 256;

/// The longest the label of a custom field may be, in characters.
pub const MAX_FIELD_LABEL_CHARS: usize = 256;

/// The longest the value of a custom field may be, in bytes.
pub const MAX_FIELD_VALUE_BYTES: usize = 64 * 1024;

/// The most addresses one entry may have.
pub const MAX_URLS: usize = 32;

/// The longest one address may be, in characters.
pub const MAX_URL_CHARS: usize = 2048;

/// The longest the name of a folder may be, in characters.
pub const MAX_FOLDER_NAME_CHARS: usize = 128;

/// One custom field, as it arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftField {
    /// The row this field already is, or `None` for one somebody has just added.
    ///
    /// Nothing here judges it. It is not something a person types, and there is no shape it could
    /// be wrong in that this module would be able to see. It travels through the check so that the
    /// repository can pair what arrives with what is stored **by identifier**, which is the only
    /// pairing that survives somebody reordering the boxes or deleting one in the middle: paired
    /// by position, a form that dropped the first field would hand the second field's row the
    /// first one's secret.
    pub id: Option<Uuid>,
    /// What the field is called, as it is drawn beside the value.
    pub label: String,
    /// What it holds.
    pub value: String,
    /// Whether the interface hides it until somebody asks.
    pub kind: FieldKind,
}

/// An entry as somebody typed it, before anything has been decided about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryDraft {
    /// Whether this is an account or a note.
    pub kind: EntryKind,
    /// What the entry is called. The one thing every entry must have.
    pub title: String,
    /// The user name, if there is one.
    pub username: Option<String>,
    /// The password, if there is one.
    pub password: Option<String>,
    /// The notes, if there are any.
    pub notes: Option<String>,
    /// Every address, in the order somebody put them in.
    pub urls: Vec<String>,
    /// Every custom field, in order.
    pub fields: Vec<DraftField>,
    /// The folder it goes in, or `None` for one at the root.
    pub folder_id: Option<Uuid>,
    /// Whether somebody marked it as a favourite.
    pub favorite: bool,
}

/// Which field was wrong, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    /// The field, named the way the interface names it, so the message lands on the right box.
    pub field: &'static str,
    /// Which position in a list, for the fields that are lists.
    ///
    /// The position in the list as it arrived, blank rows included, because that is the list the
    /// interface drew and the boxes it has to put a message under. Counting the positions after
    /// the blank rows were discarded would point at the wrong box the moment somebody leaves one
    /// empty above the one they got wrong.
    pub index: Option<usize>,
    /// What was wrong with it.
    pub problem: Problem,
}

impl FieldError {
    /// One problem about a field that is not a list.
    const fn at(field: &'static str, problem: Problem) -> Self {
        Self {
            field,
            index: None,
            problem,
        }
    }

    /// One problem about one position of a list.
    const fn at_index(field: &'static str, index: usize, problem: Problem) -> Self {
        Self {
            field,
            index: Some(index),
            problem,
        }
    }
}

/// What was wrong with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// Required and not there, or there and empty once trimmed.
    Missing,
    /// Longer than it may be.
    TooLong {
        /// The most it may be, in whatever unit that field is measured in.
        limit: usize,
        /// What it actually is, in the same unit.
        actual: usize,
    },
    /// More items than the list may hold.
    TooMany {
        /// The most the list may hold.
        limit: usize,
        /// How many arrived.
        actual: usize,
    },
    /// Contains a character no field of this application accepts.
    Control,
}

/// An entry that has passed validation, and therefore one the repository may write.
///
/// The only way to build one is [`EntryDraft::validate`], so a function taking this type does
/// not re-check anything: whatever holds a `ValidEntry` holds something already judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidEntry(EntryDraft);

impl ValidEntry {
    /// The draft inside, trimmed and with its blank rows already discarded.
    #[must_use]
    pub const fn draft(&self) -> &EntryDraft {
        &self.0
    }

    /// Takes the draft out, for a caller that owns what it is about to write.
    #[must_use]
    pub fn into_draft(self) -> EntryDraft {
        self.0
    }
}

impl EntryDraft {
    /// Trims, checks and answers with every problem at once.
    ///
    /// Text is trimmed at both ends before it is measured, so a title of three hundred spaces is
    /// empty rather than too long. An optional field that is empty once trimmed becomes absent:
    /// a box somebody cleared and a box somebody never filled are the same fact about an entry,
    /// and keeping them apart here would push the difference on to every screen that draws one.
    ///
    /// A blank address, and a custom field whose label and value are both blank, are discarded
    /// without complaint. They are the empty row a form with an "add another" button leaves
    /// behind, and refusing them would mean somebody cannot save until they have tidied up after
    /// a button they did not press.
    ///
    /// Nothing here decides what a URL is. It is not parsed, not normalised and not completed
    /// with a scheme. What somebody wrote there is text they wrote for themselves to read, and a
    /// URL validator would refuse `miservidor:8080` while accepting `javascript:alert(1)`, which
    /// is the wrong answer in both directions at once. Whoever draws it, draws it as text.
    ///
    /// [`EntryKind`] changes nothing about what is accepted. A note carrying a user name and a
    /// password is stored as it stands and the interface decides what it shows; a restriction
    /// here would mean changing the kind of an entry could lose what was typed into it.
    ///
    /// # Errors
    ///
    /// Returns every [`FieldError`] found, never the first one: somebody filling a form wants to
    /// be told about the four things at once, not four times about one thing each.
    pub fn validate(self) -> Result<ValidEntry, Vec<FieldError>> {
        let mut problems = Vec::new();

        let title = self.title.trim().to_owned();
        if title.is_empty() {
            problems.push(FieldError::at("title", Problem::Missing));
        } else {
            check_chars(
                "title",
                &title,
                MAX_TITLE_CHARS,
                Allows::Nothing,
                &mut problems,
            );
        }

        let username = trimmed(self.username.as_deref());
        if let Some(value) = username.as_deref() {
            check_chars(
                "username",
                value,
                MAX_USERNAME_CHARS,
                Allows::Nothing,
                &mut problems,
            );
        }

        let password = trimmed(self.password.as_deref());
        if let Some(value) = password.as_deref() {
            check_bytes(
                "password",
                value,
                MAX_PASSWORD_BYTES,
                Allows::Nothing,
                &mut problems,
            );
        }

        let notes = trimmed(self.notes.as_deref());
        if let Some(value) = notes.as_deref() {
            check_bytes(
                "notes",
                value,
                MAX_NOTES_BYTES,
                Allows::LinesAndTabs,
                &mut problems,
            );
        }

        let urls = self.checked_urls(&mut problems);
        let fields = self.checked_fields(&mut problems);

        if problems.is_empty() {
            Ok(ValidEntry(Self {
                kind: self.kind,
                title,
                username,
                password,
                notes,
                urls,
                fields,
                folder_id: self.folder_id,
                favorite: self.favorite,
            }))
        } else {
            Err(problems)
        }
    }

    /// The addresses that survive trimming, with everything wrong about them recorded.
    fn checked_urls(&self, problems: &mut Vec<FieldError>) -> Vec<String> {
        let mut kept = Vec::new();

        for (index, url) in self.urls.iter().enumerate() {
            let url = url.trim();
            if url.is_empty() {
                continue;
            }

            check_chars_at("urls", index, url, MAX_URL_CHARS, Allows::Nothing, problems);
            kept.push(url.to_owned());
        }

        // Counted after the blank rows have gone, because a form that left ten of them behind
        // has not asked for forty addresses, it has asked for thirty.
        if kept.len() > MAX_URLS {
            problems.push(FieldError::at(
                "urls",
                Problem::TooMany {
                    limit: MAX_URLS,
                    actual: kept.len(),
                },
            ));
        }

        kept
    }

    /// The custom fields that survive trimming, with everything wrong about them recorded.
    fn checked_fields(&self, problems: &mut Vec<FieldError>) -> Vec<DraftField> {
        let mut kept = Vec::new();

        for (index, field) in self.fields.iter().enumerate() {
            let label = field.label.trim();
            let value = field.value.trim();
            if label.is_empty() && value.is_empty() {
                continue;
            }

            if label.is_empty() {
                // A value with nothing naming it. The other way round is fine: a label with an
                // empty value is a field somebody has made room for and not filled in yet.
                problems.push(FieldError::at_index("fields", index, Problem::Missing));
            } else {
                check_chars_at(
                    "fields",
                    index,
                    label,
                    MAX_FIELD_LABEL_CHARS,
                    Allows::Nothing,
                    problems,
                );
            }

            check_bytes_at(
                "fields",
                index,
                value,
                MAX_FIELD_VALUE_BYTES,
                Allows::LinesAndTabs,
                problems,
            );

            kept.push(DraftField {
                id: field.id,
                label: label.to_owned(),
                value: value.to_owned(),
                kind: field.kind,
            });
        }

        if kept.len() > MAX_FIELDS {
            problems.push(FieldError::at(
                "fields",
                Problem::TooMany {
                    limit: MAX_FIELDS,
                    actual: kept.len(),
                },
            ));
        }

        kept
    }
}

/// Trims and checks a folder name on its own, which is the only thing a folder has.
///
/// # Errors
///
/// Returns the problem with it.
pub fn validate_folder_name(name: &str) -> Result<String, FieldError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FieldError::at("name", Problem::Missing));
    }

    let mut problems = Vec::new();
    check_chars(
        "name",
        name,
        MAX_FOLDER_NAME_CHARS,
        Allows::Nothing,
        &mut problems,
    );

    // One name, so at most one complaint reaches the caller and the first is the one that is
    // true: the character check runs before the length check inside `check_chars`.
    problems
        .into_iter()
        .next()
        .map_or_else(|| Ok(name.to_owned()), Err)
}

/// Which control characters a field tolerates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Allows {
    /// None at all. A line break in a title breaks every list it appears in, and a carriage
    /// return in a label is an old way of making a log say something it does not say.
    Nothing,
    /// Line breaks and tabs, which is what a box somebody writes prose into is for.
    LinesAndTabs,
}

impl Allows {
    /// Whether this field accepts that character.
    const fn accepts(self, character: char) -> bool {
        if !character.is_control() {
            return true;
        }

        match self {
            Self::Nothing => false,
            Self::LinesAndTabs => matches!(character, '\n' | '\t'),
        }
    }
}

/// Checks a field measured in characters.
fn check_chars(
    field: &'static str,
    value: &str,
    limit: usize,
    allows: Allows,
    problems: &mut Vec<FieldError>,
) {
    if let Some(problem) = char_problem(value, limit, allows) {
        problems.push(FieldError::at(field, problem));
    }
}

/// Checks one position of a list measured in characters.
fn check_chars_at(
    field: &'static str,
    index: usize,
    value: &str,
    limit: usize,
    allows: Allows,
    problems: &mut Vec<FieldError>,
) {
    if let Some(problem) = char_problem(value, limit, allows) {
        problems.push(FieldError::at_index(field, index, problem));
    }
}

/// Checks a field measured in bytes.
fn check_bytes(
    field: &'static str,
    value: &str,
    limit: usize,
    allows: Allows,
    problems: &mut Vec<FieldError>,
) {
    if let Some(problem) = byte_problem(value, limit, allows) {
        problems.push(FieldError::at(field, problem));
    }
}

/// Checks one position of a list measured in bytes.
fn check_bytes_at(
    field: &'static str,
    index: usize,
    value: &str,
    limit: usize,
    allows: Allows,
    problems: &mut Vec<FieldError>,
) {
    if let Some(problem) = byte_problem(value, limit, allows) {
        problems.push(FieldError::at_index(field, index, problem));
    }
}

/// What is wrong with a value measured in characters, if anything.
///
/// The character check comes first, so that a title pasted out of a spreadsheet is reported as
/// carrying something it may not rather than as being long.
fn char_problem(value: &str, limit: usize, allows: Allows) -> Option<Problem> {
    if value.chars().any(|character| !allows.accepts(character)) {
        return Some(Problem::Control);
    }

    let actual = value.chars().count();
    (actual > limit).then_some(Problem::TooLong { limit, actual })
}

/// What is wrong with a value measured in bytes, if anything.
fn byte_problem(value: &str, limit: usize, allows: Allows) -> Option<Problem> {
    if value.chars().any(|character| !allows.accepts(character)) {
        return Some(Problem::Control);
    }

    let actual = value.len();
    (actual > limit).then_some(Problem::TooLong { limit, actual })
}

/// Trims an optional text, treating one that is empty afterwards as absent.
fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{
        DraftField, EntryDraft, EntryKind, FieldError, FieldKind, MAX_FIELD_VALUE_BYTES,
        MAX_FIELDS, MAX_FOLDER_NAME_CHARS, MAX_NOTES_BYTES, MAX_PASSWORD_BYTES, MAX_TITLE_CHARS,
        MAX_URLS, Problem, validate_folder_name,
    };

    /// The smallest draft that is acceptable: a title and nothing else.
    fn a_draft(kind: EntryKind) -> EntryDraft {
        EntryDraft {
            kind,
            title: "Banco".to_owned(),
            username: None,
            password: None,
            notes: None,
            urls: Vec::new(),
            fields: Vec::new(),
            folder_id: None,
            favorite: false,
        }
    }

    /// A text field made of one character repeated.
    fn repeated(character: char, times: usize) -> String {
        std::iter::repeat_n(character, times).collect()
    }

    /// The problems a draft is refused with, failing if it was accepted.
    fn refusal(draft: EntryDraft) -> Vec<FieldError> {
        match draft.validate() {
            Ok(valid) => panic!("the draft was accepted: {valid:?}"),
            Err(problems) => problems,
        }
    }

    /// The one problem about a field, failing if there is not exactly one.
    fn only_problem(draft: EntryDraft, field: &str) -> Problem {
        let problems = refusal(draft);
        let about: Vec<&FieldError> = problems
            .iter()
            .filter(|problem| problem.field == field)
            .collect();

        match about.as_slice() {
            [one] => one.problem,
            other => panic!("expected one problem about {field}, got {other:?}"),
        }
    }

    #[test]
    fn the_smallest_acceptable_draft_of_either_kind_is_accepted() {
        for kind in [EntryKind::Account, EntryKind::Note] {
            let valid = a_draft(kind).validate().expect("a title is enough");
            assert_eq!(valid.draft().kind, kind);
            assert_eq!(valid.draft().title, "Banco");
        }
    }

    #[test]
    fn a_title_is_measured_in_characters_and_the_limit_is_the_last_one_accepted() {
        let mut at_the_limit = a_draft(EntryKind::Account);
        at_the_limit.title = repeated('a', MAX_TITLE_CHARS);
        assert!(at_the_limit.validate().is_ok());

        let mut one_past = a_draft(EntryKind::Account);
        one_past.title = repeated('a', MAX_TITLE_CHARS + 1);
        assert_eq!(
            only_problem(one_past, "title"),
            Problem::TooLong {
                limit: MAX_TITLE_CHARS,
                actual: MAX_TITLE_CHARS + 1
            }
        );
    }

    #[test]
    fn a_title_of_nothing_but_spaces_is_missing_rather_than_long() {
        let mut draft = a_draft(EntryKind::Account);
        draft.title = repeated(' ', 300);

        assert_eq!(only_problem(draft, "title"), Problem::Missing);
    }

    #[test]
    fn a_line_break_is_refused_in_a_title_and_accepted_in_the_notes() {
        let mut titled = a_draft(EntryKind::Account);
        titled.title = "Banco\nSantander".to_owned();
        assert_eq!(only_problem(titled, "title"), Problem::Control);

        let mut noted = a_draft(EntryKind::Note);
        noted.notes = Some("primera línea\nsegunda\tcolumna".to_owned());
        assert!(noted.validate().is_ok());
    }

    #[test]
    fn a_password_is_measured_in_bytes_and_not_in_characters() {
        let mut at_the_limit = a_draft(EntryKind::Account);
        at_the_limit.password = Some(repeated('a', MAX_PASSWORD_BYTES));
        assert!(at_the_limit.validate().is_ok());

        let mut one_past = a_draft(EntryKind::Account);
        one_past.password = Some(repeated('a', MAX_PASSWORD_BYTES + 1));
        assert_eq!(
            only_problem(one_past, "password"),
            Problem::TooLong {
                limit: MAX_PASSWORD_BYTES,
                actual: MAX_PASSWORD_BYTES + 1
            }
        );

        // Four hundred characters, three bytes each. Well inside the limit if it were counted in
        // characters, and well past it because it is not.
        let mut multibyte = a_draft(EntryKind::Account);
        multibyte.password = Some(repeated('€', 400));
        assert!(matches!(
            only_problem(multibyte, "password"),
            Problem::TooLong { .. }
        ));
    }

    #[test]
    fn a_title_is_measured_in_characters_and_not_in_bytes() {
        // The mirror of the test above, and the reason the two limits are written in different
        // units on purpose: a title of accented letters is as long as it looks.
        let mut draft = a_draft(EntryKind::Account);
        draft.title = repeated('é', MAX_TITLE_CHARS);

        assert!(draft.validate().is_ok());
    }

    #[test]
    fn the_notes_stop_at_sixty_four_kibibytes_of_bytes() {
        let mut at_the_limit = a_draft(EntryKind::Note);
        at_the_limit.notes = Some(repeated('a', MAX_NOTES_BYTES));
        assert!(at_the_limit.validate().is_ok());

        let mut one_past = a_draft(EntryKind::Note);
        one_past.notes = Some(repeated('a', MAX_NOTES_BYTES + 1));
        assert!(matches!(
            only_problem(one_past, "notes"),
            Problem::TooLong { .. }
        ));
    }

    #[test]
    fn thirty_two_addresses_are_enough_and_thirty_three_are_too_many() {
        let mut at_the_limit = a_draft(EntryKind::Account);
        at_the_limit.urls = (0..MAX_URLS).map(|n| format!("banco{n}.es")).collect();
        assert!(at_the_limit.validate().is_ok());

        let mut one_past = a_draft(EntryKind::Account);
        one_past.urls = (0..=MAX_URLS).map(|n| format!("banco{n}.es")).collect();
        assert_eq!(
            only_problem(one_past, "urls"),
            Problem::TooMany {
                limit: MAX_URLS,
                actual: MAX_URLS + 1
            }
        );
    }

    #[test]
    fn a_blank_address_is_discarded_rather_than_refused() {
        let mut draft = a_draft(EntryKind::Account);
        draft.urls = (0..40)
            .map(|n| {
                if n % 4 == 0 {
                    "   ".to_owned()
                } else {
                    format!("banco{n}.es")
                }
            })
            .collect();

        let valid = draft.validate().expect("the blank rows are not a problem");
        assert_eq!(valid.draft().urls.len(), 30);
    }

    #[test]
    fn two_hundred_and_fifty_six_fields_are_enough_and_one_more_is_too_many() {
        let field = |n: usize| DraftField {
            id: None,
            label: format!("campo {n}"),
            value: "x".to_owned(),
            kind: FieldKind::Text,
        };

        let mut at_the_limit = a_draft(EntryKind::Account);
        at_the_limit.fields = (0..MAX_FIELDS).map(field).collect();
        assert!(at_the_limit.validate().is_ok());

        let mut one_past = a_draft(EntryKind::Account);
        one_past.fields = (0..=MAX_FIELDS).map(field).collect();
        assert_eq!(
            only_problem(one_past, "fields"),
            Problem::TooMany {
                limit: MAX_FIELDS,
                actual: MAX_FIELDS + 1
            }
        );
    }

    #[test]
    fn a_value_with_nothing_naming_it_is_reported_at_the_position_it_was_drawn_at() {
        let mut draft = a_draft(EntryKind::Account);
        draft.fields = (0..5)
            .map(|n| DraftField {
                id: None,
                label: if n == 3 {
                    String::new()
                } else {
                    format!("campo {n}")
                },
                value: "un valor".to_owned(),
                kind: FieldKind::Text,
            })
            .collect();

        assert_eq!(
            refusal(draft),
            vec![FieldError {
                field: "fields",
                index: Some(3),
                problem: Problem::Missing,
            }]
        );
    }

    #[test]
    fn a_field_somebody_has_named_and_not_filled_in_is_accepted() {
        let mut draft = a_draft(EntryKind::Account);
        draft.fields = vec![DraftField {
            id: None,
            label: "PIN".to_owned(),
            value: String::new(),
            kind: FieldKind::Secret,
        }];

        let valid = draft.validate().expect("an empty value is not a problem");
        assert_eq!(valid.draft().fields.len(), 1);
    }

    #[test]
    fn a_field_with_neither_a_name_nor_a_value_is_discarded() {
        let mut draft = a_draft(EntryKind::Account);
        draft.fields = vec![
            DraftField {
                id: None,
                label: "  ".to_owned(),
                value: "\t".to_owned(),
                kind: FieldKind::Text,
            },
            DraftField {
                id: None,
                label: "PIN".to_owned(),
                value: "1234".to_owned(),
                kind: FieldKind::Secret,
            },
        ];

        let valid = draft.validate().expect("the blank row is not a problem");
        assert_eq!(valid.draft().fields.len(), 1);
    }

    #[test]
    fn four_things_wrong_are_reported_as_four_problems_and_not_as_one() {
        // The property the whole module is shaped around. Somebody filling a form is told about
        // everything at once, because the alternative is four saves and four refusals.
        let mut draft = a_draft(EntryKind::Account);
        draft.title = String::new();
        draft.username = Some("alguien\rmás".to_owned());
        draft.password = Some(repeated('a', MAX_PASSWORD_BYTES + 1));
        draft.fields = vec![DraftField {
            id: None,
            label: "PIN".to_owned(),
            value: repeated('a', MAX_FIELD_VALUE_BYTES + 1),
            kind: FieldKind::Secret,
        }];

        let problems = refusal(draft);
        assert_eq!(problems.len(), 4, "got {problems:?}");
        for field in ["title", "username", "password", "fields"] {
            assert!(
                problems.iter().any(|problem| problem.field == field),
                "nothing was said about {field}: {problems:?}"
            );
        }
    }

    #[test]
    fn a_folder_name_is_judged_on_its_own() {
        assert_eq!(
            validate_folder_name("   "),
            Err(FieldError {
                field: "name",
                index: None,
                problem: Problem::Missing
            })
        );

        let at_the_limit = repeated('a', MAX_FOLDER_NAME_CHARS);
        assert_eq!(
            validate_folder_name(&at_the_limit),
            Ok(at_the_limit.clone())
        );

        assert_eq!(
            validate_folder_name(&repeated('a', MAX_FOLDER_NAME_CHARS + 1)),
            Err(FieldError {
                field: "name",
                index: None,
                problem: Problem::TooLong {
                    limit: MAX_FOLDER_NAME_CHARS,
                    actual: MAX_FOLDER_NAME_CHARS + 1
                }
            })
        );

        assert_eq!(
            validate_folder_name("Banc\tos"),
            Err(FieldError {
                field: "name",
                index: None,
                problem: Problem::Control
            })
        );
    }

    #[test]
    fn a_note_carrying_a_user_name_and_a_password_is_accepted() {
        // The kind is a label, not a restriction. Making it one would mean that changing an
        // account into a note, or back, could quietly lose what was typed into it.
        let mut draft = a_draft(EntryKind::Note);
        draft.username = Some("alguien@ejemplo".to_owned());
        draft.password = Some("una contraseña".to_owned());

        let valid = draft.validate().expect("a note may carry both");
        assert_eq!(valid.draft().username.as_deref(), Some("alguien@ejemplo"));
    }

    #[test]
    fn nothing_here_decides_what_an_address_is() {
        // Neither of these is a URL a parser would accept, and both are things somebody may
        // reasonably write down. The module stores text and says nothing about it.
        let mut draft = a_draft(EntryKind::Account);
        draft.urls = vec!["miservidor:8080".to_owned(), "el router de casa".to_owned()];

        let valid = draft.validate().expect("an address is text");
        assert_eq!(valid.draft().urls.len(), 2);
        assert_eq!(
            valid.draft().urls.first().map(String::as_str),
            Some("miservidor:8080"),
            "an address was rewritten on the way through"
        );
    }
}
