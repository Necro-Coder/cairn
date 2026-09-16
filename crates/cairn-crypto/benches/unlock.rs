//! How long unlocking takes, at the parameters a vault might be written with.
//!
//! Argon2id is the only thing in this project that is slow on purpose, and the number it
//! produces is the one figure a person actually feels: the wait between typing the master
//! password and seeing their data. The budget is eight hundred milliseconds on a desktop
//! machine, and the phase that measures it on a phone comes later.
//!
//! This records rather than enforces. A benchmark that fails the build on a slow morning
//! teaches everybody to ignore it, and the machine a contributor runs it on is not the
//! machine the budget was written for. What it is for is having a number to compare against
//! when somebody says the application feels slower than it used to.
//!
//! Run with `cargo bench -p cairn-crypto`. Deliberately not a criterion harness: three
//! samples of a half second operation is all the statistics this deserves, and a benchmark
//! framework here would be a dependency and a compile time bought for nothing.

#![allow(
    clippy::print_stdout,
    clippy::expect_used,
    reason = "a benchmark is a program run by hand whose entire output is what it prints, and whose parameters are literals written three lines above the call that rejects them; the lints that forbid printing and panicking constructs only relax themselves inside test functions"
)]

use std::hint::black_box;
use std::time::Instant;

use cairn_crypto::{Argon2Params, derive_kek};

/// A password of about the length somebody actually types.
const PASSWORD: &str = "una contrasena de ejemplo para medir";

/// A fixed salt. The cost of Argon2id does not depend on its value, and a fixed one keeps
/// two runs comparable.
const SALT: [u8; 16] = [0x41; 16];

/// How many times each set of parameters is measured.
const SAMPLES: usize = 3;

fn main() {
    let cases = [
        ("default, m=64 MiB t=3 p=1", Argon2Params::DEFAULT),
        (
            "fallback, m=48 MiB t=3 p=1",
            Argon2Params::new(48 * 1024, 3, 1).expect("48 MiB is inside the allowed range"),
        ),
        (
            "floor, m=32 MiB t=3 p=1",
            Argon2Params::new(32 * 1024, 3, 1).expect("32 MiB is the floor"),
        ),
    ];

    println!("derivation of the key encryption key, {SAMPLES} samples each, median reported");

    for (label, params) in cases {
        let mut timings = Vec::with_capacity(SAMPLES);

        for _ in 0..SAMPLES {
            let started = Instant::now();
            let key = derive_kek(PASSWORD, &SALT, params).expect("the parameters are in range");
            // Kept alive across the measurement so that nothing is optimised away.
            black_box(&key);
            timings.push(started.elapsed());
        }

        timings.sort_unstable();
        let median = timings
            .get(SAMPLES.div_ceil(2) - 1)
            .copied()
            .unwrap_or_default()
            .as_millis();

        println!("  {label:<26} {median:>5} ms");
    }
}
