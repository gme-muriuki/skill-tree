//! HTML generation for `skill-tree embed`: wrap the rendered SVG in a
//! self-contained page (or an embeddable fragment) with a toolbar, a
//! status legend, and a click-to-detail side panel.
//!
//! See `md/design/html-embed.md` for the design. `build_records`,
//! `stats`, `legend_html`, and `assemble` are pure and testable without
//! the `dot` binary.

use std::collections::{BTreeMap, HashMap, HashSet};

use pulldown_cmark::{Options, Parser, html};
use serde::Serialize;

use super::RenderOpts;
use crate::graph::{EdgeKind, Graph, Label, NodeId, NodeKind};

const PANEL_CSS: &str = include_str!("assets/panel.css");
const PANEL_JS: &str = include_str!("assets/panel.js");
const STANDALONE_TEMPLATE: &str = include_str!("assets/standalone.html");
const FRAGMENT_TEMPLATE: &str = include_str!("assets/fragment.html");

/// Output shape: a full self-contained document, or an embeddable `<div>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Standalone,
    Fragment,
}

/// One node record in the embedded JSON map, keyed by NodeId string.
/// Covers issue, pull-request, and ghost nodes (ghosts so relationship
/// lists can name off-board neighbors). Absent optional fields are
/// omitted from the JSON; the panel script treats a missing field as
/// empty.
#[derive(Debug, Serialize)]
struct Record {
    /// Node display label, e.g. `#12: Parser rewrite`.
    title: String,
    number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cluster: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    assignees: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    labels: Vec<LabelRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    body_html: String,
    /// Upstream dependencies: blockers and sub-issue children (sources of
    /// `Blocks`/`SubIssue` edges into this node), as NodeId strings.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    depends_on: Vec<String>,
    /// Downstream: nodes this one blocks and the parent it is a sub-issue
    /// of (targets of its `Blocks`/`SubIssue` edges).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    blocks: Vec<String>,
    /// Cross-reference and see-also neighbors, either direction. Decorative
    /// connections, not dependencies.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related: Vec<String>,
    /// Ready to pick up — see [`Graph::unblocked`].
    unblocked: bool,
}

/// Serializable mirror of [`crate::graph::Label`] for embedding in the
/// JSON map. `color` is a 6-char hex string without `#`, straight from
/// GitHub.
#[derive(Debug, Serialize)]
struct LabelRecord {
    name: String,
    color: String,
}

impl From<&Label> for LabelRecord {
    fn from(label: &Label) -> Self {
        Self {
            name: label.name.clone(),
            color: label.color.clone(),
        }
    }
}

/// Render `graph` + its already-generated `svg` into the final HTML.
/// `opts` supplies the page title and the status→color legend.
///
/// Deterministic: byte-identical output for byte-identical inputs.
pub fn to_html(graph: &Graph, svg: &[u8], shape: Shape, opts: &RenderOpts) -> String {
    let records = build_records(graph);
    let data_json = serde_json::to_string(&records).unwrap_or_else(|_| "{}".to_owned());

    let (node_count, unblocked_count) = stats(graph);
    let stats_text = format!("{node_count} nodes · {unblocked_count} unblocked");
    let legend = legend_html(&opts.colors);
    let title = opts.project_title.as_deref().unwrap_or("");

    assemble(
        &svg_body(svg),
        &data_json,
        shape,
        title,
        &stats_text,
        &legend,
    )
}

