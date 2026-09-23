//! What the habits module costs to read, at the size a decade of use produces.
//!
//! Three of the four budgets the phase set live here, and all three are the same shape: a
//! statement over the calendar index, and then the classification of every day of a span in
//! Rust. Splitting the two is the point. When one of these numbers goes red, the question that
//! decides what to do about it is whether the time went to SQLite or to the walk over the days,
//! and a single total cannot answer it. Each measurement therefore reports the query, the
//! aggregation and the sum.
//!
//! The fourth budget, cold start to the lock screen, is not measurable from here: it covers the
//! process, the window, the WebView and the bundle, none of which exist in this crate. It is
//! taken the way `docs/development/quality-gates.md` describes and written down there.
//!
//! Seeding is never inside a measurement. A benchmark that timed the insertion of three and a
//! half thousand encrypted rows would report the cost of writing a history that is written once
//! a day in real life, and would hide the cost of the reading it exists to measure. Each of the
//! three builds its own database file and removes it afterwards, so no measurement inherits a
//! page cache, a statement cache or a row from the one before it.
//!
//! This records rather than enforces, for the reason the benchmark in `cairn-crypto` gives: the
//! machine a contributor runs it on is not the machine the budget was written for, and a
//! benchmark that fails the build on a slow morning teaches everybody to ignore it. What it is
//! for is having a number to compare against.
//!
//! Run with `cargo bench -p cairn-db`.

#![allow(
    clippy::print_stdout,
    clippy::expect_used,
    clippy::panic,
    reason = "a benchmark is a program run by hand whose entire output is what it prints, and whose inputs are literals written in this file; the lints that forbid printing and panicking constructs only relax themselves inside test functions"
)]

use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::process;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cairn_db::codec::FieldCodec;
use cairn_db::device::DeviceId;
use cairn_db::error::DbError;
use cairn_db::open::Database;
use cairn_db::repositories::habits as repository;
use cairn_db::{Connection, DATABASE_FILE, migrations};
use cairn_domain::habits::calendar::{shift, span};
use cairn_domain::habits::spec::{HabitRow, HabitSpec};
use cairn_domain::habits::{DayState, Entry, classify, completion, streak};
use cairn_domain::hlc::Hlc;
use cairn_domain::time::CivilDay;
use uuid::Uuid;

/// The day every measurement calls today.
///
/// Fixed rather than read from a clock, so that two runs a week apart seed the same history and
/// produce comparable numbers. Nothing in this crate reads a clock anyway.
const TODAY: (u16, u8, u8) = (2026, 9, 17);

/// How many days of history the ten year habit carries. Ten years, leap days included.
const HISTORY_DAYS: i32 = 3_650;

/// The year measurement one reads.
///
/// A whole year, well inside the history, so that the statement and the classification are both
/// at full size rather than cut short by either end of the record.
const HEATMAP_YEAR: u16 = 2021;

/// How many days that year holds. Not a leap year, and named so the check below is a comparison
/// against a number somebody wrote rather than against whatever the seeding happened to produce.
const DAYS_IN_HEATMAP_YEAR: usize = 365;

/// How wide one streak window is, copied from the command layer this is measuring.
///
/// Four hundred days is what `src-tauri/src/commands/habits.rs` asks for, and the second window
/// doubles it. A different number here would measure a reading the application never performs.
const WINDOW_DAYS: i32 = 400;

/// How many habits the list measurement holds.
const HABIT_COUNT: usize = 30;

/// How long the one long run is, in days. Longer than a window, which is what forces the second.
const LONG_STREAK_DAYS: i32 = 500;

/// How far back the other habits' history reaches.
const SHORT_HISTORY_DAYS: i32 = WINDOW_DAYS;

/// How many days ago the gap in those histories sits, which is what ends their run.
const GAP_DAYS_AGO: i32 = 30;

/// How many times each measurement is taken.
const SAMPLES: usize = 5;

/// The moment every seeded row is stamped with. Fixed, like the day: nothing here reads a clock.
const NOW_US: i64 = 1_700_000_000_000_000;

fn main() {
    let today = day(TODAY.0, TODAY.1, TODAY.2);

    println!("habits: {SAMPLES} samples each, median reported, query and aggregation split out");
    println!("        seeding is outside every measurement");
    println!();

    report(
        "1  one year plus the calendar",
        20,
        &measure_one_year(today),
    );
    report(
        "2  the list's streak windows",
        10,
        &measure_streak_windows(today),
    );
    report(
        "3  the whole history plus longest",
        50,
        &measure_whole_history(today),
    );

    println!();
    println!("4  cold start to the lock screen: taken the way docs/development/quality-gates.md");
    println!("   describes, on an installed release build. Not measurable from this crate.");
}

