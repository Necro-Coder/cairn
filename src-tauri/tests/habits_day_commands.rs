//! Drives the three habit commands about a single day against a real vault and a real calendar.
//!
//! What the domain means by a square, a run and a percentage is settled by the unit tests of the
//! domain. What is settled here is the sequence these three commands actually run: read the
//! zone, read the preference, refuse a day nobody may write to, open one transaction, read the
//! mark, write the mark, and classify what is left. That sequence is where the mistakes that
//! matter live, because every one of them is invisible from any single crate.
//!
//! Three assertions are worth naming. A day outside the thirty that may be marked must be
//! refused with nothing written, because a refusal that still wrote is worse than no limit at
//! all. A quantity must keep the target it was judged by, so that raising the goal never turns
//! a year of met days red. And the heat map must answer for one year and still know the first
//! year of ten, which is a pair of facts a single read of one year could not produce: it is the
//! only thing observable from outside this process about how many reads the history took.
//!
//! The zone and the moment are parameters everywhere below, so the same assertions hold in every
//! season and on any build agent.
// Every function in an integration test file is test code, but the lint that forbids panicking
// constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]` functions. The
// helpers below are neither, and a helper that cannot panic would have to return a Result that
// every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use cairn_clock::{CivilClock as _, DayStart, SystemZone, zone_named};
use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES};
use cairn_db::repositories::habits::{self as repository, Mark};
use cairn_db::{Connection, DbError, FieldCodec};
use cairn_domain::habits::calendar::shift;
use cairn_domain::{CivilDay, Timestamp};
use cairn_lib::commands::habits::{
    DayCell, DayStateDto, HabitDraft, HabitsError, create, heatmap, stats, toggle_day, update,
};
use cairn_lib::commands::vault::create as create_vault;
use cairn_lib::state::AppState;
use cairn_lib::storage::{DataDirectory, Storage};
use cairn_lib::vault::Vault;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Not a real password. A phrase invented for these tests, in a directory removed afterwards.
const NOT_A_REAL_PASSWORD: &str = "una frase larga inventada para la prueba";

/// A moment in the middle of the range, so the arithmetic either side of it is ordinary.
///
/// The fourteenth of November 2023, late in the evening in UTC, which is the zone every test
/// below names. Fixed rather than read, so the calendar these assertions walk is the same one in
/// every season and on every machine.
const NOW_US: i64 = 1_700_000_000_000_000;

/// The same evening, thirteen days earlier, which lands on the first of the month.
///
/// The month percentage is the one number whose answer depends on how much of the month is
/// over, so it needs a day where that is a known quantity rather than whatever today happens
/// to be.
const NOW_ON_THE_FIRST_US: i64 = 1_698_876_800_000_000;

/// The zone every test names, so that "today" is a fact rather than a property of the agent.
const TEST_ZONE: &str = "UTC";

/// A year in the middle of the ten the history tests build, and not a leap year.
const MIDDLE_YEAR: u16 = 2019;

/// The first year of that history.
const FIRST_YEAR: u16 = 2014;

/// The thirty-first of February: eight digits in the shape of a date, and not a day.
const NOT_A_DAY: u32 = 20_260_231;

/// A scratch directory that belongs to one test and is removed when it ends.
struct Scratch {
    directory: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);

        Self {
            directory: std::env::temp_dir().join(format!(
                "cairn-habit-days-{name}-{}-{unique}",
                std::process::id()
            )),
        }
    }

    /// A fresh application state over the same directory, with no vault opened.
    fn state(&self) -> AppState {
        AppState::new(
            Vault::open_at(&self.directory).expect("the directory can be read"),
            DataDirectory::new(self.directory.clone()),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// The cheapest parameters the cryptographic crate accepts.
fn cheap() -> Argon2Params {
    Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES).expect("the lowest accepted values")
}

/// A created, open vault in a scratch directory.
fn unlocked(scratch: &Scratch) -> AppState {
    let state = scratch.state();
    let status = tauri::async_runtime::block_on(create_vault(
        &state,
        Zeroizing::new(NOT_A_REAL_PASSWORD.to_owned()),
        cheap(),
        NOW_US,
    ))
    .expect("a vault can be created in an empty directory");

    assert!(status.unlocked, "the vault has to be open for these tests");

    state
}

/// The zone these tests count in.
fn zone() -> SystemZone {
    zone_named(TEST_ZONE).expect("the bundled time zone database names UTC")
}

/// The day a given moment falls on in that zone.
fn day_at(now: i64) -> CivilDay {
    zone()
        .day_of(Timestamp::from_micros(now), DayStart::MIDNIGHT)
        .expect("a moment in 2023 is a day this calendar can name")
}

/// The day [`NOW_US`] falls on, which is what every test below calls today.
fn today() -> CivilDay {
    day_at(NOW_US)
}

/// The day a given number of days before today.
fn days_ago(count: i32) -> CivilDay {
    shift(today(), -count).expect("a day in this millennium has a predecessor")
}

/// A day of a year this calendar can name.
fn day(year: u16, month: u8, number: u8) -> CivilDay {
    CivilDay::new(year, month, number).expect("the test named a day that exists")
}

/// Runs something against the open database, for the assertions that read rows directly.
fn in_database<T>(
    state: &AppState,
    work: impl FnOnce(&Storage, &FieldCodec<'_>, &Connection) -> Result<T, DbError>,
) -> Result<T, DbError> {
    state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage
                .database()
                .with(|connection| work(storage, &codec, connection))
        })
        .expect("the vault is open")
}

