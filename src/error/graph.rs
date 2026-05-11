//! Graph build and validation errors.

use crate::graph::{EdgeKind, NodeId};

/// Failures detected during [`crate::graph::Graph::from_fetch`].
///
/// Intentionally narrow: duplicate `NodeId`s are not modeled (items
/// normalize by identity — a duplicate would be a fetch-layer bug), and
/// dangling targets are not modeled (off-board endpoints become ghost
/// nodes for `SubIssue`/`Blocks` or drop for `CrossReference`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// An edge whose source and target are the same node.
    #[error("self-edge on {node} (kind: {kind:?})")]
    SelfEdge { node: NodeId, kind: EdgeKind },
}

impl BuildError {
    /// Process exit code for CLI propagation. A self-edge is unrenderable
    /// input — same category as a malformed GraphQL response.
    pub fn exit_code(&self) -> u8 {
        1
    }
}

/// A cycle discovered by [`crate::graph::Graph::validate`].
///
/// `cycle` carries the back-edge path with the first node repeated at the
/// end (`[A, B, C, A]`). `kinds` is the edge kind for each step and has
/// length `cycle.len() - 1`.
#[derive(Debug, Clone, thiserror::Error)]
pub struct CycleReport {
    pub cycle: Vec<NodeId>,
    pub kinds: Vec<EdgeKind>,
}

impl CycleReport {
    /// Process exit code for CLI propagation. A cycle is unrenderable
    /// input — same category as a malformed GraphQL response.
    pub fn exit_code(&self) -> u8 {
        1
    }
}

impl std::fmt::Display for CycleReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cycle: ")?;
        for (i, node) in self.cycle.iter().enumerate() {
            if i > 0 {
                f.write_str(" → ")?;
            }
            write!(f, "{node}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(owner: &str, repo: &str, number: u64) -> NodeId {
        NodeId::Issue {
            owner: owner.into(),
            repo: repo.into(),
            number,
        }
    }

    #[test]
    fn build_error_self_edge_displays_node_and_kind() {
        let err = BuildError::SelfEdge {
            node: issue("o", "r", 1),
            kind: EdgeKind::SubIssue,
        };
        assert_eq!(err.to_string(), "self-edge on o/r#1 (kind: SubIssue)");
    }

    #[test]
    fn build_error_exits_with_invalid_data_code() {
        let err = BuildError::SelfEdge {
            node: issue("o", "r", 1),
            kind: EdgeKind::Blocks,
        };
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn cycle_report_displays_arrow_separated_path() {
        let report = CycleReport {
            cycle: vec![issue("o", "r", 1), issue("o", "r", 2), issue("o", "r", 1)],
            kinds: vec![EdgeKind::Blocks, EdgeKind::Blocks],
        };
        assert_eq!(report.to_string(), "cycle: o/r#1 → o/r#2 → o/r#1");
    }

    #[test]
    fn cycle_report_exits_with_invalid_data_code() {
        let report = CycleReport {
            cycle: vec![issue("o", "r", 1), issue("o", "r", 1)],
            kinds: vec![EdgeKind::SubIssue],
        };
        assert_eq!(report.exit_code(), 1);
    }
}
