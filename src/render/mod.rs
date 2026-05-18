//! DOT and SVG rendering for a [`Graph`].
//!
//! Layout decisions live in `md/design/render.md`. Per-node label,
//! tooltip, and styling decisions live in `md/design/node-display.md`.

mod body;
mod label;
mod style;
mod svg;

pub use svg::dot_to_svg;

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use crate::graph::{Edge, EdgeKind, Graph, Node, NodeId, NodeKind};

use self::body::{BODY_TOOLTIP_LIMIT, clean_body};
use self::label::format_label;
use self::style::{cross_ref_color, darken_hex, pick_text_color};

/// Fallback fill color when a node has no status, the status is not in
/// `[colors.values]`, or `RenderOpts.default_color` is empty. Reused by
/// `cli::render` so the CLI and renderer agree on one chrome tone.
pub const DEFAULT_COLOR: &str = "#dddddd";

/// Portable Graphviz font chain. Helvetica covers macOS and modern
/// Windows; Arial covers older Windows; `sans-serif` is the abstract
/// fallback Graphviz resolves to whatever the system considers default.
const FONT_NAME: &str = "Helvetica,Arial,sans-serif";

/// Each RGB channel of the fill is multiplied by this factor to derive
/// the border color. 80% gives the border a slightly darker tone than
/// the fill, which reads as a deliberate outline rather than a noisy
/// second color.
const BORDER_DARKEN_FACTOR: f32 = 0.80;

/// Identifier of the synthetic project root node. The double-underscore
/// guard never collides with a real GitHub identifier (`owner/repo#N`,
/// `DI_…`, `PVTI_…`).
const PROJECT_ROOT_ID: &str = "__project__";

/// Identifier of the synthetic Uncategorized cluster header.
const UNCATEGORIZED_ID: &str = "__cluster_uncategorized__";

/// Fill color of the project root node. Dark to anchor the tree spine
/// and read as a heading rather than another data point.
const PROJECT_ROOT_FILL: &str = "#222222";

/// Fill color of cluster header nodes — one rung lighter than the root.
const CLUSTER_HEADER_FILL: &str = "#666666";

/// Render-time options derived from [`crate::config::Config`] plus CLI
/// flags. Owns its data so `to_dot` is independent of the config layer.
#[derive(Debug, Clone, Default)]
pub struct RenderOpts {
    /// Option-name → hex (from `[colors.values]`).
    pub colors: HashMap<String, String>,
    /// Option-name → display label (from `[cluster.values]`).
    pub cluster_labels: HashMap<String, String>,
    /// Fallback fill color when a node has no status or its status is
    /// not in `colors`.
    pub default_color: String,
    /// Title of the underlying GitHub Project, threaded through from
    /// `ProjectMeta.title`. When `Some`, render emits a synthetic root
    /// node at the head of the tree; when `None`, the cluster headers
    /// are top-level. Unit tests use `None`; the CLI always populates.
    pub project_title: Option<String>,
}

