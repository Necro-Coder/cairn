//! The clock that orders writes made on two machines whose wall clocks disagree.
//!
//! `updated_at` is not enough, and the reason is not theoretical. Two devices whose clocks differ
//! by a few seconds produce a merge that picks the wrong winner, silently, and the person only
//! finds out when something they edited yesterday comes back. A hybrid logical clock keeps the
//! wall reading, because a human has to be able to read it, and adds a counter that makes two
//! writes in the same millisecond orderable, and the device, which breaks the remaining ties the
//! same way on every machine.
//!
//! ```text
//! hlc = wall_ms: u64 | counter: u16 | device: u48      16 bytes, big endian
//! ```
//!
//! Big endian, and that is the whole layout decision. Written most significant first, the sixteen
//! bytes sort as bytes in exactly the order they sort as clocks, so SQLite can index them, compare
//! them and page through them without decoding anything, and the watermark comparison the merge
//! does on every row is a memcmp.
//!
//! The rule for advancing is from the design and is three lines: the wall reading never goes
//! backwards, two readings in the same millisecond differ in the counter, and a counter that runs
//! out borrows a millisecond from the future rather than repeating itself. What that buys is the
//! only property anything here depends on: a clock reading from one device is strictly greater
//! than every reading that device produced before it, whatever the operating system's clock does.

use std::fmt;

/// How many bytes a clock reading occupies, in a row and on the wire.
pub const HLC_LEN: usize = 16;

/// How many bytes of the reading identify the device.
///
/// Six, the first six bytes of that device's identifier. Not a separate number: a second
/// identifier that can drift apart from the first is a second identifier one of which is wrong.
pub const DEVICE_LEN: usize = 6;

/// A reading of the hybrid logical clock.
///
/// Ordered by the wall reading, then the counter, then the device. The device is last because it
/// is a tie-break and nothing else: it makes two devices that wrote in the same millisecond with
/// the same counter order the same way everywhere, which is what stops a merge from disagreeing
/// with itself depending on which side runs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    wall_ms: u64,
    counter: u16,
    device: [u8; DEVICE_LEN],
}

impl Hlc {
    /// A reading from its three parts.
    #[must_use]
    pub const fn new(wall_ms: u64, counter: u16, device: [u8; DEVICE_LEN]) -> Self {
        Self {
            wall_ms,
            counter,
            device,
        }
    }

    /// The wall clock reading, in milliseconds since the epoch.
    #[must_use]
    pub const fn wall_ms(self) -> u64 {
        self.wall_ms
    }

    /// How many readings this device has already produced in that millisecond.
    #[must_use]
    pub const fn counter(self) -> u16 {
        self.counter
    }

    /// The device that produced the reading.
    #[must_use]
    pub const fn device(self) -> [u8; DEVICE_LEN] {
        self.device
    }

    /// The sixteen bytes, in the order they are stored and compared in.
    #[must_use]
    pub fn to_bytes(self) -> [u8; HLC_LEN] {
        let mut bytes = [0_u8; HLC_LEN];
        bytes[..8].copy_from_slice(&self.wall_ms.to_be_bytes());
        bytes[8..10].copy_from_slice(&self.counter.to_be_bytes());
        bytes[10..].copy_from_slice(&self.device);

        bytes
    }

    /// A reading from the sixteen bytes it is stored as.
    ///
    /// Total: every sixteen bytes are a reading. There is nothing to refuse, because every
    /// combination of a wall reading, a counter and a device is one that some machine could have
    /// produced, and a decoder that invented a rule here would refuse rows written by a device
    /// whose clock was merely wrong.
    #[must_use]
    pub fn from_bytes(bytes: [u8; HLC_LEN]) -> Self {
        let mut wall = [0_u8; 8];
        let mut counter = [0_u8; 2];
        let mut device = [0_u8; DEVICE_LEN];
        wall.copy_from_slice(&bytes[..8]);
        counter.copy_from_slice(&bytes[8..10]);
        device.copy_from_slice(&bytes[10..]);

        Self {
            wall_ms: u64::from_be_bytes(wall),
            counter: u16::from_be_bytes(counter),
            device,
        }
    }
}

