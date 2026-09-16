//! Proves that the compiler refuses to reuse a nonce.
//!
//! The claim made everywhere else in this crate is that nonce reuse is impossible rather
//! than discouraged. That claim is worth exactly as much as the evidence for it, and the
//! only evidence that means anything is a compiler that says no.
//!
//! So the cases in `tests/ui` are programs that must not build, and their expected error
//! is recorded beside them. If somebody adds a `Clone` to the nonce, or changes `seal` to
//! take it by reference, these start compiling and this test goes red.
//!
//! The cost is that the expected output is tied to the wording the compiler uses. The
//! toolchain is pinned to an exact version, so that wording only moves in the pull request
//! that moves the toolchain, which is the right place to notice it.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "every function in an integration test file is test code, but the lints that forbid panicking constructs only relax themselves inside #[cfg(test)] modules and #[test] functions"
)]

#[test]
fn reusing_a_nonce_does_not_compile() {
    let harness = trybuild::TestCases::new();
    harness.compile_fail("tests/ui/*.rs");
}