/// A draft of the simplest habit there is, done or not, every day, starting today.
fn plain_draft(name: &str) -> HabitDraft {
    HabitDraft {
        name: name.to_owned(),
        notes: None,
        icon: None,
        color: None,
        period: "daily".to_owned(),
        unit: None,
        target: None,
        aggregation: "sum".to_owned(),
        direction: "atLeast".to_owned(),
        schedule_mask: 0,
        started_on: today().as_number(),
    }
}

/// A draft of a habit that counts millilitres towards a target.
fn quantity_draft(target: i64) -> HabitDraft {
    HabitDraft {
        unit: Some("ml".to_owned()),
        target: Some(target),
        ..plain_draft("Beber agua")
    }
}

/// Writes a habit through the command and answers its identifier.
fn written(state: &AppState, draft: &HabitDraft) -> Uuid {
    let id = create(state, &zone(), draft, NOW_US)
        .expect("a valid draft is written")
        .id;

    Uuid::parse_str(&id).expect("the command answers with a hyphenated UUID")
}

/// A habit that started a given number of days ago, done or not, every day.
fn habit_started(state: &AppState, days: i32) -> Uuid {
    let mut draft = plain_draft("Leer");
    draft.started_on = days_ago(days).as_number();

    written(state, &draft)
}

/// How many live marks one habit carries, and the two days its history reaches between.
fn history(state: &AppState, habit: Uuid) -> repository::HistorySpan {
    in_database(state, |_storage, _codec, connection| {
        repository::history_span(connection, habit)
    })
    .expect("the span of a history can be read")
}

/// The mark one day carries, if it carries one.
fn mark_on(state: &AppState, habit: Uuid, on: CivilDay) -> Option<repository::StoredEntry> {
    in_database(state, |_storage, _codec, connection| {
        repository::window(connection, habit, on, on)
    })
    .expect("one day of a window can be read")
    .into_iter()
    .next()
}

/// Marks the given days directly, for the histories a command could never write.
fn mark_days(state: &AppState, habit: Uuid, days: &[CivilDay]) {
    state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage.database().in_transaction(|transaction| {
                for marked in days {
                    let hlc = storage.next_hlc(1);
                    repository::mark(
                        transaction,
                        &codec,
                        storage.device(),
                        hlc,
                        NOW_US,
                        Mark {
                            habit_id: habit,
                            day: *marked,
                            amount: 1,
                            note: None,
                            target_snapshot: None,
                        },
                    )?;
                }

                Ok(())
            })
        })
        .expect("the vault is open")
        .expect("a set of days can be marked");
}

/// A habit that started at the beginning of [`FIRST_YEAR`], marked once in each of ten years.
///
/// One mark per year rather than a full decade of them, because what these tests are about is
/// which years a call reads, not how many rows a year holds.
fn ten_years_of_marks(state: &AppState) -> Uuid {
    let mut draft = plain_draft("Correr");
    draft.started_on = day(FIRST_YEAR, 1, 1).as_number();
    let id = written(state, &draft);

    let days: Vec<CivilDay> = (FIRST_YEAR..=today().year())
        .map(|year| day(year, 6, 15))
        .collect();
    mark_days(state, id, &days);

    id
}

