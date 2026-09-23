//! The closed shape of a habit, built from the open one the schema stores.
//!
//! The table is deliberately permissive: integers with a range check, two columns that may or
//! may not be null, and one column that two different features read in two different ways. That
//! is the right shape for something a second device writes into, because a row that arrives with
//! a value this version does not know about has to be storable before it can be judged.
//!
//! It is the wrong shape for arithmetic. A streak counted over "kind is zero or one" is a streak
//! counted over an integer, and every place that reads it has to remember which integer meant
//! what. So the columns are turned into a type once, here, and everything after this point
//! matches on a variant instead of comparing a number.
//!
//! Nothing is guessed on the way through. A value outside what the product exposes is an error
//! that names the column and what it held, not a default quietly substituted for it: the row was
//! written by something this version does not understand, and pretending otherwise would turn a
//! version mismatch into a wrong number on a screen.

use crate::habits::calendar::Weekday;
use crate::time::CivilDay;

/// Every day of the week set, which is what a schedule of no particular days means.
const WHOLE_WEEK: u8 = 0b111_1111;

/// How often the habit is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// Every day the schedule names is judged on its own.
    Daily,
    /// The week as a whole is judged, against a count of days inside it.
    Weekly,
}

impl Period {
    /// The period the column holds.
    fn from_column(value: i64) -> Result<Self, SpecError> {
        match value {
            0 => Ok(Self::Daily),
            1 => Ok(Self::Weekly),
            other => Err(SpecError::Period { value: other }),
        }
    }
}

/// Which way the target is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// More is better. The day counts when the amount reaches the target.
    AtLeast,
    /// Less is better. The day counts when the amount stays at or under it, and a day with no
    /// entry at all is the best possible day.
    AtMost,
}

impl Direction {
    /// The direction the column holds.
    fn from_column(value: i64) -> Result<Self, SpecError> {
        match value {
            0 => Ok(Self::AtLeast),
            1 => Ok(Self::AtMost),
            other => Err(SpecError::Direction { value: other }),
        }
    }
}

/// How the days of a period combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aggregation {
    /// The amounts of the days are added together.
    Sum,
    /// The largest amount of any day stands for the period.
    Highest,
}

impl Aggregation {
    /// The aggregation the column holds.
    ///
    /// Read for every row, including the ones that count nothing. The schema allows a third
    /// value that this product does not expose, and a habit carrying it is a habit whose days
    /// would combine in a way nothing here implements, whatever its kind says.
    fn from_column(value: i64) -> Result<Self, SpecError> {
        match value {
            0 => Ok(Self::Sum),
            1 => Ok(Self::Highest),
            other => Err(SpecError::Aggregation { value: other }),
        }
    }
}

/// What is being counted, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Measure {
    /// Done or not done. There is no unit and no target.
    DoneOrNot,
    /// A quantity, in the smallest unit the habit counts in.
    Quantity {
        /// What the amounts are counted in, as it is shown next to them.
        unit: String,
        /// How much the period has to reach, or stay under, in that unit.
        target: i64,
        /// How the days of the period combine into the amount that is compared.
        aggregation: Aggregation,
    },
}

impl Measure {
    /// What the kind column, and the two columns that only mean anything beside it, add up to.
    ///
    /// The target column is not part of this decision for a weekly habit that counts nothing:
    /// there the same column holds how many days of the week are expected, which is a different
    /// question with a different answer, read in [`weekly_count`].
    fn from_columns(
        kind: i64,
        period: Period,
        unit: Option<String>,
        target: Option<i64>,
        aggregation: Aggregation,
    ) -> Result<Self, SpecError> {
        match kind {
            0 => {
                if unit.is_some() {
                    return Err(SpecError::MeasureMismatch);
                }
                if matches!(period, Period::Daily) && target.is_some() {
                    return Err(SpecError::MeasureMismatch);
                }

                Ok(Self::DoneOrNot)
            }
            1 => {
                let (Some(unit), Some(target)) = (unit, target) else {
                    return Err(SpecError::QuantityIncomplete);
                };
                if target <= 0 {
                    return Err(SpecError::Target { target });
                }

                Ok(Self::Quantity {
                    unit,
                    target,
                    aggregation,
                })
            }
            other => Err(SpecError::Kind { value: other }),
        }
    }
}

/// Which days of the week the habit is expected on.
///
/// Never empty: a mask of zero means every day, which is what the schema's default means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule(u8);

impl Schedule {
    /// The schedule a mask of seven bits describes, Monday lowest.
    ///
    /// # Errors
    /// [`SpecError::ScheduleMask`] if any bit above the seventh is set.
    pub fn from_mask(mask: u8) -> Result<Self, SpecError> {
        if mask > WHOLE_WEEK {
            return Err(SpecError::ScheduleMask { mask });
        }

        if mask == 0 {
            return Ok(Self(WHOLE_WEEK));
        }

        Ok(Self(mask))
    }

