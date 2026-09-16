//! A nonce cannot be used twice, because the first use consumed it.

use cairn_crypto::{Aad, DataKey, FreshNonce, seal};

fn main() {
    let key = DataKey::from_bytes([0x11; 32]);
    let aad = Aad::record(1, "credentials", &[0x22; 16], "password", 1, &[0x33; 16]).unwrap();
    let nonce = FreshNonce::generate().unwrap();

    let _first = seal(&key, nonce, &aad, b"primero").unwrap();
    let _second = seal(&key, nonce, &aad, b"segundo").unwrap();
}