impl fmt::Display for Hlc {
    /// Prints the reading as its three parts, for a log and not for a person.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}@", self.wall_ms, self.counter)?;
        for byte in self.device {
            write!(formatter, "{byte:02x}")?;
        }

        Ok(())
    }
}

/// The clock of one device, which is the only thing that produces readings.
///
/// Holds the last reading it gave out and nothing else. Not a global, and it does not read the
/// system clock itself: the moment arrives as an argument, like everywhere else in this
/// workspace, so that a test can hand it a clock that jumps backwards and check what happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    last: Hlc,
}

impl Clock {
    /// A clock for a device that has never written anything.
    ///
    /// Starts at a wall reading of zero rather than at the current moment, so that the first
    /// reading it gives out is the first moment it is asked about. A clock that started at
    /// "now" would have to read a clock to be built, which is the dependency this avoids.
    #[must_use]
    pub const fn starting(device: [u8; DEVICE_LEN]) -> Self {
        Self {
            last: Hlc::new(0, 0, device),
        }
    }

    /// A clock that carries on from a reading already written down.
    ///
    /// What a device does when it starts up: the highest reading in its own database is where it
    /// must continue from, or the first write after a restart would repeat a reading that has
    /// already been used for a different row.
    #[must_use]
    pub const fn resuming(last: Hlc) -> Self {
        Self { last }
    }

    /// The last reading this clock gave out.
    #[must_use]
    pub const fn last(self) -> Hlc {
        self.last
    }

    /// The next reading, given what the wall clock says now.
    ///
    /// The three lines of the rule. The wall reading is the later of the operating system's and
    /// the one already used, so a clock that jumps backwards cannot make this go back with it.
    /// Two readings in the same millisecond differ in the counter. A counter that has run out
    /// takes the next millisecond instead of repeating, which is the case the naive version gets
    /// wrong: sixty-five thousand writes in one millisecond is not something a person does, but
    /// it is exactly what a bulk import does, and a repeated reading is two rows that no merge
    /// can order.
    pub fn tick(&mut self, now_ms: u64) -> Hlc {
        let wall = now_ms.max(self.last.wall_ms);

        let next = if wall == self.last.wall_ms {
            match self.last.counter.checked_add(1) {
                Some(counter) => Hlc::new(wall, counter, self.last.device),
                None => Hlc::new(wall.saturating_add(1), 0, self.last.device),
            }
        } else {
            Hlc::new(wall, 0, self.last.device)
        };

        self.last = next;
        next
    }

    /// Moves the clock past a reading received from another device.
    ///
    /// Called with every row a synchronisation brings in. Without it, two devices exchanging
    /// writes would each keep producing readings below what the other has already used, and the
    /// order of the merged result would depend on which side ran the merge.
    pub fn observe(&mut self, seen: Hlc) {
        if seen.wall_ms > self.last.wall_ms {
            self.last = Hlc::new(seen.wall_ms, seen.counter, self.last.device);
        } else if seen.wall_ms == self.last.wall_ms && seen.counter > self.last.counter {
            self.last = Hlc::new(self.last.wall_ms, seen.counter, self.last.device);
        }
    }
}

/// How many times a row has been written.
///
/// Part of what every encrypted value in the row is authenticated against, which is why it is a
/// type of its own rather than a number somebody increments. A revision that went backwards, or
/// that was reused, would let a ciphertext from an earlier revision be put back into the row and
/// still verify, and that is a rollback nobody can detect afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Rev(u64);

impl Rev {
    /// The revision a row is written at the first time.
    pub const FIRST: Self = Self(0);

    /// A revision from the number it is stored as.
    #[must_use]
    pub const fn from_number(value: u64) -> Self {
        Self(value)
    }

    /// The number, for the associated data and for the column.
    #[must_use]
    pub const fn as_number(self) -> u64 {
        self.0
    }