    /// The mask this schedule is stored as, which is never zero.
    #[must_use]
    pub const fn as_mask(self) -> u8 {
        self.0
    }

    /// Whether the habit is expected on that day of the week.
    #[must_use]
    pub const fn includes(self, weekday: Weekday) -> bool {
        self.0 & weekday.bit() != 0
    }

    /// How many days of the week are active. Always between one and seven.
    ///
    /// Counted a bit at a time rather than through `count_ones`, whose answer is a `u32`: the
    /// narrowing would take a cast that is only sound because of the invariant above it, and a
    /// loop over the bits states the same thing without one.
    #[must_use]
    pub const fn active_days(self) -> u8 {
        let mut remaining = self.0;
        let mut count = 0;

        while remaining != 0 {
            count += remaining & 1;
            remaining >>= 1;
        }

        count
    }
}

/// A habit, as the rest of this crate is allowed to see one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HabitSpec {
    /// How often the habit is judged.
    pub period: Period,
    /// What is being counted, if anything.
    pub measure: Measure,
    /// Which way the target is read.
    pub direction: Direction,
    /// Which days of the week the habit is expected on.
    pub schedule: Schedule,
    /// The first day the habit is judged on. Nothing before it counts, in either direction.
    pub started_on: CivilDay,
    /// How many times the period has to be met, for a weekly habit. Always one for a daily one.
    pub target_per_period: u8,
}

/// What a row can be wrong about.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SpecError {
    /// The period column holds something other than daily or weekly.
    #[error("the period column holds {value}, which is not a period this product has")]
    Period {
        /// What the column held.
        value: i64,
    },

    /// The kind column holds something other than done-or-not or a quantity.
    #[error("the kind column holds {value}, which is not a kind this product has")]
    Kind {
        /// What the column held.
        value: i64,
    },

    /// The aggregation column holds something other than the sum or the highest.
    ///
    /// The schema allows a third way of combining days that this product never offered. A row
    /// carrying it came from somewhere else, and reading it as one of the two would silently
    /// change what the habit measures.
    #[error("the aggregation column holds {value}, which is not an aggregation this product has")]
    Aggregation {
        /// What the column held.
        value: i64,
    },

    /// The direction column holds something other than more-is-better or less-is-better.
    #[error("the direction column holds {value}, which is not a direction this product has")]
    Direction {
        /// What the column held.
        value: i64,
    },

    /// The schedule mask has bits set outside the seven days of a week.
    #[error("the schedule mask {mask} has bits above the seven days of a week")]
    ScheduleMask {
        /// The mask that was refused.
        mask: u8,
    },

    /// A habit that counts a quantity is missing its unit, its target, or both.
    #[error("a habit that counts a quantity needs both a unit and a target")]
    QuantityIncomplete,

    /// A habit that counts nothing carries a unit, or a target where none can be read.
    #[error("a habit that is only done or not done may not carry a unit or a target")]
    MeasureMismatch,

    /// The quantity a habit aims at is not a positive number.
    #[error("a target of {target} is not a quantity")]
    Target {
        /// What the column held.
        target: i64,
    },

    /// A weekly habit expects more days than its schedule leaves available.
    #[error("a weekly target of {target} does not fit in {active} active days")]
    WeeklyTarget {
        /// How many days of the week the habit asks for.
        target: u8,
        /// How many days the schedule actually has.
        active: u8,
    },
}

/// The columns of one row, exactly as the schema names them.
#[derive(Debug, Clone)]
pub struct HabitRow {
    /// How often the habit is judged: zero daily, one weekly.
    pub period: i64,
    /// What is counted: zero done-or-not, one a quantity.
    pub kind: i64,
    /// Which way the target is read: zero more is better, one less is better.
    pub direction: i64,
    /// How the days of a period combine: zero the sum, one the highest.
    pub aggregation: i64,
    /// Seven bits, one per day of the week, Monday lowest. Zero means every day.
    pub schedule_mask: i64,
    /// What the amounts are counted in, for a habit that counts a quantity.
    pub unit: Option<String>,
    /// The quantity a habit aims at, or how many days a week a weekly habit is expected on.
    pub target_per_period: Option<i64>,
    /// The first day the habit is judged on.
    pub started_on: CivilDay,
}

impl HabitSpec {
    /// The habit those columns describe.
    ///
    /// # Errors
    /// See [`SpecError`]. Every failure names the column and the value.
    pub fn from_row(row: HabitRow) -> Result<Self, SpecError> {
        let period = Period::from_column(row.period)?;
        let direction = Direction::from_column(row.direction)?;
        let aggregation = Aggregation::from_column(row.aggregation)?;
        let schedule = schedule_from_column(row.schedule_mask)?;
        let measure = Measure::from_columns(
            row.kind,
            period,
            row.unit,
            row.target_per_period,
            aggregation,
        )?;
        let target_per_period = weekly_count(period, &measure, row.target_per_period, schedule)?;

        Ok(Self {
            period,
            measure,
            direction,
            schedule,
            started_on: row.started_on,
            target_per_period,
        })
    }
}

