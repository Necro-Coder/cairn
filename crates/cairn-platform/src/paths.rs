//! Where this machine keeps the vault, and what a profile name is allowed to be.
//!
//! One directory, decided here and nowhere else. The alternative that was considered and
//! rejected is a configurable path, because a database file inside a folder that a cloud
//! client synchronises is a corrupt database file: two machines writing the same SQLite
//! pages through a service that resolves conflicts by keeping one whole copy will destroy a
//! vault, quietly, and the damage is discovered long after the copy that would have saved it
//! was overwritten.
//!
//! That is also why the environment variable carries a **name** and never a path. Accepting
//! a path in the variable would reopen exactly the thing the fixed directory closes, through
//! a door nobody is looking at. The name is matched against a closed alphabet, so it cannot
//! contain a separator, cannot be `..`, and cannot escape the directory it is joined to.
//!
//! Nothing here reads the vault or knows what is in it. It answers where, and it creates the
//! directory if it has to.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The environment variable that selects an alternative profile.
///
/// Exists for trying something out against a vault that is not the real one. A profile is a
/// whole separate directory, so a test run cannot reach the data of an ordinary run even by
/// mistake.
pub const PROFILE_VARIABLE: &str = "CAIRN_PROFILE";

/// The longest a profile name may be, in characters.
///
/// Thirty-two is well under every path length limit once it is joined to a base directory,
/// which means a valid name can never be the reason a path is too long to create.
pub const MAX_PROFILE_NAME_LEN: usize = 32;

/// The directory every profile lives under, inside the data directory.
const PROFILES_DIRECTORY: &str = "profiles";

/// The name of the application directory inside the platform's data location.
const APPLICATION_DIRECTORY: &str = "Cairn";

/// Why a name or a directory was refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PathError {
    /// The profile name was not a name this accepts.
    #[error("the profile name is not valid: {reason}")]
    ProfileName {
        /// What is wrong with it, in a form that fits the sentence above.
        reason: ProfileNameProblem,
    },

    /// The platform did not say where per-user data belongs.
    ///
    /// On Windows this means `%LOCALAPPDATA%` is not set, which is a broken environment
    /// rather than an ordinary condition. Refusing is the only safe answer: guessing a
    /// directory would put a vault somewhere nobody would think to back up.
    #[error("this system did not say where per-user application data belongs")]
    NoDataDirectory,

    /// The directory could not be created.
    #[error("the data directory could not be created")]
    Create {
        /// The underlying failure, kept so the cause is not lost on the way out.
        #[source]
        cause: io::Error,
    },
}

/// What is wrong with a profile name.
///
/// Each one says which rule was broken rather than repeating the rule, because somebody who
/// typed `../Desktop` is helped by being told that separators are not allowed and not by
/// being shown a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProfileNameProblem {
    /// The name was empty.
    #[error("it is empty")]
    Empty,

    /// The name was longer than [`MAX_PROFILE_NAME_LEN`].
    #[error("it is longer than {MAX_PROFILE_NAME_LEN} characters")]
    TooLong,

    /// The name contained something outside the allowed alphabet.
    ///
    /// Everything hostile lands here: a separator, a colon, a dot, a wildcard, an upper case
    /// letter and every non-ASCII character. There is no denylist to keep up to date, because
    /// the rule is stated as what is allowed.
    #[error("it may only contain lower case letters, digits and hyphens")]
    Disallowed,
}

/// A profile name that has been checked.
///
/// A type rather than a validated string, because the value it carries is joined to a
/// directory path. Something that reaches a `join` has to arrive already proven safe, and the
/// only way to build one of these is through [`ProfileName::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileName(String);

impl ProfileName {
    /// Checks a name against the closed alphabet and keeps it if it passes.
    ///
    /// The alphabet is lower case ASCII letters, digits and the hyphen. That is an allowlist,
    /// which is the only kind of check worth writing here: a denylist of dangerous characters
    /// is a list somebody has to be right about forever, and being wrong once is a path
    /// traversal.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileNameProblem::Empty`] for an empty name,
    /// [`ProfileNameProblem::TooLong`] past [`MAX_PROFILE_NAME_LEN`] characters, and
    /// [`ProfileNameProblem::Disallowed`] for anything outside the alphabet.
    pub fn parse(value: &str) -> Result<Self, ProfileNameProblem> {
        if value.is_empty() {
            return Err(ProfileNameProblem::Empty);
        }
        // Counted in characters rather than bytes, because the message says characters and a
        // message that measures something other than what it names teaches nobody anything.
        // Anything outside the alphabet is refused below in any case, so for an accepted name
        // the two counts agree.
        if value.chars().count() > MAX_PROFILE_NAME_LEN {
            return Err(ProfileNameProblem::TooLong);
        }
        if !value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        }) {
            return Err(ProfileNameProblem::Disallowed);
        }

        Ok(Self(value.to_owned()))
    }

    /// The name, as the single path component it is.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The profile this process was started with, if any.
