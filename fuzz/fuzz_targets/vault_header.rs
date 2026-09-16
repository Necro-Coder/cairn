//! Feeds arbitrary bytes to the vault header parser.
//!
//! This is the only parser in the project whose input somebody who has stolen the file gets
//! to write, which makes it the definition of what is worth fuzzing. It is also small: a
//! fixed size layout with no length fields and no loops, so there is very little for a
//! fuzzer to find. That is the point. A target that finds nothing after an hour is evidence;
//! not having the target is not.
//!
//! What is under test is first that returning at all is possible for every input, and then
//! that anything which did parse survives being written out and read back.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(header) = cairn_crypto::VaultHeader::parse(data) {
        // Anything that parsed has to serialise again, and the bytes have to parse a second
        // time. A round trip that loses a field is a header that opens once and never again.
        let written = header.to_bytes();
        let reparsed = cairn_crypto::VaultHeader::parse(&written);
        assert!(
            reparsed.is_ok(),
            "a parsed header did not survive being written out"
        );

        // And the associated data has to be derivable without panicking, because that is what
        // the unwrapping does next.
        let _ = header.wrap_aad();
    }
});