/// The schedule the mask column describes.
///
/// The mask and its refusal are both written in a byte, and the column is not one. A number
/// outside a byte is outside the seven bits a week has, at either end, so it is refused here
/// rather than narrowed into one that might fit: a wrapped number could land back inside the
/// seven bits and turn a refusal into an acceptance. The number the message then names is the
/// largest a mask can be rather than what the column held, which is the one thing about this
/// path that is approximate; the schema's own range check keeps it out of reach for a row this
/// application wrote.
fn schedule_from_column(value: i64) -> Result<Schedule, SpecError> {
    match u8::try_from(value) {
        Ok(mask) => Schedule::from_mask(mask),
        Err(_outside_a_byte) => Err(SpecError::ScheduleMask { mask: u8::MAX }),
    }
}

/// How many days of the week a weekly habit is expected on.
///
/// One for everything else, and that includes a weekly habit that counts a quantity: there the
/// column already holds the quantity, and there is nowhere left in the row to put the count of
/// days. Splitting the two readings apart is a column the schema does not have yet, so a weekly
/// habit that counts is judged once a week until it does.
fn weekly_count(
    period: Period,
    measure: &Measure,
    column: Option<i64>,
    schedule: Schedule,
) -> Result<u8, SpecError> {
    if !matches!((period, measure), (Period::Weekly, Measure::DoneOrNot)) {
        return Ok(1);
    }

    let Some(value) = column else {
        return Ok(1);
    };

    let active = schedule.active_days();
    // Capped at both ends for the same reason the mask is: every number outside the byte is
    // outside the one to seven a week can hold, and a wrapped one could land back inside it.
    let target = match u8::try_from(value) {
        Ok(count) => count,
        Err(_outside_a_byte) if value.is_negative() => 0,
        Err(_outside_a_byte) => u8::MAX,
    };

    if target < 1 || target > active {
        return Err(SpecError::WeeklyTarget { target, active });
    }

    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::{
        Aggregation, Direction, HabitRow, HabitSpec, Measure, Period, Schedule, SpecError,
        WHOLE_WEEK,
    };
    use crate::habits::calendar::Weekday;
    use crate::time::CivilDay;

    /// Monday, Wednesday and Friday, which is the schedule the weekly cases are judged against.
    const MONDAY_WEDNESDAY_FRIDAY: i64 = 0b001_0101;

    /// The whole week of days, in the order the mask counts them.
    const EVERY_WEEKDAY: [Weekday; 7] = [
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ];

    /// The plainest row there is: daily, done or not done, more is better, no fixed days.
    ///
    /// Every test below changes the one column it is about and leaves the rest alone, so a
    /// failure names the column rather than the combination.
    fn row() -> HabitRow {
        HabitRow {
            period: 0,
            kind: 0,
            direction: 0,
            aggregation: 0,
            schedule_mask: 0,
            unit: None,
            target_per_period: None,
            started_on: CivilDay::new(2026, 1, 1).expect("the first of January exists"),
        }
    }

    #[test]
    fn a_daily_yes_or_no_habit_with_no_fixed_days_is_expected_every_day() {
        let spec = HabitSpec::from_row(row()).expect("the plainest row there is");

        assert_eq!(spec.period, Period::Daily);
        assert_eq!(spec.measure, Measure::DoneOrNot);
        assert_eq!(spec.direction, Direction::AtLeast);
        assert_eq!(spec.schedule.active_days(), 7);
        assert_eq!(spec.target_per_period, 1);
    }

    #[test]
    fn a_daily_habit_that_counts_carries_its_unit_target_and_aggregation() {
        let spec = HabitSpec::from_row(HabitRow {
            kind: 1,
            unit: Some("ml".to_owned()),
            target_per_period: Some(2000),
            ..row()
        })
        .expect("a quantity with both of the columns it needs");

        assert_eq!(
            spec.measure,
            Measure::Quantity {
                unit: "ml".to_owned(),
                target: 2000,
                aggregation: Aggregation::Sum,
            }
        );
        assert_eq!(spec.target_per_period, 1, "a daily habit is judged once");
    }

    #[test]
    fn a_weekly_habit_is_expected_as_many_days_as_its_column_says() {
        let spec = HabitSpec::from_row(HabitRow {
            period: 1,
            schedule_mask: i64::from(WHOLE_WEEK),
            target_per_period: Some(3),
            ..row()
        })
        .expect("three days out of seven fit");

        assert_eq!(spec.period, Period::Weekly);
        assert_eq!(spec.target_per_period, 3);
    }

    #[test]
    fn a_weekly_habit_may_not_ask_for_more_days_than_its_schedule_has() {
        let refused = HabitSpec::from_row(HabitRow {
            period: 1,
            schedule_mask: MONDAY_WEDNESDAY_FRIDAY,
            target_per_period: Some(5),
            ..row()
        });

        assert_eq!(
            refused,
            Err(SpecError::WeeklyTarget {
                target: 5,
                active: 3,
            })
        );
    }

    #[test]
    fn a_habit_that_counts_without_a_unit_is_not_a_habit_that_counts() {
        let refused = HabitSpec::from_row(HabitRow {
            kind: 1,
            target_per_period: Some(2000),
            ..row()
        });

        assert_eq!(refused, Err(SpecError::QuantityIncomplete));
    }

    #[test]
    fn a_habit_that_counts_without_a_target_is_not_a_habit_that_counts() {
        let refused = HabitSpec::from_row(HabitRow {
            kind: 1,
            unit: Some("ml".to_owned()),
            ..row()
        });

        assert_eq!(refused, Err(SpecError::QuantityIncomplete));
    }

    #[test]
    fn a_yes_or_no_habit_may_not_carry_a_unit() {
        let refused = HabitSpec::from_row(HabitRow {
            unit: Some("ml".to_owned()),
            ..row()
        });

        assert_eq!(refused, Err(SpecError::MeasureMismatch));
    }

    #[test]
    fn a_daily_yes_or_no_habit_may_not_carry_a_target_either() {
        let refused = HabitSpec::from_row(HabitRow {
            target_per_period: Some(3),
            ..row()
        });

        assert_eq!(
            refused,
            Err(SpecError::MeasureMismatch),
            "a daily habit has no second reading for that column"
        );
    }

    #[test]
    fn a_target_that_is_not_a_positive_number_is_not_a_quantity() {
        for target in [0, -1, -2000] {
            let refused = HabitSpec::from_row(HabitRow {
                kind: 1,
                unit: Some("ml".to_owned()),
                target_per_period: Some(target),
                ..row()
            });

            assert_eq!(refused, Err(SpecError::Target { target }));
        }
    }

    #[test]
    fn a_period_this_product_does_not_have_is_refused_rather_than_read_as_one_it_does() {
        let refused = HabitSpec::from_row(HabitRow { period: 2, ..row() });

        assert_eq!(refused, Err(SpecError::Period { value: 2 }));
    }

    #[test]
    fn the_third_aggregation_the_schema_allows_is_one_this_product_never_offered() {
        let refused = HabitSpec::from_row(HabitRow {
            aggregation: 2,
            ..row()
        });

        assert_eq!(refused, Err(SpecError::Aggregation { value: 2 }));
    }

    #[test]
    fn a_direction_outside_the_two_is_refused() {
        let refused = HabitSpec::from_row(HabitRow {
            direction: 7,
            ..row()
        });

        assert_eq!(refused, Err(SpecError::Direction { value: 7 }));
    }

    #[test]
    fn a_mask_with_bits_above_the_week_is_refused() {
        for mask in [128, 255] {
            let refused = HabitSpec::from_row(HabitRow {
                schedule_mask: mask,
                ..row()
            });

            assert_eq!(
                refused,
                Err(SpecError::ScheduleMask {
                    mask: u8::try_from(mask).expect("the test named a mask that fits in a byte"),
                })
            );
        }
    }

    #[test]
    fn a_mask_of_nothing_is_a_mask_of_every_day() {
        let schedule = Schedule::from_mask(0).expect("no fixed days is every day");

        assert_eq!(schedule.active_days(), 7);
        assert_eq!(schedule.as_mask(), WHOLE_WEEK);
        for weekday in EVERY_WEEKDAY {
            assert!(
                schedule.includes(weekday),
                "{weekday:?} is not in a schedule of every day"
            );
        }
    }

    #[test]
    fn a_mask_of_two_days_holds_those_two_and_no_others() {
        let schedule = Schedule::from_mask(0b000_0101).expect("Monday and Wednesday");

        assert_eq!(schedule.active_days(), 2);
        for weekday in EVERY_WEEKDAY {
            let expected = matches!(weekday, Weekday::Monday | Weekday::Wednesday);

            assert_eq!(
                schedule.includes(weekday),
                expected,
                "{weekday:?} is on the wrong side of the mask"
            );
        }
    }

    #[test]
    fn a_habit_somebody_is_cutting_down_is_the_same_target_read_the_other_way_round() {
        let spec = HabitSpec::from_row(HabitRow {
            direction: 1,
            ..row()
        })
        .expect("less is better is a direction this product has");

        assert_eq!(spec.direction, Direction::AtMost);
        assert_eq!(spec.measure, Measure::DoneOrNot);
    }
}
