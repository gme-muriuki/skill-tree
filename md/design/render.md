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
}
```

`RenderOpts` carries only what render needs, owned values, not a `&Config` borrow. `cli/render.rs` builds it from `&Config`. This keeps `to_dot` independent of the config layer — easy to test with synthetic options.

## Primary goal: legible diagrams on real boards

On boards with ~80 issues the naive rendering is hard to read: arrows overlap, spines fan out, cross-reference edges cross the body of the tree. A pipeline that produces unreadable SVGs is not useful.

Slice 3 aims to ship a layout that stays legible at that scale before any other render feature.

Five techniques apply independently and stack:

**Layout engine.** Pin `dot` (hierarchical) for the spine. `neato`/`sfdp` (force-directed) look messier on issue-dependency graphs because they optimize edge length rather than rank.

**Cluster subgraphs.** Group nodes by the `[cluster]` field into Graphviz `subgraph "cluster_<id>" { ... }` blocks. Graphviz lays clusters tight; cross-cluster edges pull to the periphery and stop crossing intra-cluster structure.

**Edge constraint separation.** Add `constraint=false` to cross-reference edges. Blocking and sub-issue edges define the rank structure; cross-refs decorate without warping the spine. This is the single biggest lever on chaos: most "spaghetti" on a real board is cross-refs being treated as first-class layout edges.

**Edge style.** Solid for sub-issue and blocking, dashed for cross-reference. The eye latches onto the spine, treats dashed lines as soft signals.

**Transitive reduction (slice 4 flag).** `--transitive-reduce` removes implied edges (`A → B → C` plus `A → C` drops the direct `A → C`). Off by default — opt-in, since reduction discards user-authored edges.

## Node conventions

Attribute-styled DOT, not HTML-like labels. HTML labels (`<TABLE>...</TABLE>`) frequently fail to round-trip through draw.io; attributes do.

**Identifier.** The quoted `NodeId::Display()` string: `"o/r#42"` for Issue/Ghost, raw id (`"DI_xyz"`, `"PVTI_abc"`) for Draft and Redacted. Issue and Ghost share the `(owner, repo, number)` tuple but never coexist for the same triple, so no collision.

**Label.**
- `NodeKind::Issue`, `NodeKind::PullRequest` — `#<number>: <title>`.
- `NodeKind::DraftIssue` — `<title>`.
- `NodeKind::Redacted` — `[redacted]`.
- `NodeKind::Ghost` — `<owner>/<repo>#<number>`.

**Shape.**
- Issue, PR — `shape=box`.
- DraftIssue — `shape=note` (folded-corner page; visually marks "not yet a real issue").
- Redacted — `shape=box, style="dashed,filled"`, placeholder fill.
- Ghost — `shape=box, style="dashed"`, no fill ("off-board reference").

**Color.** Lookup `Node.status` in `opts.colors`. If present, `fillcolor=<hex>, style=filled`. If `None` or unmapped, `fillcolor=<opts.default_color>, style=filled`. Status color is the only state signal — no separate styling for `state == "CLOSED"`.

**URL.** When `Node.url.is_some()`, emit `URL="<url>"`. Drives clickability in the rendered SVG.

**Tooltip.** Multi-line `tooltip="<lines joined by \n>"`:
- Line 1: `State: <state>` when `Node.state.is_some()`.
- Line 2: `Assignees: <login>, <login>` when `!assignees.is_empty()`.
- Blank line, then `Node.body` truncated to 200 chars (suffix `…` if truncated).
- `NodeKind::Ghost` and `NodeKind::Redacted` — no tooltip attribute (no data).

## Cluster conventions

One cluster per distinct `Node.cluster` value, in **first-occurrence order** under node sort. Cluster ordering is tied to node sort — stable across runs, predictable across commits.

**Name.** Quoted `"cluster_<raw option name>"`. Quoting sidesteps DOT identifier sanitization for names with spaces or hyphens.

**Label.** Lookup the raw option name in `opts.cluster_labels`. If present, display the friendly label; otherwise display the raw name.

**Style.** `style="rounded"`, no fill. Clusters are a grouping signal, not a color signal — node fill color already carries status info; cluster fill would compete.

**Unclustered nodes.** Nodes with `cluster == None` are emitted at the top level, outside any subgraph. No synthetic "Misc" cluster.

## Edge conventions

**Style per kind.**
- `EdgeKind::SubIssue`, `EdgeKind::Blocks` — solid (Graphviz default).
- `EdgeKind::CrossReference` — `style=dashed, constraint=false`.

**Color and arrowhead.** Defaults for all kinds — `color=black`, `arrowhead=normal`. Edge kind is signalled by style (solid vs dashed), not by a second color dimension that would compete with node fill.

**Labels.** None inline. Edge kind is implied by style; adding text labels per edge adds noise on dense boards.

**Tooltips.** `tooltip="<kind>"` per edge (`"sub-issue"`, `"blocks"`, `"cross-reference"`). Invisible in static contexts; shows on hover in the SVG.

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

`--transitive-reduce` is a slice 4 addition, riding on petgraph adoption.

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
    #[error("`dot` exited with status {status}: {stderr}")]
    DotFailed { status: i32, stderr: String },
}
```

Both variants exit with code `1` (unrenderable output — same category as missing data).

## Testing

Snapshot tests via [`insta`](https://crates.io/crates/insta) for byte-stability coverage: ~4-6 representative scenarios (empty graph, single node, with clusters, with mixed edge kinds, with ghosts, with closed-state items).

Structural assertion tests in parallel for invariants that should survive future render changes:
- Every `Node` produces a DOT entry with a `URL=` attribute when `Node.url.is_some()`.
- Every edge references a `NodeId` declared in the nodes block.
- Every cluster contains at least one node.
- Cross-reference edges carry `constraint=false`.

## Future: petgraph adoption + `--transitive-reduce`

Transitive reduction is the moment to introduce [`petgraph`](https://crates.io/crates/petgraph) as a dependency. petgraph provides `dag_to_transitive_reduction`, `toposort`, and `tarjan_scc` — all useful in slice 3+ and beyond. [`Graph::validate`](./graph-build.md) currently runs a custom DFS; when petgraph lands in slice 4, the cycle path reconstruction migrates over so the codebase has one graph backing.
