//! Graph validation: cycle detection via iterative DFS over the
//! dependency-bearing edge kinds.
//!
//! Self-edges are rejected by [`crate::graph::Graph::from_fetch`] and
//! never reach this layer. Off-board endpoints already either became
//! ghost nodes or were dropped — every edge in the input has both
//! endpoints in `Graph.nodes`.
//!
//! **Cross-references are excluded from cycle detection.** GitHub
//! cross-references are commonly bidirectional (A mentions #B, B
//! mentions #A) and the render layer already treats them as decorative
//! via `constraint=false` — they do not represent a dependency
//! relationship. Including them here would reject perfectly fine boards
//! as cyclic.
//!
//! Strategy: classic three-colour DFS with parent pointers, walked from
//! each `Graph.nodes` entry in stored (sorted) order. The first
//! detected back-edge produces a [`CycleReport`]. Finding *all* simple
//! cycles (Johnson's algorithm) is deferred — see
//! `md/design/graph-build.md`.

use std::collections::HashMap;

use crate::error::graph::CycleReport;
use crate::graph::{EdgeKind, Graph, NodeId};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    White,
    Gray,
    Black,
}

impl Graph {
    /// Walk the graph once and report the first cycle, if any. Returns
    /// `Ok(())` when the graph is a DAG.
    ///
    /// The reported cycle starts and ends at the same `NodeId`; its
    /// `kinds` vector lists the edge kind for each step, so a cycle of
    /// length `n` has `n` nodes (first repeated at the end) and
    /// `n - 1` edge kinds — except by convention we report the path
    /// `[A, B, C, A]` (`n + 1` nodes, `n` kinds) so the cycle is
    /// human-readable end-to-end. See [`CycleReport`].
    pub fn validate(&self) -> Result<(), CycleReport> {
        // Adjacency: source NodeId -> Vec<(EdgeKind, target NodeId)>. Edges
        // are already sorted by (source, kind, target), so each list is
        // built in deterministic order. CrossReference edges are skipped
        // — see module doc.
        let mut adjacency: HashMap<&NodeId, Vec<(EdgeKind, &NodeId)>> = HashMap::new();
        for edge in &self.edges {
            if matches!(edge.kind, EdgeKind::CrossReference) {
                continue;
            }
            adjacency
                .entry(&edge.source)
                .or_default()
                .push((edge.kind, &edge.target));
        }

        let mut color: HashMap<&NodeId, Color> =
            self.nodes.iter().map(|n| (&n.id, Color::White)).collect();
        let mut parent: HashMap<&NodeId, (&NodeId, EdgeKind)> = HashMap::new();

        for start in &self.nodes {
            if color.get(&start.id).copied() != Some(Color::White) {
                continue;
            }
            if let Some(report) = dfs(&start.id, &adjacency, &mut color, &mut parent) {
                return Err(report);
            }
        }

        Ok(())
    }
}

