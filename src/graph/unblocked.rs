//! "Unblocked" computation: which issues are ready to pick up.
//!
//! An `Issue`/`PullRequest` node is **unblocked** when its state is `OPEN`
//! and every issue that blocks it is done — or it has no blockers. A
//! blocker is "done" when its state is anything other than `OPEN`
//! (`CLOSED`, or a PR's `MERGED`). Off-board ghost blockers carry no
//! state and so count as still-blocking, which is the conservative
//! choice.
//!
//! Both `Blocks` (blocker → blocked) and `SubIssue` (child → parent)
//! edges gate readiness: a parent is blocked by its open children, just
//! as a blocked issue is blocked by its open blockers. Cross-reference
//! and see-also edges are decorative and do not. This is the shared
//! definition behind the embed "Ready to pick up" style and the
//! `unblocked` subcommand.

use std::collections::{HashMap, HashSet};

use crate::graph::{EdgeKind, Graph, NodeId, NodeKind};

impl Graph {
    /// NodeIds of every issue/PR that is ready to pick up. See the module
    /// docs for the rule.
    pub fn unblocked(&self) -> HashSet<NodeId> {
        let state_by_id: HashMap<&NodeId, Option<&str>> = self
            .nodes
            .iter()
            .map(|n| (&n.id, n.state.as_deref()))
            .collect();

        // downstream node -> its upstream dependencies. A Blocks edge
        // (blocker → blocked) and a SubIssue edge (child → parent) both
        // make the target depend on the source.
        let mut blockers: HashMap<&NodeId, Vec<&NodeId>> = HashMap::new();
        for edge in &self.edges {
            if matches!(edge.kind, EdgeKind::Blocks | EdgeKind::SubIssue) {
                blockers.entry(&edge.target).or_default().push(&edge.source);
            }
        }

        self.nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Issue | NodeKind::PullRequest))
            .filter(|n| n.state.as_deref() == Some("OPEN"))
            .filter(|n| {
                blockers.get(&n.id).is_none_or(|bs| {
                    bs.iter().all(|b| {
                        // done = present and not OPEN; ghost/unknown (None) blocks.
                        matches!(state_by_id.get(b), Some(Some(s)) if *s != "OPEN")
                    })
                })
            })
            .map(|n| n.id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, Node};

    fn issue(owner: &str, repo: &str, number: u64, state: &str) -> Node {
        Node {
            id: NodeId::Issue {
                owner: owner.into(),
                repo: repo.into(),
                number,
            },
            kind: NodeKind::Issue,
            label: format!("#{number}"),
            url: None,
            status: None,
            cluster: None,
            body: None,
            state: Some(state.into()),
            assignees: vec![],
            labels: vec![],
        }
    }

    fn id(owner: &str, repo: &str, number: u64) -> NodeId {
        NodeId::Issue {
            owner: owner.into(),
            repo: repo.into(),
            number,
        }
    }

    fn blocks(src: NodeId, tgt: NodeId) -> Edge {
        Edge {
            kind: EdgeKind::Blocks,
            source: src,
            target: tgt,
        }
    }

    fn sub_issue(child: NodeId, parent: NodeId) -> Edge {
        Edge {
            kind: EdgeKind::SubIssue,
            source: child,
            target: parent,
        }
    }

    fn graph(nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
        Graph { nodes, edges }
    }

    #[test]
    fn open_issue_with_no_blockers_is_unblocked() {
        let g = graph(vec![issue("o", "r", 1, "OPEN")], vec![]);
        assert!(g.unblocked().contains(&id("o", "r", 1)));
    }

    #[test]
    fn closed_issue_is_not_listed() {
        let g = graph(vec![issue("o", "r", 1, "CLOSED")], vec![]);
        assert!(g.unblocked().is_empty());
    }

    #[test]
    fn open_blocker_keeps_target_blocked() {
        // #2 (open) blocks #1 → #1 is not ready.
        let g = graph(
            vec![issue("o", "r", 1, "OPEN"), issue("o", "r", 2, "OPEN")],
            vec![blocks(id("o", "r", 2), id("o", "r", 1))],
        );
        let unblocked = g.unblocked();
        assert!(!unblocked.contains(&id("o", "r", 1)), "blocked by open #2");
        assert!(unblocked.contains(&id("o", "r", 2)), "#2 has no blockers");
    }

    #[test]
    fn closed_blocker_frees_target() {
        // #2 (closed) blocks #1 → #1 is ready; #2 is closed so not listed.
        let g = graph(
            vec![issue("o", "r", 1, "OPEN"), issue("o", "r", 2, "CLOSED")],
            vec![blocks(id("o", "r", 2), id("o", "r", 1))],
        );
        let unblocked = g.unblocked();
        assert!(unblocked.contains(&id("o", "r", 1)));
        assert!(!unblocked.contains(&id("o", "r", 2)));
    }

    #[test]
    fn open_sub_issue_child_blocks_its_parent() {
        // #2 (open) is a sub-issue (child) of parent #1 → parent #1 is not
        // ready until #2 is done.
        let g = graph(
            vec![issue("o", "r", 1, "OPEN"), issue("o", "r", 2, "OPEN")],
            vec![sub_issue(id("o", "r", 2), id("o", "r", 1))],
        );
        let unblocked = g.unblocked();
        assert!(
            !unblocked.contains(&id("o", "r", 1)),
            "parent has open child"
        );
        assert!(unblocked.contains(&id("o", "r", 2)), "leaf child is ready");
    }

    #[test]
    fn merged_pr_blocker_counts_as_done() {
        let mut pr = issue("o", "r", 2, "MERGED");
        pr.kind = NodeKind::PullRequest;
        let g = graph(
            vec![issue("o", "r", 1, "OPEN"), pr],
            vec![blocks(id("o", "r", 2), id("o", "r", 1))],
        );
        assert!(g.unblocked().contains(&id("o", "r", 1)));
    }
}
