//! Keys have no `PartialEq`, so the comparison that leaks timing does not compile.

use cairn_crypto::DataKey;

fn main() {
    let first = DataKey::from_bytes([0x11; 32]);
    let second = DataKey::from_bytes([0x22; 32]);
    let _same = first == second;
}
