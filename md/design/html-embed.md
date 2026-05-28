# HTML embed

The `skill-tree embed` subcommand wraps the rendered SVG in an HTML page with
a toolbar, a status legend, and a click-to-detail side panel. It targets
embedding a board into a documentation site (an mdbook chapter, a blog post, a
project wiki): the reader sees the whole dependency tree, clicks a node, and
reads that issue's details — state, status, assignees, what it depends on, what
it blocks, and its body — in a panel beside the graph, without leaving the page.

`render` produces the graph as DOT or SVG. `embed` produces a self-contained
interactive document built around that same SVG.

## CLI surface

```bash
skill-tree embed [--fragment] [-o graph.html] [--config path]
```

- Default output is a **standalone** self-contained `.html` document.
- `--fragment` emits an embeddable **`<div>`** instead of a full document, for
  pasting into a host page (mdbook, blog) whose `<html>`/`<head>` already exist.
- `-o`/`--output` writes to a file; omitted, it writes to stdout. `--config`
  discovery matches `render` (explicit path, else walk up from CWD).

There is no `--format` — `embed` always emits HTML. The standalone/fragment
choice is the only output-shape switch.

Theme is fixed per shape in this version: the standalone document uses dark
chrome, the fragment uses light chrome so it sits inside a typical
light documentation page. A `--theme` override is a possible later addition.

## Relationship to `render`

`embed` reuses the entire `render` pipeline up to the SVG, then wraps it. The
shared steps — fetch the project, fetch issue edges, build the `Graph`,
validate, emit DOT, shell out to `dot -Tsvg` — are factored into a helper used
by both subcommands:

```rust
async fn fetch_graph(client: &GitHubClient, config: &Config)
    -> Result<(Graph, String /* project title */), CliError>;
```

`render` calls it and then formats; `embed` calls it, runs `to_dot` +
`dot_to_svg`, and passes the SVG plus the `Graph` to the HTML assembler.
`embed` therefore depends on the same system `dot` binary as `render --format
svg`, and surfaces the same [`RenderError`] when `dot` is missing or fails.

## The data channel

The side panel needs fields the SVG does not carry — status and assignees in
particular. Rather than scrape the DOM, `embed` emits a structured data map
alongside the inline SVG:

```html
<script type="application/json" class="st-data"> { … } </script>
```

The script tag lives inside each widget (a `class`, not a page-global `id`) so a
page may carry more than one embed without the data maps colliding. `</` in the
serialized JSON is escaped to `<\/` — valid JSON that the browser restores on
parse — so an issue body can never close the `<script>` element early.

The map is keyed by **NodeId string** (e.g. `owner/repo#123`). Each graphviz
node carries that same string as its SVG `<title>`, so the panel script joins a
clicked node to its record by reading the node group's `<title>`. NodeId is
stable across renders, so the join key is stable; the graphviz-assigned `nodeN`
element ids, which are sequential and unstable, are not used. This also means a
node's identity is discoverable from the DOM without extra attributes.

Issue, pull-request, and **ghost** nodes get a record. Ghosts (off-board edge
endpoints) are included so the panel's relationship lists can name them, even
though clicking one only shows its label and link. Synthetic project-root and
cluster-header nodes (`__project__`, `__cluster_*__`) carry no issue data and
are skipped by the panel script. Draft and redacted nodes — which have no issue
number, body, or edges — are omitted from the map.

Each record:

```json
{
  "title":      "#12: Parser rewrite",
  "number":     12,
  "state":      "OPEN",
  "status":     "In Progress",
  "cluster":    "Borrow checker",
  "assignees":  ["octocat"],
  "labels":     [{ "name": "T-compiler", "color": "bfd4f2" }],
  "url":        "https://github.com/owner/repo/issues/12",
  "body_html":  "<p>rendered, sanitized markdown…</p>",
  "depends_on": ["owner/repo#8"],
  "blocks":     ["owner/repo#34", "owner/repo#35"],
  "related":    ["owner/repo#41"],
  "unblocked":  false
}
```

Most fields come from the `Node` the graph already holds: `title` is the node
display label (`#<number>: <title>`), and `number`, `state`, `status`,
`cluster`, `assignees`, `labels`, `url`, and the rendered `body` follow.
Each label carries GitHub's `color` (6-char hex, no `#`) so the panel can
render chips that match the repo's actual label palette; the JS picks a
black/white text color by luminance for readability. `depends_on`
and `blocks` are the node's incoming and outgoing dependency edges — both
`Blocks` (blocker → blocked) and `SubIssue` (child → parent, so a parent
depends on its children) — as NodeId strings the panel resolves against the
same map. `related` carries cross-reference and see-also neighbors (either
direction, deduplicated), which are decorative rather than dependencies.
`unblocked` is the ready-to-pick-up flag (see below). Empty optional fields are omitted from the
JSON; the panel script treats a missing field as empty. The map is built into a
`BTreeMap` keyed by the NodeId string, so its serialization is byte-stable for a
fixed graph.

### Body rendering

`Node.body` reaches this layer with the See-also front-matter table already
stripped by the graph builder (see [See-also edges](./see-also.md)). `embed`
renders that remaining markdown to HTML with `pulldown-cmark`, then runs the
result through `ammonia` to sanitize it before embedding. Sanitizing at
generation time means the panel can inject `body_html` directly; the browser
never parses untrusted markup. The escaped-plain-text and excerpt-only
alternatives were rejected in favor of formatted bodies, which read better in a
documentation context.

## Toolbar, legend, and relationships

