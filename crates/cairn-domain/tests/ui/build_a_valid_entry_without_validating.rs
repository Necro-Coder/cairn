//! A `ValidEntry` is a promise that something was judged, and the only way to make one is to
//! have it judged. The version of this application where a draft can be wrapped by hand is the
//! one where a repository writes a title nobody measured.

use cairn_domain::vault::{EntryDraft, EntryKind, ValidEntry};

fn main() {
    let draft = EntryDraft {
        kind: EntryKind::Account,
        title: String::new(),
        username: None,
        password: None,
        notes: None,
        urls: Vec::new(),
        fields: Vec::new(),
        folder_id: None,
        favorite: false,
    };

    let _wrapped = ValidEntry(draft);
}