/// Every problem a refusal carries, or a failure naming what came back instead.
fn problems_of(outcome: &HabitsError) -> &[cairn_lib::commands::habits::FieldProblem] {
    match outcome {
        HabitsError::Invalid { problems } => problems,
        other => panic!("expected a refusal naming a field, got {other:?}"),
    }
}

/// Whether a refusal names that field.
fn names_field(outcome: &HabitsError, field: &str) -> bool {
    problems_of(outcome)
        .iter()
        .any(|problem| problem.field == field)
}

#[test]
fn all_three_refuse_while_the_vault_is_locked() {
    let scratch = Scratch::new("locked");
    let state = scratch.state();
    let id = Uuid::new_v4().to_string();

    // Every argument is a valid one on purpose. A refusal about the day or the year would hide
    // the fact that nothing here should have reached the point of judging it.
    assert_eq!(
        toggle_day(&state, &zone(), &id, today().as_number(), None, NOW_US).unwrap_err(),
        HabitsError::Locked
    );
    assert_eq!(
        heatmap(&state, &zone(), &id, today().year(), NOW_US).unwrap_err(),
        HabitsError::Locked
    );
    assert_eq!(
        stats(&state, &zone(), &id, NOW_US).unwrap_err(),
        HabitsError::Locked
    );
}

#[test]
fn marking_today_on_a_habit_that_is_simply_done_writes_the_day() {
    let scratch = Scratch::new("mark-today");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 0);

    let state_of_day = toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        today().as_number(),
        None,
        NOW_US,
    )
    .expect("today may be marked");

    assert_eq!(
        state_of_day,
        DayStateDto::Done {
            amount: 1,
            target: 1
        }
    );
    assert!(
        mark_on(&state, id, today()).is_some(),
        "the day the command reported as done has to carry a row"
    );
}

#[test]
fn marking_today_twice_leaves_the_day_and_the_table_as_they_were() {
    let scratch = Scratch::new("mark-twice");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 0);

    let mark = |state: &AppState| {
        toggle_day(
            state,
            &zone(),
            &id.to_string(),
            today().as_number(),
            None,
            NOW_US,
        )
        .expect("today may be marked")
    };

    mark(&state);
    let second = mark(&state);

    assert_eq!(
        second,
        DayStateDto::Missed {
            amount: 0,
            target: 1
        },
        "the second tap takes the mark off, and an unmarked day of a daily habit is a miss"
    );
    assert_eq!(
        history(&state, id).entries,
        0,
        "unmarking leaves no live row behind for that day"
    );
}

#[test]
fn marking_today_three_times_leaves_exactly_one_live_row() {
    let scratch = Scratch::new("mark-thrice");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 0);

    let mut last = None;
    for _tap in 0..3 {
        last = Some(
            toggle_day(
                &state,
                &zone(),
                &id.to_string(),
                today().as_number(),
                None,
                NOW_US,
            )
            .expect("today may be marked"),
        );
    }

    assert_eq!(
        last,
        Some(DayStateDto::Done {
            amount: 1,
            target: 1
        })
    );
    // One row and not two. Marking a day that was already marked revises the row it found, and
    // two live marks on one square is not a state the calendar could draw.
    assert_eq!(history(&state, id).entries, 1);
}

#[test]
fn tomorrow_is_refused_and_writes_nothing() {
    let scratch = Scratch::new("tomorrow");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 0);
    let tomorrow = shift(today(), 1).expect("today has a successor");

    let refusal = toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        tomorrow.as_number(),
        None,
        NOW_US,
    )
    .unwrap_err();

    assert_eq!(refusal, HabitsError::DayInFuture);
    assert_eq!(
        history(&state, id).entries,
        0,
        "a refusal that still wrote would be worse than no limit at all"
    );
}

#[test]
fn thirty_days_back_may_still_be_filled_in_and_thirty_one_may_not() {
    let scratch = Scratch::new("markable-window");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 60);

    let marked = toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        days_ago(30).as_number(),
        None,
        NOW_US,
    )
    .expect("the oldest markable day is markable");

    assert_eq!(
        marked,
        DayStateDto::Done {
            amount: 1,
            target: 1
        }
    );

    let refusal = toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        days_ago(31).as_number(),
        None,
        NOW_US,
    )
    .unwrap_err();

    assert_eq!(refusal, HabitsError::DayTooOld);
    assert_eq!(
        history(&state, id).entries,
        1,
        "only the day that was accepted is written"
    );
}

