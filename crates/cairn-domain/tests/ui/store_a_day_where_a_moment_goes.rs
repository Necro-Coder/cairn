//! The two are stored in columns of the same SQL type, which is exactly why the Rust types have
//! to be different: nothing downstream of here can tell them apart.

use cairn_domain::{CivilDay, Timestamp};

fn takes_a_moment(_moment: Timestamp) {}

fn main() {
    let day = CivilDay::new(2026, 9, 17).unwrap();
    takes_a_moment(day);
}
