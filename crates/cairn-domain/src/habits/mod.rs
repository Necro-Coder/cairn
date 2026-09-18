//! Habits: the rules that turn a column of marked days into a streak.
//!
//! Everything under here is pure. A habit is the one part of this application whose meaning
//! depends on the calendar rather than on the clock, so the calendar comes first and comes on
//! its own: the rest of the module is built on it.

pub mod calendar;
pub mod day;
pub mod spec;

pub use day::{DayState, Entry, classify};
pub use spec::{Aggregation, Direction, HabitRow, HabitSpec, Measure, Period, Schedule, SpecError};
