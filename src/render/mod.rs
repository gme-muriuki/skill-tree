//! DOT and SVG rendering for a [`Graph`].
//!
//! See `md/design/render.md` for the design decisions that drive the
//! attribute set, the cluster grouping, the chaos-reduction tactics
//! (`constraint=false` on cross-references, `style=rounded` clusters
//! with no fill), and the deterministic output guarantee.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use crate::graph::{Edge, EdgeKind, Graph, Node, NodeKind};

const DEFAULT_COLOR: &str = "#dddddd";
const BODY_TOOLTIP_LIMIT: usize = 200;

/// Render-time options derived from [`crate::config::Config`] plus CLI
/// flags. Owns its data so `to_dot` is independent of the config layer
/// — easier to drive from unit tests.
#[derive(Debug, Clone, Default)]
pub struct RenderOpts {
    /// Option-name → hex (from `[colors.values]`).
    pub colors: HashMap<String, String>,
    /// Option-name → display label (from `[cluster.values]`).
    pub cluster_labels: HashMap<String, String>,
    /// Fallback fill color when a node has no status or its status is
    /// not in `colors`.
    pub default_color: String,
}

/// Render `graph` to a Graphviz DOT document. Infallible — `Graph` is
/// already validated before it reaches this layer.
///
/// Deterministic: byte-identical output for byte-identical inputs. The
/// implementation walks `graph.nodes` and `graph.edges` in their stored
/// (sorted) order; no `HashMap` iteration appears in the output path.
pub fn to_dot(graph: &Graph, opts: &RenderOpts) -> String {
    let default_color = if opts.default_color.is_empty() {
        DEFAULT_COLOR
    } else {
        opts.default_color.as_str()
    };

    let mut out = String::new();
    writeln!(out, "digraph SkillTree {{").unwrap();
    writeln!(out, "    rankdir = \"LR\";").unwrap();

    // Partition nodes into clusters (in first-appearance order under
    // node sort) and a top-level "unclustered" bucket. We need the
    // cluster's display label *and* the ordered list of node indices,
    // so we collect both in one pass.
    let (cluster_order, cluster_members, unclustered) = partition_clusters(&graph.nodes);

    for idx in &unclustered {
        emit_node(&mut out, &graph.nodes[*idx], opts, default_color, 1);
    }

    for cluster_key in &cluster_order {
        emit_cluster(
            &mut out,
            cluster_key,
            cluster_members
                .get(cluster_key)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            &graph.nodes,
            opts,
            default_color,
        );
    }

    for edge in &graph.edges {
        emit_edge(&mut out, edge);
    }

    writeln!(out, "}}").unwrap();
    out
}

/// Walk `nodes` in stored order, producing:
///
/// - `cluster_order` — every distinct `Node.cluster` value in
///   first-appearance order. Drives the order clusters are emitted.
/// - `cluster_members` — for each cluster value, the indices of its
///   member nodes.
/// - `unclustered` — indices of nodes with `cluster == None`.
fn partition_clusters(nodes: &[Node]) -> (Vec<String>, HashMap<String, Vec<usize>>, Vec<usize>) {
    let mut order: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut members: HashMap<String, Vec<usize>> = HashMap::new();
    let mut unclustered: Vec<usize> = Vec::new();

    for (idx, node) in nodes.iter().enumerate() {
        match &node.cluster {
            Some(key) => {
                if seen.insert(key.clone()) {
                    order.push(key.clone());
                }
                members.entry(key.clone()).or_default().push(idx);
            }
            None => unclustered.push(idx),
        }
    }

    (order, members, unclustered)
}

/// One DOT line for a node, fully attribute-styled. `indent_level`
/// gives the leading-space count in multiples of 4 — 1 for top-level,
/// 2 for nodes inside a cluster subgraph.
///
/// `Node.label` is already in the right per-kind form (built by
/// [`crate::graph::Graph::from_fetch`]), so render just propagates it
/// verbatim.
fn emit_node(
    out: &mut String,
    node: &Node,
    opts: &RenderOpts,
    default_color: &str,
    indent_level: usize,
) {
    let indent = "    ".repeat(indent_level);
    let id = quote(&node.id.to_string());
    let label = quote(&node.label);

    write!(out, "{indent}{id} [label={label}").unwrap();

    match node.kind {
        NodeKind::Issue | NodeKind::PullRequest => {
            write!(out, ", shape=box, style=filled").unwrap();
            write!(
                out,
                ", fillcolor={}",
                quote(fill_color(node, opts, default_color))
            )
            .unwrap();
        }
        NodeKind::DraftIssue => {
            write!(out, ", shape=note, style=filled").unwrap();
            write!(
                out,
                ", fillcolor={}",
                quote(fill_color(node, opts, default_color))
            )
            .unwrap();
        }
        NodeKind::Redacted => {
            // `style` carries two values, must be quoted as one.
            write!(out, ", shape=box, style=\"dashed,filled\"").unwrap();
            write!(out, ", fillcolor={}", quote(default_color)).unwrap();
        }
        NodeKind::Ghost => {
            write!(out, ", shape=box, style=dashed").unwrap();
        }
    }

    if let Some(url) = &node.url {
        write!(out, ", URL={}", quote(url)).unwrap();
    }

    if let Some(tip) = node_tooltip(node) {
        write!(out, ", tooltip={}", quote(&tip)).unwrap();
    }

    writeln!(out, "];").unwrap();
}

