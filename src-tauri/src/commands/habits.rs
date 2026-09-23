//! Reading habits, writing them down, and changing what they are.
//!
//! The eight commands the habit screens work with, short of the three about a single day. They
//! are a thin shell over three things that already exist: the repository, which reads and writes
//! rows; the domain, which turns a row into a habit and a column of marks into a streak; and the
//! clock crate, which is the only place allowed to ask the operating system what day it is.
//! Nothing here counts anything by itself.
//!
//! Three decisions are worth the words, because all three are invisible from any single line.
//!
//! The note never appears in a list. It is the one sealed column of this module, and the list is
//! what gets painted every time somebody opens the application; a summary of the note, a length
//! of it, even a flag saying there is one, would be sealed content crossing the bridge on the
//! busiest screen there is. It travels in [`habits_get`] alone, which is a screen somebody asked
//! for by name.
//!
//! Today is read once per call and passed down. Which day today is depends on where the person
//! is standing and on the hour they consider a day to start at, and neither is a question the
//! domain is allowed to ask; a habit crate that read a clock would count streaks that cannot be
//! reproduced. If the device does not say what zone it is in, these commands say so and stop.
//! Falling back to UTC would produce a streak that is wrong in a way nobody can see by looking
//! at it.
//!
//! The warning about an edit is worked out here rather than on the screen, and before the write
//! rather than after it. Somebody who changes which days a habit is expected on is changing what
//! their run of two hundred days counted, and they have to be told while they can still say no;
//! the alternative is a second streak walk written in JavaScript, beside the one in the domain
//! and free to disagree with it. `update_preview` answers that question and writes nothing at
//! all, which is the one property of this module worth asserting on the stored revision.

use std::collections::HashMap;

use cairn_clock::{CivilClock, DayStart, SystemZone};
use cairn_db::codec::FieldCodec;
use cairn_db::repositories::habits::{
    self as repository, Habit, MAX_COLOR_LEN, MAX_ICON_LEN, MAX_NAME_LEN, MAX_PAGE, MAX_UNIT_LEN,
    Mark, NewHabit, StoredEntry,
};
use cairn_db::repositories::settings;
use cairn_db::{Connection, DbError};
use cairn_domain::habits::calendar::{shift, span, weekday};
use cairn_domain::habits::spec::SpecError;
use cairn_domain::habits::{
    Aggregation, DayState, Direction, Entry, HabitRow, HabitSpec, Measure, Period, Ratio, Streak,
    classify, completion, streak,
};
use cairn_domain::time::{MAX_YEAR, MIN_YEAR};
use cairn_domain::{CivilDay, Hlc, Timestamp};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::clock::{now_ms, now_us};
use crate::state::AppState;
use crate::storage::Storage;

/// The preference that says how far from midnight this person's day starts.
///
/// Absent means midnight. A row that is present and does not hold a number of minutes is not
/// read as midnight: a damaged preference that silently becomes the default is a streak counted
/// against the wrong day with nothing on the screen to say so.
pub const DAY_START_KEY: &str = "time.day_start_offset_minutes";

/// How many days one streak window covers, counting today.
///
/// Four hundred is a year plus the slack somebody needs to still be looking at a run that
/// started before the last one. A streak still alive at the oldest day of it asks for one more
/// window of the same size and no more, so a habit costs at most two statements.
const WINDOW_DAYS: i32 = 400;

/// The oldest day that may still be marked, counted back from today.
///
/// Thirty. Far enough to fill in a week somebody forgot, near enough that a history is a history
/// and not something anybody can rewrite at will.
pub const MARKABLE_DAYS_BACK: u32 = 30;

/// How many years back a whole history may reach.
///
/// The one read in this module with no natural limit needs an unnatural one. The first day a
/// habit is judged from arrives from the other side of the bridge, and the calendar accepts any
/// year from the first: a habit dated to the year one would be two thousand statements and a
/// vector of three and a half million squares, decided by a number somebody typed into a form.
///
/// Fifty, because that is longer than anybody has been keeping the habit and short enough that
/// the worst case is fifty statements and a few hundred kilobytes. A habit older than this is
/// counted from fifty years ago, and the number it reports is a record over that, which is the
/// one place in this module where a bound is preferred to an answer.
const MAX_HISTORY_YEARS: u16 = 50;

/// The largest quantity one day may be marked with.
///
/// The column has no ceiling of its own — the finance tables have one and this one does not —
/// and this command is the first path from the WebView to it. Nothing compares amounts today
/// except [`classify`], which only compares, so an enormous one is merely absurd; the day the
/// weekly aggregation adds a period's days together, an unbounded one becomes an overflow, and
/// an overflow in this build is a process that stops rather than a number that wraps.
///
/// A million million, which is the ceiling the finance schema already uses. Wide enough for
/// anything counted in millilitres, steps or pages, and narrow enough that a week of them
/// cannot overflow anything.
const MAX_DAY_AMOUNT: i64 = 1_000_000_000_000;

/// Every bit of a schedule mask set, which is the widest one there is.
const WHOLE_WEEK: u8 = 0b111_1111;

/// Why a habit operation did not happen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[non_exhaustive]
pub enum HabitsError {
    /// The vault is closed, and every one of these needs it open.
    #[error("the vault is locked")]
    Locked,

    /// There is no live habit with that identifier.
    #[error("there is no such habit")]
    NotFound,

    /// A field of the draft is not acceptable. Every problem, not the first one.
    #[error("the habit as described is not one this product has")]
    #[serde(rename_all = "camelCase")]
    Invalid {
        /// Everything wrong with the draft, one entry per field.
        problems: Vec<FieldProblem>,
    },

    /// A day was offered that has not happened yet.
    #[error("that day has not happened yet")]
    DayInFuture,

    /// A day was offered from further back than may be marked.
    #[error("that day is further back than may be marked")]
    DayTooOld,

    /// The order offered is not the set of habits there are.
    #[error("the order given is not the set of habits there are")]
    IncompleteOrder,

    /// The device did not say what time zone it is in, so there is no today to count against.
    #[error("the device did not report a time zone")]
    NoZone,

    /// The database refused. Deliberately without the reason.
    ///
    /// Whoever is repairing a machine reaches the cause through the database's own error; what
    /// reaches the screen is that storage failed, for the same reason an unlock says the vault
    /// did not open and nothing else.
    #[error("the database could not complete the operation")]
    Storage,
}

impl From<DbError> for HabitsError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::Closed => Self::Locked,
            DbError::NotFound => Self::NotFound,
            DbError::IncompleteOrder => Self::IncompleteOrder,
            _other => Self::Storage,
        }
    }
}

/// One thing wrong with a draft, named by the field it is wrong about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldProblem {
    /// The field, spelled as the draft spells it.
    pub field: String,
    /// What is wrong with it, as a word the interface turns into a sentence.
    pub code: String,
}

impl FieldProblem {
    /// One problem about one field.
    fn new(field: &str, code: &str) -> Self {
        Self {
            field: field.to_owned(),
            code: code.to_owned(),
        }
    }
}

/// Which habits to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HabitFilter {
    /// The ones still being tracked.
    Active,
    /// The ones put away.
    Archived,
}

impl HabitFilter {
    /// Whether this habit belongs in the list that was asked for.
    const fn admits(self, habit: &Habit) -> bool {
        match self {
            Self::Active => habit.archived_at.is_none(),
            Self::Archived => habit.archived_at.is_some(),
        }
    }
}

/// What one square of the calendar says, in the shape the bridge carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum DayStateDto {
    /// Expected that day, and met.
    Done {
        /// What was actually done, in the habit's unit.
        amount: i64,
        /// What was asked of that day.
        target: i64,
    },
    /// Expected that day, and not met.
    Missed {
        /// What was actually done, in the habit's unit.
        amount: i64,
        /// What was asked of that day.
        target: i64,
    },
    /// Not expected that day.
    NotScheduled {
        /// What was done anyway, which is zero when there is no mark at all.
        amount: i64,
    },
    /// Not expected that day, but met anyway.
    Extra {
        /// What was done, in the habit's unit.
        amount: i64,
        /// What would have been asked, had the day been one of the habit's.
        target: i64,
    },
    /// Before the habit existed, or after today.
    NoData,
}

