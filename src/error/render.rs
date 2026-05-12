//! Render errors.
//!
//! `to_dot` itself is infallible (the graph is already validated before
//! it reaches this layer). These variants cover the SVG-generation
//! path, which shells out to the system `dot` binary.

/// Failures during SVG generation.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The system `dot` binary was not found on PATH. The CLI surfaces
    /// the install pointer in the user-facing error.
    #[error(
        "`dot` binary not found in PATH — install graphviz from https://graphviz.org/download/"
    )]
    DotNotFound,

    /// `dot` exited non-zero. The captured stderr is included verbatim.
    #[error("`dot` exited with status {status}: {stderr}")]
    DotFailed { status: i32, stderr: String },
}

impl RenderError {
    /// Process exit code. Unrenderable output → exit 1, same category
    /// as missing data or self-edges.
    pub fn exit_code(&self) -> u8 {
        1
    }
}