/// Iterative DFS rooted at `start`. Returns `Some(CycleReport)` the
/// instant a back-edge is found; otherwise marks every reachable node
/// `Black` and returns `None`.
fn dfs<'g>(
    start: &'g NodeId,
    adjacency: &HashMap<&'g NodeId, Vec<(EdgeKind, &'g NodeId)>>,
    color: &mut HashMap<&'g NodeId, Color>,
    parent: &mut HashMap<&'g NodeId, (&'g NodeId, EdgeKind)>,
) -> Option<CycleReport> {
    // Each stack frame is (node, next-edge-index-to-explore). Pushing
    // `(child, 0)` mimics a recursive call; popping mimics return.
    let mut stack: Vec<(&'g NodeId, usize)> = vec![(start, 0)];
    color.insert(start, Color::Gray);

    while let Some(&(node, idx)) = stack.last() {
        let neighbours = adjacency.get(node).map(Vec::as_slice).unwrap_or(&[]);
        if idx >= neighbours.len() {
            // Finished exploring this node.
            color.insert(node, Color::Black);
            stack.pop();
            continue;
        }
        let (edge_kind, next) = neighbours[idx];
        // Advance the parent frame's edge index.
        if let Some(last) = stack.last_mut() {
            last.1 += 1;
        }

        match color.get(&next).copied().unwrap_or(Color::White) {
            Color::White => {
                color.insert(next, Color::Gray);
                parent.insert(next, (node, edge_kind));
                stack.push((next, 0));
            }
            Color::Gray => {
                // Back-edge → cycle.
                return Some(reconstruct_cycle(node, next, edge_kind, parent));
            }
            Color::Black => {
                // Cross / forward edge — already fully explored, no cycle.
            }
        }
    }

    None
}

/// Build the [`CycleReport`] for a back-edge `cur → target`. Walks
/// parent pointers from `cur` until it reaches `target`, then assembles
/// the path `[target, …, cur, target]` so the cycle is end-to-end
/// readable.
fn reconstruct_cycle(
    cur: &NodeId,
    target: &NodeId,
    back_edge_kind: EdgeKind,
    parent: &HashMap<&NodeId, (&NodeId, EdgeKind)>,
) -> CycleReport {
    let mut nodes_rev: Vec<NodeId> = vec![cur.clone()];
    let mut kinds_rev: Vec<EdgeKind> = Vec::new();
    let mut n: &NodeId = cur;
    while n != target {
        let (p, k) = parent
            .get(&n)
            .copied()
            .expect("parent must exist for every gray node except the DFS root");
        kinds_rev.push(k);
        nodes_rev.push(p.clone());
        n = p;
    }
    // nodes_rev = [cur, A_k, …, A_1, target]; reverse to [target, A_1, …, A_k, cur].
    nodes_rev.reverse();
    kinds_rev.reverse();
    // Close the loop with the back-edge.
    nodes_rev.push(target.clone());
    kinds_rev.push(back_edge_kind);
    CycleReport {
        cycle: nodes_rev,
        kinds: kinds_rev,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, EdgeKind, Node, NodeKind};

    fn iss(owner: &str, repo: &str, number: u64) -> NodeId {
        NodeId::Issue {
            owner: owner.into(),
            repo: repo.into(),
            number,
        }
    }

    fn node(id: NodeId) -> Node {
        Node {
            id,
            kind: NodeKind::Issue,
            label: String::new(),
            url: None,
            status: None,
            cluster: None,
            body: None,
            state: None,
            assignees: vec![],
        }
    }

    fn edge(kind: EdgeKind, source: NodeId, target: NodeId) -> Edge {
        Edge {
            kind,
            source,
            target,
        }
    }

    fn graph_with(nodes: Vec<NodeId>, edges: Vec<Edge>) -> Graph {
        let mut g = Graph {
            nodes: nodes.into_iter().map(node).collect(),
            edges,
        };
        g.nodes.sort_by(|a, b| a.id.cmp(&b.id));
        g.edges.sort_by(|a, b| {
            a.source
                .cmp(&b.source)
                .then_with(|| a.kind.cmp(&b.kind))
                .then_with(|| a.target.cmp(&b.target))
        });
        g
    }

    #[test]
    fn empty_graph_validates() {
        let g = Graph::default();
        assert!(g.validate().is_ok());
    }

    #[test]
    fn graph_with_no_edges_validates() {
        let g = graph_with(vec![iss("o", "r", 1), iss("o", "r", 2)], vec![]);
        assert!(g.validate().is_ok());
    }

    #[test]
    fn dag_validates() {
        // 1 -> 2 -> 3, 1 -> 3
        let g = graph_with(
            vec![iss("o", "r", 1), iss("o", "r", 2), iss("o", "r", 3)],
            vec![
                edge(EdgeKind::Blocks, iss("o", "r", 1), iss("o", "r", 2)),
                edge(EdgeKind::Blocks, iss("o", "r", 2), iss("o", "r", 3)),
                edge(EdgeKind::Blocks, iss("o", "r", 1), iss("o", "r", 3)),
            ],
        );
        assert!(g.validate().is_ok());
    }

    #[test]
    fn two_cycle_reports_both_nodes() {
        // 1 -> 2 -> 1
        let g = graph_with(
            vec![iss("o", "r", 1), iss("o", "r", 2)],
            vec![
                edge(EdgeKind::Blocks, iss("o", "r", 1), iss("o", "r", 2)),
                edge(EdgeKind::Blocks, iss("o", "r", 2), iss("o", "r", 1)),
            ],
        );
        let report = g.validate().unwrap_err();
        assert_eq!(
            report.cycle,
            vec![iss("o", "r", 1), iss("o", "r", 2), iss("o", "r", 1)]
        );
        assert_eq!(report.kinds, vec![EdgeKind::Blocks, EdgeKind::Blocks]);
    }

    #[test]
    fn three_cycle_reports_path_in_order() {
        // 1 -> 2 -> 3 -> 1
        let g = graph_with(
            vec![iss("o", "r", 1), iss("o", "r", 2), iss("o", "r", 3)],
            vec![
                edge(EdgeKind::Blocks, iss("o", "r", 1), iss("o", "r", 2)),
                edge(EdgeKind::Blocks, iss("o", "r", 2), iss("o", "r", 3)),
                edge(EdgeKind::Blocks, iss("o", "r", 3), iss("o", "r", 1)),
            ],
        );
        let report = g.validate().unwrap_err();
        assert_eq!(
            report.cycle,
            vec![
                iss("o", "r", 1),
                iss("o", "r", 2),
                iss("o", "r", 3),
                iss("o", "r", 1),
            ]
        );
        assert_eq!(report.kinds.len(), 3);
    }

    #[test]
    fn cycle_across_sub_issue_and_blocks_is_detected() {
        // 1 -SubIssue-> 2 -Blocks-> 3 -Blocks-> 1
        let g = graph_with(
            vec![iss("o", "r", 1), iss("o", "r", 2), iss("o", "r", 3)],
            vec![
                edge(EdgeKind::SubIssue, iss("o", "r", 1), iss("o", "r", 2)),
                edge(EdgeKind::Blocks, iss("o", "r", 2), iss("o", "r", 3)),
                edge(EdgeKind::Blocks, iss("o", "r", 3), iss("o", "r", 1)),
            ],
        );
        let report = g.validate().unwrap_err();
        assert_eq!(
            report.kinds,
            vec![EdgeKind::SubIssue, EdgeKind::Blocks, EdgeKind::Blocks]
        );
    }

    #[test]
    fn bidirectional_cross_reference_is_not_a_cycle() {
        // A common GitHub pattern: A mentions #B, B mentions #A. Both
        // edges are CrossReference and must be ignored by validation.
        let g = graph_with(
            vec![iss("o", "r", 1), iss("o", "r", 2)],
            vec![
                edge(EdgeKind::CrossReference, iss("o", "r", 1), iss("o", "r", 2)),
                edge(EdgeKind::CrossReference, iss("o", "r", 2), iss("o", "r", 1)),
            ],
        );
        assert!(g.validate().is_ok());
    }

    #[test]
    fn cross_reference_closing_a_dependency_path_is_not_a_cycle() {
        // 1 -SubIssue-> 2 -Blocks-> 3 -CrossReference-> 1. The CrossRef
        // edge does not close the cycle for validation purposes.
        let g = graph_with(
            vec![iss("o", "r", 1), iss("o", "r", 2), iss("o", "r", 3)],
            vec![
                edge(EdgeKind::SubIssue, iss("o", "r", 1), iss("o", "r", 2)),
                edge(EdgeKind::Blocks, iss("o", "r", 2), iss("o", "r", 3)),
                edge(EdgeKind::CrossReference, iss("o", "r", 3), iss("o", "r", 1)),
            ],
        );
        assert!(g.validate().is_ok());
    }

    #[test]
    fn disconnected_clean_component_does_not_mask_cycle_in_other_component() {
        // Component A: 1 -> 2 (clean). Component B: 3 -> 4 -> 3 (cycle).
        let g = graph_with(
            vec![
                iss("o", "r", 1),
                iss("o", "r", 2),
                iss("o", "r", 3),
                iss("o", "r", 4),
            ],
            vec![
                edge(EdgeKind::Blocks, iss("o", "r", 1), iss("o", "r", 2)),
                edge(EdgeKind::Blocks, iss("o", "r", 3), iss("o", "r", 4)),
                edge(EdgeKind::Blocks, iss("o", "r", 4), iss("o", "r", 3)),
            ],
        );
        let report = g.validate().unwrap_err();
        assert_eq!(report.cycle.first(), report.cycle.last());
        // Cycle must reference nodes 3 and 4 only.
        for n in &report.cycle {
            let NodeId::Issue { number, .. } = n else {
                panic!("expected issue node")
            };
            assert!(*number == 3 || *number == 4);
        }
    }

    #[test]
    fn forward_and_cross_edges_do_not_trigger_false_cycle() {
        // Diamond: 1 -> 2 -> 4; 1 -> 3 -> 4. No back-edges.
        let g = graph_with(
            vec![
                iss("o", "r", 1),
                iss("o", "r", 2),
                iss("o", "r", 3),
                iss("o", "r", 4),
            ],
            vec![
                edge(EdgeKind::Blocks, iss("o", "r", 1), iss("o", "r", 2)),
                edge(EdgeKind::Blocks, iss("o", "r", 1), iss("o", "r", 3)),
                edge(EdgeKind::Blocks, iss("o", "r", 2), iss("o", "r", 4)),
                edge(EdgeKind::Blocks, iss("o", "r", 3), iss("o", "r", 4)),
            ],
        );
        assert!(g.validate().is_ok());
    }
}
