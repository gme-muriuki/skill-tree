//! Error types for skill-tree.
//!
//! All errors in skill-tree are organized into modules by origin:
//! - [`github`] — GitHub API errors
//! - [`config`] — configuration file errors
//!
//! Each error type implements `.exit_code()` to map to the appropriate
//! process exit code (1, 3, or 4).

pub mod config;
pub mod github;

pub use config::{ConfigError, ConfigIssue};
pub use github::{GitHubError, NetworkErrorKind};