#[test]
fn a_number_that_is_not_a_date_names_the_day_field() {
    let scratch = Scratch::new("not-a-date");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 0);

    let refusal =
        toggle_day(&state, &zone(), &id.to_string(), NOT_A_DAY, None, NOW_US).unwrap_err();

    assert!(names_field(&refusal, "day"), "{refusal:?}");
}

#[test]
fn a_negative_amount_names_the_amount_field() {
    let scratch = Scratch::new("negative");
    let state = unlocked(&scratch);
    let id = written(&state, &quantity_draft(2000));

    let refusal = toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        today().as_number(),
        Some(-1),
        NOW_US,
    )
    .unwrap_err();

    assert!(names_field(&refusal, "amount"), "{refusal:?}");
    assert_eq!(history(&state, id).entries, 0, "nothing was written");
}

#[test]
fn an_amount_beyond_what_anybody_counts_names_the_amount_field() {
    let scratch = Scratch::new("enormous");
    let state = unlocked(&scratch);
    let id = written(&state, &quantity_draft(2000));

    // The column has no ceiling of its own, and this command is the first path to it from the
    // other side of the bridge. Refused here so that the day a period's days are added together
    // there is nothing stored that could overflow when they are.
    let refusal = toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        today().as_number(),
        Some(i64::MAX),
        NOW_US,
    )
    .unwrap_err();

    assert!(names_field(&refusal, "amount"), "{refusal:?}");
    assert_eq!(history(&state, id).entries, 0, "nothing was written");
}

#[test]
fn a_habit_that_counts_a_quantity_cannot_be_toggled_without_one() {
    let scratch = Scratch::new("no-amount");
    let state = unlocked(&scratch);
    let id = written(&state, &quantity_draft(2000));

    let refusal = toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        today().as_number(),
        None,
        NOW_US,
    )
    .unwrap_err();

    assert!(names_field(&refusal, "amount"), "{refusal:?}");
    assert_eq!(
        history(&state, id).entries,
        0,
        "a quantity nobody named is not a quantity to invent"
    );
}

#[test]
fn a_quantity_keeps_the_target_it_was_judged_by_when_the_goal_moves() {
    let scratch = Scratch::new("snapshot");
    let state = unlocked(&scratch);
    let id = written(&state, &quantity_draft(2000));

    let met = toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        today().as_number(),
        Some(2000),
        NOW_US,
    )
    .expect("two litres of a two litre habit");

    assert_eq!(
        met,
        DayStateDto::Done {
            amount: 2000,
            target: 2000
        }
    );
    assert_eq!(
        mark_on(&state, id, today())
            .expect("the day carries a row")
            .target_snapshot,
        Some(2000),
        "the row remembers what it was judged by"
    );

    update(
        &state,
        &zone(),
        &id.to_string(),
        &quantity_draft(3000),
        NOW_US,
    )
    .expect("the goal may be raised");

    let year = heatmap(&state, &zone(), &id.to_string(), today().year(), NOW_US)
        .expect("the year is drawn");
    let square = year
        .days
        .into_iter()
        .find(|cell| cell.day == today().as_number())
        .expect("today is one of the squares of this year");

    assert_eq!(
        square.state,
        DayStateDto::Done {
            amount: 2000,
            target: 2000
        },
        "raising the goal must not turn a day that was met into one that was not"
    );
}

#[test]
fn an_amount_of_zero_clears_the_day_whatever_the_habit_counts() {
    let scratch = Scratch::new("clear");
    let state = unlocked(&scratch);
    let id = written(&state, &quantity_draft(2000));

    toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        today().as_number(),
        Some(2000),
        NOW_US,
    )
    .expect("the day may be marked");

    let cleared = toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        today().as_number(),
        Some(0),
        NOW_US,
    )
    .expect("the day may be cleared");

    assert_eq!(
        cleared,
        DayStateDto::Missed {
            amount: 0,
            target: 2000
        }
    );
    assert_eq!(history(&state, id).entries, 0, "no live row is left");
}

#[test]
fn an_identifier_that_names_nothing_is_not_found() {
    let scratch = Scratch::new("no-habit");
    let state = unlocked(&scratch);

    let refusal = toggle_day(
        &state,
        &zone(),
        &Uuid::new_v4().to_string(),
        today().as_number(),
        None,
        NOW_US,
    )
    .unwrap_err();

    assert_eq!(refusal, HabitsError::NotFound);
}