/// Build the NodeId-keyed record map. Issue, pull-request, and ghost
/// nodes get a record; drafts and redacted nodes (no issue number) are
/// omitted. Keyed by `BTreeMap` so the serialized JSON is byte-stable
/// for a fixed graph.
fn build_records(graph: &Graph) -> BTreeMap<String, Record> {
    let unblocked = graph.unblocked();

    // Dependency edges: Blocks (blocker → blocked) and SubIssue
    // (child → parent) both make the target depend on the source.
    // depends_on = upstream (in), blocks = downstream (out). Keyed by
    // NodeId string.
    let mut depends_on: HashMap<String, Vec<String>> = HashMap::new();
    let mut blocks: HashMap<String, Vec<String>> = HashMap::new();
    // Cross-reference / see-also neighbors, treated as undirected: both
    // endpoints list each other. Decorative, not dependencies.
    let mut related: HashMap<String, Vec<String>> = HashMap::new();
    for e in &graph.edges {
        let (s, t) = (e.source.to_string(), e.target.to_string());
        match e.kind {
            EdgeKind::Blocks | EdgeKind::SubIssue => {
                depends_on.entry(t.clone()).or_default().push(s.clone());
                blocks.entry(s).or_default().push(t);
            }
            EdgeKind::CrossReference | EdgeKind::SeeAlso => {
                related.entry(s.clone()).or_default().push(t.clone());
                related.entry(t).or_default().push(s);
            }
        }
    }

    let mut map = BTreeMap::new();
    for node in &graph.nodes {
        let number = match &node.id {
            NodeId::Issue { number, .. } | NodeId::Ghost { number, .. } => *number,
            // Drafts and redacted items have no issue number, body, or edges.
            NodeId::Draft(_) | NodeId::Redacted(_) => continue,
        };
        let key = node.id.to_string();
        let dep = depends_on.remove(&key).unwrap_or_default();
        let blk = blocks.remove(&key).unwrap_or_default();
        let rel = dedup_preserving(related.remove(&key).unwrap_or_default());
        map.insert(
            key,
            Record {
                title: node.label.clone(),
                number,
                state: node.state.clone(),
                status: node.status.clone(),
                cluster: node.cluster.clone(),
                assignees: node.assignees.clone(),
                labels: node.labels.iter().map(LabelRecord::from).collect(),
                url: node.url.clone(),
                body_html: render_markdown(node.body.as_deref().unwrap_or("")),
                depends_on: dep,
                blocks: blk,
                related: rel,
                unblocked: unblocked.contains(&node.id),
            },
        );
    }
    map
}

/// Drop duplicate ids, keeping first-occurrence order. Symmetric
/// cross-references (A→B and B→A) otherwise list a neighbor twice.
fn dedup_preserving(ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// `(on-board issue/PR count, unblocked count)` for the toolbar.
fn stats(graph: &Graph) -> (usize, usize) {
    let nodes = graph
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Issue | NodeKind::PullRequest))
        .count();
    (nodes, graph.unblocked().len())
}

/// Legend chips: one per configured `[colors.values]` (sorted by name for
/// determinism), then a dashed "Ready to pick up" chip for unblocked.
fn legend_html(colors: &HashMap<String, String>) -> String {
    let mut names: Vec<&String> = colors.keys().collect();
    names.sort();

    let mut out = String::new();
    for name in names {
        out.push_str(&format!(
            "<span class=\"st-legend-item\"><span class=\"st-legend-dot\" style=\"background:{}\"></span>{}</span>",
            esc_html(&colors[name]),
            esc_html(name),
        ));
    }
    out.push_str(
        "<span class=\"st-legend-item\"><span class=\"st-legend-dot st-legend-dashed\"></span>Ready to pick up</span>",
    );
    out
}

/// Render issue-body markdown to sanitized HTML. `pulldown-cmark` produces
/// the HTML; `ammonia` strips anything unsafe (scripts, event handlers,
/// unknown tags) before it is embedded, so the panel can inject it as-is.
fn render_markdown(md: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(md, options);
    let mut unsanitized = String::new();
    html::push_html(&mut unsanitized, parser);
    ammonia::clean(&unsanitized)
}

/// First `<svg …>` onward, dropping the XML prologue / DOCTYPE that
/// `dot -Tsvg` emits, so the SVG inlines cleanly into HTML.
fn svg_body(svg: &[u8]) -> String {
    let text = String::from_utf8_lossy(svg);
    match text.find("<svg") {
        Some(i) => text[i..].to_owned(),
        None => text.into_owned(),
    }
}