impl From<DayState> for DayStateDto {
    fn from(state: DayState) -> Self {
        match state {
            DayState::Done { amount, target } => Self::Done { amount, target },
            DayState::Missed { amount, target } => Self::Missed { amount, target },
            DayState::NotScheduled { amount } => Self::NotScheduled { amount },
            DayState::Extra { amount, target } => Self::Extra { amount, target },
            DayState::NoData => Self::NoData,
        }
    }
}

/// How the week in progress is going, in the shape the bridge carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekProgressDto {
    /// Days met so far this week. Not capped: somebody who did five of three sees five.
    pub done: u32,
    /// Days the week asks for.
    pub target: u32,
}

/// A streak, in the shape the bridge carries.
///
/// Exactly the three things the domain answers with. Nothing is added: a field the domain does
/// not have would be a number the interface trusts and nothing computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreakDto {
    /// How long it is. Days for a daily habit, weeks for a weekly one.
    pub days: u32,
    /// Whether today can still be saved and has not been done yet.
    pub at_risk: bool,
    /// For a weekly habit, how the week in progress is going.
    pub week_progress: Option<WeekProgressDto>,
}

impl From<Streak> for StreakDto {
    fn from(value: Streak) -> Self {
        Self {
            days: value.days,
            at_risk: value.at_risk,
            week_progress: value.week_progress.map(|progress| WeekProgressDto {
                done: progress.done,
                target: progress.target,
            }),
        }
    }
}

/// One habit, as the list shows it.
///
/// No note, ever. It is the only sealed column of this module and it travels in [`habits_get`]
/// alone; a summary of it in the list would be sealed content crossing the bridge on every paint
/// of the screen that gets opened most.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HabitSummary {
    /// The row's identifier, as a hyphenated UUID.
    pub id: String,
    /// What it is called.
    pub name: String,
    /// What it is drawn with, if anything was chosen.
    pub icon: Option<String>,
    /// What colour it is drawn in, if anything was chosen.
    pub color: Option<String>,
    /// How often it is judged: `daily` or `weekly`.
    pub period: String,
    /// What the amounts are counted in, for a habit that counts a quantity.
    pub unit: Option<String>,
    /// The quantity a period aims at, or how many days a week a weekly habit is expected on.
    pub target: Option<i64>,
    /// Which way the target is read: `atLeast` or `atMost`.
    pub direction: String,
    /// Seven bits, one per day of the week, Monday lowest.
    ///
    /// Never zero. The schema's zero means "no particular days", which the domain reads as all
    /// seven, and an interface draws days rather than defaults.
    pub schedule_mask: u8,
    /// Where it sits in the person's own order.
    pub position: i64,
    /// Whether it has been put away.
    pub archived: bool,
    /// Which day that square is, as `YYYYMMDD`.
    ///
    /// The core's own answer to what day it is, which depends on the device's time zone and on
    /// how far from midnight this person's day starts. It travels with the square so that a
    /// screen marking today hands the day straight back rather than working one out from a
    /// clock that knows neither of those two things.
    pub today_day: u32,
    /// What today's square says.
    pub today: DayStateDto,
    /// The run as it stands.
    pub streak: StreakDto,
}

/// One habit in full, note included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HabitDetail {
    /// The row's identifier, as a hyphenated UUID.
    pub id: String,
    /// What it is called.
    pub name: String,
    /// What it is drawn with, if anything was chosen.
    pub icon: Option<String>,
    /// What colour it is drawn in, if anything was chosen.
    pub color: Option<String>,
    /// How often it is judged: `daily` or `weekly`.
    pub period: String,
    /// What the amounts are counted in, for a habit that counts a quantity.
    pub unit: Option<String>,
    /// The quantity a period aims at, or how many days a week a weekly habit is expected on.
    pub target: Option<i64>,
    /// Which way the target is read: `atLeast` or `atMost`.
    pub direction: String,
    /// Seven bits, one per day of the week, Monday lowest. Never zero.
    pub schedule_mask: u8,
    /// Where it sits in the person's own order.
    pub position: i64,
    /// Whether it has been put away.
    pub archived: bool,
    /// Which day that square is, as `YYYYMMDD`. See [`HabitSummary::today_day`].
    pub today_day: u32,
    /// What today's square says.
    pub today: DayStateDto,
    /// The run as it stands.
    pub streak: StreakDto,
    /// The note, decrypted, if it has one.
    pub notes: Option<String>,
    /// The first day it is judged on, as `YYYYMMDD`.
    pub started_on: u32,
    /// How the days of a period combine: `sum` or `highest`.
    pub aggregation: String,
}

/// What the form sends.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HabitDraft {
    /// What to call it.
    pub name: String,
    /// The note, or nothing.
    pub notes: Option<String>,
    /// What to draw it with, or nothing.
    pub icon: Option<String>,
    /// What colour to draw it in, or nothing.
    pub color: Option<String>,
    /// How often it is judged: `daily` or `weekly`.
    pub period: String,
    /// What the amounts are counted in. Naming one is what makes the habit count a quantity.
    pub unit: Option<String>,
    /// The quantity a period aims at, or how many days a week a weekly habit is expected on.
    pub target: Option<i64>,
    /// How the days of a period combine: `sum` or `highest`.
    pub aggregation: String,
    /// Which way the target is read: `atLeast` or `atMost`.
    pub direction: String,
    /// Seven bits, one per day of the week, Monday lowest. Zero means every day.
    pub schedule_mask: u8,
    /// The first day it is judged on, as `YYYYMMDD`.
    pub started_on: u32,
}

/// What an edit changed, and whether the streak now means something different.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HabitUpdateOutcome {
    /// The habit as it now stands, note included.
    pub habit: HabitDetail,
    /// Whether the run on the screen is counted by different rules than it was a moment ago.
    pub streak_meaning_changed: bool,
}

/// What saving this draft would do, without saving it.
///
/// Exists because the warning has to appear **before** the write, and the alternative would be
/// working a streak out in JavaScript, which is the one thing this module never does anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateImpact {
    /// Whether the run would be counted by different rules.
    pub streak_meaning_changed: bool,
    /// The run as it stands, counted by the rules the habit has now.
    pub current_streak_before: u32,
    /// The run the same calendar produces once the draft is the habit.
    pub current_streak_after: u32,
    /// Marks that sit on days the new schedule does not include. They stay, dimmed, and count
    /// for nothing.
    pub entries_outside_new_schedule: u32,
}

/// One square of the year, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayCell {
    /// Which square, as `YYYYMMDD`. The whole date and not the day of the month, because that
    /// is what a day is everywhere else on this bridge.
    pub day: u32,
    /// What that square says.
    pub state: DayStateDto,
}

/// A whole year, and how far back the arrows may go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Heatmap {
    /// The year these squares belong to.
    pub year: u16,
    /// Every day of it, oldest first and with no gaps: three hundred and sixty-five of them,
    /// or three hundred and sixty-six in a leap year.
    pub days: Vec<DayCell>,
    /// The earliest year this habit has a mark in, if it has any at all.
    pub first_year_with_data: Option<u16>,
}

/// A part over a whole, in the shape the bridge carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RatioDto {
    /// How many were met.
    pub done: u32,
    /// How many were asked for and are over.
    pub of: u32,
    /// The two above as a percentage, rounded the way the domain rounds it.
    ///
    /// Carried rather than divided on the other side. A percentage worked out in JavaScript is
    /// a second rounding rule, free to disagree with this one about the same two numbers.
    pub percent: u32,
}

impl From<Ratio> for RatioDto {
    fn from(value: Ratio) -> Self {
        Self {
            done: value.done,
            of: value.of,
            percent: value.percent(),
        }
    }
}

/// Everything the detail screen shows in numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HabitStats {
    /// The run as it stands.
    pub current: StreakDto,
    /// The longest run the whole history holds, whether or not it is the one running now.
    pub longest: u32,
    /// How much of the month that holds today has been met.
    pub month_completion: RatioDto,
    /// How many days were ever marked and not unmarked.
    pub total_entries: u32,
    /// The first day ever marked, as `YYYYMMDD`.
    pub first_day: Option<u32>,
    /// The last day ever marked, as `YYYYMMDD`.
    pub last_day: Option<u32>,
}

/// A list, and how much of the file it could not read.
///
/// The count is on this value rather than on the command's because the command's answer is fixed
/// by the bridge contract. A row a synchronisation brought in that this version cannot read is
/// still something that happened, so it is not swallowed: it is counted, it is carried out to
/// everything inside this process, and the tests assert on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// The habits that could be read, in the person's own order.
    pub habits: Vec<HabitSummary>,
    /// How many rows the domain refused, and which are therefore missing above.
    pub unreadable: usize,
}

