//! SVG generation by shelling out to the system `dot` binary.
//!
//! See `md/design/render.md` for why we shell out rather than link
//! libgraphviz or use a pure-Rust DOT engine.

use std::io::Write as _;
use std::process::{Command, Stdio};

use crate::error::RenderError;

/// Pipe `dot_input` through `dot -Tsvg` and return the SVG bytes.
///
/// Maps:
/// - `ErrorKind::NotFound` on spawn → [`RenderError::DotNotFound`]
/// - any other spawn / stdin I/O error → [`RenderError::DotSpawn`]
/// - non-zero exit from `dot` → [`RenderError::DotFailed`]
pub fn dot_to_svg(dot_input: &str) -> Result<Vec<u8>, RenderError> {
    let mut child = Command::new("dot")
        .arg("-Tsvg")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RenderError::DotNotFound
            } else {
                RenderError::DotSpawn(e)
            }
        })?;

    // `stdin` is `Some` because we set `Stdio::piped()` above.
    child
        .stdin
        .as_mut()
        .expect("stdin was piped")
        .write_all(dot_input.as_bytes())
        .map_err(RenderError::DotSpawn)?;

    let output = child.wait_with_output().map_err(RenderError::DotSpawn)?;

    if !output.status.success() {
        return Err(RenderError::DotFailed {
            status: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns `true` when `dot -V` runs successfully — the SVG-path
    /// tests below short-circuit to a `println!` skip otherwise so the
    /// suite stays green on machines without graphviz.
    fn dot_available() -> bool {
        Command::new("dot")
            .arg("-V")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[test]
    fn dot_to_svg_round_trips_a_minimal_graph() {
        if !dot_available() {
            println!("skipping: `dot` not on PATH");
            return;
        }
        let svg = dot_to_svg("digraph { a -> b }").expect("dot should accept a minimal graph");
        let text = String::from_utf8_lossy(&svg);
        assert!(text.contains("<svg"), "output not SVG: {text}");
        assert!(text.contains("</svg>"), "output not closed: {text}");
    }

    #[test]
    fn dot_to_svg_surfaces_dot_failed_on_invalid_input() {
        if !dot_available() {
            println!("skipping: `dot` not on PATH");
            return;
        }
        let err = dot_to_svg("not a dot file at all").expect_err("malformed DOT must fail");
        match err {
            RenderError::DotFailed { stderr, .. } => {
                assert!(!stderr.is_empty(), "stderr should carry dot's complaint");
            }
            other => panic!("expected DotFailed, got {other:?}"),
        }
    }
}