/// Substitute the inlined assets, SVG, and data into the shape's template.
/// Title/stats/legend are substituted first and the SVG/data last, so
/// injected content is never rescanned for a later sentinel.
fn assemble(
    svg: &str,
    data_json: &str,
    shape: Shape,
    title: &str,
    stats: &str,
    legend: &str,
) -> String {
    let template = match shape {
        Shape::Standalone => STANDALONE_TEMPLATE,
        Shape::Fragment => FRAGMENT_TEMPLATE,
    };
    // Escape `</` so the embedded JSON cannot close the <script> early.
    let data_safe = data_json.replace("</", "<\\/");
    // CommonMark ends a `<div>` HTML block at the first blank line, so a
    // markdown host (mdbook, Hugo, ...) would parse our CSS/JS as
    // markdown the moment it hit one. Compact inlined payloads to a
    // single contiguous block. Safe for CSS, JS (newlines preserved), and
    // JSON (already single-line).
    let css = strip_blank_lines(PANEL_CSS);
    let js = strip_blank_lines(PANEL_JS);
    let data_safe = strip_blank_lines(&data_safe);
    template
        .replace("__TITLE__", &esc_html(title))
        .replace("__STATS__", &esc_html(stats))
        .replace("__LEGEND__", legend)
        .replace("__CSS__", &css)
        .replace("__JS__", &js)
        .replace("__SVG__", svg)
        .replace("__DATA__", &data_safe)
}