/// Every habit of one kind, with today's square and the run so far.
///
/// Separate from the command so that the whole thing can be driven by a test without a window,
/// and so that the zone and the moment are parameters rather than calls to the machine.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::NoZone`] if the clock cannot
/// say what day it is, and [`HabitsError::Storage`] if the database refuses.
pub fn list(
    state: &AppState,
    clock: &dyn CivilClock,
    filter: HabitFilter,
    now: i64,
) -> Result<Listing, HabitsError> {
    in_storage(state, |_storage, codec, connection| {
        let today = today_of(connection, codec, clock, now)?;
        let mut habits = all_habits(connection, codec)?;

        // In the person's own order, not the order the rows were last written in. The clock
        // order `page` walks is the order of the index it pages by, and a list drawn in that
        // order reshuffles itself every time somebody marks a day.
        habits.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut summaries = Vec::new();
        let mut unreadable = 0_usize;

        for habit in habits.iter().filter(|habit| filter.admits(habit)) {
            // A row this version cannot read does not take the screen down with it. It arrived
            // from a device that knows something this build does not, and somebody who opened
            // the application to mark a habit is not helped by an error page about a different
            // one. Counted rather than ignored.
            let Ok(spec) = HabitSpec::from_row(row_of(habit)) else {
                unreadable = unreadable.saturating_add(1);
                continue;
            };

            let snapshot = snapshot_of(connection, habit.id, &spec, today)?;
            summaries.push(summary_of(habit, &spec, snapshot));
        }

        Ok(Listing {
            habits: summaries,
            unreadable,
        })
    })
}

/// One habit in full, note included.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::NotFound`] if the identifier
/// does not name a live habit or is not an identifier at all, [`HabitsError::NoZone`] if the
/// clock cannot say what day it is, and [`HabitsError::Storage`] if the database refuses, if the
/// row is not a habit this version understands, or if the note is not text.
pub fn get(
    state: &AppState,
    clock: &dyn CivilClock,
    id: &str,
    now: i64,
) -> Result<HabitDetail, HabitsError> {
    // An identifier that is not one cannot name a row, so it is the same answer as an identifier
    // that names nothing. Nothing is built out of it before it has been parsed.
    let id = Uuid::parse_str(id).map_err(|_not_a_uuid| HabitsError::NotFound)?;

    in_storage(state, |_storage, codec, connection| {
        let today = today_of(connection, codec, clock, now)?;
        let habit = repository::get(connection, codec, id)?.ok_or(HabitsError::NotFound)?;

        detail_of(connection, &habit, today)
    })
}

/// Writes a habit down and answers what it wrote.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::Invalid`] carrying every
/// problem the draft has, [`HabitsError::NoZone`] if the clock cannot say what day it is, and
/// [`HabitsError::Storage`] if the database refuses.
pub fn create(
    state: &AppState,
    clock: &dyn CivilClock,
    draft: &HabitDraft,
    now: i64,
) -> Result<HabitDetail, HabitsError> {
    // Judged before the vault is touched. A draft that was never going to be accepted needs no
    // connection to be refused, and refusing it here keeps the whole of the validation in one
    // place instead of half of it inside a closure.
    let columns = Columns::of(draft)?;
    let millis = now_ms();

    in_storage(state, |storage, codec, connection| {
        let today = today_of(connection, codec, clock, now)?;
        let hlc = storage.next_hlc(millis);
        let position = next_position(connection, codec)?;

        let habit = repository::create(
            connection,
            codec,
            storage.device(),
            hlc,
            now,
            columns.as_new_habit(position),
        )?;

        detail_of(connection, &habit, today)
    })
}

/// Changes a habit to what the draft describes and answers what it now is.
///
/// Does not archive and does not reorder, because [`HabitDraft`] carries neither and a form that
/// could hide either is a habit that leaves the screen because somebody changed its colour.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::NotFound`] if the identifier
/// does not name a live habit or is not an identifier at all, [`HabitsError::Invalid`] carrying
/// every problem the draft has, [`HabitsError::NoZone`] if the clock cannot say what day it is,
/// and [`HabitsError::Storage`] if the database refuses or the stored row is not a habit this
/// version understands.
pub fn update(
    state: &AppState,
    clock: &dyn CivilClock,
    id: &str,
    draft: &HabitDraft,
    now: i64,
) -> Result<HabitUpdateOutcome, HabitsError> {
    let id = Uuid::parse_str(id).map_err(|_not_a_uuid| HabitsError::NotFound)?;
    // Judged before the vault is touched, exactly as `create` does it. A draft that was never
    // going to be accepted needs no connection to be refused, and refusing it out here is also
    // what makes it impossible for a bad draft to leave half a write behind.
    let columns = Columns::of(draft)?;
    let millis = now_ms();

    in_storage(state, |storage, codec, connection| {
        let today = today_of(connection, codec, clock, now)?;
        let stored = repository::get(connection, codec, id)?.ok_or(HabitsError::NotFound)?;

        // Read before the write, because afterwards there is nothing left to compare against.
        let before = spec_of(&stored)?;
        let after = spec_of_columns(&columns)?;
        let meaning = meaning_changed(&before, &after);

        let hlc = storage.next_hlc(millis);
        // The position the row already holds. `update` leaves that column alone; handing it the
        // stored value rather than a zero means the argument says what is true even so.
        let habit = repository::update(
            connection,
            codec,
            hlc,
            now,
            id,
            columns.as_new_habit(stored.position),
        )?;

        Ok(HabitUpdateOutcome {
            habit: detail_of(connection, &habit, today)?,
            streak_meaning_changed: meaning,
        })
    })
}

/// What saving this draft would do, without saving it.
///
/// Writes nothing: no row, no revision, no entry. Everything it touches is a read, and a test
/// holds the stored revision to that.
///
/// One window, read once, classified twice. The two streaks are a comparison rather than a
/// measurement, so both are clipped at the same four hundred days; asking for the older window
/// that [`list`] sometimes asks for would double the reads of a screen somebody is still typing
/// into, and would move the pair of numbers without ever moving the difference between them.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::NotFound`] if the identifier
/// does not name a live habit or is not an identifier at all, [`HabitsError::Invalid`] carrying
/// every problem the draft has, [`HabitsError::NoZone`] if the clock cannot say what day it is,
/// and [`HabitsError::Storage`] if the database refuses or the stored row is not a habit this
/// version understands.
pub fn update_preview(
    state: &AppState,
    clock: &dyn CivilClock,
    id: &str,
    draft: &HabitDraft,
    now: i64,
) -> Result<UpdateImpact, HabitsError> {
    let id = Uuid::parse_str(id).map_err(|_not_a_uuid| HabitsError::NotFound)?;
    let columns = Columns::of(draft)?;

    in_storage(state, |_storage, codec, connection| {
        let today = today_of(connection, codec, clock, now)?;
        let stored = repository::get(connection, codec, id)?.ok_or(HabitsError::NotFound)?;
        let before = spec_of(&stored)?;
        let after = spec_of_columns(&columns)?;

        // The earlier of the two starts, so the one read covers whichever of the two habits
        // reaches further back. Both are already clamped at four hundred days, so the wider of
        // them is still a window the repository accepts.
        let oldest_before = window_start(&before, today, WINDOW_DAYS - 1);
        let oldest_after = window_start(&after, today, WINDOW_DAYS - 1);
        let from = oldest_before.min(oldest_after);
        let entries = repository::window(connection, id, from, today)?;

        Ok(UpdateImpact {
            streak_meaning_changed: meaning_changed(&before, &after),
            current_streak_before: streak_over(&before, from, today, &entries),
            current_streak_after: streak_over(&after, from, today, &entries),
            entries_outside_new_schedule: outside_schedule(&after, &entries),
        })
    })
}

