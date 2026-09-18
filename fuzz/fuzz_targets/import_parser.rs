//! Feeds arbitrary bytes to the record stream an import reads.
//!
//! This is what a backup looks like once the password has been right: the chunks have opened,
//! zstd has undone itself, and what is left is a stream of lines that used to be somebody's
//! vault. Every limit that bounds a hostile file is applied here rather than earlier, so this
//! is the layer where a file that is a Cairn backup, and is signed by its own password, can
//! still be an attack — which is exactly the case somebody restoring their own backup from
//! five years ago is in.
//!
//! Two things are under test and neither is that parsing succeeds. The first is that every
//! input returns: a line of four megabytes, a thousand nested arrays, a number where a string
//! belongs, a manifest that is not first. The second is that the splitter and the parser agree
//! about what a line is, because the splitter is what enforces the length limit and the parser
//! is what enforces everything inside it.
//!
//! Almost nothing here will parse, and that is the expected answer.
#![no_main]

use cairn_db::backup::format::{Line, LineSplitter};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut splitter = LineSplitter::new();

    // Pushed in two pieces rather than one, at an offset the input itself chooses. A stream
    // arrives from a decompressor in whatever sizes the decompressor felt like, and a splitter
    // that only ever saw whole inputs would never be asked to hold half a line across a call —
    // which is the one piece of state it has and therefore the one place it can be wrong.
    let split_at = data.first().map_or(0, |byte| usize::from(*byte)).min(data.len());
    let (first, second) = data.split_at(split_at);

    let _first = splitter.push(first, |line| {
        let _parsed = Line::parse(line)?;
        Ok(())
    });
    let _second = splitter.push(second, |line| {
        let _parsed = Line::parse(line)?;
        Ok(())
    });

    // The end of the stream, which is where a line with no newline after it is refused. A
    // reader that accepted one would accept a file that was cut off mid-record.
    let _finished = splitter.finish();
});