///
/// An absent variable and an empty one are the same thing: no profile. A variable that is set
/// to something the alphabet refuses is an error rather than a fallback to the real vault,
/// because silently using the real data after being asked for a test profile is the one
/// outcome nobody would want.
///
/// # Errors
///
/// Returns [`PathError::ProfileName`] if the variable is set to something that is not a name.
pub fn profile_from_environment() -> Result<Option<ProfileName>, PathError> {
    profile_from_value(env::var_os(PROFILE_VARIABLE).as_deref())
}

/// The profile a variable's value names, if any.
///
/// Split out from the reading so that every case, including the ones a test cannot produce
/// by setting a variable, is checked as a pure function of an argument. Mutating the
/// environment inside a test is shared state across the whole test binary, and a suite that
/// does it has tests whose result depends on which other tests are running.
///
/// # Errors
///
/// Returns [`PathError::ProfileName`] if the value is set to something that is not a name.
pub fn profile_from_value(value: Option<&OsStr>) -> Result<Option<ProfileName>, PathError> {
    let Some(value) = value else {
        return Ok(None);
    };
    // Bytes that are not UTF-8 cannot be in the alphabet, so they are refused rather than
    // replaced. Lossy conversion here would turn an unreadable name into a different readable
    // one, and the directory it named would not be the one anybody meant.
    let Some(value) = value.to_str() else {
        return Err(PathError::ProfileName {
            reason: ProfileNameProblem::Disallowed,
        });
    };
    if value.is_empty() {
        return Ok(None);
    }

    ProfileName::parse(value)
        .map(Some)
        .map_err(|reason| PathError::ProfileName { reason })
}

/// Where per-user application data belongs on this system, with the application name joined.
///
/// On Windows this is `%LOCALAPPDATA%\Cairn`. Local rather than roaming on purpose: a roaming
/// profile is copied between machines by the domain, which is the same failure as a
/// synchronised folder with a slower fuse on it.
///
/// # Errors
///
/// Returns [`PathError::NoDataDirectory`] if the system does not say where that is.
pub fn base_directory() -> Result<PathBuf, PathError> {
    platform_data_root().map(|root| root.join(APPLICATION_DIRECTORY))
}

/// Where the vault lives, given the base directory and the profile.
///
/// Pure, so that every hostile name can be checked without touching a disk. The real vault is
/// the base directory itself; a profile is a directory two levels below it, which keeps the
/// two from ever being confused by a listing.
#[must_use]
pub fn data_directory_under(base: &Path, profile: Option<&ProfileName>) -> PathBuf {
    match profile {
        None => base.to_path_buf(),
        Some(profile) => base.join(PROFILES_DIRECTORY).join(profile.as_str()),
    }
}

/// Where this process keeps its vault, creating the directory if it is not there.
///
/// The one function the application calls. Everything it is built from is separately testable,
/// which is what this is not: it reads the environment and writes to a disk.
///
/// # Errors
///
/// Returns [`PathError::ProfileName`] for a profile the alphabet refuses,
/// [`PathError::NoDataDirectory`] if the system does not say where data belongs, and
/// [`PathError::Create`] if the directory cannot be created.
pub fn data_directory() -> Result<PathBuf, PathError> {
    let profile = profile_from_environment()?;
    let directory = data_directory_under(&base_directory()?, profile.as_ref());

    fs::create_dir_all(&directory).map_err(|cause| PathError::Create { cause })?;

    Ok(directory)
}

/// The system directory that per-user application data belongs under.
///
/// Three branches rather than one, because the answer is genuinely different on each and a
/// single variable that happens to be set on two of them would be a coincidence to rely on.
fn platform_data_root() -> Result<PathBuf, PathError> {
    if cfg!(windows) {
        return non_empty("LOCALAPPDATA").ok_or(PathError::NoDataDirectory);
    }

    if cfg!(target_vendor = "apple") {
        return non_empty("HOME")
            .map(|home| home.join("Library").join("Application Support"))
            .ok_or(PathError::NoDataDirectory);
    }

    // Everything else follows the freedesktop layout, which is what the pipeline runs on.
    non_empty("XDG_DATA_HOME")
        .or_else(|| non_empty("HOME").map(|home| home.join(".local").join("share")))
        .ok_or(PathError::NoDataDirectory)
}

/// Reads an environment variable, treating an empty value as an absent one.
///
/// An empty variable would otherwise become the current directory, and a vault relative to
/// wherever the process happened to be started is a vault that moves.
fn non_empty(name: &str) -> Option<PathBuf> {
    non_empty_value(env::var_os(name).as_deref())
}