/// What one measurement produced: where the time went, and how much of it there was.
#[derive(Debug, Clone, Copy, Default)]
struct Timing {
    /// Time inside the repository, which is SQLite and the decoding of the rows it answers with.
    query: Duration,
    /// Time in the domain: classifying the span, and whatever is computed over it.
    aggregation: Duration,
}

impl Timing {
    /// The number the budget is written against.
    const fn total(&self) -> Duration {
        self.query.saturating_add(self.aggregation)
    }
}

/// Prints one measurement against its budget.
///
/// The verdict is printed rather than returned as an exit code, deliberately. This is a record,
/// not a gate: see the note at the top of the file.
fn report(label: &str, budget_ms: u128, samples: &[Timing]) {
    let query = median(samples.iter().map(|timing| timing.query));
    let aggregation = median(samples.iter().map(|timing| timing.aggregation));
    let total = median(samples.iter().map(Timing::total));

    let verdict = if total.as_millis() <= budget_ms {
        "within"
    } else {
        "OVER"
    };

    println!("{label}");
    println!(
        "   query {:>8.2} ms   aggregation {:>8.2} ms   total {:>8.2} ms   budget {budget_ms:>3} ms   {verdict}",
        millis(query),
        millis(aggregation),
        millis(total),
    );
}

/// The middle value of a list of timings.
///
/// The median rather than the mean, because one sample that landed while the machine was busy
/// says nothing about the code and would move a mean by more than the change anybody would be
/// looking for.
fn median(values: impl Iterator<Item = Duration>) -> Duration {
    let mut sorted: Vec<Duration> = values.collect();
    sorted.sort_unstable();

    sorted
        .get(sorted.len().div_ceil(2).saturating_sub(1))
        .copied()
        .unwrap_or_default()
}

/// A duration in milliseconds, with the fraction kept.
///
/// Floating point is forbidden in this workspace wherever it could reach money or a stored
/// value, and this is neither: it is the formatting of a number on its way to a terminal, and
/// the alternative is printing a count of microseconds that nobody can compare to a budget
/// written in milliseconds without dividing it in their head.
#[allow(
    clippy::cast_precision_loss,
    clippy::float_arithmetic,
    reason = "formatting a timing for a terminal, which never reaches a stored value or an amount of money"
)]
fn millis(value: Duration) -> f64 {
    value.as_nanos() as f64 / 1_000_000.0
}

/// Measurement one: a year of one habit's calendar, read and classified.
///
/// What the heat map costs. One statement over the index that covers exactly this condition,
/// then every day of the year turned into a square.
fn measure_one_year(today: CivilDay) -> Vec<Timing> {
    let sandbox = Sandbox::new("bench-year");
    let started = shift(today, -(HISTORY_DAYS - 1)).expect("a day inside the calendar");
    let habit = sandbox.seed_habit("Un habito con diez anos", started, 0);
    sandbox.seed_every_day(habit, started, today);

    sandbox.expect_history(habit, HISTORY_DAYS);

    let spec = sandbox.spec_of(habit);
    let from = day(HEATMAP_YEAR, 1, 1);
    let to = day(HEATMAP_YEAR, 12, 31);

    sandbox.sample(|connection| {
        let started_at = Instant::now();
        let entries = repository::year_entries(connection, habit, HEATMAP_YEAR)?;
        let query = started_at.elapsed();

        // A year that came back empty would be measured in microseconds and would mean nothing.
        // Checked inside the sample rather than before it, because what has to be a whole year
        // is the vector the classification below actually walks.
        assert_eq!(
            entries.len(),
            DAYS_IN_HEATMAP_YEAR,
            "the year read is a whole year of marks"
        );

        let started_at = Instant::now();
        let days = classified(&spec, from, to, &entries, today);
        black_box(&days);
        let aggregation = started_at.elapsed();

        Ok(Timing { query, aggregation })
    })
}