/// Puts a habit away, or brings it back, and answers it as a list shows it.
///
/// Idempotent in both directions: archiving something already archived keeps the day it was
/// first put away, which is a fact about that habit and not about the last time somebody
/// pressed the button.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::NotFound`] if the identifier
/// does not name a live habit or is not an identifier at all, [`HabitsError::NoZone`] if the
/// clock cannot say what day it is, and [`HabitsError::Storage`] if the database refuses or the
/// stored row is not a habit this version understands.
pub fn archive(
    state: &AppState,
    clock: &dyn CivilClock,
    id: &str,
    archived: bool,
    now: i64,
) -> Result<HabitSummary, HabitsError> {
    let id = Uuid::parse_str(id).map_err(|_not_a_uuid| HabitsError::NotFound)?;
    let millis = now_ms();

    in_storage(state, |storage, codec, connection| {
        let today = today_of(connection, codec, clock, now)?;

        // Read, and asked whether this build understands it, before anything is written. The
        // answer this command owes is a summary, and a summary needs the habit the row
        // describes; finding out afterwards that there is none would mean reporting a failure
        // for a row that had already been archived.
        let stored = repository::get(connection, codec, id)?.ok_or(HabitsError::NotFound)?;
        let spec = spec_of(&stored)?;

        let hlc = storage.next_hlc(millis);
        let habit = repository::archive(connection, codec, hlc, now, id, archived)?;

        // The run comes back with it. Archiving does not touch a single entry, so the number a
        // habit carries out of here is the number it carried in, and that is what lets the
        // screen that brings one back draw it without a second call.
        let snapshot = snapshot_of(connection, habit.id, &spec, today)?;

        Ok(summary_of(&habit, &spec, snapshot))
    })
}

/// Removes a habit and every day it was ever marked on.
///
/// Answers nothing. What a screen has to redraw afterwards it asks for with [`list`]: handing
/// back the remaining habits here would be a list built at the moment of a deletion, which is
/// the one moment it is certain to be about to be asked for anyway.
///
/// Takes no clock, because nothing here is about a calendar.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::NotFound`] if the identifier
/// does not name a live habit or is not an identifier at all, and [`HabitsError::Storage`] if
/// the database refuses, in which case nothing at all was written.
pub fn delete(state: &AppState, id: &str, now: i64) -> Result<(), HabitsError> {
    let id = Uuid::parse_str(id).map_err(|_not_a_uuid| HabitsError::NotFound)?;
    let millis = now_ms();

    in_storage(state, |storage, _codec, connection| {
        let hlc = storage.next_hlc(millis);

        repository::delete(connection, hlc, now, id)?;

        Ok(())
    })
}

/// Sets the order of every habit still being tracked, in one transaction.
///
/// The list has to be exactly that set. A partial one cannot tell a habit that moved from a
/// habit dropped by a bug on the other side of the bridge, and the repository refuses it.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::IncompleteOrder`] if the list
/// is not that set or carries something that is not an identifier, and [`HabitsError::Storage`]
/// if the transaction fails, in which case nothing at all was written.
pub fn reorder(state: &AppState, ids: &[String], now: i64) -> Result<(), HabitsError> {
    // Parsed out here, before anything is opened. A list carrying text that is not an identifier
    // is refused without a connection, and the repository never sees a half-converted order.
    let parsed = parsed_ids(ids)?;
    let millis = now_ms();

    in_storage(state, |storage, codec, connection| {
        let hlc = storage.next_hlc(millis);

        repository::reorder(connection, codec, hlc, now, &parsed)?;

        Ok(())
    })
}

/// Marks, unmarks or sets the quantity of one day, and answers what that day now is.
///
/// One command with two behaviours rather than two commands: without `amount` it toggles a
/// done-or-not habit, with it sets a quantity, and `Some(0)` clears it. It answers the resulting
/// state rather than `()` so that two quick taps cannot leave the interface guessing which of
/// them won.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::NotFound`] if the identifier
/// does not name a live habit or is not an identifier at all, [`HabitsError::Invalid`] if the
/// day is not a date or the amount is not a quantity this habit can hold,
/// [`HabitsError::DayInFuture`] and [`HabitsError::DayTooOld`] for a day outside what may be
/// marked, [`HabitsError::NoZone`] if the clock cannot say what day it is, and
/// [`HabitsError::Storage`] if the database refuses, in which case nothing was written.
pub fn toggle_day(
    state: &AppState,
    clock: &dyn CivilClock,
    id: &str,
    day: u32,
    amount: Option<i64>,
    now: i64,
) -> Result<DayStateDto, HabitsError> {
    let id = Uuid::parse_str(id).map_err(|_not_a_uuid| HabitsError::NotFound)?;
    // Judged before the vault is touched, like every other draft in this file. A number that is
    // not a date and a quantity that is not a quantity need no connection to be refused.
    let day = CivilDay::from_number(day).map_err(|_not_a_day| one_problem("day", "notADay"))?;
    if amount.is_some_and(i64::is_negative) {
        return Err(one_problem("amount", "negative"));
    }
    if amount.is_some_and(|quantity| quantity > MAX_DAY_AMOUNT) {
        return Err(one_problem("amount", "tooLarge"));
    }

    let millis = now_ms();

    in_storage(state, |storage, codec, connection| {
        let today = today_of(connection, codec, clock, now)?;

        // Before the transaction, and that is a rule rather than an optimisation: a day nobody
        // may write to must not take a write lock on the file on its way to being refused.
        if day > today {
            return Err(HabitsError::DayInFuture);
        }
        // A habit older than the calendar itself has no day further back to refuse, so the
        // absence of an oldest day is the absence of the limit and not a reason to refuse
        // everything.
        if let Ok(oldest) = shift(today, -markable_days_back())
            && day < oldest
        {
            return Err(HabitsError::DayTooOld);
        }

        let hlc = storage.next_hlc(millis);

        // One transaction over the one connection. Two taps arriving together become two
        // transactions one after the other, rather than two reads that both saw the day
        // unmarked and two writes that then disagree about which of them won.
        let transaction = connection
            .unchecked_transaction()
            .map_err(|_no_transaction| HabitsError::Storage)?;

        let habit = repository::get(&transaction, codec, id)?.ok_or(HabitsError::NotFound)?;
        let spec = spec_of(&habit)?;
        let written = write_day(
            &transaction,
            codec,
            storage,
            hlc,
            now,
            &spec,
            Target {
                id,
                day,
                day_amount: amount,
            },
        )?;

        transaction
            .commit()
            .map_err(|_not_committed| HabitsError::Storage)?;

        Ok(classify(&spec, day, written, today).into())
    })
}

/// One year of squares, ready to draw, and how far back the arrows may go.
///
/// Two statements over the history and no more: the year, and the earliest year there is
/// anything in. Which square was scheduled, which is before the habit existed and which has
/// nothing is worked out here, over those rows, because reading the day of the week out of a
/// `YYYYMMDD` integer in SQL is string surgery on the very column the index covers.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::NotFound`] if the identifier
/// does not name a live habit or is not an identifier at all, [`HabitsError::Invalid`] if the
/// year is outside the calendar this application can name, [`HabitsError::NoZone`] if the clock
/// cannot say what day it is, and [`HabitsError::Storage`] if the database refuses or the stored
/// row is not a habit this version understands.
pub fn heatmap(
    state: &AppState,
    clock: &dyn CivilClock,
    id: &str,
    year: u16,
    now: i64,
) -> Result<Heatmap, HabitsError> {
    let id = Uuid::parse_str(id).map_err(|_not_a_uuid| HabitsError::NotFound)?;
    // Before anything is built, and named as a field rather than reported as storage trouble. A
    // zero is a sentinel somebody used instead of an option and a ten thousand does not fit the
    // eight digits a day is stored in; either would come back empty, which on a screen reads
    // like a person with no history.
    if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
        return Err(one_problem("year", "outOfRange"));
    }

    in_storage(state, |_storage, codec, connection| {
        let today = today_of(connection, codec, clock, now)?;
        let habit = repository::get(connection, codec, id)?.ok_or(HabitsError::NotFound)?;
        let spec = spec_of(&habit)?;

        let entries = repository::year_entries(connection, id, year)?;
        let first_year_with_data = repository::first_year_with_data(connection, id)?;

        // The first and the last day of a year exist in every year there is, and the year has
        // already been checked against the range a day may hold, so neither end can be refused.
        // Reported as storage trouble rather than unwrapped, because this project does not
        // panic to prove a point.
        let (Ok(from), Ok(to)) = (CivilDay::new(year, 1, 1), CivilDay::new(year, 12, 31)) else {
            return Err(HabitsError::Storage);
        };

        let days = classified(&spec, from, to, &entries, today)
            .into_iter()
            .map(|(day, state)| DayCell {
                day: day.as_number(),
                state: state.into(),
            })
            .collect();

        Ok(Heatmap {
            year,
            days,
            first_year_with_data,
        })
    })
}

