//! Configuration file errors.

use std::fmt;
use std::path::PathBuf;

/// Error returned when loading or validating a config file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// I/O error reading the config file.
    #[error("failed to read config file {path}: {source}")]
    Io {
        /// Path to the config file that failed to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// TOML parsing error.
    #[error("failed to parse config file {path}: {source}")]
    Parse {
        /// Path to the config file that failed to parse.
        path: PathBuf,
        /// The underlying TOML parse error.
        #[source]
        source: toml::de::Error,
    },

    /// Invalid hex color in the config.
    #[error("invalid hex color in [colors.values]: {key} = {value}")]
    InvalidColor {
        /// The key that had the invalid color.
        key: String,
        /// The invalid color value.
        value: String,
    },

    /// `.skill-tree.toml` was not found in `start` nor any parent directory.
    /// Surfaced only by [`crate::config::SkillTree::discover`] — explicit
    /// `--config PATH` takes a different path that fails with `Io` instead.
    #[error("no .skill-tree.toml found in {start} or any parent directory")]
    NotFound {
        /// The directory the walk started from (typically CWD).
        start: PathBuf,
    },

    /// `std::env::current_dir()` failed — the CWD was deleted or is
    /// unreadable. Surfaced only by [`crate::config::SkillTree::discover`]
    /// when invoked without an explicit `--config` path.
    #[error("could not read current directory: {0}")]
    CwdUnreadable(#[source] std::io::Error),
}

/// One concrete way `.skill-tree.toml` disagrees with the project metadata
/// returned by GitHub. Multiple issues may be reported together via
/// `GitHubError::ConfigMismatch`.
#[derive(Debug, Clone)]
pub enum ConfigIssue {
    /// Config references a field name that does not exist on the project.
    FieldNotFound {
        /// Which TOML section referenced the missing field
        /// (e.g. `"colors"`, `"field"`).
        section: &'static str,
        /// The name as it appears in `.skill-tree.toml`.
        name: String,
    },
    /// Config references a real field but the field is the wrong kind
    /// (e.g. `[colors] github-name` pointed at a TEXT field).
    FieldWrongType {
        /// Which TOML section referenced the field.
        section: &'static str,
        /// The field name.
        name: String,
        /// The kind the section requires.
        expected: &'static str,
        /// The kind GitHub actually reports.
        actual: &'static str,
    },
    /// Config references a SingleSelect option name that is not on the
    /// field's option list (typo, or the option was removed in GitHub).
    OptionNotFound {
        /// Which TOML section listed the option (e.g. `"colors.values"`,
        /// `"cluster.values"`).
        section: &'static str,
        /// The field whose options were searched.
        field: String,
        /// The value name from the TOML table.
        value: String,
    },
}

impl fmt::Display for ConfigIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigIssue::FieldNotFound { section, name } => write!(
                f,
                "[{section}] field {name:?} does not exist on the project"
            ),
            ConfigIssue::FieldWrongType {
                section,
                name,
                expected,
                actual,
            } => write!(
                f,
                "[{section}] field {name:?} is type {actual}, expected {expected}"
            ),
            ConfigIssue::OptionNotFound {
                section,
                field,
                value,
            } => write!(f, "[{section}] {value:?} is not an option of {field:?}"),
        }
    }
}

impl ConfigError {
    /// Return the process exit code for this error.
    pub fn exit_code(&self) -> u8 {
        4 // config error
    }
}