#[test]
fn a_leap_year_has_one_square_more_than_an_ordinary_one() {
    let scratch = Scratch::new("year-lengths");
    let state = unlocked(&scratch);
    let id = ten_years_of_marks(&state);

    let leap = heatmap(&state, &zone(), &id.to_string(), 2020, NOW_US).expect("a leap year");
    let ordinary =
        heatmap(&state, &zone(), &id.to_string(), 2021, NOW_US).expect("an ordinary one");

    assert_eq!(leap.days.len(), 366);
    assert_eq!(ordinary.days.len(), 365);

    // The same holds for a year nobody has lived yet, because the length of a year is a fact
    // about the calendar and not about the history.
    let ahead = heatmap(&state, &zone(), &id.to_string(), 2024, NOW_US).expect("a year ahead");
    let after = heatmap(&state, &zone(), &id.to_string(), 2025, NOW_US).expect("the one after");

    assert_eq!(ahead.days.len(), 366);
    assert_eq!(after.days.len(), 365);
}

#[test]
fn a_year_before_the_habit_existed_is_every_square_empty() {
    let scratch = Scratch::new("before-the-start");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 0);

    let earlier = heatmap(&state, &zone(), &id.to_string(), today().year() - 1, NOW_US)
        .expect("a year before the habit started is still a year");

    assert_eq!(earlier.days.len(), 365);
    assert!(
        earlier
            .days
            .iter()
            .all(|cell| cell.state == DayStateDto::NoData),
        "nothing before a habit existed can be judged, in either direction"
    );
}

#[test]
fn the_squares_after_today_say_nothing_at_all() {
    let scratch = Scratch::new("rest-of-the-year");
    let state = unlocked(&scratch);
    // Started on the first of January, so that every square up to today is one the habit was
    // alive for and the only reason a square can say nothing is that nobody has lived it yet.
    let mut draft = plain_draft("Estirar");
    draft.started_on = day(today().year(), 1, 1).as_number();
    let id = written(&state, &draft);

    let year = heatmap(&state, &zone(), &id.to_string(), today().year(), NOW_US)
        .expect("the year in progress is drawn");

    let (lived, ahead): (Vec<&DayCell>, Vec<&DayCell>) = year
        .days
        .iter()
        .partition(|cell| cell.day <= today().as_number());

    assert!(
        !lived.is_empty() && !ahead.is_empty(),
        "the year is in progress"
    );
    assert!(
        ahead.iter().all(|cell| cell.state == DayStateDto::NoData),
        "a day nobody has lived yet cannot have been missed"
    );
    assert!(
        lived.iter().all(|cell| cell.state != DayStateDto::NoData),
        "every day the habit has lived through says something"
    );
}

#[test]
fn a_year_outside_the_calendar_names_the_year_field() {
    let scratch = Scratch::new("year-range");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 0);

    for year in [0, 10_000] {
        let refusal = heatmap(&state, &zone(), &id.to_string(), year, NOW_US).unwrap_err();

        assert!(names_field(&refusal, "year"), "{year}: {refusal:?}");
    }
}

#[test]
fn one_year_of_a_ten_year_history_carries_that_year_and_the_first_year_of_all() {
    let scratch = Scratch::new("ten-years");
    let state = unlocked(&scratch);
    let id = ten_years_of_marks(&state);

    let year = heatmap(&state, &zone(), &id.to_string(), MIDDLE_YEAR, NOW_US)
        .expect("a year in the middle of the history");

    // The vector is that year and no other, which is what a single read of one year produces
    // and a read of the history never would.
    assert_eq!(year.days.len(), 365);
    let first_of_year = day(MIDDLE_YEAR, 1, 1).as_number();
    let last_of_year = day(MIDDLE_YEAR, 12, 31).as_number();
    assert!(
        year.days
            .iter()
            .all(|cell| cell.day >= first_of_year && cell.day <= last_of_year),
        "a square of a neighbouring year reached the vector"
    );

    let met: Vec<u32> = year
        .days
        .iter()
        .filter(|cell| matches!(cell.state, DayStateDto::Done { .. }))
        .map(|cell| cell.day)
        .collect();
    assert_eq!(
        met,
        vec![day(MIDDLE_YEAR, 6, 15).as_number()],
        "only the mark of this year counts towards this year"
    );

    // And the second read happened. No call about the year above could know this, so the pair
    // of facts is the only thing observable from outside about how the history was read.
    assert_eq!(year.first_year_with_data, Some(FIRST_YEAR));
}