/// Everything the detail screen shows in numbers.
///
/// The one reading in this module with no fixed budget of statements, because a record cut to a
/// window stops being a record: the history is read whole, one year per statement. That is also
/// why it is the one that must not run on the thread drawing the window.
///
/// # Errors
///
/// [`HabitsError::Locked`] if the vault is closed, [`HabitsError::NotFound`] if the identifier
/// does not name a live habit or is not an identifier at all, [`HabitsError::NoZone`] if the
/// clock cannot say what day it is, and [`HabitsError::Storage`] if the database refuses or the
/// stored row is not a habit this version understands.
pub fn stats(
    state: &AppState,
    clock: &dyn CivilClock,
    id: &str,
    now: i64,
) -> Result<HabitStats, HabitsError> {
    let id = Uuid::parse_str(id).map_err(|_not_a_uuid| HabitsError::NotFound)?;

    in_storage(state, |_storage, codec, connection| {
        let today = today_of(connection, codec, clock, now)?;
        let habit = repository::get(connection, codec, id)?.ok_or(HabitsError::NotFound)?;
        let spec = spec_of(&habit)?;

        // The two extremes and the count come from one statement of three aggregates, not from
        // walking the vector below. A habit with ten years of marks answers those three in the
        // same time as one with a week.
        let span = repository::history_span(connection, id)?;

        let from = history_start(&spec, today);
        let entries = whole_history(connection, id, from, today)?;
        let days = classified(&spec, from, today, &entries, today);

        Ok(HabitStats {
            current: streak::current(&spec, &days, today).streak.into(),
            longest: streak::longest(&spec, &days, today),
            month_completion: completion::month(&spec, &days, today.year(), today.month(), today)
                .into(),
            total_entries: span.entries,
            first_day: span.first.map(CivilDay::as_number),
            last_day: span.last.map(CivilDay::as_number),
        })
    })
}

/// Every habit of one kind, with today's square and the run so far.
///
/// Marked to run off the drawing thread. It walks up to eight hundred days of calendar for every
/// habit there is, and the thread this would otherwise run on is the one drawing the window.
///
/// # Errors
///
/// See [`list`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_list(
    state: tauri::State<'_, AppState>,
    filter: HabitFilter,
) -> Result<Vec<HabitSummary>, HabitsError> {
    // Read here, and read on every call. The zone is a snapshot of where the device says it is,
    // and somebody who flies east has to get the new one without restarting.
    let zone = SystemZone::detect().map_err(|_no_zone| HabitsError::NoZone)?;

    list(&state, &zone, filter, now_us()).map(|listing| listing.habits)
}

/// One habit in full, note included.
///
/// # Errors
///
/// See [`get`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_get(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<HabitDetail, HabitsError> {
    let zone = SystemZone::detect().map_err(|_no_zone| HabitsError::NoZone)?;

    get(&state, &zone, &id, now_us())
}

/// Writes a habit down and answers what it wrote.
///
/// # Errors
///
/// See [`create`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_create(
    state: tauri::State<'_, AppState>,
    draft: HabitDraft,
) -> Result<HabitDetail, HabitsError> {
    let zone = SystemZone::detect().map_err(|_no_zone| HabitsError::NoZone)?;

    create(&state, &zone, &draft, now_us())
}

/// Changes a habit to what the draft describes and answers what it now is.
///
/// Marked to run off the drawing thread for the reason [`habits_list`] is: it walks the calendar
/// of the habit it just wrote.
///
/// # Errors
///
/// See [`update`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_update(
    state: tauri::State<'_, AppState>,
    id: String,
    draft: HabitDraft,
) -> Result<HabitUpdateOutcome, HabitsError> {
    let zone = SystemZone::detect().map_err(|_no_zone| HabitsError::NoZone)?;

    update(&state, &zone, &id, &draft, now_us())
}

/// What saving this draft would do, without saving it.
///
/// Marked to run off the drawing thread: it walks four hundred days twice, and the screen that
/// asks for it is a form somebody is still typing into.
///
/// # Errors
///
/// See [`update_preview`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_update_preview(
    state: tauri::State<'_, AppState>,
    id: String,
    draft: HabitDraft,
) -> Result<UpdateImpact, HabitsError> {
    let zone = SystemZone::detect().map_err(|_no_zone| HabitsError::NoZone)?;

    update_preview(&state, &zone, &id, &draft, now_us())
}

/// Puts a habit away, or brings it back, and answers it as a list shows it.
///
/// # Errors
///
/// See [`archive`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_archive(
    state: tauri::State<'_, AppState>,
    id: String,
    archived: bool,
) -> Result<HabitSummary, HabitsError> {
    let zone = SystemZone::detect().map_err(|_no_zone| HabitsError::NoZone)?;

    archive(&state, &zone, &id, archived, now_us())
}

/// Removes a habit and every day it was ever marked on.
///
/// # Errors
///
/// See [`delete`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_delete(state: tauri::State<'_, AppState>, id: String) -> Result<(), HabitsError> {
    delete(&state, &id, now_us())
}

/// Sets the order of every habit still being tracked.
///
/// # Errors
///
/// See [`reorder`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_reorder(
    state: tauri::State<'_, AppState>,
    ids: Vec<String>,
) -> Result<(), HabitsError> {
    reorder(&state, &ids, now_us())
}

/// Marks, unmarks or sets the quantity of one day, and answers what that day now is.
///
/// # Errors
///
/// See [`toggle_day`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_toggle_day(
    state: tauri::State<'_, AppState>,
    id: String,
    day: u32,
    amount: Option<i64>,
) -> Result<DayStateDto, HabitsError> {
    let zone = SystemZone::detect().map_err(|_no_zone| HabitsError::NoZone)?;

    toggle_day(&state, &zone, &id, day, amount, now_us())
}

/// One year of squares, ready to draw, and how far back the arrows may go.
///
/// Marked to run off the drawing thread: it classifies three hundred and sixty-six squares.
///
/// # Errors
///
/// See [`heatmap`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_heatmap(
    state: tauri::State<'_, AppState>,
    id: String,
    year: u16,
) -> Result<Heatmap, HabitsError> {
    let zone = SystemZone::detect().map_err(|_no_zone| HabitsError::NoZone)?;

    heatmap(&state, &zone, &id, year, now_us())
}

/// Everything the detail screen shows in numbers.
///
/// Marked to run off the drawing thread, and of the eleven it is the one that most needs to be:
/// it reads a whole history, however many years that is.
///
/// # Errors
///
/// See [`stats`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn habits_stats(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<HabitStats, HabitsError> {
    let zone = SystemZone::detect().map_err(|_no_zone| HabitsError::NoZone)?;

    stats(&state, &zone, &id, now_us())
}

/// Runs something that needs the keys and the open database, and collapses the two error types.
///
/// The closure answers in this module's error rather than the database's, so the repository's
/// reasons never reach a screen: everything that is not a missing row, a closed vault or a
/// refused order comes out as [`HabitsError::Storage`] and nothing else.
fn in_storage<T>(
    state: &AppState,
    work: impl FnOnce(&Storage, &FieldCodec<'_>, &Connection) -> Result<T, HabitsError>,
) -> Result<T, HabitsError> {
    let outcome = state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage
                .database()
                .with(|connection| Ok(work(storage, &codec, connection)))
        })
        .ok_or(HabitsError::Locked)?;

    outcome.map_err(HabitsError::from)?
}

/// Which day it is for the person holding this device.
///
/// Never falls back to UTC, and never falls back to midnight for a preference that is there and
/// unreadable. Either fallback would produce a streak that is wrong in a way nothing on the
/// screen could show.
fn today_of(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    clock: &dyn CivilClock,
    now: i64,
) -> Result<CivilDay, HabitsError> {
    let start = day_start(connection, codec)?;

    clock
        .day_of(Timestamp::from_micros(now), start)
        .map_err(|_no_day| HabitsError::NoZone)
}

