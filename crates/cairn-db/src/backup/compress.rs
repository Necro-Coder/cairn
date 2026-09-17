//! Compression, and the two limits that stop a small file from becoming a large one.
//!
//! The record stream inside a backup is repetitive structured text, so compressing it
//! before encrypting it is most of the reason a backup is a size somebody is willing to
//! copy to a phone. The usual objection to compressing before encrypting is CRIME and
//! BREACH, and it does not reach here: both need an attacker who can inject text into the
//! plaintext and watch the compressed size over and over. A backup is produced once, on
//! request, locally, and nobody sees its size but the person who asked for it.
//!
//! What does reach here is the decompression bomb (CWE-409). A few kilobytes of zeroes
//! expand to gigabytes, and a reader that finds that out by running out of memory has
//! already lost. So there are two limits and they are checked **while** the stream is being
//! read, not after:
//!
//! - an absolute ceiling on bytes produced, and
//! - a ceiling on the ratio of bytes produced to bytes consumed.
//!
//! The ratio only starts being enforced after the first mebibyte. A small file legitimately
//! has a wild ratio at the start, because a compressed stream begins with a frame header
//! that produces nothing, and refusing from the first byte would reject perfectly good
//! backups. Past a mebibyte of output, a hundred to one is not data any more.

use core::fmt;
use std::cell::Cell;
use std::io::{self, Read as _};
use std::rc::Rc;

use zeroize::Zeroizing;

use crate::error::DbError;

/// The most a backup may expand to while being read, in bytes.
///
/// One gibibyte. Above the largest file the size limit allows through and far below what
/// exhausting this machine would take, which is the shape a denial of service ceiling has
/// to have: high enough never to be hit by real data, low enough to be hit long before the
/// process dies.
pub const MAX_DECOMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;

/// The most a backup may expand by, as a multiple.
pub const MAX_EXPANSION_RATIO: u64 = 100;

/// How much has to come out before the ratio is enforced, in bytes.
///
/// One mebibyte. Below it the ratio says nothing: a compressed stream starts with a header
/// that produces no output at all, so the ratio at the first check is whatever the first
/// block happens to be, and a legitimate backup of a nearly empty vault would be refused.
pub const RATIO_GRACE_BYTES: u64 = 1024 * 1024;

/// How much is read out of the decompressor at a time, in bytes.
///
/// Sixty-four kibibytes, the same as one chunk of the file format. It is what makes the
/// limits above enforceable during rather than after: the check runs once per pass round
/// this buffer, so the most that can be produced past a limit is one bufferful.
const READ_BUFFER_LEN: usize = 64 * 1024;

/// The compression level.
///
/// Three, the library default. Measured against nine on a seeded database: nine took
/// several times as long and produced a file within a couple of per cent of the same size,
/// which is the usual shape for structured text, and this runs on a phone.
const LEVEL: i32 = 3;

/// Compresses a stream, handing every piece of output to a sink as it appears.
///
/// The sink is where the chunk sealer lives, so nothing here ever holds more than a buffer:
/// the record writer pushes rows in, this compresses them, and the sealer encrypts and
/// writes them out. There is no point in the path where the whole backup exists at once,
/// which is what makes this work on a device with a few hundred megabytes to spare.
pub struct Compressor<W: io::Write> {
    inner: zstd::stream::write::Encoder<'static, W>,
}

impl<W: io::Write> fmt::Debug for Compressor<W> {
    /// Names the type and nothing else.
    ///
    /// Written by hand because the encoder underneath has no `Debug` of its own, and
    /// because there is nothing about it worth printing: what passes through it is the
    /// plaintext of every record in the vault.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Compressor(open)")
    }
}

impl<W: io::Write> Compressor<W> {
    /// Starts compressing into a sink.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Io`] if the encoder cannot be created, which means the library
    /// refused the level.
    pub fn new(sink: W) -> Result<Self, DbError> {
        let inner =
            zstd::stream::write::Encoder::new(sink, LEVEL).map_err(|cause| DbError::Io {
                what: "the backup",
                operation: "compressed",
                cause,
            })?;

        Ok(Self { inner })
    }

    /// Finishes the stream and hands the sink back.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Io`] if the trailing frame cannot be written.
    pub fn finish(self) -> Result<W, DbError> {
        self.inner.finish().map_err(|cause| DbError::Io {
            what: "the backup",
            operation: "compressed",
            cause,
        })
    }
}

