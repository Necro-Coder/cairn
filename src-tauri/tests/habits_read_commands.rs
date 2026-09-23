//! Drives the three habit commands against a real vault, a real database and a real calendar.
//!
//! The unit tests in the domain cover what a streak is. This covers the sequence the commands
//! actually run: read the zone, read the preference, page the habits, walk the calendar, classify
//! it, and answer. That sequence is where the mistakes that matter live, because every one of
//! them is invisible from any single crate.
//!
//! Three of them are worth naming. A list must never carry a note, and that is asserted against
//! the text of the serialised answer rather than against the type, because the type is exactly
//! what somebody adds a field to. A streak longer than one window must come back whole, and a
//! streak longer than two windows must stop at two, because between them those two numbers pin
//! the query budget from the outside: neither is reachable by any other number of reads. And a
//! row this build cannot understand must be left out of the list without taking the list with
//! it, because a habit that arrived from a newer device is not a reason for somebody to lose
//! their screen.
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
use cairn_db::repositories::habits::{self as repository, Mark, NewHabit};
use cairn_db::{Connection, DbError, FieldCodec};
use cairn_domain::habits::calendar::shift;
use cairn_domain::{CivilDay, Timestamp};
use cairn_lib::commands::habits::{
    DayStateDto, HabitDraft, HabitFilter, HabitsError, create, get, list, toggle_day,
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

/// The zone every test names, so that "today" is a fact rather than a property of the agent.
const TEST_ZONE: &str = "UTC";

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
                "cairn-habits-{name}-{}-{unique}",
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
///
/// Not what a vault ships with. What the real ones are is asserted in the crate that owns them;
/// what these are for is finishing a derivation in milliseconds.
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

/// The day [`NOW_US`] falls on in that zone, which is what every command below calls today.
fn today() -> CivilDay {
    zone()
        .day_of(Timestamp::from_micros(NOW_US), DayStart::MIDNIGHT)
        .expect("a moment in 2023 is a day this calendar can name")
}

/// The day a given number of days before today.
fn days_ago(count: i32) -> CivilDay {
    shift(today(), -count).expect("a day in this millennium has a predecessor")
}

/// Runs something against the open database of a state, for the tests that need to write rows
/// the commands themselves cannot write: an archived habit, and one this build cannot read.
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

/// Writes a habit through the command under test.
fn written(state: &AppState, draft: &HabitDraft) -> String {
    create(state, &zone(), draft, NOW_US)
        .expect("a valid draft is written")
        .id
}

/// Every problem a refusal carries, or a failure naming what came back instead.
fn problems_of(outcome: &HabitsError) -> &[cairn_lib::commands::habits::FieldProblem] {
    match outcome {
        HabitsError::Invalid { problems } => problems,
        other => panic!("expected a refusal about the draft, got {other:?}"),
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

    assert_eq!(
        list(&state, &zone(), HabitFilter::Active, NOW_US).unwrap_err(),
        HabitsError::Locked
    );
    assert_eq!(
        get(&state, &zone(), &Uuid::new_v4().to_string(), NOW_US).unwrap_err(),
        HabitsError::Locked
    );
    // The draft is a valid one on purpose. A refusal about the draft would hide the fact that
    // nothing here should have reached the point of judging it.
    assert_eq!(
        create(&state, &zone(), &plain_draft("Leer"), NOW_US).unwrap_err(),
        HabitsError::Locked
    );
}

#[test]
fn a_file_with_no_habits_lists_nothing_and_does_not_fail() {
    let scratch = Scratch::new("empty");
    let state = unlocked(&scratch);

    let listing = list(&state, &zone(), HabitFilter::Active, NOW_US).expect("an empty file lists");

    assert!(listing.habits.is_empty(), "there is nothing to list");
    assert_eq!(listing.unreadable, 0, "nothing was skipped either");
}

#[test]
fn the_filter_separates_what_is_tracked_from_what_was_put_away() {
    let scratch = Scratch::new("filter");
    let state = unlocked(&scratch);

    for name in ["Leer", "Correr", "Beber agua"] {
        written(&state, &plain_draft(name));
    }
    let put_away = written(&state, &plain_draft("Meditar"));

    // Archiving is the next task's command. Here it is done through the repository, because
    // what is under test is the filter and not the way a habit gets archived.
    let id = Uuid::parse_str(&put_away).expect("the command answers with a hyphenated UUID");
    in_database(&state, |storage, codec, connection| {
        let hlc = storage.next_hlc(1);
        repository::archive(connection, codec, hlc, NOW_US, id, true).map(|_habit| ())
    })
    .expect("a habit can be archived");

    let active = list(&state, &zone(), HabitFilter::Active, NOW_US).expect("the active list");
    let archived = list(&state, &zone(), HabitFilter::Archived, NOW_US).expect("the archived list");

    assert_eq!(active.habits.len(), 3, "three are still being tracked");
    assert_eq!(archived.habits.len(), 1, "one was put away");
    assert_eq!(archived.habits[0].name, "Meditar");
    assert!(archived.habits[0].archived, "and it says so");
    assert!(
        active.habits.iter().all(|habit| !habit.archived),
        "nothing in the active list is archived"
    );
}

#[test]
fn a_list_never_carries_the_note_in_any_form() {
    let scratch = Scratch::new("note-free");
    let state = unlocked(&scratch);

    // A phrase that appears nowhere else, so that finding it in the text can only mean the note
    // came through.
    let secret = "una nota que no debe cruzar el puente jamas";
    let mut draft = plain_draft("Leer");
    draft.notes = Some(secret.to_owned());
    written(&state, &draft);

    let listing = list(&state, &zone(), HabitFilter::Active, NOW_US).expect("the list");
    let encoded = serde_json::to_string(&listing.habits).expect("summaries are plain data");

    // Asserted against the text and not against the type. The type is exactly what somebody
    // adds a field to, and a test that reads fields by name would not notice a new one.
    assert!(
        !encoded.contains(secret),
        "the note reached the list: {encoded}"
    );
    assert!(
        !encoded.contains("note"),
        "the list carries something called a note: {encoded}"
    );
}

#[test]
fn a_streak_longer_than_one_window_comes_back_whole() {
    let scratch = Scratch::new("long-streak");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 599);

    // Five hundred consecutive days ending today. A single window reaches four hundred, so five
    // hundred is a number no one read can produce: getting it back is the proof that the second
    // window was asked for. Counting the statements would be a weaker assertion, because a count
    // of two says nothing about whether the second one was used.
    mark_run(&state, id, 500);

    let listing = list(&state, &zone(), HabitFilter::Active, NOW_US).expect("the list");

    assert_eq!(listing.habits.len(), 1);
    assert_eq!(
        listing.habits[0].streak.days, 500,
        "the run is five hundred days long and one window holds four hundred"
    );
    assert!(
        !listing.habits[0].streak.at_risk,
        "today is marked, so nothing is at risk"
    );
}

#[test]
fn a_streak_longer_than_two_windows_stops_at_two() {
    let scratch = Scratch::new("capped-streak");
    let state = unlocked(&scratch);
    let id = habit_started(&state, 999);

    // Nine hundred consecutive days against a budget of two windows of four hundred. The answer
    // is eight hundred, and it is eight hundred for the reason the budget exists: a third read
    // is never asked for, however long the run is. Together with the test above, the two numbers
    // pin the query budget from outside the process.
    mark_run(&state, id, 900);

    let listing = list(&state, &zone(), HabitFilter::Active, NOW_US).expect("the list");

    assert_eq!(
        listing.habits[0].streak.days, 800,
        "two windows of four hundred days, and no third read"
    );
}

#[test]
fn a_row_this_build_cannot_read_is_left_out_without_taking_the_list_with_it() {
    let scratch = Scratch::new("strange-row");
    let state = unlocked(&scratch);
    written(&state, &plain_draft("Leer"));

    // The schema allows three ways of combining the days of a period and this product exposes
    // two, so a habit whose aggregation is the third is a row a newer build wrote. It is the one
    // value the file can actually hold that the domain refuses; a period of two, which the task
    // named, cannot be stored at all, and the assertion below keeps that true.
    in_database(&state, |storage, codec, connection| {
        let hlc = storage.next_hlc(1);
        repository::create(
            connection,
            codec,
            storage.device(),
            hlc,
            NOW_US,
            NewHabit {
                aggregation: 2,
                ..NewHabit::plain("Del futuro", today(), 50)
            },
        )
        .map(|_habit| ())
    })
    .expect("the schema accepts the third aggregation");

    let listing = list(&state, &zone(), HabitFilter::Active, NOW_US).expect("the list still comes");

    assert_eq!(listing.habits.len(), 1, "the ordinary habit is still there");
    assert_eq!(listing.habits[0].name, "Leer");
    assert_eq!(listing.unreadable, 1, "and the strange row was counted");
}

#[test]
fn the_schema_makes_a_period_this_product_does_not_have_impossible() {
    let scratch = Scratch::new("no-third-period");
    let state = unlocked(&scratch);

    // Migration 0006 checks the column rather than leaving the refusal to whoever reads the row.
    // Asserted here because the rest of this module is written as though it were true.
    let refused = in_database(&state, |storage, codec, connection| {
        let hlc = storage.next_hlc(1);
        repository::create(
            connection,
            codec,
            storage.device(),
            hlc,
            NOW_US,
            NewHabit {
                period: 2,
                ..NewHabit::plain("Mensual", today(), 0)
            },
        )
        .map(|_habit| ())
    });

    assert!(
        refused.is_err(),
        "a monthly habit must not be storable in the first place"
    );
}

#[test]
fn reading_a_habit_that_is_not_there_says_so() {
    let scratch = Scratch::new("missing");
    let state = unlocked(&scratch);

    assert_eq!(
        get(&state, &zone(), &Uuid::new_v4().to_string(), NOW_US).unwrap_err(),
        HabitsError::NotFound
    );
}

#[test]
fn text_that_is_not_an_identifier_is_the_same_answer_and_never_a_panic() {
    let scratch = Scratch::new("not-a-uuid");
    let state = unlocked(&scratch);

    for text in ["", "leer", "../../etc/passwd", "'; DROP TABLE habits; --"] {
        assert_eq!(
            get(&state, &zone(), text, NOW_US).unwrap_err(),
            HabitsError::NotFound,
            "{text} is not an identifier and must be answered as a missing habit"
        );
    }
}

#[test]
fn reading_one_habit_by_name_brings_its_note_back_in_the_clear() {
    let scratch = Scratch::new("with-note");
    let state = unlocked(&scratch);

    let note = "Treinta páginas, por la mañana.";
    let mut draft = plain_draft("Leer");
    draft.notes = Some(note.to_owned());
    draft.icon = Some("book".to_owned());
    draft.color = Some("#336699".to_owned());
    let id = written(&state, &draft);

    let detail = get(&state, &zone(), &id, NOW_US).expect("the habit is there");

    assert_eq!(detail.notes.as_deref(), Some(note));
    assert_eq!(detail.name, "Leer");
    assert_eq!(detail.icon.as_deref(), Some("book"));
    assert_eq!(detail.color.as_deref(), Some("#336699"));
    assert_eq!(detail.period, "daily");
    assert_eq!(detail.direction, "atLeast");
    assert_eq!(detail.started_on, today().as_number());
    // Zero in the column means no particular days, which is every day.
    assert_eq!(detail.schedule_mask, 0b111_1111);
}

#[test]
fn a_written_habit_comes_back_in_full_and_goes_last() {
    let scratch = Scratch::new("create");
    let state = unlocked(&scratch);

    let first = create(&state, &zone(), &plain_draft("Leer"), NOW_US).expect("the first");
    let second = create(&state, &zone(), &plain_draft("Correr"), NOW_US).expect("the second");
    let third = create(&state, &zone(), &plain_draft("Nadar"), NOW_US).expect("the third");

    assert_eq!(first.position, 0);
    assert_eq!(second.position, 1);
    assert_eq!(
        third.position, 2,
        "a new habit goes after every one there is"
    );
    assert_eq!(third.name, "Nadar");
    assert_eq!(third.notes, None);
    assert!(!third.archived);

    let listing = list(&state, &zone(), HabitFilter::Active, NOW_US).expect("the list");
    let order: Vec<&str> = listing
        .habits
        .iter()
        .map(|habit| habit.name.as_str())
        .collect();

    assert_eq!(
        order,
        ["Leer", "Correr", "Nadar"],
        "the list is in the person's own order, not the order the rows were written in"
    );
}

#[test]
fn a_habit_that_counts_a_quantity_keeps_its_unit_and_its_target() {
    let scratch = Scratch::new("quantity");
    let state = unlocked(&scratch);

    let mut draft = plain_draft("Beber agua");
    draft.unit = Some("ml".to_owned());
    draft.target = Some(2000);
    draft.aggregation = "sum".to_owned();

    let detail = create(&state, &zone(), &draft, NOW_US).expect("a quantity habit is a habit");

    assert_eq!(detail.unit.as_deref(), Some("ml"));
    assert_eq!(detail.target, Some(2000));
    assert_eq!(detail.aggregation, "sum");
}

#[test]
fn a_name_that_is_not_a_name_is_refused_and_says_which_field() {
    let scratch = Scratch::new("bad-name");
    let state = unlocked(&scratch);

    let too_long = "a".repeat(121);
    for name in ["", "     ", too_long.as_str()] {
        let mut draft = plain_draft("placeholder");
        draft.name = name.to_owned();

        let refusal = create(&state, &zone(), &draft, NOW_US)
            .expect_err("a habit with no usable name is not a habit");

        assert!(
            names_field(&refusal, "name"),
            "the refusal for {name:?} has to name the field: {refusal:?}"
        );
    }
}

#[test]
fn a_quantity_with_nothing_to_count_it_in_is_refused_with_at_least_one_reason() {
    let scratch = Scratch::new("no-unit");
    let state = unlocked(&scratch);

    let mut draft = plain_draft("Beber agua");
    draft.target = Some(2000);

    let refusal = create(&state, &zone(), &draft, NOW_US)
        .expect_err("a number with nothing to count it in is not a habit");

    assert!(
        !problems_of(&refusal).is_empty(),
        "a refusal with no reasons is a form that cannot be corrected"
    );
    assert!(
        names_field(&refusal, "unit"),
        "the missing thing is the unit: {refusal:?}"
    );
}

#[test]
fn a_schedule_with_bits_outside_the_week_is_refused() {
    let scratch = Scratch::new("bad-mask");
    let state = unlocked(&scratch);

    let mut draft = plain_draft("Leer");
    draft.schedule_mask = 255;

    let refusal = create(&state, &zone(), &draft, NOW_US)
        .expect_err("a week has seven days and the mask has eight bits");

    assert!(
        names_field(&refusal, "scheduleMask"),
        "the refusal has to name the mask: {refusal:?}"
    );
}

#[test]
fn a_draft_with_a_field_this_build_does_not_know_does_not_deserialise() {
    // The whole point of `deny_unknown_fields`: a field nobody here wrote is a message from
    // something that is not this application's form, and guessing what it meant is how a habit
    // is written with a column somebody else chose.
    let extra = r#"{
        "name": "Leer",
        "notes": null,
        "icon": null,
        "color": null,
        "period": "daily",
        "unit": null,
        "target": null,
        "aggregation": "sum",
        "direction": "atLeast",
        "scheduleMask": 0,
        "startedOn": 20231114,
        "kind": 1
    }"#;

    assert!(
        serde_json::from_str::<HabitDraft>(extra).is_err(),
        "an unknown field has to be refused rather than ignored"
    );

    // The same text without the extra field parses, so the refusal above is about that field
    // and not about something else being wrong with the message.
    let accepted = extra.replace(",\n        \"kind\": 1", "");
    assert!(
        serde_json::from_str::<HabitDraft>(&accepted).is_ok(),
        "the rest of the message is a draft this build understands"
    );
}

