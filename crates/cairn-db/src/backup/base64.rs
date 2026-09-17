//! Base64, written here rather than taken from a crate.
//!
//! A hundred lines against one more dependency in a project that holds a password vault,
//! where every package in the graph is somebody else's release process standing between an
//! attacker and this binary. The trade is only worth making because the job is this small
//! and this completely specified: the standard alphabet of RFC 4648 with padding, no line
//! breaks, no alternative alphabet, no streaming. The published test vectors are in the
//! suite below, so this is interoperable rather than merely self-consistent.
//!
//! The decoder is strict on purpose. It refuses a character outside the alphabet, a length
//! that is not a multiple of four, padding anywhere but at the end, and a final group whose
//! leftover bits are not zero. That last one is not pedantry: a lenient decoder gives two
//! different texts that decode to the same bytes, and this decodes a file somebody else may
//! have written.

/// The standard alphabet of RFC 4648, in order.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The padding character.
const PAD: char = '=';

/// One alphabet character for a six bit value.
///
/// The value is always below sixty-four, because every caller has masked it, so the
/// fallback is unreachable. It is written rather than indexed because indexing is denied
/// across this workspace, and a panic inside an encoder is a worse answer than a wrong
/// character would be.
fn symbol(value: u8) -> char {
    char::from(ALPHABET.get(usize::from(value)).copied().unwrap_or(b'A'))
}

/// Encodes bytes.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for group in bytes.chunks(3) {
        let first = group.first().copied().unwrap_or(0);
        let second = group.get(1).copied().unwrap_or(0);
        let third = group.get(2).copied().unwrap_or(0);

        out.push(symbol(first >> 2));
        out.push(symbol(((first & 0x03) << 4) | (second >> 4)));

        if group.len() > 1 {
            out.push(symbol(((second & 0x0f) << 2) | (third >> 6)));
        } else {
            out.push(PAD);
        }

        if group.len() > 2 {
            out.push(symbol(third & 0x3f));
        } else {
            out.push(PAD);
        }
    }

    out
}

/// Decodes text, refusing anything that is not exactly what [`encode`] would have written.
///
/// Answers `None` rather than an error describing which rule was broken. The caller is an
/// import, and an import is allowed to say four things in total; a decoder that explained
/// itself would be a fifth, and a more detailed one than any of them.
#[must_use]
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }

    let groups = bytes.len().div_euclid(4);
    let mut out = Vec::with_capacity(groups * 3);

    for (index, group) in bytes.chunks(4).enumerate() {
        let is_last = index + 1 == groups;
        let mut values = [0_u8; 4];
        let mut padding = 0_usize;

        for (position, character) in group.iter().enumerate() {
            if *character == b'=' {
                // Padding only ever appears at the end of the last group. Anywhere else it
                // is a second spelling of the same bytes.
                if !is_last || position < 2 {
                    return None;
                }
                padding += 1;
            } else {
                if padding > 0 {
                    return None;
                }
                *values.get_mut(position)? = value_of(*character)?;
            }
        }

        let (first, second, third, fourth) = (
            values.first().copied().unwrap_or(0),
            values.get(1).copied().unwrap_or(0),
            values.get(2).copied().unwrap_or(0),
            values.get(3).copied().unwrap_or(0),
        );

        // The bits a short group leaves over have to be zero, or two texts decode alike.
        match padding {
            0 => {}
            1 if third.is_multiple_of(4) => {}
            2 if second.is_multiple_of(16) => {}
            _ => return None,
        }

        out.push((first << 2) | (second >> 4));
        if padding < 2 {
            out.push(((second & 0x0f) << 4) | (third >> 2));
        }
        if padding < 1 {
            out.push(((third & 0x03) << 6) | fourth);
        }
    }

    Some(out)
}