/// Remove lines that are empty or whitespace-only. Newlines between
/// non-blank lines are preserved, so JS ASI and CSS rule boundaries are
/// unaffected.
fn strip_blank_lines(s: &str) -> String {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Minimal HTML text escaping for interpolated text.
fn esc_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, Node};

    fn issue_node(owner: &str, repo: &str, number: u64, body: &str) -> Node {
        Node {
            id: NodeId::Issue {
                owner: owner.into(),
                repo: repo.into(),
                number,
            },
            kind: NodeKind::Issue,
            label: format!("#{number}: title"),
            url: Some(format!("https://github.com/{owner}/{repo}/issues/{number}")),
            status: Some("In Progress".into()),
            cluster: Some("Borrow checker".into()),
            body: Some(body.into()),
            state: Some("OPEN".into()),
            assignees: vec!["octocat".into()],
            labels: vec![],
        }
    }

    fn draft_node() -> Node {
        Node {
            id: NodeId::Draft("DI_x".into()),
            kind: NodeKind::DraftIssue,
            label: "drafty".into(),
            url: None,
            status: None,
            cluster: None,
            body: Some("body".into()),
            state: None,
            assignees: vec![],
            labels: vec![],
        }
    }

    fn graph_of(nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
        Graph { nodes, edges }
    }

    fn opts() -> RenderOpts {
        RenderOpts {
            project_title: Some("My <Title>".into()),
            colors: HashMap::from([("Done".to_owned(), "#57a85a".to_owned())]),
            ..Default::default()
        }
    }

    // -- build_records --------------------------------------------------------

    #[test]
    fn records_cover_issues_and_skip_drafts() {
        let g = graph_of(vec![issue_node("o", "r", 1, "x"), draft_node()], vec![]);
        let records = build_records(&g);
        assert_eq!(records.len(), 1, "only the issue node gets a record");
        assert!(records.contains_key("o/r#1"));
    }

    #[test]
    fn record_carries_node_fields_and_unblocked() {
        let g = graph_of(vec![issue_node("o", "r", 7, "x")], vec![]);
        let rec = &build_records(&g)["o/r#7"];
        assert_eq!(rec.number, 7);
        assert_eq!(rec.state.as_deref(), Some("OPEN"));
        assert_eq!(rec.status.as_deref(), Some("In Progress"));
        assert_eq!(rec.cluster.as_deref(), Some("Borrow checker"));
        assert_eq!(rec.assignees, vec!["octocat"]);
        assert!(rec.unblocked, "open issue with no blockers is unblocked");
    }

    #[test]
    fn records_carry_depends_on_and_blocks() {
        // #2 blocks #1: #1 depends_on #2; #2 blocks #1.
        let g = graph_of(
            vec![issue_node("o", "r", 1, "x"), issue_node("o", "r", 2, "x")],
            vec![Edge {
                kind: EdgeKind::Blocks,
                source: NodeId::Issue {
                    owner: "o".into(),
                    repo: "r".into(),
                    number: 2,
                },
                target: NodeId::Issue {
                    owner: "o".into(),
                    repo: "r".into(),
                    number: 1,
                },
            }],
        );
        let records = build_records(&g);
        assert_eq!(records["o/r#1"].depends_on, vec!["o/r#2"]);
        assert_eq!(records["o/r#2"].blocks, vec!["o/r#1"]);
        assert!(!records["o/r#1"].unblocked, "blocked by open #2");
    }

    #[test]
    fn related_collects_cross_refs_both_directions_deduped() {
        let a = NodeId::Issue {
            owner: "o".into(),
            repo: "r".into(),
            number: 1,
        };
        let b = NodeId::Issue {
            owner: "o".into(),
            repo: "r".into(),
            number: 2,
        };
        let xref = |s: NodeId, t: NodeId| Edge {
            kind: EdgeKind::CrossReference,
            source: s,
            target: t,
        };
        // Symmetric pair A→B and B→A must collapse to one neighbor each way.
        let g = graph_of(
            vec![issue_node("o", "r", 1, "x"), issue_node("o", "r", 2, "x")],
            vec![xref(a.clone(), b.clone()), xref(b, a)],
        );
        let records = build_records(&g);
        assert_eq!(records["o/r#1"].related, vec!["o/r#2"]);
        assert_eq!(records["o/r#2"].related, vec!["o/r#1"]);
        // cross-refs are not dependencies
        assert!(records["o/r#1"].depends_on.is_empty());
        assert!(records["o/r#1"].blocks.is_empty());
    }

    // -- render_markdown ------------------------------------------------------

    #[test]
    fn markdown_is_rendered() {
        let html = render_markdown("**bold** and a [link](https://example.com)");
        assert!(html.contains("<strong>bold</strong>"), "got: {html}");
        assert!(html.contains("href=\"https://example.com\""), "got: {html}");
    }

    #[test]
    fn markdown_sanitizes_scripts_and_handlers() {
        let html = render_markdown("ok <script>alert(1)</script> <img src=x onerror=alert(2)>");
        assert!(!html.contains("<script"), "script not stripped: {html}");
        assert!(!html.contains("onerror"), "handler not stripped: {html}");
    }

    // -- stats / legend -------------------------------------------------------

    #[test]
    fn stats_count_issues_and_unblocked() {
        let g = graph_of(vec![issue_node("o", "r", 1, "x"), draft_node()], vec![]);
        assert_eq!(stats(&g), (1, 1), "one issue, unblocked; draft not counted");
    }

    #[test]
    fn legend_lists_configured_colors_and_ready_chip() {
        let colors = HashMap::from([("Done".to_owned(), "#57a85a".to_owned())]);
        let html = legend_html(&colors);
        assert!(html.contains("#57a85a") && html.contains("Done"));
        assert!(html.contains("Ready to pick up"));
    }

    // -- assemble -------------------------------------------------------------

    #[test]
    fn standalone_is_a_full_document_with_assets_data_and_toolbar() {
        let g = graph_of(vec![issue_node("o", "r", 1, "x")], vec![]);
        let svg = b"<?xml version=\"1.0\"?>\n<svg width=\"10\"></svg>";
        let out = to_html(&g, svg, Shape::Standalone, &opts());
        assert!(out.starts_with("<!doctype html>"));
        assert!(out.contains("class=\"theme-dark\""));
        assert!(out.contains("<svg width=\"10\">"), "svg not inlined");
        assert!(out.contains("st-data"));
        assert!(out.contains("My &lt;Title&gt;"), "title not escaped");
        assert!(out.contains("1 nodes · 1 unblocked"), "stats missing");
        assert!(out.contains("Ready to pick up"), "legend missing");
        assert!(
            !out.contains("__SVG__") && !out.contains("__DATA__") && !out.contains("__STATS__")
        );
    }

    #[test]
    fn fragment_is_a_scoped_div_without_doctype() {
        let g = graph_of(vec![issue_node("o", "r", 1, "x")], vec![]);
        let svg = b"<svg></svg>";
        let out = to_html(&g, svg, Shape::Fragment, &opts());
        assert!(
            out.trim_start()
                .starts_with("<div class=\"st-widget st-embed")
        );
        assert!(!out.contains("<!doctype"));
        assert!(!out.contains("__SVG__"));
    }

    #[test]
    fn svg_prologue_is_stripped() {
        assert_eq!(
            svg_body(b"<?xml ?>\n<!DOCTYPE>\n<svg>x</svg>"),
            "<svg>x</svg>"
        );
        assert_eq!(svg_body(b"<svg>y</svg>"), "<svg>y</svg>");
    }
}