/// Measurement two: what a list of thirty habits pays for their streaks.
///
/// One window each, and a second one for the habit whose run is still going at the oldest day of
/// the first. That second window is the case worth seeding: it is the only path in the module
/// that runs two statements for one habit, and it also classifies eight hundred days instead of
/// four hundred.
fn measure_streak_windows(today: CivilDay) -> Vec<Timing> {
    let sandbox = Sandbox::new("bench-list");
    let long_start = shift(today, -(LONG_STREAK_DAYS - 1)).expect("a day inside the calendar");
    let short_start = shift(today, -(SHORT_HISTORY_DAYS - 1)).expect("a day inside the calendar");
    let gap = shift(today, -GAP_DAYS_AGO).expect("a day inside the calendar");

    // The one with the long run first, so it is also the first the list walks.
    let long = sandbox.seed_habit("Racha larga", long_start, 0);
    sandbox.seed_every_day(long, long_start, today);

    for position in 1..HABIT_COUNT {
        let habit = sandbox.seed_habit(
            &format!("Habito {position}"),
            short_start,
            i64::try_from(position).expect("a number below thirty fits"),
        );
        sandbox.seed_every_day_except(habit, short_start, today, gap);
    }

    let specs = sandbox.all_specs();
    assert_eq!(
        specs.len(),
        HABIT_COUNT,
        "every seeded habit is in the listing"
    );
    sandbox.expect_history(long, LONG_STREAK_DAYS);

    // The whole point of the long one is that it makes the module ask twice. A run that stopped
    // inside the first window would measure thirty single windows, which is not what the budget
    // was written for, and nothing on the report would say so.
    let long_spec = sandbox.spec_of(long);
    let from = window_start(&long_spec, today, WINDOW_DAYS - 1);
    let reached = sandbox
        .database
        .with(|connection| {
            let entries = repository::window(connection, long, from, today)?;
            let days = classified(&long_spec, from, today, &entries, today);

            Ok(streak::current(&long_spec, &days, today).reached_window_start)
        })
        .expect("the long habit's window reads");
    assert!(reached, "the long run forces the second window");

    sandbox.sample(|connection| {
        let mut timing = Timing::default();

        for (id, spec) in &specs {
            let snapshot = snapshot_of(connection, *id, spec, today)?;
            timing.query = timing.query.saturating_add(snapshot.query);
            timing.aggregation = timing.aggregation.saturating_add(snapshot.aggregation);
        }

        Ok(timing)
    })
}

/// Measurement three: the whole record, and the longest run in it.
///
/// A record cut to a window stops being a record, so this is the one reading in the module with
/// no fixed budget of statements: one per calendar year the habit has lived through, plus the
/// three aggregates the statistics screen shows beside the streaks.
fn measure_whole_history(today: CivilDay) -> Vec<Timing> {
    let sandbox = Sandbox::new("bench-history");
    let started = shift(today, -(HISTORY_DAYS - 1)).expect("a day inside the calendar");
    let habit = sandbox.seed_habit("Un habito con diez anos", started, 0);
    sandbox.seed_every_day(habit, started, today);

    sandbox.expect_history(habit, HISTORY_DAYS);

    let spec = sandbox.spec_of(habit);

    sandbox.sample(|connection| {
        let started_at = Instant::now();
        let history = repository::history_span(connection, habit)?;
        let mut entries = Vec::new();
        for year in started.year()..=today.year() {
            entries.extend(repository::year_entries(connection, habit, year)?);
        }
        let query = started_at.elapsed();

        // Same reason as measurement one: what is being timed below is the walk over the whole
        // decade, and a vector that arrived short would still produce a number and a verdict.
        assert_eq!(
            entries.len(),
            usize::try_from(HISTORY_DAYS).expect("ten years of days fit"),
            "the history read is the whole decade"
        );

        let started_at = Instant::now();
        let days = classified(&spec, started, today, &entries, today);
        let longest = streak::longest(&spec, &days, today);
        let current = streak::current(&spec, &days, today);
        let month = completion::month(&spec, &days, today.year(), today.month(), today);
        black_box((&history, longest, &current, &month));
        let aggregation = started_at.elapsed();

        Ok(Timing { query, aggregation })
    })
}

/// What one habit's snapshot cost, split the way the report prints it.
#[derive(Debug, Clone, Copy)]
struct Snapshot {
    /// Time in the repository.
    query: Duration,
    /// Time in the domain.
    aggregation: Duration,
}