    /// The next revision.
    ///
    /// Saturating. A row written eighteen quintillion times is not a situation worth modelling,
    /// and wrapping back to a revision that has been used before would let an old ciphertext
    /// start verifying again, which is the one thing this type exists to prevent.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// The revision as SQLite stores it.
    ///
    /// Signed, because SQLite has no unsigned integer. Saturating for the same reason as above: a
    /// revision that came back negative is one the codec cannot reproduce, and every encrypted
    /// value in that row would stop opening.
    #[must_use]
    pub fn as_stored(self) -> i64 {
        i64::try_from(self.0).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Clock, DEVICE_LEN, HLC_LEN, Hlc, Rev};

    const DEVICE: [u8; DEVICE_LEN] = [1, 2, 3, 4, 5, 6];
    const OTHER_DEVICE: [u8; DEVICE_LEN] = [9, 9, 9, 9, 9, 9];

    #[test]
    fn a_reading_survives_being_stored_as_sixteen_bytes() {
        let reading = Hlc::new(1_700_000_000_123, 7, DEVICE);
        let bytes = reading.to_bytes();

        assert_eq!(bytes.len(), HLC_LEN);
        assert_eq!(Hlc::from_bytes(bytes), reading);
        assert_eq!(reading.wall_ms(), 1_700_000_000_123);
        assert_eq!(reading.counter(), 7);
        assert_eq!(reading.device(), DEVICE);
    }

    #[test]
    fn two_readings_in_the_same_millisecond_differ_in_the_counter() {
        let mut clock = Clock::starting(DEVICE);

        let first = clock.tick(1_000);
        let second = clock.tick(1_000);

        assert_eq!((first.wall_ms(), first.counter()), (1_000, 0));
        assert_eq!((second.wall_ms(), second.counter()), (1_000, 1));
        assert!(second > first);
    }

    #[test]
    fn a_new_millisecond_puts_the_counter_back_to_zero() {
        let mut clock = Clock::starting(DEVICE);

        clock.tick(1_000);
        clock.tick(1_000);
        let later = clock.tick(1_001);

        assert_eq!((later.wall_ms(), later.counter()), (1_001, 0));
    }

    #[test]
    fn a_wall_clock_that_jumps_backwards_does_not_take_the_reading_with_it() {
        // The failure this whole type exists for. A machine that resynchronises its clock, or
        // one whose battery died, hands back a moment in the past; a reading that followed it
        // would sort before writes that have already happened, and the merge would pick the
        // older one as the winner.
        let mut clock = Clock::starting(DEVICE);

        let before = clock.tick(5_000);
        let after = clock.tick(1_000);

        assert!(after > before, "the reading went backwards with the clock");
        assert_eq!(after.wall_ms(), 5_000);
        assert_eq!(after.counter(), 1);
    }

    #[test]
    fn a_counter_that_runs_out_borrows_the_next_millisecond() {
        let mut clock = Clock::resuming(Hlc::new(1_000, u16::MAX, DEVICE));

        let next = clock.tick(1_000);

        assert_eq!((next.wall_ms(), next.counter()), (1_001, 0));
        assert!(next > Hlc::new(1_000, u16::MAX, DEVICE));
    }

    #[test]
    fn a_clock_that_resumes_does_not_repeat_what_was_already_written() {
        let mut clock = Clock::resuming(Hlc::new(9_000, 3, DEVICE));

        // The moment the operating system reports is older than the newest row in the database,
        // which is the ordinary case after a restart on a machine whose clock is a little slow.
        let next = clock.tick(8_000);

        assert_eq!((next.wall_ms(), next.counter()), (9_000, 4));
    }

    #[test]
    fn a_reading_from_a_peer_moves_this_clock_past_it_without_taking_its_device() {
        let mut clock = Clock::starting(DEVICE);
        clock.tick(1_000);

        clock.observe(Hlc::new(5_000, 8, OTHER_DEVICE));
        let next = clock.tick(1_000);

        assert_eq!(next.wall_ms(), 5_000);
        assert_eq!(next.counter(), 9);
        assert_eq!(
            next.device(),
            DEVICE,
            "this device started signing its writes as another one"
        );
    }