/// How far from midnight this person's day starts.
fn day_start(connection: &Connection, codec: &FieldCodec<'_>) -> Result<DayStart, HabitsError> {
    let Some(setting) = settings::get(connection, codec, DAY_START_KEY)? else {
        return Ok(DayStart::MIDNIGHT);
    };
    let Some(stored) = setting.value else {
        return Ok(DayStart::MIDNIGHT);
    };

    let text = std::str::from_utf8(&stored).map_err(|_not_text| HabitsError::Storage)?;
    let minutes = text
        .trim()
        .parse::<i32>()
        .map_err(|_not_a_number| HabitsError::Storage)?;

    DayStart::from_minutes(minutes).map_err(|_outside_half_a_day| HabitsError::Storage)
}

/// Every live habit in the file, archived ones included.
///
/// Paged rather than read in one statement, because the repository caps a page and that cap is
/// what stops a number from the other side of the bridge deciding how much memory this process
/// reserves. A person has tens of habits, so this is one statement in practice.
fn all_habits(connection: &Connection, codec: &FieldCodec<'_>) -> Result<Vec<Habit>, DbError> {
    let mut all: Vec<Habit> = Vec::new();
    let mut after = None;

    loop {
        let page = repository::page(connection, codec, after, MAX_PAGE)?;
        let Some(last) = page.last() else { break };

        after = Some(last.hlc);
        let was_full = page.len() == MAX_PAGE;
        all.extend(page);

        if !was_full {
            break;
        }
    }

    Ok(all)
}

/// Where the next habit goes, which is after every one there is.
///
/// Counted over the archived ones as well. A position is unique across the file, and reusing the
/// number an archived habit holds would put the two on top of each other the day it comes back.
fn next_position(connection: &Connection, codec: &FieldCodec<'_>) -> Result<i64, DbError> {
    let last = all_habits(connection, codec)?
        .iter()
        .map(|habit| habit.position)
        .max();

    Ok(last.map_or(0, |position| position.saturating_add(1)))
}

/// The columns of a stored habit, in the shape the domain reads.
fn row_of(habit: &Habit) -> HabitRow {
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

/// Today's square and the run so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Snapshot {
    /// Which day that square is, as `YYYYMMDD`.
    today_day: u32,
    /// What today's square says.
    today: DayStateDto,
    /// The run as it stands.
    streak: StreakDto,
}

/// Reads a habit's recent calendar and works out both of the numbers a screen shows.
///
/// At most two statements, and the second only when the run was still alive at the oldest day of
/// the first. A window is clamped at the day the habit started, so a young habit is read once and
/// the domain never reports that there is older calendar to ask for when there is none.
fn snapshot_of(
    connection: &Connection,
    id: Uuid,
    spec: &HabitSpec,
    today: CivilDay,
) -> Result<Snapshot, HabitsError> {
    let from = window_start(spec, today, WINDOW_DAYS - 1);
    let mut entries = repository::window(connection, id, from, today)?;
    let mut days = classified(spec, from, today, &entries, today);
    let mut run = streak::current(spec, &days, today);

    // One more window of the same size, and only one. It ends on the day before the first one
    // began, so the two are contiguous and the walk sees an unbroken calendar.
    if run.reached_window_start
        && let Ok(older_end) = shift(today, -WINDOW_DAYS)
    {
        let older_start = window_start(spec, today, (WINDOW_DAYS * 2) - 1);
        let mut older = repository::window(connection, id, older_start, older_end)?;

        older.append(&mut entries);
        entries = older;
        days = classified(spec, older_start, today, &entries, today);
        run = streak::current(spec, &days, today);
    }

    let mark = entries.iter().find(|entry| entry.day == today);

    Ok(Snapshot {
        today_day: today.as_number(),
        today: classify(spec, today, mark.map(entry_of), today).into(),
        streak: run.streak.into(),
    })
}

/// The oldest day a window may reach, never before the habit existed.
///
/// Clamped rather than asked for. A window that starts before the habit started is a statement
/// over days that cannot be judged, and it would also make the domain report that there is an
/// older window when there is nothing behind it.
fn window_start(spec: &HabitSpec, today: CivilDay, days_back: i32) -> CivilDay {
    let reached = shift(today, -days_back).unwrap_or(spec.started_on);

    reached.max(spec.started_on)
}

/// One stored mark, as the domain reads one.
fn entry_of(entry: &StoredEntry) -> Entry {
    Entry {
        day: entry.day,
        amount: entry.amount,
        target_snapshot: entry.target_snapshot,
    }
}

/// Every day of the span, classified, oldest first and with no gaps.
///
/// The contiguity is the contract the streak walk depends on: one entry per calendar day, so a
/// day with no mark is present and says so rather than being absent and looking like the day
/// before it.
fn classified(
    spec: &HabitSpec,
    from: CivilDay,
    to: CivilDay,
    entries: &[StoredEntry],
    today: CivilDay,
) -> Vec<(CivilDay, DayState)> {
    let marks: HashMap<u32, &StoredEntry> = entries
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

/// One habit, as the list shows it.
fn summary_of(habit: &Habit, spec: &HabitSpec, snapshot: Snapshot) -> HabitSummary {
    HabitSummary {
        id: habit.id.to_string(),
        name: habit.name.clone(),
        icon: habit.icon.clone(),
        color: habit.color.clone(),
        period: period_name(spec.period).to_owned(),
        unit: habit.unit.clone(),
        target: habit.target_per_period,
        direction: direction_name(spec.direction).to_owned(),
        schedule_mask: spec.schedule.as_mask(),
        position: habit.position,
        archived: habit.archived_at.is_some(),
        today_day: snapshot.today_day,
        today: snapshot.today,
        streak: snapshot.streak,
    }
}

/// One habit in full, note included.
fn detail_of(
    connection: &Connection,
    habit: &Habit,
    today: CivilDay,
) -> Result<HabitDetail, HabitsError> {
    // A habit asked for by name is not a list: a row this version cannot read has to say so
    // here, because there is nothing else on the screen to fall back to.
    let spec = HabitSpec::from_row(row_of(habit)).map_err(|_not_a_habit| HabitsError::Storage)?;
    let snapshot = snapshot_of(connection, habit.id, &spec, today)?;
    let summary = summary_of(habit, &spec, snapshot);

    // Not a lossy conversion. A note that is not text is a note that was written by something
    // else or damaged, and replacing the bytes it cannot read would hand back a note nobody
    // ever wrote.
    let notes = habit
        .notes
        .as_deref()
        .map(|bytes| String::from_utf8(bytes.clone()).map_err(|_not_text| HabitsError::Storage))
        .transpose()?;

    Ok(HabitDetail {
        id: summary.id,
        name: summary.name,
        icon: summary.icon,
        color: summary.color,
        period: summary.period,
        unit: summary.unit,
        target: summary.target,
        direction: summary.direction,
        schedule_mask: summary.schedule_mask,
        position: summary.position,
        archived: summary.archived,
        today_day: summary.today_day,
        today: summary.today,
        streak: summary.streak,
        notes,
        started_on: habit.started_on.as_number(),
        aggregation: aggregation_name(&spec).to_owned(),
    })
}

/// How often the habit is judged, as the bridge spells it.
const fn period_name(period: Period) -> &'static str {
    match period {
        Period::Daily => "daily",
        Period::Weekly => "weekly",
    }
}

/// Which way the target is read, as the bridge spells it.
const fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::AtLeast => "atLeast",
        Direction::AtMost => "atMost",
    }
}

/// How the days of a period combine, as the bridge spells it.
///
/// A habit that counts nothing keeps the column at the sum, which is what the schema's default
/// is and what the form shows the moment somebody gives the habit a unit.
fn aggregation_name(spec: &HabitSpec) -> &'static str {
    match spec.measure {
        Measure::Quantity {
            aggregation: Aggregation::Highest,
            ..
        } => "highest",
        Measure::Quantity { .. } | Measure::DoneOrNot => "sum",
    }
}

/// A draft that has been judged, in the columns the repository writes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Columns {
    /// What to call it, with the surrounding space taken off.
    name: String,
    /// The note, or nothing.
    notes: Option<String>,
    /// What to draw it with, or nothing.
    icon: Option<String>,
    /// What colour to draw it in, or nothing.
    color: Option<String>,
    /// How often it is judged.
    period: i64,
    /// What is counted.
    kind: i64,
    /// Which way the target is read.
    direction: i64,
    /// How the days of a period combine.
    aggregation: i64,
    /// Seven bits, one per day of the week.
    schedule_mask: i64,
    /// What the amounts are counted in, or nothing.
    unit: Option<String>,
    /// The quantity a period aims at, or nothing.
    target: Option<i64>,
    /// The first day it is judged on.
    started_on: CivilDay,
}

