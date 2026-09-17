//! A moment and a day are not comparable. They are not the same kind of thing, and the version
//! of this application where they compare is the one where a habit marked late at night lands on
//! the wrong square.

use cairn_domain::{CivilDay, Timestamp};

fn main() {
    let moment = Timestamp::from_micros(1_700_000_000_000_000);
    let day = CivilDay::new(2026, 9, 17).unwrap();

    let _same = moment == day;
}