    #[test]
    fn an_older_reading_from_a_peer_changes_nothing() {
        let mut clock = Clock::starting(DEVICE);
        let mine = clock.tick(5_000);

        clock.observe(Hlc::new(1_000, 99, OTHER_DEVICE));

        assert_eq!(clock.last(), mine);
    }

    #[test]
    fn the_device_only_breaks_ties_after_the_wall_reading_and_the_counter() {
        let low = Hlc::new(1_000, 0, [0; DEVICE_LEN]);
        let high = Hlc::new(1_000, 0, [255; DEVICE_LEN]);

        assert!(high > low, "the device does not break the tie");
        assert!(
            Hlc::new(1_000, 1, [0; DEVICE_LEN]) > high,
            "the device outranked the counter"
        );
        assert!(
            Hlc::new(1_001, 0, [0; DEVICE_LEN]) > Hlc::new(1_000, u16::MAX, [255; DEVICE_LEN]),
            "the counter outranked the wall reading"
        );
    }

    #[test]
    fn a_revision_never_goes_backwards_even_at_the_top_of_the_range() {
        assert_eq!(Rev::FIRST.as_number(), 0);
        assert_eq!(Rev::FIRST.next().as_number(), 1);

        let highest = Rev::from_number(u64::MAX);
        assert_eq!(highest.next(), highest);
        assert_eq!(highest.as_stored(), i64::MAX);
    }

    proptest! {
        /// The one property everything else rests on: a reading is always greater than the one
        /// before it, whatever the operating system's clock does between calls.
        #[test]
        fn every_reading_is_greater_than_the_one_before_it(
            moments in proptest::collection::vec(0_u64..u64::MAX, 1..200),
        ) {
            let mut clock = Clock::starting(DEVICE);
            let mut previous = None;

            for moment in moments {
                let reading = clock.tick(moment);
                if let Some(before) = previous {
                    prop_assert!(reading > before, "{reading} did not follow {before}");
                }
                previous = Some(reading);
            }
        }

        /// Sixteen bytes compare as bytes in the same order the readings compare as clocks.
        ///
        /// This is what lets SQLite order, index and page by the column without decoding it, and
        /// it is a property of the layout rather than of the type: swapping two fields would
        /// still round trip and would break this.
        #[test]
        fn the_bytes_sort_in_the_same_order_as_the_readings(
            first_wall in 0_u64..u64::MAX,
            first_counter in 0_u16..u16::MAX,
            first_device in proptest::array::uniform6(0_u8..=255),
            second_wall in 0_u64..u64::MAX,
            second_counter in 0_u16..u16::MAX,
            second_device in proptest::array::uniform6(0_u8..=255),
        ) {
            let first = Hlc::new(first_wall, first_counter, first_device);
            let second = Hlc::new(second_wall, second_counter, second_device);

            prop_assert_eq!(first.cmp(&second), first.to_bytes().cmp(&second.to_bytes()));
        }

        /// Every sixteen bytes are a reading, and the same one when written out again.
        #[test]
        fn any_sixteen_bytes_read_back_as_themselves(
            bytes in proptest::array::uniform16(0_u8..=255),
        ) {
            prop_assert_eq!(Hlc::from_bytes(bytes).to_bytes(), bytes);
        }

        /// Observing a peer never lowers this clock, whatever the peer sends.
        #[test]
        fn observing_a_peer_never_lowers_the_clock(
            wall in 0_u64..u64::MAX,
            counter in 0_u16..u16::MAX,
            seen_wall in 0_u64..u64::MAX,
            seen_counter in 0_u16..u16::MAX,
        ) {
            let mut clock = Clock::resuming(Hlc::new(wall, counter, DEVICE));
            let before = clock.last();

            clock.observe(Hlc::new(seen_wall, seen_counter, OTHER_DEVICE));

            prop_assert!(clock.last() >= before);
            prop_assert_eq!(clock.last().device(), DEVICE);
        }
    }
}
