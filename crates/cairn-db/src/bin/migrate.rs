//! A tool for looking at, raising and lowering the schema of a vault by hand.
//!
//! It exists for two things the application deliberately cannot do. It reverts a migration,
//! which the application never does because a build that can lower its own schema is a build
//! that can do it by accident. And it says what version a file is at without starting a window,
//! which is what somebody repairing a machine needs first.
//!
//! The password is read from standard input and never from an argument. Arguments appear in the
//! process table, in shell history and in the logs of whatever supervises the process, and a
//! vault password in any of those places is the whole vault. Standard input is echoed by the
//! terminal, because this has no terminal dependency and will not grow one to hide a prompt: it
//! is a repair tool, run by the person who owns the machine, in front of the machine.
//!
//! Reverting drops tables. The runner takes a copy of the file first, and the path of that copy
//! is printed, but a copy is not a restore and the last word before anything is dropped is the
//! person typing the command.

// A command line tool whose entire output is its answer. The workspace denies printing so that
// leftover debugging cannot reach a commit; here the printing is the product.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_crypto::{VaultHeader, unlock};
use cairn_db::{
    DATABASE_FILE, Database, HEADER_FILE, LATEST_VERSION,
    migrations::{self, MIGRATIONS},
};
use zeroize::Zeroizing;

/// What the tool was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    /// Say what version the file is at and which migrations this build carries.
    Status,
    /// Apply every migration the file is missing.
    Up,
    /// Revert down to a version, dropping what the migrations above it created.
    Down {
        /// The version to stop at. Zero leaves a file with no tables of ours in it.
        target: u32,
    },
}

/// What the tool was asked to do, and where.
#[derive(Debug)]
struct Request {
    command: Command,
    directory: Option<PathBuf>,
}