/// Render `graph` to a Graphviz DOT document. Infallible — `Graph` is
/// already validated before it reaches this layer.
///
/// Deterministic: byte-identical output for byte-identical inputs.
pub fn to_dot(graph: &Graph, opts: &RenderOpts) -> String {
    let default_color = if opts.default_color.is_empty() {
        DEFAULT_COLOR
    } else {
        opts.default_color.as_str()
    };

    let mut out = String::new();
    writeln!(out, "digraph SkillTree {{").unwrap();
    writeln!(out, "    rankdir = \"LR\";").unwrap();
    writeln!(out, "    graph [fontname=\"{FONT_NAME}\"];").unwrap();
    writeln!(
        out,
        "    node  [shape=box, style=\"rounded,filled\", fontname=\"{FONT_NAME}\", \
fontsize=11, margin=\"0.18,0.08\", penwidth=1.5];"
    )
    .unwrap();

    let (cluster_order, cluster_members, unclustered) = partition_clusters(&graph.nodes);
    let has_root = opts.project_title.is_some();
    let uncategorized_used = has_root && !unclustered.is_empty();

    // Synthetic project root sits at rank 0 when a title is set.
    if let Some(title) = &opts.project_title {
        emit_project_root(&mut out, title);
    }

    // Cluster headers at rank 1: real clusters in first-occurrence
    // order, then optional Uncategorized so the eye reads meaningful
    // categories first.
    for key in &cluster_order {
        emit_cluster_header(&mut out, key, opts);
    }
    if uncategorized_used {
        emit_uncategorized_header(&mut out);
    }

    // Issue nodes flat (no subgraph grouping). The tree edge from
    // header → issue conveys cluster membership.
    for node in &graph.nodes {
        emit_node(&mut out, node, opts, default_color, 1);
    }

    // Tree edges drive the layout. Project → cluster (when root
    // present), then cluster → issue.
    if has_root {
        for key in &cluster_order {
            emit_tree_edge(&mut out, PROJECT_ROOT_ID, &cluster_header_id(key));
        }
        if uncategorized_used {
            emit_tree_edge(&mut out, PROJECT_ROOT_ID, UNCATEGORIZED_ID);
        }
    }
    for key in &cluster_order {
        let header_id = cluster_header_id(key);
        if let Some(members) = cluster_members.get(key) {
            for idx in members {
                emit_tree_edge(&mut out, &header_id, &graph.nodes[*idx].id.to_string());
            }
        }
    }
    if uncategorized_used {
        for idx in &unclustered {
            emit_tree_edge(
                &mut out,
                UNCATEGORIZED_ID,
                &graph.nodes[*idx].id.to_string(),
            );
        }
    }

    // Data edges (sub-issue, blocks, cross-ref) all carry
    // constraint=false so the tree spine wins layout. Cross-refs are
    // deduped per unordered pair; symmetric pairs emit with dir=both.
    emit_data_edges(&mut out, &graph.edges);

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

/// One DOT line for a node. Graph-level defaults (set in `to_dot`)
/// cover shape, base style, font, and margin; per-node lines only emit
/// what differs.
fn emit_node(
    out: &mut String,
    node: &Node,
    opts: &RenderOpts,
    default_color: &str,
    indent_level: usize,
) {
    let indent = "    ".repeat(indent_level);
    let id = quote(&node.id.to_string());
    let label = quote(&format_label(node));

    write!(out, "{indent}{id} [label={label}").unwrap();
    write_chrome(out, node, opts, default_color);

    if let Some(url) = &node.url {
        write!(out, ", URL={}", quote(url)).unwrap();
    }
    if let Some(tip) = node_tooltip(node) {
        write!(out, ", tooltip={}", quote(&tip)).unwrap();
    }

    writeln!(out, "];").unwrap();
}

/// Per-kind chrome: shape/style overrides plus fill/border/text colors.
fn write_chrome(out: &mut String, node: &Node, opts: &RenderOpts, default_color: &str) {
    match node.kind {
        NodeKind::Issue | NodeKind::PullRequest => {
            write_color_attrs(out, fill_color(node, opts, default_color));
        }
        NodeKind::DraftIssue => {
            write!(out, ", shape=note").unwrap();
            write_color_attrs(out, fill_color(node, opts, default_color));
        }
        NodeKind::Redacted => {
            write!(out, ", style=\"rounded,dashed,filled\"").unwrap();
            write_color_attrs(out, default_color);
        }
        NodeKind::Ghost => {
            write!(out, ", style=\"rounded,dashed\"").unwrap();
        }
    }
}

/// `fillcolor`, `fontcolor` (luma-based), and `color` (darkened border).
fn write_color_attrs(out: &mut String, fill: &str) {
    write!(out, ", fillcolor={}", quote(fill)).unwrap();
    write!(out, ", fontcolor={}", quote(pick_text_color(fill))).unwrap();
    write!(
        out,
        ", color={}",
        quote(&darken_hex(fill, BORDER_DARKEN_FACTOR))
    )
    .unwrap();
}

/// Resolve a node's fill color: `opts.colors[status]` when present,
/// `default_color` otherwise.
fn fill_color<'a>(node: &Node, opts: &'a RenderOpts, default_color: &'a str) -> &'a str {
    node.status
        .as_deref()
        .and_then(|s| opts.colors.get(s))
        .map(String::as_str)
        .unwrap_or(default_color)
}

/// Tooltip: cleaned issue body, or `None` to omit the attribute. State
/// and assignees already appear in the label, so the tooltip does not
/// repeat them.
fn node_tooltip(node: &Node) -> Option<String> {
    if matches!(node.kind, NodeKind::Ghost | NodeKind::Redacted) {
        return None;
    }
    let body = node.body.as_deref()?;
    let cleaned = clean_body(body, BODY_TOOLTIP_LIMIT);
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Quoted DOT identifier for the cluster header derived from a raw
/// `[cluster]` option value.
fn cluster_header_id(key: &str) -> String {
    format!("__cluster_{key}__")
}

/// Synthetic project root at rank 0. Fixed dark fill, double border,
/// white text — visually distinct from issues but uses only attributes
/// that round-trip cleanly through draw.io.
fn emit_project_root(out: &mut String, title: &str) {
    emit_header_node(out, PROJECT_ROOT_ID, title, PROJECT_ROOT_FILL);
}

/// One synthetic header per distinct `[cluster]` value at rank 1. Label
/// from `opts.cluster_labels` lookup, falling back to the raw key.
fn emit_cluster_header(out: &mut String, key: &str, opts: &RenderOpts) {
    let label = opts
        .cluster_labels
        .get(key)
        .map(String::as_str)
        .unwrap_or(key);
    emit_header_node(out, &cluster_header_id(key), label, CLUSTER_HEADER_FILL);
}

/// Synthetic Uncategorized header — same chrome as a real cluster
/// header so unclustered nodes don't look like orphans.
fn emit_uncategorized_header(out: &mut String) {
    emit_header_node(out, UNCATEGORIZED_ID, "Uncategorized", CLUSTER_HEADER_FILL);
}

/// Shared emission for project root, cluster headers, and Uncategorized.
fn emit_header_node(out: &mut String, id: &str, label: &str, fill: &str) {
    writeln!(
        out,
        "    {} [label={}, style=\"rounded,filled\", peripheries=2, \
fillcolor={}, fontcolor=\"white\"];",
        quote(id),
        quote(label),
        quote(fill),
    )
    .unwrap();
}

/// Tree edge `src -> tgt;` with no attribute block: solid by default,
/// constraint=true by default, no tooltip needed (the relationship is
/// obvious from the structure).
fn emit_tree_edge(out: &mut String, src: &str, tgt: &str) {
    writeln!(out, "    {} -> {};", quote(src), quote(tgt)).unwrap();
}

/// Emit every data edge with `constraint=false` so the tree spine
/// drives layout. Sub-issue and blocking pass through one-to-one.
/// Cross-references dedupe per unordered pair: a symmetric pair
/// (A↔B) emits once with `dir=both`, preserving the mutual-mention
/// semantic without doubling the visual clutter.
fn emit_data_edges(out: &mut String, edges: &[Edge]) {
    use std::collections::HashSet;

    let cross_ref_dirs: HashSet<(&NodeId, &NodeId)> = edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::CrossReference))
        .map(|e| (&e.source, &e.target))
        .collect();
    let mut emitted_pairs: HashSet<(&NodeId, &NodeId)> = HashSet::new();

    for edge in edges {
        match edge.kind {
            EdgeKind::SubIssue | EdgeKind::Blocks => emit_solid_edge(out, edge),
            EdgeKind::CrossReference => {
                let (lo, hi) = if edge.source <= edge.target {
                    (&edge.source, &edge.target)
                } else {
                    (&edge.target, &edge.source)
                };
                if !emitted_pairs.insert((lo, hi)) {
                    continue;
                }
                let symmetric = cross_ref_dirs.contains(&(hi, lo));
                emit_cross_ref_edge(out, lo, hi, symmetric);
            }
        }
    }
}