/// The path a variable's value names, treating an empty value as an absent one.
///
/// Pure, for the same reason [`profile_from_value`] is: the case worth testing is the empty
/// one, and producing it by setting a real variable would change what every other test in the
/// binary sees.
fn non_empty_value(value: Option<&OsStr>) -> Option<PathBuf> {
    let value = value?;
    if value.is_empty() {
        return None;
    }

    Some(PathBuf::from(value))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::{Component, Path, PathBuf};

    use super::{
        MAX_PROFILE_NAME_LEN, PathError, ProfileName, ProfileNameProblem, data_directory_under,
        non_empty_value, profile_from_value,
    };

    fn a_base() -> PathBuf {
        Path::new("base").join("Cairn")
    }

    #[test]
    fn an_ordinary_name_is_accepted() {
        for name in ["pruebas", "test-2", "a", "0", "-"] {
            assert!(
                ProfileName::parse(name).is_ok(),
                "{name} should have been accepted"
            );
        }
    }

    #[test]
    fn an_empty_name_is_refused() {
        assert_eq!(ProfileName::parse(""), Err(ProfileNameProblem::Empty));
    }

    #[test]
    fn a_name_of_exactly_the_limit_is_accepted_and_one_more_is_not() {
        let at_limit = "a".repeat(MAX_PROFILE_NAME_LEN);
        assert!(ProfileName::parse(&at_limit).is_ok());

        let over = "a".repeat(MAX_PROFILE_NAME_LEN + 1);
        assert_eq!(ProfileName::parse(&over), Err(ProfileNameProblem::TooLong));
    }

    #[test]
    fn every_way_out_of_the_directory_is_refused() {
        // The list that matters. Each of these, joined to a base directory, would either
        // climb out of it or name something on another volume, and every one of them is
        // outside the alphabet rather than on a list of things to remember.
        let hostile = [
            "..",
            ".",
            "../..",
            r"..\..\Desktop",
            "../../Desktop",
            "/etc",
            r"C:\Windows",
            r"\\server\share",
            "a/b",
            r"a\b",
            "a:b",
            "con", // reserved on Windows only as a whole name; the colon cases cover devices
            "a b",
            "a\u{0}b",
            "Pruebas",
            "PRUEBAS",
            "pruebás",
            "прувас",
            "a*",
            "a?",
            "a|b",
            "a\"b",
            "a\nb",
        ];

        for name in hostile {
            // `con` is in the allowed alphabet and is accepted, which is correct here: it is a
            // directory name and not a file name, and it cannot leave the base directory. It is
            // in the list so that the assertion below has to reason about it rather than the
            // reader assuming it was considered.
            if name == "con" {
                assert!(ProfileName::parse(name).is_ok());
                continue;
            }
            assert!(
                ProfileName::parse(name).is_err(),
                "{name:?} should have been refused"
            );
        }
    }

    #[test]
    fn an_accepted_name_is_always_a_single_component_inside_the_base() {
        // The property the alphabet exists to give. Stated as a property rather than as a
        // list, so a future change to the alphabet is measured against what the check is for.
        let base = a_base();

        for name in ["pruebas", "a", "-", "0", &"z".repeat(MAX_PROFILE_NAME_LEN)] {
            let profile = ProfileName::parse(name).expect("the name is in the alphabet");
            let directory = data_directory_under(&base, Some(&profile));

            assert!(
                directory.starts_with(&base),
                "{} escaped {}",
                directory.display(),
                base.display()
            );
            assert!(
                !directory
                    .components()
                    .any(|component| component == Component::ParentDir),
                "{} contains a parent directory component",
                directory.display()
            );
            assert_eq!(
                directory.components().count(),
                base.components().count() + 2,
                "a profile is exactly two components below the base directory"
            );
        }
    }

    #[test]
    fn no_profile_means_the_base_directory_itself() {
        let base = a_base();
        assert_eq!(data_directory_under(&base, None), base);
    }

    #[test]
    fn an_empty_variable_is_not_a_directory() {
        // An empty variable would otherwise become the current directory, and a vault relative
        // to wherever the process was started is a vault that moves between runs.
        assert_eq!(non_empty_value(Some(OsStr::new(""))), None);
        assert_eq!(non_empty_value(None), None);
        assert_eq!(
            non_empty_value(Some(OsStr::new("somewhere"))),
            Some(PathBuf::from("somewhere"))
        );
    }

    #[test]
    fn an_absent_or_empty_profile_variable_means_the_real_vault() {
        assert_eq!(
            profile_from_value(None).expect("absent is not a failure"),
            None
        );
        assert_eq!(
            profile_from_value(Some(OsStr::new(""))).expect("an empty value is not a failure"),
            None
        );
    }

    #[test]
    fn a_hostile_profile_variable_is_refused_rather_than_ignored() {
        // Refused and not quietly replaced by the real vault. Somebody who asked for a test
        // profile and silently got their real data is the one outcome nobody would want.
        let refused = profile_from_value(Some(OsStr::new(r"..\..\Desktop")))
            .expect_err("a value with separators in it is not a name");

        assert!(matches!(
            refused,
            PathError::ProfileName {
                reason: ProfileNameProblem::Disallowed
            }
        ));
    }

    #[test]
    fn a_profile_variable_in_the_alphabet_names_a_directory_under_the_base() {
        let profile = profile_from_value(Some(OsStr::new("pruebas")))
            .expect("the name is in the alphabet")
            .expect("a non-empty value is a profile");

        assert_eq!(profile.as_str(), "pruebas");
        assert_eq!(
            data_directory_under(Path::new("base"), Some(&profile)),
            Path::new("base").join("profiles").join("pruebas")
        );
    }
}
