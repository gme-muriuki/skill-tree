//! Top-level CLI error envelope.
//!
//! Each subsystem owns its own error type with its own `.exit_code()`.
//! `CliError` wraps them so a single `main()` dispatch can route any
//! failure to a stderr message and a process exit code.

use std::path::PathBuf;

use crate::error::{BuildError, ConfigError, CycleReport, GitHubError, RenderError};

/// Any error that can surface from a CLI subcommand.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Reading / parsing / validating `.skill-tree.toml`.
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// Talking to GitHub (network, GraphQL, rate limit, config mismatch).
    #[error(transparent)]
    GitHub(#[from] GitHubError),

    /// Building the typed graph (self-edges; only this for now).
    #[error(transparent)]
    Build(#[from] BuildError),

    /// Validating the graph: a cycle was found.
    #[error(transparent)]
    Cycle(#[from] CycleReport),

    /// SVG generation failed (missing `dot`, dot exited non-zero, etc).
    #[error(transparent)]
    Render(#[from] RenderError),

    /// Writing the rendered output to a file failed.
    #[error("failed to write output to {path}: {source}")]
    FileWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Writing the rendered output to stdout failed.
    #[error("failed to write output to stdout: {0}")]
    StdoutWrite(#[source] std::io::Error),
}

impl CliError {
    /// Process exit code. Defers to the wrapped subsystem error so the
    /// existing 1 / 3 / 4 conventions are preserved.
    pub fn exit_code(&self) -> u8 {
        match self {
            CliError::Config(e) => e.exit_code(),
            CliError::GitHub(e) => e.exit_code(),
            CliError::Build(e) => e.exit_code(),
            CliError::Cycle(e) => e.exit_code(),
            CliError::Render(e) => e.exit_code(),
            CliError::FileWrite { .. } | CliError::StdoutWrite(_) => 1,
        }
    }
}
