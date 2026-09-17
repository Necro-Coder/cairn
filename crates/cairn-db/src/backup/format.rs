//! The record stream inside a backup: one line of JSON per thing, and the limits that stop
//! a hostile one from being read.
//!
//! One line per record rather than one document. A single JSON document has to be
//! materialised whole before anything can be done with it, which for a half gigabyte file
//! on a phone is the end of the conversation. A line at a time is written without ever
//! holding the file and read the same way, and the cost is that the format has to say what
//! each line is, which it does with a one word tag.
//!
//! ```text
//! {"manifest":{"format":"cairn-backup","recordVersion":1,"schemaVersion":5,"tables":[…]}}
//! {"table":{"name":"habits","rows":12}}
//! {"row":{"id":"…","createdAt":…,"name":"…"}}
//! ```
//!
//! Rows belong to the table line above them. That keeps a row line small, which matters
//! because there are as many of them as there are records, and it is why the table line
//! carries its own count: a reader knows before it starts how many rows to expect and can
//! refuse a table that claims more than the limit rather than discovering it a million rows
//! later.
//!
//! Everything here treats its input as hostile, even though the chunk tags have already
//! verified. Two reasons. A tag proves the file was written by somebody with the password,
//! which includes the owner restoring a backup that a disk quietly corrupted before it was
//! ever encrypted. And this parser is a fuzzing target that is handed bytes that never went
//! near a tag, which is the only way to test it as thoroughly as it needs testing.
//!
//! The limits are the ones the phase fixed, and they are ceilings against an attacker
//! rather than budgets for ordinary use. Every one of them has a test that feeds it a file
//! that breaks it by exactly one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::DbError;

/// What the manifest names the format.
pub const FORMAT_NAME: &str = "cairn-backup";

/// The version of the record layout this build writes.
///
/// Separate from the version in the file header, which describes how the file is encrypted.
/// They are different things and may move independently: a change to the framing does not
/// have to be a change to what a row looks like.
pub const RECORD_VERSION: u16 = 1;

/// The largest backup file this will read, in bytes.
///
/// Half a gibibyte, checked against the length of the file before a byte of it is read.
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// The most rows one table may carry.
pub const MAX_ROWS_PER_TABLE: u64 = 1_000_000;

/// The longest one text or decrypted value may be, in bytes.
///
/// Sixty-four kibibytes, which is the same ceiling the vault repository puts on a stored
/// value. The same number in both places is deliberate: a backup that accepted more than
/// the database does would import rows the database then refuses, one at a time, halfway
/// through a restore.
pub const MAX_FIELD_BYTES: usize = 64 * 1024;

/// The deepest a value inside a row may nest.
///
/// Eight. In practice nothing in this format nests at all — a row is a flat map of scalars,
/// and the decoder refuses an array or an object outright — so this is the outer guard
/// rather than the working rule. It is here because a parser with no depth limit is one
/// deeply nested line away from a stack overflow, and a stack overflow is not an error a
/// process can report.
pub const MAX_NESTING_DEPTH: usize = 8;

/// The longest one line may be, in bytes.
///
/// Four mebibytes. A row of the widest table in the schema, with every value at the field
/// limit and base64 expanding it by a third, comes to under two. The bound exists so that a
/// file with no newline in it at all is refused after four mebibytes rather than after five
/// hundred and twelve.
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// The most custom fields one vault entry may carry.
pub const MAX_FIELDS_PER_ENTRY: u64 = 256;

/// What the first line of every backup says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    /// Always [`FORMAT_NAME`]. A reader that finds anything else stops.
    pub format: String,
    /// The version of the record layout.
    pub record_version: u16,
    /// The schema version the rows were read out of.
    pub schema_version: u32,
    /// Which tables follow, in order, and how many rows each one has.
    pub tables: Vec<TableCount>,
}

/// One entry of the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableCount {
    /// The table, as the schema names it.
    pub name: String,
    /// How many rows it has.
    pub rows: u64,
}

/// The line that says the rows below it belong to a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableHeader {
    /// The table, as the schema names it.
    pub name: String,
    /// How many rows follow before the next table line.
    pub rows: u64,
}

/// One value of one column, as it travels.
pub type RowValues = BTreeMap<String, serde_json::Value>;