/// One habit's recent calendar and the run it holds, timed in two halves.
///
/// The same walk as `snapshot_of` in `src-tauri/src/commands/habits.rs`, and it has to be a copy:
/// this crate cannot depend on the application, and measuring a different walk would produce a
/// number that has nothing to do with the budget. The shape is what matters — one window, and a
/// second only when the run was still alive at the oldest day of the first.
fn snapshot_of(
    connection: &Connection,
    id: Uuid,
    spec: &HabitSpec,
    today: CivilDay,
) -> Result<Snapshot, DbError> {
    let from = window_start(spec, today, WINDOW_DAYS - 1);

    let started_at = Instant::now();
    let mut entries = repository::window(connection, id, from, today)?;
    let mut query = started_at.elapsed();

    let started_at = Instant::now();
    let mut days = classified(spec, from, today, &entries, today);
    let mut run = streak::current(spec, &days, today);
    let mut aggregation = started_at.elapsed();

    if run.reached_window_start
        && let Ok(older_end) = shift(today, -WINDOW_DAYS)
    {
        let older_start = window_start(spec, today, (WINDOW_DAYS * 2) - 1);

        let started_at = Instant::now();
        let mut older = repository::window(connection, id, older_start, older_end)?;
        query = query.saturating_add(started_at.elapsed());

        let started_at = Instant::now();
        older.append(&mut entries);
        entries = older;
        days = classified(spec, older_start, today, &entries, today);
        run = streak::current(spec, &days, today);
        aggregation = aggregation.saturating_add(started_at.elapsed());
    }

    let started_at = Instant::now();
    let mark = entries.iter().find(|entry| entry.day == today);
    let square = classify(spec, today, mark.map(entry_of), today);
    black_box((&run, square));
    aggregation = aggregation.saturating_add(started_at.elapsed());

    Ok(Snapshot { query, aggregation })
}

/// The oldest day a window may reach, never before the habit existed.
fn window_start(spec: &HabitSpec, today: CivilDay, days_back: i32) -> CivilDay {
    let reached = shift(today, -days_back).unwrap_or(spec.started_on);

    reached.max(spec.started_on)
}

/// Every day of the span, classified, oldest first and with no gaps.
fn classified(
    spec: &HabitSpec,
    from: CivilDay,
    to: CivilDay,
    entries: &[repository::StoredEntry],
    today: CivilDay,
) -> Vec<(CivilDay, DayState)> {
    let marks: HashMap<u32, &repository::StoredEntry> = entries
        .iter()
        .map(|entry| (entry.day.as_number(), entry))
        .collect();

    span(from, to)
        .into_iter()
        .map(|day| {
            let mark = marks.get(&day.as_number()).copied().map(entry_of);

            (day, classify(spec, day, mark, today))
        })
        .collect()
}

/// One stored mark, as the domain reads one.
fn entry_of(entry: &repository::StoredEntry) -> Entry {
    Entry {
        day: entry.day,
        amount: entry.amount,
        target_snapshot: entry.target_snapshot,
    }
}

/// The columns of a stored habit, as the domain reads them.
fn row_of(habit: &repository::Habit) -> HabitRow {
    HabitRow {
        period: habit.period,
        kind: habit.kind,
        direction: habit.direction,
        aggregation: habit.aggregation,
        schedule_mask: habit.schedule_mask,
        unit: habit.unit.clone(),
        target_per_period: habit.target_per_period,
        started_on: habit.started_on,
    }
}

/// A day that exists.
fn day(year: u16, month: u8, of_month: u8) -> CivilDay {
    CivilDay::new(year, month, of_month).expect("a day written in this file exists")
}

/// A database file of its own, with the vault that opens it, removed when this is dropped.
///
/// What `src/test_support.rs` gives the tests, written again here because that module is compiled
/// only for tests and a benchmark is a separate target that cannot see it.
struct Sandbox {
    /// Named rather than ignored so the directory is removed when this is dropped.
    directory: PathBuf,
    vault: cairn_crypto::UnlockedVault,
    database: Database,
    /// The next clock reading to stamp a write with. Every row gets a higher one than the row
    /// before it, which is what the paging in `page` walks.
    next_step: Cell<u64>,
    device: DeviceId,
}

impl Sandbox {
    /// A new vault and a freshly migrated database, at the cheapest parameters that are still a
    /// real Argon2id run. What the derivation costs is measured in `cairn-crypto`, not here.
    fn new(label: &str) -> Self {
        let params = cairn_crypto::Argon2Params::new(
            cairn_crypto::MIN_MEMORY_KIB,
            cairn_crypto::MIN_PASSES,
            1,
        )
        .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("cairn-db-{label}-{}-{nanos}", process::id()));
        fs::create_dir_all(&directory).expect("a temporary directory can be created");

        let database = Database::open(&directory.join(DATABASE_FILE), &vault.database_key())
            .expect("a new database file can be created");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");