#[test]
fn the_longest_run_is_never_shorter_than_the_one_running_now() {
    let scratch = Scratch::new("longest");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 60);

    // Five days ending today, and a longer run of seven that ended a fortnight ago.
    let days: Vec<CivilDay> = (0..5).chain(10..17).map(days_ago).collect();
    mark_days(&state, id, &days);

    let numbers = stats(&state, &zone(), &id.to_string(), NOW_US).expect("the numbers");

    assert_eq!(numbers.current.days, 5, "today and the four days before it");
    assert_eq!(
        numbers.longest, 7,
        "the run that is over was the longer one"
    );
    assert!(
        numbers.longest >= numbers.current.days,
        "a record cannot be shorter than the run it is a record over"
    );
}

#[test]
fn a_habit_nobody_has_marked_has_no_first_day_and_no_last() {
    let scratch = Scratch::new("no-history");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 30);

    let numbers = stats(&state, &zone(), &id.to_string(), NOW_US).expect("the numbers");

    assert_eq!(numbers.longest, 0);
    assert_eq!(numbers.total_entries, 0);
    assert_eq!(numbers.first_day, None);
    assert_eq!(numbers.last_day, None);
}

#[test]
fn a_habit_dated_to_the_first_year_of_the_calendar_is_still_bounded_work() {
    let scratch = Scratch::new("absurd-start");
    let state = unlocked(&scratch);

    // The first day the calendar can name. It reaches this command from a form on the other
    // side of the bridge, so it is a number somebody can choose rather than one the product
    // produces, and without a bound it would be two thousand statements and a vector of three
    // and a half million squares.
    let mut draft = plain_draft("Respirar");
    draft.started_on = day(1, 1, 1).as_number();
    let id = written(&state, &draft);
    mark_days(&state, id, &[day(1850, 3, 1), today()]);

    let numbers = stats(&state, &zone(), &id.to_string(), NOW_US).expect("the numbers");

    // The two extremes and the count are exact whatever the bound is: they come from three
    // aggregates over the whole table, not from the span that was walked.
    assert_eq!(numbers.first_day, Some(day(1850, 3, 1).as_number()));
    assert_eq!(numbers.last_day, Some(today().as_number()));
    assert_eq!(numbers.total_entries, 2);

    // And the visible consequence of the bound: a mark from before the window counts for
    // nothing, so the record is the one day inside it.
    assert_eq!(numbers.current.days, 1);
    assert_eq!(numbers.longest, 1);
}

#[test]
fn ten_years_of_marks_are_counted_whole_and_not_to_the_edge_of_a_window() {
    let scratch = Scratch::new("whole-history");
    let state = unlocked(&scratch);
    let id = ten_years_of_marks(&state);

    let numbers = stats(&state, &zone(), &id.to_string(), NOW_US).expect("the numbers");

    // Ten years is far beyond the seven hundred and sixty-six days one window may span, so
    // these three numbers are only reachable by reading the history whole.
    assert_eq!(numbers.first_day, Some(day(FIRST_YEAR, 6, 15).as_number()));
    assert_eq!(
        numbers.last_day,
        Some(day(today().year(), 6, 15).as_number())
    );
    assert_eq!(numbers.total_entries, 10);
}

#[test]
fn the_month_percentage_on_the_first_of_the_month_is_about_that_one_day() {
    let scratch = Scratch::new("month");
    let state = unlocked(&scratch);

    let first = day_at(NOW_ON_THE_FIRST_US);
    let mut draft = plain_draft("Meditar");
    draft.started_on = first.as_number();
    let id = written(&state, &draft);

    // Nothing marked yet. The denominator of the month is the days that are over, and a day
    // still being lived is only over once it has been met, so both numbers are zero rather
    // than a percentage that falls every midnight.
    let empty = stats(&state, &zone(), &id.to_string(), NOW_ON_THE_FIRST_US).expect("the numbers");

    assert_eq!(empty.month_completion.done, 0);
    assert_eq!(empty.month_completion.of, 0);
    assert_eq!(empty.month_completion.percent, 0);

    toggle_day(
        &state,
        &zone(),
        &id.to_string(),
        first.as_number(),
        None,
        NOW_ON_THE_FIRST_US,
    )
    .expect("the first of the month may be marked");

    let met = stats(&state, &zone(), &id.to_string(), NOW_ON_THE_FIRST_US).expect("the numbers");

    assert_eq!(met.month_completion.done, 1);
    assert_eq!(met.month_completion.of, 1);
    assert_eq!(met.month_completion.percent, 100);
}