impl Columns {
    /// The columns a draft describes, or every reason it describes none.
    ///
    /// Every problem, never the first one. A form that reports one mistake at a time is a form
    /// somebody submits four times, and each of those submissions is a round trip.
    fn of(draft: &HabitDraft) -> Result<Self, HabitsError> {
        let mut problems: Vec<FieldProblem> = Vec::new();

        let name = draft.name.trim().to_owned();
        if name.is_empty() {
            problems.push(FieldProblem::new("name", "empty"));
        } else if name.chars().count() > MAX_NAME_LEN {
            problems.push(FieldProblem::new("name", "tooLong"));
        }

        let icon = optional_text(draft.icon.as_deref(), "icon", MAX_ICON_LEN, &mut problems);
        let color = optional_text(
            draft.color.as_deref(),
            "color",
            MAX_COLOR_LEN,
            &mut problems,
        );
        let unit = optional_text(draft.unit.as_deref(), "unit", MAX_UNIT_LEN, &mut problems);

        let period = named(&draft.period, &["daily", "weekly"], "period", &mut problems);
        let direction = named(
            &draft.direction,
            &["atLeast", "atMost"],
            "direction",
            &mut problems,
        );
        let aggregation = named(
            &draft.aggregation,
            &["sum", "highest"],
            "aggregation",
            &mut problems,
        );

        if draft.schedule_mask > WHOLE_WEEK {
            problems.push(FieldProblem::new("scheduleMask", "outOfRange"));
        }
        if draft.target.is_some_and(|target| target <= 0) {
            problems.push(FieldProblem::new("target", "notPositive"));
        }

        let started_on = CivilDay::from_number(draft.started_on).ok();
        if started_on.is_none() {
            problems.push(FieldProblem::new("startedOn", "notADay"));
        }

        // A draft counts a quantity exactly when it names a unit. Without one, a number on a
        // weekly habit is how many days of the week it is expected on, which is the other
        // question that column answers, and a number on a daily one is neither.
        let kind = i64::from(unit.is_some());

        let (Some(period), Some(direction), Some(aggregation), Some(started_on)) =
            (period, direction, aggregation, started_on)
        else {
            return Err(HabitsError::Invalid { problems });
        };

        let columns = Self {
            name,
            notes: draft.notes.clone(),
            icon,
            color,
            period,
            kind,
            direction,
            aggregation,
            schedule_mask: i64::from(draft.schedule_mask),
            unit,
            target: draft.target,
            started_on,
        };

        // Through the domain rather than beside it. Whether these columns are a habit this
        // product has is a question with one answer, and asking it twice in two places is how
        // the two answers drift apart.
        if let Err(refusal) = HabitSpec::from_row(columns.as_row()) {
            problems.push(spec_problem(&refusal, columns.unit.is_none()));
        }

        if problems.is_empty() {
            Ok(columns)
        } else {
            Err(HabitsError::Invalid { problems })
        }
    }

    /// The same columns, in the shape the domain reads.
    fn as_row(&self) -> HabitRow {
        HabitRow {
            period: self.period,
            kind: self.kind,
            direction: self.direction,
            aggregation: self.aggregation,
            schedule_mask: self.schedule_mask,
            unit: self.unit.clone(),
            target_per_period: self.target,
            started_on: self.started_on,
        }
    }

    /// The same columns, in the shape the repository writes.
    fn as_new_habit(&self, position: i64) -> NewHabit<'_> {
        NewHabit {
            name: &self.name,
            notes: self.notes.as_deref().map(str::as_bytes),
            icon: self.icon.as_deref(),
            color: self.color.as_deref(),
            period: self.period,
            kind: self.kind,
            direction: self.direction,
            aggregation: self.aggregation,
            schedule_mask: self.schedule_mask,
            unit: self.unit.as_deref(),
            target_per_period: self.target,
            started_on: self.started_on,
            position,
        }
    }
}

/// A field that may be absent, checked against the limit the schema has for it.
///
/// Absent and blank are the same answer, because a colour of no characters is a value the schema
/// refuses and a form that sends an empty box means the box was left alone.
fn optional_text(
    value: Option<&str>,
    field: &str,
    max: usize,
    problems: &mut Vec<FieldProblem>,
) -> Option<String> {
    let trimmed = value.map(str::trim).filter(|text| !text.is_empty())?;

    if trimmed.chars().count() > max {
        problems.push(FieldProblem::new(field, "tooLong"));
    }

    Some(trimmed.to_owned())
}

/// The column a word stands for, or a problem naming the field that carried it.
fn named(
    value: &str,
    words: &[&str; 2],
    field: &str,
    problems: &mut Vec<FieldProblem>,
) -> Option<i64> {
    let Some(index) = words.iter().position(|word| *word == value) else {
        problems.push(FieldProblem::new(field, "unknown"));
        return None;
    };

    // Two words, so the index is zero or one and the conversion always has an answer.
    // Saturating rather than panicking, as everywhere else in this project that narrows.
    Some(i64::try_from(index).unwrap_or(0))
}

/// The field a refusal from the domain is about.
///
/// The domain names the column and what it held; a form names the input somebody typed. The one
/// translation worth remarking on is the mismatch: a target with no unit beside it is somebody
/// who meant to count something and did not say what, so the field that is wrong is the unit.
fn spec_problem(error: &SpecError, without_a_unit: bool) -> FieldProblem {
    match error {
        SpecError::Period { .. } => FieldProblem::new("period", "unknown"),
        SpecError::Kind { .. } => FieldProblem::new("unit", "unknown"),
        SpecError::Aggregation { .. } => FieldProblem::new("aggregation", "unknown"),
        SpecError::Direction { .. } => FieldProblem::new("direction", "unknown"),
        SpecError::ScheduleMask { .. } => FieldProblem::new("scheduleMask", "outOfRange"),
        SpecError::QuantityIncomplete => FieldProblem::new("target", "missing"),
        SpecError::MeasureMismatch if without_a_unit => FieldProblem::new("unit", "missing"),
        SpecError::MeasureMismatch => FieldProblem::new("unit", "notAllowed"),
        SpecError::Target { .. } => FieldProblem::new("target", "notPositive"),
        SpecError::WeeklyTarget { .. } => FieldProblem::new("target", "tooManyDays"),
        // The enum is open so that a refusal added later does not break this build. A new one
        // would be a column this form does not offer, so there is no field to name and the
        // draft is refused as a whole.
        _added_later => FieldProblem::new("period", "unknown"),
    }
}

/// The habit a stored row describes, or the refusal a screen gets when it is not one.
fn spec_of(habit: &Habit) -> Result<HabitSpec, HabitsError> {
    HabitSpec::from_row(row_of(habit)).map_err(|_not_a_habit| HabitsError::Storage)
}

/// The habit a judged draft describes.
///
/// Cannot fail. [`Columns::of`] asks the domain the same question before it hands the columns
/// back, so a refusal here would mean the two calls disagreed about the same values. Reported as
/// storage trouble rather than unwrapped, because this project does not panic to prove a point.
fn spec_of_columns(columns: &Columns) -> Result<HabitSpec, HabitsError> {
    HabitSpec::from_row(columns.as_row()).map_err(|_already_judged| HabitsError::Storage)
}

/// Whether the two descriptions count a run by different rules.
///
/// Compared through the domain rather than column by column, because two columns can differ and
/// still mean the same thing: a schedule mask of zero is the whole week, and how the days of a
/// period combine is a question a habit that is only done or not never asks.
///
/// The target is deliberately absent. It is written onto every entry as that entry is marked, so
/// raising it changes what is asked of tomorrow and leaves every day already judged exactly as
/// it was; warning about it would teach somebody to dismiss the warning that matters.
fn meaning_changed(before: &HabitSpec, after: &HabitSpec) -> bool {
    before.period != after.period
        || before.direction != after.direction
        || before.schedule.as_mask() != after.schedule.as_mask()
        || aggregation_name(before) != aggregation_name(after)
}

/// How long the run is, for one description of the habit over one window of calendar.
///
/// The window is classified rather than read again, which is what lets one read answer for two
/// descriptions of the same habit.
fn streak_over(spec: &HabitSpec, from: CivilDay, today: CivilDay, entries: &[StoredEntry]) -> u32 {
    let days = classified(spec, from, today, entries, today);

    streak::current(spec, &days, today).streak.days
}

