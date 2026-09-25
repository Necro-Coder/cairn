//! The password vault: what an entry may hold, and how long a thrown away one is kept.
//!
//! Everything under here is pure arithmetic and pure comparison. Nothing reads a clock, nothing
//! opens a file and nothing knows that SQLite exists: a draft arrives as text somebody typed, and
//! what leaves is either a judgement about it or a list of everything wrong with it.
//!
//! The one rule the whole module is built around is that a value is judged once. A repository
//! that receives a [`spec::ValidEntry`] does not check lengths again, because the only way to
//! hold one is to have passed [`spec::EntryDraft::validate`].

pub mod spec;
pub mod trash;

pub use spec::{
    DraftField, EntryDraft, EntryKind, FieldError, FieldKind, MAX_FIELD_LABEL_CHARS,
    MAX_FIELD_VALUE_BYTES, MAX_FIELDS, MAX_FOLDER_NAME_CHARS, MAX_NOTES_BYTES, MAX_PASSWORD_BYTES,
    MAX_TITLE_CHARS, MAX_URL_CHARS, MAX_URLS, MAX_USERNAME_CHARS, Problem, ValidEntry,
    validate_folder_name,
};
pub use trash::{TOMBSTONE_DAYS, TRASH_DAYS, TrashState, bin_cutoff_us, state};