fn emit_solid_edge(out: &mut String, edge: &Edge) {
    let source_str = edge.source.to_string();
    let target_str = edge.target.to_string();
    let kind_name = match edge.kind {
        EdgeKind::SubIssue => "sub-issue",
        EdgeKind::Blocks => "blocks",
        EdgeKind::CrossReference => unreachable!("cross-refs go through emit_cross_ref_edge"),
    };
    let tooltip = format!("{kind_name}: {source_str} → {target_str}");
    writeln!(
        out,
        "    {} -> {} [style=solid, constraint=false, tooltip={}];",
        quote(&source_str),
        quote(&target_str),
        quote(&tooltip),
    )
    .unwrap();
}

fn emit_cross_ref_edge(out: &mut String, source: &NodeId, target: &NodeId, symmetric: bool) {
    let source_str = source.to_string();
    let target_str = target.to_string();
    let color = cross_ref_color(&source_str);
    let (tooltip, dir_attr) = if symmetric {
        (
            format!("cross-reference (mutual): {source_str} ↔ {target_str}"),
            ", dir=both",
        )
    } else {
        (format!("cross-reference: {source_str} → {target_str}"), "")
    };
    writeln!(
        out,
        "    {} -> {} [style=dashed, constraint=false, penwidth=0.7{}, color={}, tooltip={}];",
        quote(&source_str),
        quote(&target_str),
        dir_attr,
        quote(color),
        quote(&tooltip),
    )
    .unwrap();
}