/// How many marks sit on days this schedule does not include.
///
/// They are not deleted and not moved. What somebody has to be told is that they will stop
/// counting, which is a different sentence from the one about losing them.
fn outside_schedule(spec: &HabitSpec, entries: &[StoredEntry]) -> u32 {
    let counted = entries
        .iter()
        .filter(|entry| !spec.schedule.includes(weekday(entry.day)))
        .count();

    // A window is four hundred days, so this never reaches the limit. Saturating rather than
    // panicking, as everywhere else in this file that narrows.
    u32::try_from(counted).unwrap_or(u32::MAX)
}

/// The identifiers a list of texts names, or a refusal about the list as a whole.
///
/// Text that is not an identifier is [`HabitsError::IncompleteOrder`] and never
/// [`HabitsError::NotFound`]: what arrived is an order, and what is wrong with it is that it is
/// not the set of habits there are. Answering that one habit is missing would send the interface
/// looking for a habit nobody named.
fn parsed_ids(ids: &[String]) -> Result<Vec<Uuid>, HabitsError> {
    ids.iter()
        .map(|id| Uuid::parse_str(id).map_err(|_not_a_uuid| HabitsError::IncompleteOrder))
        .collect()
}

/// A refusal about one field, in the shape a form reads.
fn one_problem(field: &str, code: &str) -> HabitsError {
    HabitsError::Invalid {
        problems: vec![FieldProblem::new(field, code)],
    }
}

/// How far back a day may be marked, as the calendar counts days.
///
/// Converted rather than written down twice. Thirty fits an `i32` with room to spare, and
/// saturating rather than panicking is what everywhere else in this file that narrows does.
fn markable_days_back() -> i32 {
    i32::try_from(MARKABLE_DAYS_BACK).unwrap_or(i32::MAX)
}

/// Which day of which habit one call to [`toggle_day`] is about, and what it asked for.
///
/// Three values that always travel together, so that the one function which writes a day does
/// not take seven arguments in an order a reader has to keep straight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Target {
    /// Which habit.
    id: Uuid,
    /// Which day of it.
    day: CivilDay,
    /// What the caller asked to write, if it named a quantity at all.
    day_amount: Option<i64>,
}

/// Writes what one call asks of one day, and answers the mark that day now carries.
///
/// Every path through here is one read and at most one write, and the caller has already opened
/// the transaction both sit inside. Answering with the mark rather than re-reading it is what
/// lets the command classify the result without a third statement.
fn write_day(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    storage: &Storage,
    hlc: Hlc,
    now: i64,
    spec: &HabitSpec,
    target: Target,
) -> Result<Option<Entry>, HabitsError> {
    let Target {
        id,
        day,
        day_amount,
    } = target;

    let wanted = match (day_amount, &spec.measure) {
        // Clearing, whatever the habit measures. Zero is the one quantity that means the day
        // was not done rather than that it was done to that extent.
        (Some(0), _) => None,
        (Some(quantity), _) => Some(quantity),
        // The one habit that can be toggled blind: what a day of it is worth is a one, and the
        // only question is whether the mark is there.
        (None, Measure::DoneOrNot) => {
            if repository::is_marked(connection, id, day)? {
                None
            } else {
                Some(1)
            }
        }
        // Nothing to toggle. Marking a habit that counts millilitres without saying how many
        // would be recording a day nobody described, and guessing the target would be worse.
        (None, Measure::Quantity { .. }) => return Err(one_problem("amount", "missing")),
    };

    let Some(quantity) = wanted else {
        // Already clear is clear. Unmarking a day that carries no mark is not a failure: the
        // state the caller asked for is the state the day is in.
        if repository::is_marked(connection, id, day)? {
            repository::unmark(connection, hlc, now, id, day)?;
        }

        return Ok(None);
    };

    let target_snapshot = match &spec.measure {
        Measure::DoneOrNot => None,
        Measure::Quantity { target, .. } => Some(*target),
    };

    repository::mark(
        connection,
        codec,
        storage.device(),
        hlc,
        now,
        Mark {
            habit_id: id,
            day,
            amount: quantity,
            // No day carries a note yet, and nothing in this phase writes one. Written down
            // because `mark` overwrites that column: the first command that does write a note
            // has to read it first, or this one will erase it.
            note: None,
            target_snapshot,
        },
    )?;

    Ok(Some(Entry {
        day,
        amount: quantity,
        target_snapshot,
    }))
}

/// The oldest day a whole history may be counted from.
///
/// The day the habit started, unless that is further back than [`MAX_HISTORY_YEARS`], and never
/// after today: a habit somebody dated into the future is one square of nothing rather than a
/// span that ends before it begins.
fn history_start(spec: &HabitSpec, today: CivilDay) -> CivilDay {
    let oldest = CivilDay::new(today.year().saturating_sub(MAX_HISTORY_YEARS), 1, 1)
        // Unreachable for any today this calendar can name, since the first of January exists in
        // every year and subtracting saturates at the first of them. Falling back to today
        // rather than panicking, which is what everywhere else in this file that narrows does.
        .unwrap_or(today);

    // Two comparisons rather than `clamp`, which panics when the bounds cross. They cannot
    // cross here, and this way nothing rests on that being noticed again later: a panic in this
    // build is not an exception, it is the process stopping.
    spec.started_on.max(oldest).min(today)
}

/// Every live mark of one habit between two days, read one calendar year at a time.
///
/// The one read in this module with no fixed budget of statements. A record cut to a window
/// stops being a record, so the history is read whole rather than through a window the
/// repository would refuse for anything past two years; one year per statement, over the index
/// that covers exactly this condition. The caller is what bounds how many years that is.
fn whole_history(
    connection: &Connection,
    id: Uuid,
    from: CivilDay,
    to: CivilDay,
) -> Result<Vec<StoredEntry>, HabitsError> {
    let mut all = Vec::new();

    for year in from.year()..=to.year() {
        all.extend(repository::year_entries(connection, id, year)?);
    }

    Ok(all)
}

#[cfg(test)]
mod tests {
    use cairn_domain::habits::{Streak, WeekProgress};

    use super::{DayState, DayStateDto, FieldProblem, HabitsError, StreakDto};

    #[test]
    fn the_error_serialises_as_a_tagged_object_the_interface_can_match_on() {
        let encoded = serde_json::to_string(&HabitsError::NoZone).expect("it serialises");
        assert_eq!(encoded, r#"{"kind":"noZone"}"#);

        let encoded = serde_json::to_string(&HabitsError::Invalid {
            problems: vec![FieldProblem::new("scheduleMask", "outOfRange")],
        })
        .expect("it serialises");
        assert!(encoded.contains(r#""kind":"invalid""#), "{encoded}");
        assert!(encoded.contains(r#""field":"scheduleMask""#), "{encoded}");
        assert!(encoded.contains(r#""code":"outOfRange""#), "{encoded}");
    }

    #[test]
    fn a_square_serialises_as_a_tagged_object_too() {
        let encoded =
            serde_json::to_string(&DayStateDto::from(DayState::NoData)).expect("it serialises");
        assert_eq!(encoded, r#"{"state":"noData"}"#);

        let encoded = serde_json::to_string(&DayStateDto::from(DayState::Done {
            amount: 2,
            target: 1,
        }))
        .expect("it serialises");
        assert_eq!(encoded, r#"{"state":"done","amount":2,"target":1}"#);
    }

    #[test]
    fn a_streak_carries_the_three_things_the_domain_answers_with_and_nothing_else() {
        // A field here that the domain does not have would be a number the interface trusts and
        // nothing computes, so the shape is pinned rather than left to review.
        let encoded = serde_json::to_string(&StreakDto::from(Streak {
            days: 12,
            at_risk: true,
            week_progress: Some(WeekProgress { done: 2, target: 3 }),
        }))
        .expect("it serialises");

        assert_eq!(
            encoded,
            r#"{"days":12,"atRisk":true,"weekProgress":{"done":2,"target":3}}"#
        );
    }

    #[test]
    fn a_daily_streak_says_there_is_no_week_in_progress_rather_than_dropping_the_field() {
        let encoded = serde_json::to_string(&StreakDto::from(Streak {
            days: 1,
            at_risk: false,
            week_progress: None,
        }))
        .expect("it serialises");

        assert_eq!(encoded, r#"{"days":1,"atRisk":false,"weekProgress":null}"#);
    }
}
