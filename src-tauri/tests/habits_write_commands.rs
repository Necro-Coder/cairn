//! Drives the five habit commands that change something against a real vault and a real file.
//!
//! What the domain already proves is what a streak is. What this proves is the sequence the five
//! commands actually run, and the three promises that only hold across the whole of it.
//!
//! The first is that the preview writes nothing. It is the one command whose value depends on
//! being harmless, because a form asks it on every keystroke that matters, and it is asserted
//! against the stored revision of the row rather than against anything the command answers: a
//! revision that moved is a write, whatever the return value said.
//!
//! The second is that the warning fires on the four things that change what a run counted and on
//! nothing else. A warning that also fires when somebody raises a goal is a warning somebody
//! learns to dismiss, and the day it matters they will dismiss that one too.
//!
//! The third is that an order is refused whole. A list that is not exactly the set of habits
//! there are leaves every position where it was, and the test for the pathological list asserts
//! that on the revisions, because a transaction that opened and rolled back would have moved
//! them.
//!
//! The zone and the moment are parameters everywhere below, so the same assertions hold in every
//! season and on any build agent, and which day of the week today is comes from the domain
//! rather than from arithmetic done in a comment.
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
use cairn_domain::habits::calendar::{shift, weekday};
use cairn_domain::{CivilDay, Timestamp};
use cairn_lib::commands::habits::{
    HabitDraft, HabitFilter, HabitSummary, HabitsError, archive, create, delete, get, list,
    reorder, update, update_preview,
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

/// Monday to Friday, which is a schedule that does not depend on what day today is.
const WEEKDAYS: u8 = 0b001_1111;

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
                "cairn-habits-write-{name}-{}-{unique}",
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

/// A schedule of one day of the week: the one today falls on.
///
/// Read from the domain rather than written down, so that these tests say what they mean on any
/// calendar and not only on the one the fixed moment happens to land in.
fn only_today_of_the_week() -> u8 {
    weekday(today()).bit()
}

/// Runs something against the open database of a state, for the assertions the commands cannot
/// make: the stored revision of a row, and the marks a deleted habit left behind.
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

/// The revision the row currently holds.
///
/// Every write to a habit raises it, so holding it still across a call is the strongest
/// statement available from outside the process that the call wrote nothing.
fn rev_of(state: &AppState, id: Uuid) -> i64 {
    in_database(state, |_storage, _codec, connection| {
        let mut statement = connection.prepare("SELECT rev FROM habits WHERE id = ?1")?;
        let rev: i64 = statement.query_row([id.as_bytes().as_slice()], |row| row.get(0))?;

        Ok(rev)
    })
    .expect("the habit is in the file")
}

/// How many marks of that habit are still live.
fn live_marks(state: &AppState, id: Uuid) -> usize {
    in_database(state, |_storage, _codec, connection| {
        repository::window(connection, id, days_ago(90), today())
    })
    .expect("a window of ninety days can be read")
    .len()
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

/// A draft of a habit that counts millilitres towards two thousand a day.
fn quantity_draft(name: &str) -> HabitDraft {
    HabitDraft {
        unit: Some("ml".to_owned()),
        target: Some(2000),
        ..plain_draft(name)
    }
}

/// Writes a habit through the create command and answers its identifier.
fn written(state: &AppState, draft: &HabitDraft) -> Uuid {
    let id = create(state, &zone(), draft, NOW_US)
        .expect("a valid draft is written")
        .id;

    Uuid::parse_str(&id).expect("the command answers with a hyphenated UUID")
}

/// Writes a habit that started a given number of days ago.
fn habit_started(state: &AppState, draft: &HabitDraft, days: i32) -> Uuid {
    let started = HabitDraft {
        started_on: days_ago(days).as_number(),
        ..draft.clone()
    };

    written(state, &started)
}

/// Marks the given days, counted backwards from today, in one transaction.
fn mark_days(state: &AppState, habit: Uuid, days_back: &[i32]) {
    state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage.database().in_transaction(|transaction| {
                for back in days_back {
                    let hlc = storage.next_hlc(1);
                    repository::mark(
                        transaction,
                        &codec,
                        storage.device(),
                        hlc,
                        NOW_US,
                        Mark {
                            habit_id: habit,
                            day: days_ago(*back),
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
        .expect("those days can be marked");
}

/// The habits of one kind, in the order the list puts them in.
fn listed(state: &AppState, filter: HabitFilter) -> Vec<HabitSummary> {
    list(state, &zone(), filter, NOW_US)
        .expect("the list comes back")
        .habits
}

/// The names of the habits still being tracked, in order.
fn active_names(state: &AppState) -> Vec<String> {
    listed(state, HabitFilter::Active)
        .into_iter()
        .map(|habit| habit.name)
        .collect()
}

#[test]
fn all_five_refuse_while_the_vault_is_locked() {
    let scratch = Scratch::new("locked");
    let state = scratch.state();
    // A real identifier and a valid draft on purpose. Either being wrong would let a refusal
    // about the argument stand in for the refusal this test is about.
    let id = Uuid::new_v4().to_string();
    let draft = plain_draft("Leer");

    assert_eq!(
        update(&state, &zone(), &id, &draft, NOW_US).unwrap_err(),
        HabitsError::Locked
    );
    assert_eq!(
        update_preview(&state, &zone(), &id, &draft, NOW_US).unwrap_err(),
        HabitsError::Locked
    );
    assert_eq!(
        archive(&state, &zone(), &id, true, NOW_US).unwrap_err(),
        HabitsError::Locked
    );
    assert_eq!(
        delete(&state, &id, NOW_US).unwrap_err(),
        HabitsError::Locked
    );
    assert_eq!(
        reorder(&state, &[id], NOW_US).unwrap_err(),
        HabitsError::Locked
    );
}

#[test]
fn renaming_a_habit_does_not_change_what_its_streak_means() {
    let scratch = Scratch::new("rename");
    let state = unlocked(&scratch);
    let id = written(&state, &plain_draft("Leer"));

    let renamed = HabitDraft {
        name: "Leer treinta páginas".to_owned(),
        ..plain_draft("Leer")
    };
    let outcome = update(&state, &zone(), &id.to_string(), &renamed, NOW_US).expect("the edit");

    assert_eq!(outcome.habit.name, "Leer treinta páginas");
    assert!(
        !outcome.streak_meaning_changed,
        "a different name counts the same days"
    );
}

#[test]
fn changing_the_direction_changes_what_the_streak_means() {
    let scratch = Scratch::new("direction");
    let state = unlocked(&scratch);
    let id = written(&state, &quantity_draft("Beber agua"));

    let reversed = HabitDraft {
        direction: "atMost".to_owned(),
        ..quantity_draft("Beber agua")
    };
    let outcome = update(&state, &zone(), &id.to_string(), &reversed, NOW_US).expect("the edit");

    assert_eq!(outcome.habit.direction, "atMost");
    assert!(
        outcome.streak_meaning_changed,
        "a day that used to pass now fails, and the other way round"
    );
}

#[test]
fn changing_the_schedule_changes_what_the_streak_means() {
    let scratch = Scratch::new("schedule");
    let state = unlocked(&scratch);
    let id = written(&state, &plain_draft("Leer"));

    let weekdays_only = HabitDraft {
        schedule_mask: WEEKDAYS,
        ..plain_draft("Leer")
    };
    let outcome =
        update(&state, &zone(), &id.to_string(), &weekdays_only, NOW_US).expect("the edit");

    assert_eq!(outcome.habit.schedule_mask, WEEKDAYS);
    assert!(
        outcome.streak_meaning_changed,
        "the days the run is counted over are not the same days"
    );
}

#[test]
fn raising_the_target_does_not_change_what_the_streak_means() {
    let scratch = Scratch::new("target");
    let state = unlocked(&scratch);
    let id = written(&state, &quantity_draft("Beber agua"));

    let higher = HabitDraft {
        target: Some(3000),
        ..quantity_draft("Beber agua")
    };
    let outcome = update(&state, &zone(), &id.to_string(), &higher, NOW_US).expect("the edit");

    assert_eq!(outcome.habit.target, Some(3000));
    // The target is written onto every entry as that entry is marked, so the days already judged
    // keep the number they were judged by. Warning here would teach somebody to dismiss the
    // warning that matters.
    assert!(
        !outcome.streak_meaning_changed,
        "a higher goal from today does not move a single day of the past"
    );
}

#[test]
fn editing_a_habit_that_was_put_away_leaves_it_put_away() {
    let scratch = Scratch::new("edit-archived");
    let state = unlocked(&scratch);
    let id = written(&state, &plain_draft("Meditar"));

    archive(&state, &zone(), &id.to_string(), true, NOW_US).expect("it can be put away");

    let renamed = HabitDraft {
        name: "Meditar diez minutos".to_owned(),
        ..plain_draft("Meditar")
    };
    let outcome = update(&state, &zone(), &id.to_string(), &renamed, NOW_US).expect("the edit");

    assert!(
        outcome.habit.archived,
        "an edit must not quietly bring a habit back"
    );
    assert!(
        active_names(&state).is_empty(),
        "and it must not reappear in the list of what is being tracked"
    );
}

#[test]
fn a_draft_that_is_not_a_habit_is_refused_and_the_row_is_left_exactly_as_it_was() {
    let scratch = Scratch::new("invalid-edit");
    let state = unlocked(&scratch);
    let id = written(&state, &plain_draft("Leer"));
    let before = rev_of(&state, id);

    let nameless = HabitDraft {
        name: "   ".to_owned(),
        ..plain_draft("Leer")
    };
    let refusal = update(&state, &zone(), &id.to_string(), &nameless, NOW_US)
        .expect_err("a habit with no usable name is not a habit");

    assert!(
        matches!(refusal, HabitsError::Invalid { .. }),
        "expected a refusal about the draft, got {refusal:?}"
    );

    let unchanged = get(&state, &zone(), &id.to_string(), NOW_US).expect("it is still there");

    assert_eq!(unchanged.name, "Leer");
    assert_eq!(
        rev_of(&state, id),
        before,
        "a refused edit must not have written anything at all"
    );
}

#[test]
fn the_preview_answers_both_runs_and_leaves_the_revision_where_it_found_it() {
    let scratch = Scratch::new("preview");
    let state = unlocked(&scratch);
    let id = habit_started(&state, &plain_draft("Leer"), 60);

    // Five marks, one a week apart, all on the day of the week today falls on. Under the habit
    // as it stands they are five islands in a sea of missed days, so the run is today alone;
    // under a schedule of that one weekday they are five in a row. Two numbers that cannot be
    // confused with each other, whatever day of the week the fixed moment lands on.
    mark_days(&state, id, &[0, 7, 14, 21, 28]);
    let before = rev_of(&state, id);

    let one_day_a_week = HabitDraft {
        schedule_mask: only_today_of_the_week(),
        started_on: days_ago(60).as_number(),
        ..plain_draft("Leer")
    };
    let impact = update_preview(&state, &zone(), &id.to_string(), &one_day_a_week, NOW_US)
        .expect("the preview");

    assert_eq!(
        impact.current_streak_before, 1,
        "yesterday was missed, so the run as it stands is today alone"
    );
    assert_eq!(
        impact.current_streak_after, 5,
        "five weeks in a row, once the schedule only asks for that weekday"
    );
    assert!(impact.streak_meaning_changed);
    assert_eq!(
        impact.entries_outside_new_schedule, 0,
        "every mark sits on a day the new schedule asks for"
    );
    assert_eq!(
        rev_of(&state, id),
        before,
        "the preview wrote something, and it must never write anything"
    );
}

#[test]
fn the_preview_counts_the_marks_the_new_schedule_would_stop_counting() {
    let scratch = Scratch::new("outside");
    let state = unlocked(&scratch);
    let id = habit_started(&state, &plain_draft("Leer"), 60);

    // Ten consecutive days ending today. Exactly two of them fall on the day of the week today
    // does, so a schedule of that one weekday leaves the other eight outside.
    mark_days(&state, id, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

    let one_day_a_week = HabitDraft {
        schedule_mask: only_today_of_the_week(),
        started_on: days_ago(60).as_number(),
        ..plain_draft("Leer")
    };
    let impact = update_preview(&state, &zone(), &id.to_string(), &one_day_a_week, NOW_US)
        .expect("the preview");

    assert_eq!(
        impact.entries_outside_new_schedule, 8,
        "ten consecutive days hold two of any given weekday"
    );
}

#[test]
fn previewing_a_habit_that_is_not_there_says_so() {
    let scratch = Scratch::new("preview-missing");
    let state = unlocked(&scratch);

    assert_eq!(
        update_preview(
            &state,
            &zone(),
            &Uuid::new_v4().to_string(),
            &plain_draft("Leer"),
            NOW_US,
        )
        .unwrap_err(),
        HabitsError::NotFound
    );
}

#[test]
fn putting_a_habit_away_takes_it_out_of_what_is_being_tracked() {
    let scratch = Scratch::new("archive");
    let state = unlocked(&scratch);
    let id = written(&state, &plain_draft("Meditar"));
    written(&state, &plain_draft("Leer"));

    let put_away = archive(&state, &zone(), &id.to_string(), true, NOW_US).expect("it goes away");

    assert!(put_away.archived);
    assert_eq!(active_names(&state), ["Leer"]);
    assert_eq!(
        listed(&state, HabitFilter::Archived)
            .into_iter()
            .map(|habit| habit.name)
            .collect::<Vec<_>>(),
        ["Meditar"]
    );
}

#[test]
fn bringing_a_habit_back_restores_it_with_the_run_it_had() {
    let scratch = Scratch::new("unarchive");
    let state = unlocked(&scratch);
    let id = habit_started(&state, &plain_draft("Correr"), 30);
    mark_days(&state, id, &[0, 1, 2, 3, 4]);

    let before = listed(&state, HabitFilter::Active)[0].streak;
    assert_eq!(before.days, 5, "five consecutive days ending today");

    archive(&state, &zone(), &id.to_string(), true, NOW_US).expect("it goes away");
    let back = archive(&state, &zone(), &id.to_string(), false, NOW_US).expect("it comes back");

    assert!(!back.archived);
    assert_eq!(
        back.streak, before,
        "archiving touches no mark, so the run is the one it left with"
    );
    assert_eq!(active_names(&state), ["Correr"]);
}

#[test]
fn putting_a_habit_away_twice_is_the_same_as_putting_it_away_once() {
    let scratch = Scratch::new("archive-twice");
    let state = unlocked(&scratch);
    let id = written(&state, &plain_draft("Meditar"));

    let first = archive(&state, &zone(), &id.to_string(), true, NOW_US).expect("the first");
    let second = archive(&state, &zone(), &id.to_string(), true, NOW_US).expect("the second");

    assert!(first.archived);
    assert!(second.archived, "the second call is not an error either");
    assert!(active_names(&state).is_empty());
}

#[test]
fn deleting_a_habit_takes_every_day_it_was_marked_on_with_it() {
    let scratch = Scratch::new("delete");
    let state = unlocked(&scratch);
    let id = habit_started(&state, &plain_draft("Leer"), 30);
    mark_days(&state, id, &[0, 1, 2, 3, 4, 5]);

    assert_eq!(live_marks(&state, id), 6, "six days were marked");

    delete(&state, &id.to_string(), NOW_US).expect("it can be removed");

    assert_eq!(
        get(&state, &zone(), &id.to_string(), NOW_US).unwrap_err(),
        HabitsError::NotFound
    );
    assert_eq!(
        live_marks(&state, id),
        0,
        "a year of somebody's calendar must not outlive the habit it belonged to"
    );
}

#[test]
fn deleting_the_same_habit_twice_says_the_second_one_is_not_there() {
    let scratch = Scratch::new("delete-twice");
    let state = unlocked(&scratch);
    let id = written(&state, &plain_draft("Leer"));

    delete(&state, &id.to_string(), NOW_US).expect("the first");

    assert_eq!(
        delete(&state, &id.to_string(), NOW_US).unwrap_err(),
        HabitsError::NotFound
    );
}

#[test]
fn an_order_of_the_whole_set_changes_the_order_the_list_comes_in() {
    let scratch = Scratch::new("reorder");
    let state = unlocked(&scratch);
    let first = written(&state, &plain_draft("Leer"));
    let second = written(&state, &plain_draft("Correr"));
    let third = written(&state, &plain_draft("Nadar"));

    assert_eq!(active_names(&state), ["Leer", "Correr", "Nadar"]);

    reorder(
        &state,
        &[third.to_string(), first.to_string(), second.to_string()],
        NOW_US,
    )
    .expect("the whole set in a new order");

    assert_eq!(active_names(&state), ["Nadar", "Leer", "Correr"]);
}

#[test]
fn the_same_order_twice_leaves_the_list_where_the_first_one_put_it() {
    let scratch = Scratch::new("reorder-twice");
    let state = unlocked(&scratch);
    let first = written(&state, &plain_draft("Leer"));
    let second = written(&state, &plain_draft("Correr"));

    let order = [second.to_string(), first.to_string()];

    reorder(&state, &order, NOW_US).expect("the first");
    reorder(&state, &order, NOW_US).expect("the second");

    assert_eq!(active_names(&state), ["Correr", "Leer"]);
}

#[test]
fn an_order_with_one_identifier_missing_changes_nothing() {
    let scratch = Scratch::new("reorder-short");
    let state = unlocked(&scratch);
    let first = written(&state, &plain_draft("Leer"));
    let second = written(&state, &plain_draft("Correr"));
    written(&state, &plain_draft("Nadar"));

    let refused = reorder(&state, &[second.to_string(), first.to_string()], NOW_US)
        .expect_err("two identifiers are not the set of habits there are");

    assert_eq!(refused, HabitsError::IncompleteOrder);
    assert_eq!(
        active_names(&state),
        ["Leer", "Correr", "Nadar"],
        "a refused order leaves every position where it was"
    );
}

#[test]
fn an_order_carrying_text_that_is_not_an_identifier_is_about_the_order() {
    let scratch = Scratch::new("reorder-garbage");
    let state = unlocked(&scratch);
    let id = written(&state, &plain_draft("Leer"));
    let untouched = rev_of(&state, id);

    for text in ["", "leer", "../../etc/passwd", "'; DROP TABLE habits; --"] {
        let refused = reorder(&state, &[text.to_owned()], NOW_US)
            .expect_err("that is not an identifier and cannot be an order");

        // Not `NotFound`. What arrived is an order, and what is wrong with it is that it is not
        // the set of habits there are; saying one habit is missing would send the interface
        // looking for a habit nobody named.
        assert_eq!(
            refused,
            HabitsError::IncompleteOrder,
            "{text} has to be refused as an order"
        );
    }

    assert_eq!(rev_of(&state, id), untouched, "and nothing was written");
}

#[test]
fn an_order_of_two_hundred_repeats_is_refused_without_writing_a_thing() {
    let scratch = Scratch::new("reorder-repeats");
    let state = unlocked(&scratch);
    let first = written(&state, &plain_draft("Leer"));
    let second = written(&state, &plain_draft("Correr"));
    let revisions = [rev_of(&state, first), rev_of(&state, second)];

    let repeated: Vec<String> = std::iter::repeat_n(first.to_string(), 200).collect();

    let refused = reorder(&state, &repeated, NOW_US)
        .expect_err("the same habit two hundred times is not an order");

    assert_eq!(refused, HabitsError::IncompleteOrder);
    // Asserted on the revisions rather than on the positions, because a transaction that opened
    // and rolled back would leave the positions right and is exactly what this must not do: the
    // list is checked before a write lock is taken, not after.
    assert_eq!(
        [rev_of(&state, first), rev_of(&state, second)],
        revisions,
        "a list that was never going to be accepted must not touch a single row"
    );
}

#[test]
fn putting_away_a_row_this_build_cannot_read_is_refused_before_anything_is_written() {
    let scratch = Scratch::new("archive-strange");
    let state = unlocked(&scratch);

    // The schema allows a third way of combining the days of a period that this product never
    // offered, so a row carrying it is one a newer build wrote. Archiving it has to be refused,
    // and refused *before* the write: reporting a failure for a habit that had already been put
    // away is the one outcome that leaves the screen and the file disagreeing.
    let id = in_database(&state, |storage, codec, connection| {
        let hlc = storage.next_hlc(1);
        repository::create(
            connection,
            codec,
            storage.device(),
            hlc,
            NOW_US,
            NewHabit {
                aggregation: 2,
                ..NewHabit::plain("Del futuro", today(), 0)
            },
        )
        .map(|habit| habit.id)
    })
    .expect("the schema accepts the third aggregation");
    let untouched = rev_of(&state, id);

    assert_eq!(
        archive(&state, &zone(), &id.to_string(), true, NOW_US).unwrap_err(),
        HabitsError::Storage
    );
    assert_eq!(
        rev_of(&state, id),
        untouched,
        "the row was written to on the way to being refused"
    );
}
