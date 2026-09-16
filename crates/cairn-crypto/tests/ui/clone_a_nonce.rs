//! A nonce cannot be duplicated, which is the other way to end up using one twice.

use cairn_crypto::FreshNonce;

fn main() {
    let nonce = FreshNonce::generate().unwrap();
    let _copy = nonce.clone();
}