fn main() -> ExitCode {
    let request = match parse(std::env::args().skip(1)) {
        Ok(Some(request)) => request,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(problem) => {
            eprintln!("{problem}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match run(&request) {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}

/// What to print when asked for help, and when an argument makes no sense.
const USAGE: &str = "\
Usage: migrate [--directory <path>] <status|up|down <version>>

  status              print the version on disk and the migrations this build carries
  up                  apply every migration the file is missing
  down <version>      revert down to <version>, dropping what the migrations above it made

  --directory <path>  the vault directory; by default the one the application uses,
                      honouring the CAIRN_PROFILE variable

The password is read from standard input. It is never taken as an argument, because
arguments are visible to every other process on the machine.";

/// Reads the arguments, answering `None` when help was asked for.
///
/// # Errors
///
/// Returns a sentence to print when an argument is unknown, missing or not a number.
fn parse(mut arguments: impl Iterator<Item = String>) -> Result<Option<Request>, String> {
    let mut directory = None;
    let mut command = None;

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" => return Ok(None),
            "--directory" => {
                let path = arguments
                    .next()
                    .ok_or_else(|| "--directory needs a path after it".to_owned())?;
                directory = Some(PathBuf::from(path));
            }
            "status" => command = Some(Command::Status),
            "up" => command = Some(Command::Up),
            "down" => {
                let target = arguments
                    .next()
                    .ok_or_else(|| "down needs a version after it".to_owned())?;
                let target = target
                    .parse::<u32>()
                    .map_err(|_not_a_number| format!("{target} is not a version number"))?;
                command = Some(Command::Down { target });
            }
            unknown => return Err(format!("{unknown} is not one of the arguments")),
        }
    }

    command
        .map(|command| Some(Request { command, directory }))
        .ok_or_else(|| "no command was given".to_owned())
}

/// Does what was asked.
///
/// # Errors
///
/// Returns a sentence to print when the directory, the header, the password or the database is
/// not what the command needs. None of them carries a cause from the cryptographic core, for
/// the same reason the application does not: the difference between a wrong password and a
/// damaged header is not something a caller is told.
fn run(request: &Request) -> Result<(), String> {
    let directory = match &request.directory {
        Some(given) => given.clone(),
        None => cairn_platform::paths::data_directory()
            .map_err(|problem| format!("the vault directory could not be resolved: {problem}"))?,
    };

    let header = std::fs::read(directory.join(HEADER_FILE))
        .map_err(|cause| format!("the vault header could not be read: {cause}"))?;
    let header = VaultHeader::parse(&header)
        .map_err(|problem| format!("the vault header is not usable: {problem}"))?;

    let password = read_password()?;
    let vault = unlock(&header, &password)
        .map_err(|_refused| "the vault did not open with that password".to_owned())?;

    let database = Database::open(&directory.join(DATABASE_FILE), &vault.database_key())
        .map_err(|problem| format!("the database could not be opened: {problem}"))?;

    let outcome = act(&database, request.command);

    // Closed whatever happened, and the failure to close is reported rather than hidden behind
    // the failure that came before it: a file this tool is still holding is a file the
    // application will refuse to open next.
    let closed = database
        .close()
        .map_err(|problem| format!("the database could not be closed: {problem}"));

    outcome.and(closed)
}

/// Runs one command against an open database.
fn act(database: &Database, command: Command) -> Result<(), String> {
    match command {
        Command::Status => {
            let found = database
                .with(migrations::applied_version)
                .map_err(|problem| format!("the schema version could not be read: {problem}"))?;

            println!("schema on disk: {found}");
            println!("this build carries: {LATEST_VERSION}");
            for migration in MIGRATIONS {
                let state = if migration.version <= found {
                    "applied"
                } else {
                    "pending"
                };
                println!("  {:04} {:<10} {state}", migration.version, migration.name);
            }

            Ok(())
        }
        Command::Up => {
            let applied = migrations::apply_all(database, now_us())
                .map_err(|problem| format!("the migrations could not be applied: {problem}"))?;

            if applied.versions.is_empty() {
                println!("nothing to apply; the file is already at {}", applied.to);
            } else {
                println!(
                    "applied {:?}: {} to {}",
                    applied.versions, applied.from, applied.to
                );
            }

            Ok(())
        }
        Command::Down { target } => {
            let reverted = migrations::revert_to(database, target)
                .map_err(|problem| format!("the migrations could not be reverted: {problem}"))?;

            if reverted.is_empty() {
                println!("nothing to revert; the file is already at {target} or below");
            } else {
                println!("reverted {reverted:?}; the file is now at {target}");
            }

            Ok(())
        }
    }
}

/// Reads the password from standard input, without the line ending.
///
/// One line, and only one. The obvious alternative — read the whole stream and keep what comes
/// before the first line ending — passes every test in this repository and cannot be used by a
/// person, which is who the tool is for: a terminal that somebody is typing into has no end of
/// file after the Enter key, so reading to the end waits for a key combination nobody thinks to
/// press. A test that feeds the tool through a pipe never notices, because closing the pipe is
/// the end of file the person does not have.
///
/// A password piped in from a file with a trailing newline is still the password and not the
/// password plus a newline, because the line ending is trimmed either way.
///
/// # Errors
///
/// Returns a sentence to print when standard input cannot be read.
fn read_password() -> Result<Zeroizing<String>, String> {
    print!("password: ");
    io::stdout()
        .flush()
        .map_err(|cause| format!("the prompt could not be written: {cause}"))?;

    let mut typed = Zeroizing::new(String::new());
    io::stdin()
        .read_line(&mut typed)
        .map_err(|cause| format!("the password could not be read: {cause}"))?;

    let first_line = typed
        .split(['\r', '\n'])
        .next()
        .unwrap_or_default()
        .to_owned();

    Ok(Zeroizing::new(first_line))
}

/// The moment, in microseconds since the epoch, UTC.
///
/// Zero if the machine's clock is set before the epoch. The value is written into the ledger as
/// the moment a migration was applied, and a clock nobody can trust produces a moment nobody can
/// trust; refusing to run over it would be refusing to repair a machine because its clock is
/// wrong, which is the opposite of what a repair tool is for.
fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_micros()).ok())
        .unwrap_or(0)
}