/// Escape a Rust string for use inside DOT double-quotes. Handles `"`
/// and `\\`, plus newline as the literal two-character escape `\n`
/// (Graphviz reads this as a line break inside `tooltip` and `label`).
///
/// ASCII control characters below 0x20 (except tab) and 0x7F (DEL) are
/// dropped: copied compiler-output bodies sometimes contain ANSI
/// escapes (`\x1B[…m`) whose bare ESC byte is invalid in XML 1.0
/// attribute values and would break `dot -Tsvg`.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            '\u{7F}' => {}
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
            default_color: DEFAULT_COLOR.to_owned(),
            ..Default::default()
        }
    }

    fn opts_with_title(title: &str) -> RenderOpts {
        RenderOpts {
            default_color: DEFAULT_COLOR.to_owned(),
            project_title: Some(title.to_owned()),
            ..Default::default()
        }
    }

    fn opts_with_colors(pairs: &[(&str, &str)]) -> RenderOpts {
        RenderOpts {
            colors: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            default_color: DEFAULT_COLOR.to_owned(),
            ..Default::default()
        }
    }

    fn opts_with_cluster_labels(pairs: &[(&str, &str)]) -> RenderOpts {
        RenderOpts {
            cluster_labels: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            default_color: DEFAULT_COLOR.to_owned(),
            ..Default::default()
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
    fn label_carries_title_state_and_assignees() {
        let mut n = issue_node("o", "r", 289, "Convert check_trait to a judgment function");
        n.assignees = vec!["alice".into(), "bob".into()];
        let g = Graph {
            nodes: vec![n],
            edges: vec![],
        };
        insta::assert_snapshot!(to_dot(&g, &opts()));
    }

    #[test]
    fn tooltip_is_cleaned_body() {
        let mut n = issue_node("o", "r", 1, "T");
        n.body = Some(
            "<!-- hidden -->\n## Heading\nFirst sentence. **Bold** word. Second sentence here."
                .into(),
        );
        let g = Graph {
            nodes: vec![n],
            edges: vec![],
        };
        insta::assert_snapshot!(to_dot(&g, &opts()));
    }

    // -- structural tests --

    #[test]
    fn graph_level_defaults_emitted_once() {
        let g = Graph::default();
        let out = to_dot(&g, &opts());
        assert_eq!(
            out.matches("node  [shape=box").count(),
            1,
            "graph-level node default should appear exactly once"
        );
        assert!(out.contains("graph [fontname="));
    }

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
    fn cross_reference_edges_carry_per_source_color_and_thinner_penwidth() {
        let nodes = vec![issue_node("o", "r", 1, "A"), issue_node("o", "r", 2, "B")];
        let edges = vec![edge(
            EdgeKind::CrossReference,
            issue_id("o", "r", 1),
            issue_id("o", "r", 2),
        )];
        let g = Graph { nodes, edges };
        let out = to_dot(&g, &opts());
        assert!(out.contains("penwidth=0.7"));
        assert!(
            out.contains(", color=\"#"),
            "cross-reference edge missing color hex: {out}"
        );
    }

    #[test]
    fn symmetric_cross_refs_render_once_with_dir_both() {
        let nodes = vec![issue_node("o", "r", 1, "A"), issue_node("o", "r", 2, "B")];
        let edges = vec![
            edge(
                EdgeKind::CrossReference,
                issue_id("o", "r", 1),
                issue_id("o", "r", 2),
            ),
            edge(
                EdgeKind::CrossReference,
                issue_id("o", "r", 2),
                issue_id("o", "r", 1),
            ),
        ];
        let g = Graph { nodes, edges };
        let out = to_dot(&g, &opts());
        // Exactly one dashed edge, with dir=both.
        assert_eq!(
            out.matches("style=dashed").count(),
            1,
            "symmetric cross-refs should collapse to one dashed edge: {out}"
        );
        assert!(out.contains("dir=both"));
        assert!(out.contains("cross-reference (mutual): o/r#1 ↔ o/r#2"));
    }

    #[test]
    fn asymmetric_cross_ref_does_not_carry_dir_both() {
        let nodes = vec![issue_node("o", "r", 1, "A"), issue_node("o", "r", 2, "B")];
        let edges = vec![edge(
            EdgeKind::CrossReference,
            issue_id("o", "r", 1),
            issue_id("o", "r", 2),
        )];
        let g = Graph { nodes, edges };
        let out = to_dot(&g, &opts());
        assert!(!out.contains("dir=both"));
        assert!(out.contains("cross-reference: o/r#1 → o/r#2"));
    }

    #[test]
    fn data_edge_tooltip_names_both_endpoints() {
        let nodes = vec![issue_node("o", "r", 1, "A"), issue_node("o", "r", 2, "B")];
        let edges = vec![
            edge(
                EdgeKind::SubIssue,
                issue_id("o", "r", 2),
                issue_id("o", "r", 1),
            ),
            edge(
                EdgeKind::CrossReference,
                issue_id("o", "r", 1),
                issue_id("o", "r", 2),
            ),
        ];
        let g = Graph { nodes, edges };
        let out = to_dot(&g, &opts());
        assert!(out.contains("tooltip=\"sub-issue: o/r#2 → o/r#1\""));
        assert!(out.contains("tooltip=\"cross-reference: o/r#1 → o/r#2\""));
    }

    #[test]
    fn all_data_edges_carry_constraint_false() {
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
            edge(
                EdgeKind::CrossReference,
                issue_id("o", "r", 1),
                issue_id("o", "r", 2),
            ),
        ];
        let g = Graph { nodes, edges };
        let out = to_dot(&g, &opts());
        assert_eq!(
            out.matches("constraint=false").count(),
            3,
            "every data edge should carry constraint=false: {out}"
        );
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

    // -- tree topology --

    #[test]
    fn project_root_appears_when_title_set() {
        let g = Graph {
            nodes: vec![issue_node("o", "r", 1, "T")],
            edges: vec![],
        };
        let out = to_dot(&g, &opts_with_title("My Project"));
        assert!(
            out.contains("\"__project__\" [label=\"My Project\""),
            "project root missing or mislabeled: {out}"
        );
    }

    #[test]
    fn no_project_root_when_title_absent() {
        let g = Graph {
            nodes: vec![issue_node("o", "r", 1, "T")],
            edges: vec![],
        };
        let out = to_dot(&g, &opts());
        assert!(!out.contains("__project__"));
    }

    #[test]
    fn cluster_header_emitted_per_distinct_cluster() {
        let mut a = issue_node("o", "r", 1, "A");
        a.cluster = Some("foo".into());
        let mut b = issue_node("o", "r", 2, "B");
        b.cluster = Some("bar".into());
        let mut c = issue_node("o", "r", 3, "C");
        c.cluster = Some("foo".into());
        let g = Graph {
            nodes: vec![a, b, c],
            edges: vec![],
        };
        let out = to_dot(&g, &opts());
        assert_eq!(out.matches("\"__cluster_foo__\" [label=").count(), 1);
        assert_eq!(out.matches("\"__cluster_bar__\" [label=").count(), 1);
    }

    #[test]
    fn cluster_header_uses_cluster_labels_override() {
        let mut a = issue_node("o", "r", 1, "A");
        a.cluster = Some("compiler-frontend".into());
        let g = Graph {
            nodes: vec![a],
            edges: vec![],
        };
        let opts = opts_with_cluster_labels(&[("compiler-frontend", "Frontend")]);
        let out = to_dot(&g, &opts);
        assert!(out.contains("\"__cluster_compiler-frontend__\" [label=\"Frontend\""));
    }

    #[test]
    fn uncategorized_header_appears_when_root_and_unclustered_mix() {
        let mut clustered = issue_node("o", "r", 1, "A");
        clustered.cluster = Some("foo".into());
        let unclustered = issue_node("o", "r", 2, "B");
        let g = Graph {
            nodes: vec![clustered, unclustered],
            edges: vec![],
        };
        let out = to_dot(&g, &opts_with_title("P"));
        assert!(out.contains("__cluster_uncategorized__"));
        assert!(out.contains("\"__project__\" -> \"__cluster_uncategorized__\";"));
        assert!(out.contains("\"__cluster_uncategorized__\" -> \"o/r#2\";"));
    }

    #[test]
    fn no_uncategorized_header_without_project_root() {
        let g = Graph {
            nodes: vec![issue_node("o", "r", 1, "A")],
            edges: vec![],
        };
        let out = to_dot(&g, &opts());
        assert!(!out.contains("__cluster_uncategorized__"));
    }

    #[test]
    fn tree_edges_emit_without_constraint_attribute() {
        let mut n = issue_node("o", "r", 1, "A");
        n.cluster = Some("foo".into());
        let g = Graph {
            nodes: vec![n],
            edges: vec![],
        };
        let out = to_dot(&g, &opts_with_title("P"));
        assert!(out.contains("\"__project__\" -> \"__cluster_foo__\";"));
        assert!(out.contains("\"__cluster_foo__\" -> \"o/r#1\";"));
    }

    // -- escaping --

    #[test]
    fn quote_escapes_double_quotes_backslashes_and_newlines() {
        assert_eq!(quote("simple"), r#""simple""#);
        assert_eq!(quote(r#"has "quotes""#), r#""has \"quotes\"""#);
        assert_eq!(quote(r"back\slash"), r#""back\\slash""#);
        assert_eq!(quote("line1\nline2"), r#""line1\nline2""#);
    }

    #[test]
    fn quote_drops_ascii_control_chars_that_break_xml() {
        assert_eq!(quote("\x1B[31mICE\x1B[0m"), r#""[31mICE[0m""#);
        assert_eq!(quote("a\x00b\x07c\x7Fd"), r#""abcd""#);
        assert_eq!(quote("a\tb"), "\"a\tb\"");
    }
}