/// The six bit value of one alphabet character, or `None` if it is not one.
fn value_of(character: u8) -> Option<u8> {
    match character {
        b'A'..=b'Z' => Some(character - b'A'),
        b'a'..=b'z' => Some(character - b'a' + 26),
        b'0'..=b'9' => Some(character - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    /// The vectors from section 10 of RFC 4648.
    ///
    /// What makes this interoperable rather than merely self-consistent. A round trip test
    /// on its own passes for any bijection at all, including one nobody else implements,
    /// and the whole reason a backup uses a named encoding is that somebody with the
    /// published format can read it without this code.
    #[test]
    fn the_published_vectors_encode_the_way_the_specification_says() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), encoded, "encoding {plain:?}");
            assert_eq!(
                decode(encoded).as_deref(),
                Some(plain.as_bytes()),
                "decoding {encoded:?}"
            );
        }
    }

    #[test]
    fn the_last_two_alphabet_characters_are_the_standard_ones() {
        // Not the URL safe variant. The difference is two characters and it is the sort of
        // thing that goes unnoticed until somebody else's decoder refuses the file.
        assert_eq!(encode(&[0xfb, 0xff]), "+/8=");
    }

    #[test]
    fn every_byte_value_survives_the_trip() {
        let all: Vec<u8> = (0..=255_u8).collect();
        assert_eq!(decode(&encode(&all)).unwrap(), all);
    }

    #[test]
    fn every_length_up_to_a_few_groups_survives_the_trip() {
        for len in 0..64_usize {
            let bytes: Vec<u8> = (0..len)
                .map(|index| u8::try_from(index * 7 % 256).unwrap_or(0))
                .collect();
            let encoded = encode(&bytes);

            assert_eq!(encoded.len() % 4, 0, "length {len} produced ragged output");
            assert_eq!(decode(&encoded).unwrap(), bytes, "length {len}");
        }
    }

    #[test]
    fn a_length_that_is_not_a_multiple_of_four_is_refused() {
        for text in ["Z", "Zg", "Zg=", "Zm9vY"] {
            assert!(decode(text).is_none(), "{text:?} was accepted");
        }
    }

    #[test]
    fn a_character_outside_the_alphabet_is_refused() {
        for text in ["Zg=!", "Z-9v", "Zm 9v", "Zm9ñ"] {
            assert!(decode(text).is_none(), "{text:?} was accepted");
        }
    }

    #[test]
    fn padding_anywhere_but_the_end_is_refused() {
        for text in ["Zg==Zm9v", "Z=9v", "==9v", "Zm9v=g=="] {
            assert!(decode(text).is_none(), "{text:?} was accepted");
        }
    }

    #[test]
    fn a_group_with_leftover_bits_set_is_refused() {
        // "Zh==" and "Zg==" would decode to the same byte under a lenient decoder, which is
        // two spellings of one value inside a file somebody else may have edited.
        assert_eq!(decode("Zg==").unwrap(), b"f");
        assert!(decode("Zh==").is_none());
        assert_eq!(decode("Zm8=").unwrap(), b"fo");
        assert!(decode("Zm9=").is_none());
    }

    #[test]
    fn a_group_that_is_all_padding_is_refused() {
        for text in ["====", "Z===", "Zm9v===="] {
            assert!(decode(text).is_none(), "{text:?} was accepted");
        }
    }

    #[test]
    fn decoding_never_panics_on_anything_at_all() {
        // Totality. This runs over content that has already been decrypted, so it is not
        // the first line of defence, and it is still a parser reading a file: the one thing
        // it may never do is take the process down.
        for len in 0..16_usize {
            for seed in 0..64_u32 {
                let text: String = (0..len)
                    .map(|index| {
                        let code = (seed
                            .wrapping_mul(31)
                            .wrapping_add(u32::try_from(index).unwrap_or(0)))
                            % 128;
                        char::from_u32(code).unwrap_or('a')
                    })
                    .collect();
                let _ignored = decode(&text);
            }
        }
    }
}
