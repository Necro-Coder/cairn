//! The boundary between the WebView and the Rust core.
//!
//! This is the only surface the frontend can reach, and it is deliberately small. A
//! command is a complete business operation, never a generic accessor: `unlock_vault`
//! rather than `get_field`. That keeps the number of round trips down and, more
//! importantly, keeps the number of things an attacker who reaches the WebView can ask
//! for down as well.
//!
//! Commands are a thin shell. They validate their input, hand the work to the core and
//! map the result. Logic that is worth testing does not live here; it lives in a crate
//! that can be tested without a window.

pub mod app_info;
pub mod backup;
pub mod diagnostics;
pub mod habits;
pub mod instance;
pub mod sample;
pub mod vault;

// Only the types are re-exported. The command functions are referenced through their full
// module path in `generate_handler!`, because the attribute macro generates companion
// items next to each function that a `pub use` does not carry along.
pub use app_info::AppInfo;
pub use backup::{
    BackupError, BackupExportReport, BackupProgress, BackupVerifyReport, PROGRESS_EVENT,
    PasswordSource,
};
pub use diagnostics::{DatabaseStatus, Diagnostics};
pub use habits::{
    DayStateDto, FieldProblem, HabitDetail, HabitDraft, HabitFilter, HabitSummary, HabitsError,
    Listing, StreakDto, WeekProgressDto,
};
pub use sample::{KeysetPage, SampleError, SampleHabit, SeedReport, SeededTable};
pub use vault::{
    ConditionReport, InactivityChoice, KdfReport, StatusInputs, StrengthReport, VaultError,
    VaultStatus,
};
