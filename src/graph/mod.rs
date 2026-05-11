//! Typed graph of nodes and edges. Built from a [`crate::github::projects::ProjectFetch`]
//! plus a [`crate::github::issues::RawIssueEdges`] by [`Graph::from_fetch`],
//! validated for cycles by [`Graph::validate`].
//!
//! See `md/design/graph-build.md` for the design.

use std::cmp::Ordering;
use std::fmt;

mod validate;

// ---------------------------------------------------------------------------
// NodeId
// ---------------------------------------------------------------------------

/// Identity of a graph node. Four kinds, one variant each.
///
/// `Issue` covers both GitHub Issues and Pull Requests — they share the
/// `<owner>/<repo>#<number>` namespace, so a single identity variant
/// avoids duplicate nodes when an Issue and a PR happen to share a number
/// across repos.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeId {
    /// On-board Issue or Pull Request, identified by `<owner>/<repo>#<number>`.
    Issue {
        owner: String,
        repo: String,
        number: u64,
    },
    /// Draft issue. Carries GitHub's project-scoped node id (`DI_xxx`).
    Draft(String),
    /// Item the token cannot read (lost permission, deleted, or unknown
    /// content type). Carries the project-item id (`PVTI_xxx`).
    Redacted(String),
    /// Off-board endpoint of a sub-issue or blocking edge, materialized
    /// so the edge can render. Identified by `<owner>/<repo>#<number>`.
    Ghost {
        owner: String,
        repo: String,
        number: u64,
    },
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeId::Issue {
                owner,
                repo,
                number,
            }
            | NodeId::Ghost {
                owner,
                repo,
                number,
            } => write!(f, "{owner}/{repo}#{number}"),
            NodeId::Draft(id) | NodeId::Redacted(id) => f.write_str(id),
        }
    }
}

impl Ord for NodeId {
    fn cmp(&self, other: &Self) -> Ordering {
        sort_key(self).cmp(&sort_key(other))
    }
}

impl PartialOrd for NodeId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Sort key tuple. Categories order Issue+Ghost first, then Draft, then
/// Redacted; within Issue+Ghost the primary key is `(owner, repo, number)`
/// and `Issue` precedes `Ghost` only as a defensive tiebreak — by
/// construction the two cannot share the same tuple (Ghosts materialize
/// only for off-board endpoints).
fn sort_key(id: &NodeId) -> (u8, &str, &str, u64, u8, &str) {
    match id {
        NodeId::Issue {
            owner,
            repo,
            number,
        } => (0, owner, repo, *number, 0, ""),
        NodeId::Ghost {
            owner,
            repo,
            number,
        } => (0, owner, repo, *number, 1, ""),
        NodeId::Draft(id) => (1, "", "", 0, 0, id),
        NodeId::Redacted(id) => (2, "", "", 0, 0, id),
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// One node in the rendered graph.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub label: String,
    pub url: Option<String>,
    /// `display_string()` of the configured `[colors]` field.
    pub status: Option<String>,
    /// `display_string()` of the configured `[cluster]` field.
    pub cluster: Option<String>,
    /// Issue/PR body. Retained for future tooltip rendering.
    pub body: Option<String>,
    /// Issue/PR state (`OPEN`, `CLOSED`, etc.). Retained for future
    /// tooltip rendering.
    pub state: Option<String>,
    /// Logins of users assigned to this node.
    pub assignees: Vec<String>,
}

/// Kind of underlying GitHub object, for render-time styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Issue,
    PullRequest,
    DraftIssue,
    Redacted,
    Ghost,
}

// ---------------------------------------------------------------------------
// Edge
// ---------------------------------------------------------------------------

/// One directed edge in the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub kind: EdgeKind,
    pub source: NodeId,
    pub target: NodeId,
}

/// Kind of relationship. See `md/design/edge-convention.md` for direction
/// and visual-style mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeKind {
    /// `child → parent` from `Issue.subIssues`.
    SubIssue,
    /// `blocker → blocked` from `Issue.trackedIssues`.
    Blocks,
    /// `mentioner → mentioned` from `Issue.timelineItems` filtered to
    /// `CROSS_REFERENCED_EVENT`.
    CrossReference,
}

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

/// Typed graph of nodes and edges. Both vectors are sorted at build time
/// (see `md/design/node-model.md` and `md/design/edge-convention.md`);
/// the value is byte-stable for fixed inputs.
///
/// Adjacency lookups for [`Graph::validate`] and the `unblocked`
/// subcommand are derived on demand — `Graph` carries no redundant
/// indices.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    fn ghost(owner: &str, repo: &str, number: u64) -> NodeId {
        NodeId::Ghost {
            owner: owner.into(),
            repo: repo.into(),
            number,
        }
    }

    // -- Display --

    #[test]
    fn display_issue_uses_owner_slash_repo_hash_number() {
        assert_eq!(issue("o", "r", 42).to_string(), "o/r#42");
    }

    #[test]
    fn display_ghost_uses_owner_slash_repo_hash_number() {
        assert_eq!(ghost("o", "r", 7).to_string(), "o/r#7");
    }

    #[test]
    fn display_draft_uses_raw_id() {
        assert_eq!(NodeId::Draft("DI_xyz".into()).to_string(), "DI_xyz");
    }

    #[test]
    fn display_redacted_uses_raw_id() {
        assert_eq!(NodeId::Redacted("PVTI_abc".into()).to_string(), "PVTI_abc");
    }

    // -- Equality --

    #[test]
    fn issue_and_ghost_with_same_tuple_are_not_equal() {
        // Same tuple but different variants must be distinct identities.
        assert_ne!(issue("o", "r", 1), ghost("o", "r", 1));
    }

    #[test]
    fn issue_equality_matches_on_all_fields() {
        assert_eq!(issue("o", "r", 1), issue("o", "r", 1));
        assert_ne!(issue("o", "r", 1), issue("o", "r", 2));
        assert_ne!(issue("o", "r", 1), issue("o", "s", 1));
    }

    // -- Ord --

    #[test]
    fn sort_orders_issues_by_owner_then_repo_then_number() {
        let mut v = vec![
            issue("o", "r", 2),
            issue("o", "r", 1),
            issue("a", "z", 9),
            issue("o", "q", 5),
        ];
        v.sort();
        assert_eq!(
            v,
            vec![
                issue("a", "z", 9),
                issue("o", "q", 5),
                issue("o", "r", 1),
                issue("o", "r", 2),
            ]
        );
    }

    #[test]
    fn sort_groups_categories_issue_ghost_then_draft_then_redacted() {
        let mut v = vec![
            NodeId::Redacted("PVTI_2".into()),
            NodeId::Draft("DI_b".into()),
            issue("o", "r", 1),
            NodeId::Redacted("PVTI_1".into()),
            ghost("o", "r", 5),
            NodeId::Draft("DI_a".into()),
        ];
        v.sort();
        // Issues + ghosts first, ordered by (owner, repo, number);
        // then drafts by id; then redacted by id.
        assert_eq!(
            v,
            vec![
                issue("o", "r", 1),
                ghost("o", "r", 5),
                NodeId::Draft("DI_a".into()),
                NodeId::Draft("DI_b".into()),
                NodeId::Redacted("PVTI_1".into()),
                NodeId::Redacted("PVTI_2".into()),
            ]
        );
    }

    #[test]
    fn sort_tiebreaks_issue_before_ghost_on_same_tuple() {
        let mut v = vec![ghost("o", "r", 1), issue("o", "r", 1)];
        v.sort();
        assert_eq!(v, vec![issue("o", "r", 1), ghost("o", "r", 1)]);
    }
}
