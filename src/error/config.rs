//! Configuration file errors.

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
}

impl ConfigError {
    /// Return the process exit code for this error.
    pub fn exit_code(&self) -> u8 {
        4 // config error
    }
}