/// Writes a habit that started a given number of days ago, and answers its identifier.
fn habit_started(state: &AppState, days: i32) -> Uuid {
    let mut draft = plain_draft("Leer");
    draft.started_on = days_ago(days).as_number();

    Uuid::parse_str(&written(state, &draft)).expect("the command answers with a hyphenated UUID")
}

/// Marks a run of consecutive days ending today, in one transaction.
fn mark_run(state: &AppState, habit: Uuid, days: i32) {
    state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage.database().in_transaction(|transaction| {
                for back in 0..days {
                    let hlc = storage.next_hlc(1);
                    repository::mark(
                        transaction,
                        &codec,
                        storage.device(),
                        hlc,
                        NOW_US,
                        Mark {
                            habit_id: habit,
                            day: days_ago(back),
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
        .expect("a run of days can be marked");
}

/// Which day the square belongs to is the core's answer, and it is the same one everywhere.
///
/// The interface has no clock of its own worth trusting for this: what day it is here depends
/// on the device's zone and on how far from midnight this person's day starts, and a `Date` in
/// a WebView knows neither. So the day travels with the square, and a screen marking today
/// hands that number straight back to [`toggle_day`].
///
/// Asserted on the list and on the single read together, because the one thing that would make
/// the field useless is the two disagreeing: a row marked from the list would then write to a
/// different day than the same row marked from its own screen.
#[test]
fn the_square_says_which_day_it_is_and_says_the_same_in_both_places() {
    let scratch = Scratch::new("today-day");
    let state = unlocked(&scratch);

    let created = create(&state, &zone(), &plain_draft("Meditar"), NOW_US).expect("the habit");

    assert_eq!(
        created.today_day,
        today().as_number(),
        "the day carried is the day the core itself judged the square against"
    );

    let listing = list(&state, &zone(), HabitFilter::Active, NOW_US).expect("the list");
    let summary = listing.habits.first().expect("the one habit");

    assert_eq!(
        summary.today_day, created.today_day,
        "the list and the single read name the same day"
    );

    let read = get(&state, &zone(), &created.id, NOW_US).expect("the habit again");

    assert_eq!(read.today_day, created.today_day);
}

/// The day the square carries is a day the core is willing to be given back.
///
/// The whole point of the field: a screen reads it off the row and hands it to `toggle_day`
/// without touching it. If the core were to call that day future or too old, the one gesture
/// this module exists for would refuse on the row that offered the number.
#[test]
fn the_day_the_square_carries_is_one_the_core_accepts_back() {
    let scratch = Scratch::new("today-day-round-trip");
    let state = unlocked(&scratch);

    let created = create(&state, &zone(), &plain_draft("Correr"), NOW_US).expect("the habit");

    let marked = toggle_day(
        &state,
        &zone(),
        &created.id,
        created.today_day,
        None,
        NOW_US,
    )
    .expect("the day the row itself named is a day that may be marked");

    assert!(
        matches!(marked, DayStateDto::Done { .. }),
        "marking today with the day the row carried is what makes today done, and it said {marked:?}"
    );
}
