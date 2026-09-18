//! Feeds arbitrary bytes to the header and the chunk framing of a backup file.
//!
//! The outermost layer of a `.cairn` file, and the one somebody who has the file gets to
//! rewrite byte by byte. Sixty-four bytes of cleartext header they choose, followed by chunks
//! whose lengths are not written down anywhere — the reader works them out from the chunk size
//! the header declares, which is a number from the same untrusted place. That is the shape
//! that produces slicing bugs, and it is why this target exists.
//!
//! What is under test is that every input returns. Almost nothing here will open, because the
//! key is fixed and the fuzzer does not have it, and that is the expected answer: an error for
//! bytes that are not a chunk this key sealed, and never a panic, an index out of range or a
//! length that wrapped round zero.
//!
//! The key derivation is deliberately not in the loop. Argon2id at real parameters is most of
//! a second, which would turn a fuzzing run of a hundred thousand cases into one of a hundred,
//! and the derivation is not what is being tested: what is being tested is the parsing that
//! happens either side of it. A fixed key also makes a crash reproducible, which a key the
//! fuzzer generated would not be.
#![no_main]

use cairn_crypto::{BACKUP_HEADER_LEN, BackupHeader, ChunkOpener, DataKey};
use libfuzzer_sys::fuzz_target;
use zeroize::Zeroizing;

/// The key the chunks are opened against.
///
/// Fixed, and not a secret: nothing here is protecting anything. What matters is that the same
/// bytes mean the same thing on every run.
const KEY: [u8; 32] = [11; 32];

fuzz_target!(|data: &[u8]| {
    let Some(header_bytes) = data.get(..BACKUP_HEADER_LEN) else {
        // Shorter than a header. The real reader refuses this before opening the file at all,
        // and there is nothing after it to frame.
        return;
    };

    let Ok(header) = BackupHeader::parse(header_bytes) else {
        return;
    };

    // Anything that parsed has to survive being written out and read back, the same property
    // the vault header target asserts and for the same reason: a round trip that loses a field
    // is a backup that opens once and never again.
    let written = header.to_bytes();
    assert!(
        BackupHeader::parse(&written).is_ok(),
        "a parsed backup header did not survive being written out"
    );

    let key = DataKey::from_bytes(KEY);
    let mut opener = ChunkOpener::new(&key, &header);
    let mut out = Zeroizing::new(Vec::new());

    // The body, in pieces the input chooses. The framing holds a partial chunk across calls,
    // so a target that only ever handed it whole bodies would never exercise the one piece of
    // state it has.
    let body = data.get(BACKUP_HEADER_LEN..).unwrap_or_default();
    let step = header_bytes.first().map_or(1, |byte| usize::from(*byte)).max(1);

    for piece in body.chunks(step) {
        if opener.push(piece, &mut out).is_err() {
            // A chunk that did not open. The reader stops here too, and carrying on would be
            // feeding the opener bytes it has already said it cannot make sense of.
            return;
        }
    }

    // The end of the file, which is where the mark that says "this was the last chunk" is
    // checked. A reader that skipped it would accept a file cut off at a chunk boundary.
    let _finished = opener.finish(&mut out);
});