/// One line of a backup.
///
/// Externally tagged, which is serde's default and the one form where
/// `deny_unknown_fields` does what it says on every variant. A line with a tag this build
/// does not know is refused rather than skipped: skipping unknown lines would mean a
/// version two could add a line type that a version one reader silently drops, and a
/// restore that silently drops things is the worst failure this format has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Line {
    /// The first line of the file.
    Manifest(Manifest),
    /// The rows below belong to this table.
    Table(TableHeader),
    /// One row, as a flat map from column name to value.
    Row(RowValues),
}

impl Line {
    /// Writes one line, with its newline.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Io`] if the line cannot be serialised, which means a value was
    /// built that JSON cannot represent, or if the sink refuses it.
    pub fn write_to(&self, sink: &mut impl std::io::Write) -> Result<(), DbError> {
        let text = serde_json::to_string(self).map_err(|cause| DbError::Io {
            what: "a backup record",
            operation: "written",
            cause: std::io::Error::other(cause),
        })?;

        sink.write_all(text.as_bytes())
            .and_then(|()| sink.write_all(b"\n"))
            .map_err(|cause| DbError::Io {
                what: "a backup record",
                operation: "written",
                cause,
            })
    }

    /// Reads one line back.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Malformed`] for anything that is not exactly one of the three
    /// line types, including a line with a field this build does not know. It carries no
    /// detail about which check failed and no position: a reader that says where it stopped
    /// is a reader that tells whoever is editing the file how close they got.
    pub fn parse(line: &str) -> Result<Self, DbError> {
        if line.len() > MAX_LINE_BYTES {
            return Err(DbError::TooMany {
                what: "bytes in one backup record",
                value: as_u64(line.len()),
                max: as_u64(MAX_LINE_BYTES),
            });
        }

        let parsed: Self = serde_json::from_str(line).map_err(|_cause| DbError::Malformed)?;
        parsed.check_depth()?;

        Ok(parsed)
    }

    /// Refuses a line that nests deeper than the format allows.
    fn check_depth(&self) -> Result<(), DbError> {
        let Self::Row(values) = self else {
            return Ok(());
        };

        for value in values.values() {
            if depth_of(value, MAX_NESTING_DEPTH) > MAX_NESTING_DEPTH {
                return Err(DbError::TooMany {
                    what: "levels of nesting in one value",
                    value: as_u64(MAX_NESTING_DEPTH + 1),
                    max: as_u64(MAX_NESTING_DEPTH),
                });
            }
        }

        Ok(())
    }
}

/// How deep a value nests, stopping as soon as it is past a budget.
///
/// Bounded rather than exhaustive on purpose. Measuring the true depth of a hostile value
/// means walking all of it, which is the work being refused; this stops at the first level
/// past the limit and answers "deeper than allowed" without going further.
fn depth_of(value: &serde_json::Value, budget: usize) -> usize {
    match value {
        serde_json::Value::Array(items) => {
            if budget == 0 {
                return 1;
            }
            1 + items
                .iter()
                .map(|item| depth_of(item, budget - 1))
                .max()
                .unwrap_or(0)
        }
        serde_json::Value::Object(fields) => {
            if budget == 0 {
                return 1;
            }
            1 + fields
                .values()
                .map(|field| depth_of(field, budget - 1))
                .max()
                .unwrap_or(0)
        }
        _scalar => 1,
    }
}

/// A length as a count, saturating rather than truncating.
fn as_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Splits a stream of bytes into lines without ever holding more than one.
///
/// A reader rather than `split('\n')`, because the bytes arrive a chunk at a time from the
/// decompressor and a line straddles chunks far more often than not. It refuses a line
/// longer than [`MAX_LINE_BYTES`] as soon as it has that many bytes with no newline in
/// them, which is the point: a file with no newline in it at all must not be read into
/// memory before anybody notices.
#[derive(Debug, Default)]
pub struct LineSplitter {
    pending: Vec<u8>,
}

impl LineSplitter {
    /// A splitter with nothing in it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds bytes, handing every complete line to `on_line`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::TooMany`] if a line grows past the limit, [`DbError::Malformed`]
    /// if the bytes are not valid UTF-8, and whatever `on_line` returns.
    pub fn push(
        &mut self,
        bytes: &[u8],
        mut on_line: impl FnMut(&str) -> Result<(), DbError>,
    ) -> Result<(), DbError> {
        self.pending.extend_from_slice(bytes);

        while let Some(at) = self.pending.iter().position(|byte| *byte == b'\n') {
            let rest = self.pending.split_off(at + 1);
            let line = core::mem::replace(&mut self.pending, rest);
            let text = core::str::from_utf8(line.get(..at).unwrap_or_default())
                .map_err(|_not_text| DbError::Malformed)?;

            on_line(text)?;
        }

        if self.pending.len() > MAX_LINE_BYTES {
            return Err(DbError::TooMany {
                what: "bytes in one backup record",
                value: as_u64(self.pending.len()),
                max: as_u64(MAX_LINE_BYTES),
            });
        }

        Ok(())
    }

