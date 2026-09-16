//! A nonce cannot be built from bytes somebody already has, only read from the system.

use cairn_crypto::FreshNonce;

fn main() {
    let _nonce = FreshNonce::from([0x00; 24]);
}