/// Resolve a node's `fillcolor`: `opts.colors[status]` when present,
/// `default_color` otherwise.
fn fill_color<'a>(node: &Node, opts: &'a RenderOpts, default_color: &'a str) -> &'a str {
    node.status
        .as_deref()
        .and_then(|s| opts.colors.get(s))
        .map(String::as_str)
        .unwrap_or(default_color)
}

/// Per-node tooltip text or `None` (skip the attribute entirely).
///
/// Ghost and Redacted carry no body/state/assignee data, so they get
/// no tooltip — the visual style (dashed border) already conveys what
/// they are.
fn node_tooltip(node: &Node) -> Option<String> {
    if matches!(node.kind, NodeKind::Ghost | NodeKind::Redacted) {
        return None;
    }

    let mut lines: Vec<String> = Vec::new();
    if let Some(state) = &node.state {
        lines.push(format!("State: {state}"));
    }
    if !node.assignees.is_empty() {
        lines.push(format!("Assignees: {}", node.assignees.join(", ")));
    }
    if let Some(body) = &node.body
        && !body.is_empty()
    {
        // Blank line separator before the body, but only if we already
        // added header lines.
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(truncate(body, BODY_TOOLTIP_LIMIT));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Truncate a string to at most `limit` chars, appending `…` when
/// truncated. Operates on chars (not bytes) so it doesn't split UTF-8
/// code points.
fn truncate(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(limit).collect();
        out.push('…');
        out
    }
}

/// One DOT subgraph for a cluster, with its label and contained nodes.
fn emit_cluster(
    out: &mut String,
    cluster_key: &str,
    member_indices: &[usize],
    nodes: &[Node],
    opts: &RenderOpts,
    default_color: &str,
) {
    let cluster_id = quote(&format!("cluster_{cluster_key}"));
    let cluster_label = opts
        .cluster_labels
        .get(cluster_key)
        .cloned()
        .unwrap_or_else(|| cluster_key.to_owned());

    writeln!(out, "    subgraph {cluster_id} {{").unwrap();
    writeln!(out, "        label = {};", quote(&cluster_label)).unwrap();
    writeln!(out, "        style = \"rounded\";").unwrap();
    for idx in member_indices {
        emit_node(out, &nodes[*idx], opts, default_color, 2);
    }
    writeln!(out, "    }}").unwrap();
}

/// One DOT line for a directed edge. Cross-references carry
/// `constraint=false` so they decorate without warping the spine.
fn emit_edge(out: &mut String, edge: &Edge) {
    let src = quote(&edge.source.to_string());
    let tgt = quote(&edge.target.to_string());
    write!(out, "    {src} -> {tgt} [").unwrap();
    match edge.kind {
        EdgeKind::SubIssue => {
            write!(out, "style=solid, tooltip=\"sub-issue\"").unwrap();
        }
        EdgeKind::Blocks => {
            write!(out, "style=solid, tooltip=\"blocks\"").unwrap();
        }
        EdgeKind::CrossReference => {
            write!(
                out,
                "style=dashed, constraint=false, tooltip=\"cross-reference\""
            )
            .unwrap();
        }
    }
    writeln!(out, "];").unwrap();
}

/// Escape a Rust string for use inside DOT double-quotes. Handles the
/// two characters DOT cares about (`\\` and `"`), plus newline
/// (encoded as the literal two-character escape `\n`, which Graphviz
/// renders as a line break inside `tooltip` and `label` values).
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::NodeId;

    // -- builders --

    fn issue_id(owner: &str, repo: &str, number: u64) -> NodeId {
        NodeId::Issue {
            owner: owner.into(),
            repo: repo.into(),
            number,
        }
    }

    fn ghost_id(owner: &str, repo: &str, number: u64) -> NodeId {
        NodeId::Ghost {
            owner: owner.into(),
            repo: repo.into(),
            number,
        }
    }

    fn issue_node(owner: &str, repo: &str, number: u64, title: &str) -> Node {
        Node {
            id: issue_id(owner, repo, number),
            kind: NodeKind::Issue,
            label: format!("#{number}: {title}"),
            url: Some(format!("https://github.com/{owner}/{repo}/issues/{number}")),
            status: None,
            cluster: None,
            body: None,
            state: Some("OPEN".into()),
            assignees: vec![],
        }
    }

    fn ghost_node_value(owner: &str, repo: &str, number: u64) -> Node {
        Node {
            id: ghost_id(owner, repo, number),
            kind: NodeKind::Ghost,
            label: format!("{owner}/{repo}#{number}"),
            url: Some(format!("https://github.com/{owner}/{repo}/issues/{number}")),
            status: None,
            cluster: None,
            body: None,
            state: None,
            assignees: vec![],
        }
    }

    fn draft_node(gh_id: &str, title: &str) -> Node {
        Node {
            id: NodeId::Draft(gh_id.into()),
            kind: NodeKind::DraftIssue,
            label: title.into(),
            url: None,
            status: None,
            cluster: None,
            body: None,
            state: None,
            assignees: vec![],
        }
    }

    fn redacted_node(item_id: &str) -> Node {
        Node {
            id: NodeId::Redacted(item_id.into()),
            kind: NodeKind::Redacted,
            label: "[redacted]".into(),
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

    fn opts() -> RenderOpts {
        RenderOpts {
            colors: HashMap::new(),
            cluster_labels: HashMap::new(),
            default_color: DEFAULT_COLOR.to_owned(),
        }
    }

    fn opts_with_colors(pairs: &[(&str, &str)]) -> RenderOpts {
        RenderOpts {
            colors: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            cluster_labels: HashMap::new(),
            default_color: DEFAULT_COLOR.to_owned(),
        }
    }

    fn opts_with_cluster_labels(pairs: &[(&str, &str)]) -> RenderOpts {
        RenderOpts {
            colors: HashMap::new(),
            cluster_labels: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            default_color: DEFAULT_COLOR.to_owned(),
        }
    }

    // -- snapshot tests --

    #[test]
    fn empty_graph() {
        let g = Graph::default();
        insta::assert_snapshot!(to_dot(&g, &opts()));
    }

    #[test]
    fn single_issue_node() {
        let g = Graph {
            nodes: vec![issue_node("o", "r", 1, "First")],
            edges: vec![],
        };
        insta::assert_snapshot!(to_dot(&g, &opts()));
    }

    #[test]
    fn one_node_per_kind() {
        let mut nodes = vec![
            issue_node("o", "r", 1, "Issue"),
            draft_node("DI_a", "Draft"),
            redacted_node("PVTI_x"),
            ghost_node_value("ext", "lib", 99),
        ];
        // The Issue+Ghost share a sort bucket; node sort would put Ghost
        // after Issue, but here we just leave them so the snapshot shows
        // every kind on its own line.
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let g = Graph {
            nodes,
            edges: vec![],
        };
        insta::assert_snapshot!(to_dot(&g, &opts()));
    }

    #[test]
    fn clusters_with_display_labels() {
        let mut n1 = issue_node("o", "r", 1, "A");
        n1.cluster = Some("compiler-frontend".into());
        let mut n2 = issue_node("o", "r", 2, "B");
        n2.cluster = Some("compiler-mir".into());
        let mut n3 = issue_node("o", "r", 3, "C");
        n3.cluster = Some("compiler-frontend".into());
        let unclustered = issue_node("o", "r", 4, "D");
        let g = Graph {
            nodes: vec![n1, n2, n3, unclustered],
            edges: vec![],
        };
        let opts =
            opts_with_cluster_labels(&[("compiler-frontend", "Frontend"), ("compiler-mir", "MIR")]);
        insta::assert_snapshot!(to_dot(&g, &opts));
    }

    #[test]
    fn color_lookup_from_status_field() {
        let mut n1 = issue_node("o", "r", 1, "Done");
        n1.status = Some("Done".into());
        let mut n2 = issue_node("o", "r", 2, "Unmapped");
        n2.status = Some("In Limbo".into()); // not in opts.colors
        let g = Graph {
            nodes: vec![n1, n2],
            edges: vec![],
        };
        let opts = opts_with_colors(&[("Done", "#57a85a")]);
        insta::assert_snapshot!(to_dot(&g, &opts));
    }

    #[test]
    fn all_three_edge_kinds() {
        let nodes = vec![
            issue_node("o", "r", 1, "A"),
            issue_node("o", "r", 2, "B"),
            issue_node("o", "r", 3, "C"),
        ];
        let edges = vec![
            edge(
                EdgeKind::SubIssue,
                issue_id("o", "r", 2),
                issue_id("o", "r", 1),
            ),
            edge(
                EdgeKind::Blocks,
                issue_id("o", "r", 3),
                issue_id("o", "r", 1),
            ),
            edge(
                EdgeKind::CrossReference,
                issue_id("o", "r", 1),
                issue_id("o", "r", 2),
            ),
        ];
        let g = Graph { nodes, edges };
        insta::assert_snapshot!(to_dot(&g, &opts()));
    }

    #[test]
    fn tooltip_includes_state_assignees_and_truncated_body() {
        let mut n = issue_node("o", "r", 1, "T");
        n.body = Some("x".repeat(250));
        n.assignees = vec!["alice".into(), "bob".into()];
        let g = Graph {
            nodes: vec![n],
            edges: vec![],
        };
        insta::assert_snapshot!(to_dot(&g, &opts()));
    }

    // -- structural tests --

    #[test]
    fn every_node_with_url_emits_url_attribute() {
        let g = Graph {
            nodes: vec![
                issue_node("o", "r", 1, "T"),
                draft_node("DI_a", "Draft"),
                ghost_node_value("ext", "lib", 99),
            ],
            edges: vec![],
        };
        let out = to_dot(&g, &opts());
        // Issue and Ghost have URLs; Draft does not.
        let url_lines = out.lines().filter(|l| l.contains("URL=")).count();
        assert_eq!(url_lines, 2, "expected URLs on Issue and Ghost only");
    }

    #[test]
    fn cross_reference_edges_carry_constraint_false() {
        let nodes = vec![issue_node("o", "r", 1, "A"), issue_node("o", "r", 2, "B")];
        let edges = vec![edge(
            EdgeKind::CrossReference,
            issue_id("o", "r", 1),
            issue_id("o", "r", 2),
        )];
        let g = Graph { nodes, edges };
        let out = to_dot(&g, &opts());
        assert!(
            out.contains("constraint=false"),
            "cross-reference edge missing constraint=false: {out}"
        );
    }

    #[test]
    fn non_cross_reference_edges_do_not_carry_constraint_false() {
        let nodes = vec![issue_node("o", "r", 1, "A"), issue_node("o", "r", 2, "B")];
        let edges = vec![
            edge(
                EdgeKind::SubIssue,
                issue_id("o", "r", 2),
                issue_id("o", "r", 1),
            ),
            edge(
                EdgeKind::Blocks,
                issue_id("o", "r", 2),
                issue_id("o", "r", 1),
            ),
        ];
        let g = Graph { nodes, edges };
        let out = to_dot(&g, &opts());
        assert!(!out.contains("constraint=false"));
    }

    #[test]
    fn every_edge_endpoint_is_declared_as_a_node() {
        let nodes = vec![issue_node("o", "r", 1, "A"), issue_node("o", "r", 2, "B")];
        let edges = vec![edge(
            EdgeKind::Blocks,
            issue_id("o", "r", 2),
            issue_id("o", "r", 1),
        )];
        let g = Graph { nodes, edges };
        let out = to_dot(&g, &opts());
        for node in &g.nodes {
            let id_quoted = format!("\"{}\"", node.id);
            assert!(
                out.contains(&format!("{id_quoted} [label=")),
                "node {id_quoted} missing from output: {out}"
            );
        }
    }

    #[test]
    fn cluster_subgraph_contains_only_its_members() {
        let mut a = issue_node("o", "r", 1, "A");
        a.cluster = Some("foo".into());
        let mut b = issue_node("o", "r", 2, "B");
        b.cluster = Some("bar".into());
        let g = Graph {
            nodes: vec![a, b],
            edges: vec![],
        };
        let out = to_dot(&g, &opts());
        // Naive substring check: each cluster subgraph block should mention
        // its sole member's identifier, not the other.
        let foo_start = out.find("subgraph \"cluster_foo\"").unwrap();
        let foo_end = foo_start + out[foo_start..].find("    }").unwrap();
        let foo_block = &out[foo_start..foo_end];
        assert!(foo_block.contains("\"o/r#1\""));
        assert!(!foo_block.contains("\"o/r#2\""));
    }

    // -- escaping (inline snapshots, since each output is one line) --

    #[test]
    fn quote_escapes_double_quotes_backslashes_and_newlines() {
        assert_eq!(quote("simple"), r#""simple""#);
        assert_eq!(quote(r#"has "quotes""#), r#""has \"quotes\"""#);
        assert_eq!(quote(r"back\slash"), r#""back\\slash""#);
        assert_eq!(quote("line1\nline2"), r#""line1\nline2""#);
    }

    #[test]
    fn truncate_keeps_short_strings() {
        assert_eq!(truncate("short", 10), "short");
    }

    #[test]
    fn truncate_appends_ellipsis_when_over_limit() {
        let out = truncate(&"x".repeat(250), 200);
        assert_eq!(out.chars().count(), 201); // 200 chars + ellipsis
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_does_not_split_utf8() {
        let s = "日本語日本語日本語"; // 9 chars, 27 bytes
        let out = truncate(s, 5);
        assert_eq!(out.chars().count(), 6); // 5 + ellipsis
        assert!(out.ends_with('…'));
    }
}