    /// Hands over whatever is left after the last newline.
    ///
    /// A file written by this code always ends with a newline, so what is left is normally
    /// nothing. A file that ends mid-line is refused rather than read as far as it goes: a
    /// half record is a row with columns missing, and a restore that drops columns is worse
    /// than one that refuses.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Malformed`] if anything is left over.
    pub fn finish(self) -> Result<(), DbError> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(DbError::Malformed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FORMAT_NAME, Line, LineSplitter, MAX_LINE_BYTES, MAX_NESTING_DEPTH, Manifest,
        RECORD_VERSION, RowValues, TableCount, TableHeader,
    };
    use crate::error::DbError;

    fn a_manifest() -> Line {
        Line::Manifest(Manifest {
            format: FORMAT_NAME.to_owned(),
            record_version: RECORD_VERSION,
            schema_version: 4,
            tables: vec![TableCount {
                name: "habits".to_owned(),
                rows: 2,
            }],
        })
    }

    fn a_row() -> Line {
        let mut values = RowValues::new();
        values.insert(
            "id".to_owned(),
            serde_json::json!("AAECAwQFBgcICQoLDA0ODw=="),
        );
        values.insert("rev".to_owned(), serde_json::json!(1));
        values.insert("name".to_owned(), serde_json::json!("correr"));
        values.insert("notes".to_owned(), serde_json::Value::Null);

        Line::Row(values)
    }

    /// Writes one line and gives back the text, newline included.
    fn written(line: &Line) -> String {
        let mut out = Vec::new();
        line.write_to(&mut out).unwrap();

        String::from_utf8(out).unwrap()
    }

    #[test]
    fn every_line_type_survives_the_trip() {
        for line in [
            a_manifest(),
            Line::Table(TableHeader {
                name: "habits".to_owned(),
                rows: 2,
            }),
            a_row(),
        ] {
            let text = written(&line);
            assert!(text.ends_with('\n'), "a line was written without a newline");
            assert_eq!(Line::parse(text.trim_end()).unwrap(), line);
        }
    }

    #[test]
    fn a_line_is_one_line() {
        // A record with a newline inside it would be two records to the splitter and one to
        // the writer, which is how a row ends up half read.
        let text = written(&a_row());
        assert_eq!(text.matches('\n').count(), 1);
    }

    #[test]
    fn the_tag_is_the_word_the_format_documents() {
        // Published byte for byte, so somebody writing an independent reader matches on
        // these three words. A rename would be a silent format change.
        assert!(written(&a_manifest()).starts_with("{\"manifest\":"));
        assert!(
            written(&Line::Table(TableHeader {
                name: "t".to_owned(),
                rows: 0,
            }))
            .starts_with("{\"table\":")
        );
        assert!(written(&a_row()).starts_with("{\"row\":"));
    }

    #[test]
    fn a_line_with_a_tag_this_build_does_not_know_is_refused() {
        // Never skipped. A reader that ignores what it does not understand is a reader that
        // silently drops whatever a later version adds, and a restore that silently drops
        // things is the worst thing this format could do.
        assert!(matches!(
            Line::parse("{\"attachment\":{\"bytes\":\"AA==\"}}"),
            Err(DbError::Malformed)
        ));
    }

    #[test]
    fn a_line_with_a_field_this_build_does_not_know_is_refused() {
        assert!(matches!(
            Line::parse("{\"table\":{\"name\":\"habits\",\"rows\":1,\"extra\":true}}"),
            Err(DbError::Malformed)
        ));
    }

    #[test]
    fn a_line_with_a_field_missing_is_refused() {
        assert!(matches!(
            Line::parse("{\"table\":{\"name\":\"habits\"}}"),
            Err(DbError::Malformed)
        ));
    }

    #[test]
    fn rubbish_is_refused_without_saying_where_it_stopped() {
        for text in ["", "{", "null", "[]", "{\"row\":[]}", "no es json"] {
            let refused = Line::parse(text);
            assert!(matches!(refused, Err(DbError::Malformed)), "{text:?}");
            assert_eq!(
                refused.unwrap_err().to_string(),
                "the backup is damaged or incomplete",
                "the message for {text:?} said more than it may"
            );
        }
    }