Around the canvas the widget lays out as a column: a **toolbar**, a **legend**,
then the stage-and-panel row (`.st-widget` is the column; `.st-main` is the
row). Both shapes carry all three; the fragment's sit inside its bordered box.

- **Toolbar** — the project title, a `N nodes · M unblocked` stat line, a
  Search-`#` input, and a Status `<select>`. `N` is the on-board issue/PR count,
  `M` the unblocked count, both computed in Rust and written into the page. The
  status options are populated by the script from the statuses present in the
  data.
- **Legend** — one chip per `[colors.values]` entry (a color dot plus the
  status name), then a dashed "Ready to pick up" chip for unblocked issues. The
  chips are built in Rust from the color config; order is by status name (the
  GitHub field's option order is a possible later refinement).
- **Panel relationships** — below the metadata, the panel shows `DEPENDS ON`
  (upstream: blockers and sub-issue children) and `BLOCKS` (downstream: blocked
  issues and the sub-issue parent) as lists; each neighbor is resolved against
  the data map for its title and marked with a check when it is done (state not
  `OPEN`). A third `RELATED` list shows cross-reference and see-also neighbors
  (either direction, no check) — decorative connections kept separate from the
  directional dependencies. On boards whose issues relate only by
  cross-reference, `RELATED` is what populates the panel.

### Unblocked rule

An issue/PR is **unblocked** ("ready to pick up") when its state is `OPEN` and
every issue it depends on is done — state not `OPEN` (`CLOSED`, or a PR's
`MERGED`) — or it depends on nothing. Both `Blocks` and `SubIssue` edges gate
readiness (a parent waits on its open children); cross-reference and see-also
edges, being decorative, do not. Off-board ghost dependencies carry no state and
so count as still-blocking (the conservative choice). This lives in
`Graph::unblocked() -> HashSet<NodeId>` so the (planned) `unblocked` subcommand
shares one definition.

### Search and status filter

Both filters **dim** non-matching issue nodes rather than hiding them, so the
fixed graphviz layout does not reflow. Search matches the issue number as a
substring; the status filter matches the node's status; both apply together. The
toolbar's owner chip from the design reference is deferred — its meaning
(project owner vs. an assignee filter) is unsettled.

## Assets and self-containment

The panel's stylesheet and script live as real files under
`src/render/assets/` (`panel.css`, `panel.js`), pulled into the binary with
`include_str!` and inlined into the output. The standalone document is a single
file with no external references; the fragment is a single `<div>` carrying its
own scoped `<style>` and `<script>`, so a copy-paste into a host page works with
no asset wiring and no iframe. All CSS is scoped under an `.st-` class prefix
plus a theme class to avoid colliding with host-page styles.

The script provides the interaction settled during design:

- **Fit-to-view by default.** The inline SVG's hard pixel `width`/`height` are
  dropped at load and it scales to its container with `preserveAspectRatio`, so
  the whole tree is visible without scrolling.
- **Pan and zoom.** Wheel zooms toward the cursor, drag pans, a small toolbar
  offers zoom-in / zoom-out / fit. This is hand-rolled (a CSS transform on the
  SVG) rather than a library, to keep the output dependency-free and
  self-contained.
- **Click opens the panel.** Clicking an issue node fills the panel and does not
  navigate; the GitHub link moves into the panel as a "View on GitHub" action.
  A drag is distinguished from a click by a small movement threshold, so panning
  never opens the panel.

## Canvas and theming

The graph canvas stays a light surface in both themes. Graphviz emits black
edges and labels and dark synthetic-node fills; keeping the canvas light keeps
them legible without recoloring the SVG. Only the surrounding chrome — panel,
toolbar, and (standalone) the title bar — carries the dark or light theme.
Graphviz's opaque white background rectangle is hidden with a CSS rule
(`svg > g > polygon:first-of-type { fill: transparent }`) so the themed canvas
shows through; `to_dot` is unchanged and the DOT/SVG outputs of `render` are
unaffected.

## Module layout

- `src/render/html.rs` — HTML generation, split into pure functions:
  - `to_html(&Graph, svg, shape, &RenderOpts) -> String` is the entry point.
  - `build_records(&Graph) -> BTreeMap<String, Record>` builds the data map.
  - `stats(&Graph)` and `legend_html(&colors)` build the toolbar stat line and
    legend chips.
  - `assemble(svg, data_json, shape, title, stats, legend)` strips the SVG's XML
    prologue, inlines the assets/data, and wraps the result per shape.

  All are pure and unit-testable without the `dot` binary.
- `src/graph/unblocked.rs` — `Graph::unblocked()`, the shared readiness rule.
- `src/cli/embed.rs` — the `embed` subcommand and its `EmbedArgs`.
- Errors reuse [`RenderError`]; a variant is added only if a genuinely new
  failure mode appears (markdown rendering, sanitizing, and JSON serialization
  of the owned record type do not fail in practice).

New dependencies, added with `cargo add`: `pulldown-cmark` and `ammonia`.
`serde_json` is already present.

## Determinism and tests

`build_records` and `assemble` are deterministic and snapshot-tested with
`insta` using a fixed in-memory graph and a stub SVG string — no `dot` needed.
One end-to-end test exercises the real `dot` path and is gated on `dot`
availability, matching `dot_to_svg`'s tests. Test infrastructure follows the
testlib pattern (see [Running tests](../contributing/running-tests.md)).

## Out of scope

- A `--theme` override and configurable synthetic-node / canvas colors.
- Image rendering policy beyond sanitization.
- Minified or externalized assets.
