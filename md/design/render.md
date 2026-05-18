# Render

The `render/` module turns a [`Graph`](./graph-build.md) into a Graphviz DOT document. An optional pipe through the system `dot` binary produces an SVG with clickable nodes.

## Public API

```rust
pub fn to_dot(graph: &Graph, opts: &RenderOpts) -> String;
```

`to_dot` is infallible — `Graph` is already validated by [`Graph::from_fetch`](./graph-build.md), so render has nothing to fail on. SVG generation is a separate fallible step (see [SVG pipeline](#svg-pipeline)).

```rust
pub struct RenderOpts {
    pub colors: HashMap<String, String>,         // option name → hex
    pub cluster_labels: HashMap<String, String>, // option name → display label
    pub default_color: String,                   // e.g. "#dddddd"
    pub project_title: Option<String>,           // ProjectMeta.title, drives the root node
}
```

`RenderOpts` carries only what render needs, owned values, not a `&Config` borrow. `cli/render.rs` builds it from `&Config` and threads `project_title` from the fetched `ProjectMeta`. This keeps `to_dot` independent of the config layer — easy to test with synthetic options. `project_title.is_none()` is a valid state used by unit tests; the CLI always populates it.

## Primary goal: legible diagrams on real boards

On boards with ~80 issues the naive rendering is hard to read: arrows overlap, spines fan out, cross-reference edges cross the body of the tree. A pipeline that produces unreadable SVGs is not useful.

Slice 4 ships the legibility layer that survives at that scale.

Five techniques apply independently and stack:

**Layout engine.** Pin `dot` (hierarchical, left-to-right) for the spine. `neato`/`sfdp` (force-directed) look messier on issue-dependency graphs because they optimize edge length rather than rank. LR rather than TB keeps multi-line node labels packed efficiently, matches the cause-→-effect reading direction of blocking edges, and stays embeddable in vertically-scrolling documentation pages.

**Explicit tree topology.** Emit a synthetic project root and one synthetic cluster header per `[cluster]` value. Issues hang off their cluster header; cluster headers hang off the project root. The three tree-edge ranks (root, headers, issues) drive the dot layout, replacing the slice-3 `subgraph "cluster_<id>"` boxes with first-class nodes. See [Tree topology](#tree-topology) below.

**Edge constraint separation.** Tree edges (root → header, header → issue) define the rank structure. All real edges from the data — sub-issue, blocking, cross-reference — carry `constraint=false` so they decorate without warping the spine. This is the single biggest lever on chaos: in slice 3 only cross-refs were de-constrained, and the sub-issue/blocking edges still occasionally pulled nodes out of their cluster.

**Edge style.** Solid for sub-issue and blocking, dashed for cross-reference. The eye latches onto the spine (tree edges), treats solid lateral arrows as real relationships, and dashed lines as soft signals.

**Transitive reduction (slice 5 flag).** `--transitive-reduce` removes implied edges (`A → B → C` plus `A → C` drops the direct `A → C`). Off by default — opt-in, since reduction discards user-authored edges. Operates on the data edges only; tree edges are never reducible (each cluster header has exactly one parent).

## Node conventions

Attribute-styled DOT, not HTML-like labels. HTML labels (`<TABLE>...</TABLE>`) frequently fail to round-trip through draw.io; attributes do.

**Identifier.** The quoted `NodeId::Display()` string: `"o/r#42"` for Issue/Ghost, raw id (`"DI_xyz"`, `"PVTI_abc"`) for Draft and Redacted. Issue and Ghost share the `(owner, repo, number)` tuple but never coexist for the same triple, so no collision. Synthetic nodes (project root, cluster headers) use `"__project__"` and `"__cluster_<key>__"` — the double-underscore guard never collides with a real GitHub identifier.

**Label, shape, color, URL, tooltip.** See [Node display](./node-display.md) for the slice-4 canonical spec — per-kind label format, plain-DOT multi-line labels, tier-2 markdown cleanup for tooltips, luma-based fontcolor, darkened-fill border. Render emits per-node only what differs from the graph-level defaults declared at the top of `to_dot`.

## Tree topology

The rendered DOT is a forest with one root: the project. Issues are leaves; cluster headers are internal nodes. Slice 3 used Graphviz `subgraph "cluster_<id>"` boxes to group nodes by `[cluster]` value; slice 4 promotes those groups to first-class nodes so the relationship between the project, its categories, and its issues is visible — and so the layout can be driven by explicit tree edges.

**Project root.** Emitted when `RenderOpts.project_title.is_some()`. Identifier `"__project__"`. Label is `project_title` verbatim. Styling: `shape=box`, `style="rounded,filled"`, `peripheries=2`, dark fill (proposal `#222222`), white fontcolor. Sits at rank 0.

**Cluster header nodes.** One per distinct `Node.cluster` value, in **first-occurrence order** under node sort. Identifier `"__cluster_<raw option name>__"`. Label lookup: `opts.cluster_labels[raw]` if present, raw value otherwise. Styling: `shape=box`, `style="rounded,filled"`, `peripheries=2`, mid-gray fill (proposal `#666666`), white fontcolor. Sits at rank 1.

**Uncategorized header.** When any node has `cluster == None` *and* the project root is emitted, a synthetic `"__cluster_uncategorized__"` header is added at the end of the cluster order, labeled `Uncategorized`. Keeps the tree shape consistent so the missing-category state is visible — a maintainer signal, not noise. When `project_title.is_none()`, unclustered nodes are emitted at top level as in slice 3 (no synthetic uncategorized header to dangle from no root).

**Tree edges.** `__project__ → __cluster_<key>__` and `__cluster_<key>__ → <issue_id>`. Solid (Graphviz default), no `constraint=false`, no tooltip, no edge label. Implementation note: tree edges precede data edges in the output stream so cross-cluster cycles (an issue-to-issue blocking edge) cannot accidentally pull a node out of its cluster header's rank.

**Cluster ordering.** Tied to node sort, same as slice 3. Stable across runs, predictable across commits.

**No subgraph boxes.** Slice 3's `subgraph "cluster_<id>" { ... }` blocks are removed. The grouping is conveyed by the tree edge from cluster header to issue, not by a visual container.

## Edge conventions

**Style per kind (data edges).**
- `EdgeKind::SubIssue`, `EdgeKind::Blocks` — `style=solid, constraint=false`, default `color=black`, default `penwidth`.
- `EdgeKind::CrossReference` — `style=dashed, constraint=false, penwidth=0.7`, plus a per-source color (see below).

**Tree edges (synthetic).** `style=solid`, no `constraint` override (defaults to true). Tree edges are the only edges that influence layout; every data edge is decorative.

**Cross-reference color hashing.** Cross-refs converge and diverge in the middle of dense boards; a single black-dashed style makes them impossible to trace. Each cross-ref takes a color from a fixed 10-hue qualitative palette (Tableau-10 inspired), keyed off the source `NodeId.to_string()`. Same source → same color across runs and across that source's outgoing cross-refs, so following "what does #265 reference" is a one-color trace. The thinner `penwidth=0.7` makes the cross-ref network recede visually behind the tree spine.

**Labels.** None inline. Edge kind is implied by style; adding text labels per edge adds noise on dense boards.

**Tooltips.** `tooltip="<kind>: <source> → <target>"` per data edge — e.g. `"cross-reference: o/r#265 → o/r#267"`. The enriched form lets a viewer hover an edge and read its endpoints without tracing the line through the graph. Tree edges carry no tooltip — the relationship they encode (membership in a cluster, membership in the project) is already obvious from the structure.

## Determinism

`to_dot` produces byte-identical output for byte-identical inputs. The implementation walks `Graph.nodes` and `Graph.edges` in their stored (sorted) order; no `HashMap` iteration ends up in the output path. This is a hard constraint — snapshot tests depend on it.

## CLI surface

Slice 3 ships `skill-tree render` only. `unblocked` and `validate` subcommands land in slice 4.

```
skill-tree render
skill-tree render --format svg --output graph.svg
skill-tree render --format dot --output graph.dot
skill-tree render --config /path/to/.skill-tree.toml
```

**Flags.**
- `--format dot|svg` — output format. When omitted and `--output` is given, infer from extension.
- `--output PATH` — destination file. When omitted, write to stdout.
- `--config PATH` — override config file location (default: walk up from CWD looking for `.skill-tree.toml`).

**Defaults.**
- `skill-tree render` with no flags → DOT to stdout. The unix-natural pipe target for piping into `dot` or `xdot`.
- `--output X.svg` → format inferred as SVG.
- `--output X.dot` → format inferred as DOT.

**Format resolution is permissive.** Explicit `--format` always wins (writing SVG bytes to a `.dot` filename is allowed — the user knows). Unknown or missing extensions fall back to DOT. No TTY detection on stdout — consistent with `dot -Tsvg` itself. Strict validation would force users to think about cases that don't matter to them.

**Config discovery vs. explicit path.** `SkillTree::discover(cwd)` walks `cwd.ancestors()` looking for `.skill-tree.toml` and surfaces `ConfigError::NotFound { start }` if nothing matches. `--config PATH` takes a different code path (`SkillTree::from_path`) that fails fast on a missing file with `ConfigError::Io` rather than falling back to discovery — explicit user intent should not silently search elsewhere.

`--transitive-reduce` is a slice 4 addition, riding on petgraph adoption.

## Orchestration

The full pipeline lives in `src/cli/render.rs`:

1. Load config — `SkillTree::from_path` if `--config` was passed; otherwise `SkillTree::discover(cwd)`.
2. Construct `GitHubClient::new(None, Duration::from_secs(60))` — token from `GITHUB_TOKEN`, 60s timeout. Token and timeout are intentionally not configurable (YAGNI; users already have `GITHUB_TOKEN` set for `gh`/`git`).
3. `fetch_project(&client, &config)` — metadata + items + sub-issue overflow.
4. Extract Issue/PullRequest IDs from project items.
5. `fetch_issue_edges(&client, &ids)` — blocking + cross-reference timeline.
6. `Graph::from_fetch(project, edges, &config)?`.
7. `graph.validate()?`.
8. Build `RenderOpts` from `config.colors.values`, `config.cluster.values`, a hardcoded default color, and `project.metadata.title` (threaded into `project_title`).
9. `to_dot(&graph, &opts)` (always).
10. If format is SVG, `dot_to_svg(&dot)?`; otherwise the DOT bytes are the output.
11. Write to `--output` PATH (`std::fs::write`) or stdout (locked handle). IO errors at this step surface as `CliError::OutputWrite { path, source }` with exit 1.

`render_to_bytes(client, config, args)` is public so integration tests can wire a mock-pointed `GitHubClient` and a `Config` parsed from a TOML string. The CLI binary itself only goes through `run(args)`, which adds the config-loading and output-writing layers.

## SVG pipeline

When `--format svg`, skill-tree shells out to the system `dot` binary:

```
echo "<dot>" | dot -Tsvg
```

The SVG is passed through verbatim — no post-processing for fonts, CSS, or attribute cleanup. `dot -Tsvg` already emits `<a href>` for clickability and `<title>` for tooltips, both of which survive draw.io import. If real-world draw.io use surfaces specific cleanup needs, they're added later as a targeted post-processing pass.

Reasons for shelling out (rather than linking libgraphviz or using a pure-Rust DOT engine):
- `dot` is universal and well-known; build deps stay minimal (no C toolchain for cross-compilation).
- `dot`'s layout quality outclasses every pure-Rust alternative.
- Failure mode is clean: `Command::new("dot")` fails with `ErrorKind::NotFound` if absent; we surface a one-line install pointer.

## Errors

`to_dot` is infallible. The SVG generation path is fallible — defined in `src/error/render.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("`dot` binary not found in PATH — install graphviz from https://graphviz.org/download/")]
    DotNotFound,
    #[error("failed to launch `dot`: {0}")]
    DotSpawn(#[source] std::io::Error),
    #[error("`dot` exited with status {status}: {stderr}")]
    DotFailed { status: i32, stderr: String },
}
```

`DotNotFound` carries an actionable install pointer; `DotSpawn` covers non-NotFound spawn failures and broken pipes on `dot`'s stdin; `DotFailed` carries the captured stderr. All three exit with code `1`.

CLI dispatch wraps these (and `ConfigError`, `GitHubError`, `BuildError`, `CycleReport`) in a top-level `CliError` envelope in `src/error/cli.rs`. `CliError::exit_code()` defers to the wrapped error so the existing 1 / 3 / 4 conventions are preserved across subcommands. Output-write failures split by destination: `CliError::FileWrite { path, source }` and `CliError::StdoutWrite(source)`, both exit 1. The split avoids using a sentinel `PathBuf` for stdout.

## Testing

Snapshot tests via [`insta`](https://crates.io/crates/insta) for byte-stability coverage: ~4-6 representative scenarios (empty graph, single node, with clusters, with mixed edge kinds, with ghosts, with closed-state items).

Structural assertion tests in parallel for invariants that should survive future render changes:
- Every `Node` produces a DOT entry with a `URL=` attribute when `Node.url.is_some()`.
- Every data edge references a `NodeId` declared in the nodes block.
- Every cluster header node is reachable from `__project__` via exactly one tree edge.
- Every issue node is reachable from exactly one cluster header (its `[cluster]` value, or `__cluster_uncategorized__` when none).
- Every data edge (sub-issue, blocking, cross-reference) carries `constraint=false`.

## Future: petgraph adoption + `--transitive-reduce`

Transitive reduction is the moment to introduce [`petgraph`](https://crates.io/crates/petgraph) as a dependency. petgraph provides `dag_to_transitive_reduction`, `toposort`, and `tarjan_scc` — all useful in slice 3+ and beyond. [`Graph::validate`](./graph-build.md) currently runs a custom DFS; when petgraph lands in slice 4, the cycle path reconstruction migrates over so the codebase has one graph backing.