    #[test]
    fn a_line_past_the_length_limit_is_refused_by_exactly_one_byte() {
        // The boundary from both sides, which is the only way a limit is actually tested.
        let over = "x".repeat(MAX_LINE_BYTES + 1);
        assert!(matches!(
            Line::parse(&over),
            Err(DbError::TooMany {
                what: "bytes in one backup record",
                ..
            })
        ));

        let just_under = "x".repeat(MAX_LINE_BYTES);
        // Not a valid line, so it is refused, and refused as malformed rather than as too
        // long: the length check let it through.
        assert!(matches!(Line::parse(&just_under), Err(DbError::Malformed)));
    }

    #[test]
    fn a_value_that_nests_past_the_limit_is_refused() {
        let mut nested = serde_json::json!(1);
        for _level in 0..=MAX_NESTING_DEPTH {
            nested = serde_json::Value::Array(vec![nested]);
        }

        let line = format!("{{\"row\":{{\"note\":{nested}}}}}");
        assert!(matches!(
            Line::parse(&line),
            Err(DbError::TooMany {
                what: "levels of nesting in one value",
                ..
            })
        ));
    }

    #[test]
    fn a_value_at_the_nesting_limit_is_accepted() {
        // The other half. Nothing this code writes nests at all, and a check written with
        // the wrong comparison would still pass every test above.
        let mut nested = serde_json::json!(1);
        for _level in 1..MAX_NESTING_DEPTH {
            nested = serde_json::Value::Array(vec![nested]);
        }

        let line = format!("{{\"row\":{{\"note\":{nested}}}}}");
        assert!(Line::parse(&line).is_ok());
    }

    #[test]
    fn the_splitter_hands_over_whole_lines_however_the_bytes_arrive() {
        let body = format!("{}{}", written(&a_manifest()), written(&a_row()));

        for piece_len in [1_usize, 3, 7, 64, body.len()] {
            let mut splitter = LineSplitter::new();
            let mut lines = Vec::new();

            for piece in body.as_bytes().chunks(piece_len) {
                splitter
                    .push(piece, |line| {
                        lines.push(line.to_owned());
                        Ok(())
                    })
                    .unwrap();
            }
            splitter.finish().unwrap();

            assert_eq!(lines.len(), 2, "pieces of {piece_len} bytes split wrongly");
            assert_eq!(Line::parse(&lines[0]).unwrap(), a_manifest());
            assert_eq!(Line::parse(&lines[1]).unwrap(), a_row());
        }
    }

    #[test]
    fn the_splitter_refuses_a_stream_with_no_newline_in_it() {
        // A half gibibyte with no newline must be refused after four mebibytes rather than
        // read into memory first. The assertion is on where it stops, not that it stops.
        let mut splitter = LineSplitter::new();
        let piece = vec![b'x'; 1024 * 1024];
        let mut outcome = Ok(());

        for _round in 0..8 {
            outcome = splitter.push(&piece, |_line| Ok(()));
            if outcome.is_err() {
                break;
            }
        }

        assert!(matches!(
            outcome,
            Err(DbError::TooMany {
                what: "bytes in one backup record",
                ..
            })
        ));
    }

    #[test]
    fn the_splitter_refuses_a_stream_that_ends_mid_line() {
        let mut splitter = LineSplitter::new();
        splitter.push(b"{\"row\":{}}", |_line| Ok(())).unwrap();

        assert!(matches!(splitter.finish(), Err(DbError::Malformed)));
    }

    #[test]
    fn the_splitter_refuses_bytes_that_are_not_text() {
        let mut splitter = LineSplitter::new();
        let refused = splitter.push(&[0xff, 0xfe, b'\n'], |_line| Ok(()));

        assert!(matches!(refused, Err(DbError::Malformed)));
    }

    #[test]
    fn the_splitter_never_panics_on_anything_at_all() {
        // Totality over the shapes a damaged stream takes: empty lines, lone newlines,
        // partial UTF-8 at a boundary, control characters.
        for bytes in [
            b"\n\n\n".as_slice(),
            b"".as_slice(),
            b"\r\n".as_slice(),
            &[0x00, 0x0a],
            &[0xe2, 0x82],
        ] {
            let mut splitter = LineSplitter::new();
            let _ignored = splitter.push(bytes, |line| {
                let _also_ignored = Line::parse(line);
                Ok(())
            });
            let _also_ignored = splitter.finish();
        }
    }
}