        Self {
            directory,
            vault,
            database,
            next_step: Cell::new(1),
            device: DeviceId::generate().expect("an identifier can be generated"),
        }
    }

    /// A codec over the vault's keys.
    fn codec(&self) -> FieldCodec<'_> {
        FieldCodec::new(self.vault.data_key(), *self.vault.key_id())
    }

    /// The next clock reading, higher than every one handed out before it.
    fn step(&self) -> Hlc {
        let step = self.next_step.get();
        self.next_step.set(step.saturating_add(1));

        Hlc::new(step, 0, [1; 6])
    }

    /// Writes one habit down and answers its identifier.
    fn seed_habit(&self, name: &str, started_on: CivilDay, position: i64) -> Uuid {
        self.database
            .with(|connection| {
                repository::create(
                    connection,
                    &self.codec(),
                    self.device,
                    self.step(),
                    NOW_US,
                    repository::NewHabit::plain(name, started_on, position),
                )
            })
            .expect("a habit can be written")
            .id
    }

    /// Marks every day of a span.
    fn seed_every_day(&self, habit: Uuid, from: CivilDay, to: CivilDay) {
        self.seed(habit, from, to, None);
    }

    /// The same, leaving one day unmarked so that the run ends there.
    fn seed_every_day_except(&self, habit: Uuid, from: CivilDay, to: CivilDay, skip: CivilDay) {
        self.seed(habit, from, to, Some(skip));
    }

    /// One transaction rather than one per row, because three and a half thousand implicit
    /// commits would spend minutes setting up a measurement that then runs in milliseconds.
    /// Nothing about what ends up stored changes; only how long the seeding takes.
    fn seed(&self, habit: Uuid, from: CivilDay, to: CivilDay, skip: Option<CivilDay>) {
        self.database
            .in_transaction(|transaction| {
                let codec = self.codec();

                for day in span(from, to) {
                    if Some(day) == skip {
                        continue;
                    }

                    repository::mark(
                        transaction,
                        &codec,
                        self.device,
                        self.step(),
                        NOW_US,
                        repository::Mark {
                            habit_id: habit,
                            day,
                            amount: 1,
                            note: None,
                            target_snapshot: None,
                        },
                    )?;
                }

                Ok(())
            })
            .expect("the history can be written");
    }

    /// Refuses to go on unless the seeding produced the history the budget was written for.
    ///
    /// A benchmark that quietly measured a tenth of the data would still print a number and
    /// still print a verdict, and the verdict would be wrong in the flattering direction. This
    /// is the one check that stops that, and it costs one statement outside every measurement.
    fn expect_history(&self, habit: Uuid, days: i32) {
        let span_of_it = self
            .database
            .with(|connection| repository::history_span(connection, habit))
            .expect("the history reads back");

        assert_eq!(
            span_of_it.entries,
            u32::try_from(days).expect("a count of days fits"),
            "the seeded history is the size the budget was written for"
        );
    }

    /// The habit a stored row describes, read once and outside every measurement.
    fn spec_of(&self, id: Uuid) -> HabitSpec {
        let habit = self
            .database
            .with(|connection| repository::get(connection, &self.codec(), id))
            .expect("the habit reads back")
            .expect("the habit is there");

        HabitSpec::from_row(row_of(&habit)).expect("a habit this benchmark wrote is a habit")
    }

    /// Every habit in the file, with the spec its columns describe.
    fn all_specs(&self) -> Vec<(Uuid, HabitSpec)> {
        self.database
            .with(|connection| {
                let mut all = Vec::new();
                let mut after = None;

                loop {
                    let page =
                        repository::page(connection, &self.codec(), after, repository::MAX_PAGE)?;
                    let Some(last) = page.last() else { break };

                    after = Some(last.hlc);
                    let was_full = page.len() == repository::MAX_PAGE;

                    for habit in page {
                        let spec = HabitSpec::from_row(row_of(&habit))
                            .expect("a habit this benchmark wrote is a habit");
                        all.push((habit.id, spec));
                    }

                    if !was_full {
                        break;
                    }
                }

                Ok(all)
            })
            .expect("the habits read back")
    }

    /// Runs one measurement [`SAMPLES`] times against this database.
    fn sample(&self, mut work: impl FnMut(&Connection) -> Result<Timing, DbError>) -> Vec<Timing> {
        (0..SAMPLES)
            .map(|_| {
                self.database
                    .with(|connection| work(connection))
                    .expect("the measurement reads")
            })
            .collect()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