impl<W: io::Write> io::Write for Compressor<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Counts the bytes that pass through it on their way into the decompressor.
///
/// The denominator of the ratio. It has to be measured here, at the compressed side, rather
/// than taken from the size of the file: a file is only partly read when a limit trips, and
/// comparing output so far against a length that has not been reached yet would let a bomb
/// run until the very end.
///
/// The count is shared rather than read back out of the decompressor, because the
/// decompressor owns the reader and there is no borrowing it back mid-stream. A cell is
/// enough: one thread owns both ends of this pipe for as long as it exists.
struct Counted<R: io::Read> {
    inner: R,
    read: Rc<Cell<u64>>,
}

impl<R: io::Read> io::Read for Counted<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let taken = self.inner.read(buffer)?;
        self.read.set(self.read.get().saturating_add(as_u64(taken)));

        Ok(taken)
    }
}

/// A length as a count, saturating rather than truncating.
///
/// The conversion cannot lose anything on any platform this is built for, and it is written
/// rather than cast so that a target where it could is a saturation rather than a number
/// that silently means something else.
fn as_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Decompresses a stream, refusing it the moment it expands further than it may.
///
/// Every piece of output goes to `emit` as it appears, so nothing here holds the whole
/// stream either. What it does hold is the two counters, and it compares them on every pass
/// round the buffer rather than at the end, which is the difference between refusing a bomb
/// and discovering afterwards that one went off.
///
/// # Errors
///
/// - [`DbError::TooMany`] naming `decompressed bytes` when the absolute ceiling is passed,
///   and naming `bytes produced for the bytes read` when the ratio is, both checked during.
/// - [`DbError::Io`] if the stream is not a valid compressed stream, or if `emit` fails.
///
/// Nothing here distinguishes a corrupt stream from a hostile one, because by this point
/// the tag over every chunk has already verified: anything that arrives is either something
/// this program wrote or something somebody wrote with the password, and a reader that
/// tried to tell those apart would be inventing a distinction it cannot make.
pub fn decompress_bounded<R: io::Read>(
    source: R,
    mut emit: impl FnMut(&[u8]) -> Result<(), DbError>,
) -> Result<u64, DbError> {
    let read = Rc::new(Cell::new(0_u64));
    let counted = Counted {
        inner: source,
        read: Rc::clone(&read),
    };

    let mut decoder = zstd::stream::read::Decoder::new(counted).map_err(|cause| DbError::Io {
        what: "the backup",
        operation: "decompressed",
        cause,
    })?;

    // Zeroized, because what comes out of here is the record stream: every password in the
    // vault passes through this buffer in the clear on the way to the staging database.
    let mut buffer = Zeroizing::new(vec![0_u8; READ_BUFFER_LEN]);
    let mut produced = 0_u64;

    loop {
        let taken = decoder.read(&mut buffer).map_err(|cause| DbError::Io {
            what: "the backup",
            operation: "decompressed",
            cause,
        })?;

        if taken == 0 {
            break;
        }

        produced = produced.saturating_add(as_u64(taken));

        if produced > MAX_DECOMPRESSED_BYTES {
            return Err(DbError::TooMany {
                what: "decompressed bytes",
                value: produced,
                max: MAX_DECOMPRESSED_BYTES,
            });
        }

        let allowed = read.get().saturating_mul(MAX_EXPANSION_RATIO);
        if produced > RATIO_GRACE_BYTES && produced > allowed {
            return Err(DbError::TooMany {
                what: "bytes produced for the bytes read",
                value: produced,
                max: allowed,
            });
        }

        emit(buffer.get(..taken).unwrap_or_default())?;
    }

    Ok(produced)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::{Compressor, MAX_EXPANSION_RATIO, RATIO_GRACE_BYTES, decompress_bounded};
    use crate::error::DbError;

    /// Compresses a body the way an export does.
    fn compressed(body: &[u8]) -> Vec<u8> {
        let mut compressor = Compressor::new(Vec::new()).unwrap();
        compressor.write_all(body).unwrap();

        compressor.finish().unwrap()
    }

    /// Decompresses into one buffer, for tests that care about the content.
    fn decompressed(stream: &[u8]) -> Result<Vec<u8>, DbError> {
        let mut out = Vec::new();
        decompress_bounded(stream, |piece| {
            out.extend_from_slice(piece);
            Ok(())
        })?;

        Ok(out)
    }

    #[test]
    fn what_goes_in_comes_out() {
        let body = b"{\"kind\":\"row\"}\n".repeat(64);
        assert_eq!(decompressed(&compressed(&body)).unwrap(), body);
    }

    #[test]
    fn an_empty_stream_round_trips() {
        // The backup of an empty vault. It still has a manifest line, but the path where
        // nothing at all is written has to work, or an edge case becomes a crash.
        assert!(decompressed(&compressed(b"")).unwrap().is_empty());
    }

    #[test]
    fn structured_text_actually_gets_smaller() {
        // The whole reason the dependency is here. If this ever stopped holding, the
        // dependency would be cost with no benefit and should be removed rather than kept
        // out of habit.
        let body = b"{\"kind\":\"row\",\"values\":{\"deleted\":0,\"rev\":1}}\n".repeat(512);
        let stream = compressed(&body);

        assert!(
            stream.len() * 10 < body.len(),
            "{} bytes of records compressed to {}, which is not worth a dependency",
            body.len(),
            stream.len()
        );
    }

    #[test]
    fn a_decompression_bomb_is_refused_before_it_finishes() {
        // A few hundred bytes of compressed zeroes expanding to sixteen mebibytes. The
        // assertion that matters is not that it fails: it is that it fails having produced a
        // bounded amount, which is what "checked during" means.
        let bomb = compressed(&vec![0_u8; 16 * 1024 * 1024]);
        let mut produced = 0_u64;

        let outcome = decompress_bounded(bomb.as_slice(), |piece| {
            produced += u64::try_from(piece.len()).unwrap_or(0);
            Ok(())
        });

        assert!(
            matches!(
                outcome,
                Err(DbError::TooMany {
                    what: "bytes produced for the bytes read",
                    ..
                })
            ),
            "a bomb was not refused: {outcome:?}"
        );
        assert!(
            produced < 4 * 1024 * 1024,
            "{produced} bytes came out before the limit tripped, which is not during"
        );
    }

    #[test]
    fn a_small_file_with_a_high_ratio_is_not_refused() {
        // The false positive the grace window exists for. Half a mebibyte of zeroes
        // compresses to almost nothing, which is a ratio in the thousands and a completely
        // ordinary backup of a nearly empty vault.
        let body = vec![0_u8; usize::midpoint(0, usize::try_from(RATIO_GRACE_BYTES).unwrap_or(0))];
        assert_eq!(decompressed(&compressed(&body)).unwrap().len(), body.len());
    }

    #[test]
    fn the_ratio_is_the_number_it_is_documented_to_be() {
        // Frozen. The limits appear in the public documentation of the format, and a test
        // that only checks "something too big is refused" passes for any limit at all.
        assert_eq!(MAX_EXPANSION_RATIO, 100);
        assert_eq!(RATIO_GRACE_BYTES, 1024 * 1024);
    }

    #[test]
    fn something_that_is_not_a_compressed_stream_is_refused() {
        assert!(matches!(
            decompressed(b"esto no es un flujo comprimido"),
            Err(DbError::Io { .. })
        ));
    }

    #[test]
    fn a_stream_cut_short_is_refused() {
        // Cannot happen through the file format, because the chunk tags catch a truncation
        // first. It is still checked: this function is also reached by the fuzzing target,
        // which hands it bytes that never went through a tag.
        let stream = compressed(&b"contenido suficiente para varios bloques".repeat(64));
        let cut = stream
            .get(..usize::midpoint(0, stream.len()))
            .unwrap_or_default();

        assert!(decompressed(cut).is_err());
    }

    #[test]
    fn a_failing_sink_stops_the_decompression() {
        let stream = compressed(&b"x".repeat(1024));
        let outcome = decompress_bounded(stream.as_slice(), |_piece| {
            Err(DbError::TooMany {
                what: "records",
                value: 1,
                max: 0,
            })
        });

        assert!(matches!(
            outcome,
            Err(DbError::TooMany {
                what: "records",
                ..
            })
        ));
    }
}
