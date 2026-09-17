//! Turning a moment into a day needs a place, and a place is a decision this crate does not have.
//! A conversion that quietly chose one would be right for whoever wrote it and wrong for whoever
//! travelled.

use cairn_domain::{CivilDay, Timestamp};

fn main() {
    let moment = Timestamp::from_micros(1_700_000_000_000_000);
    let _day: CivilDay = moment.into();
}
