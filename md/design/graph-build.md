# Graph build

The `graph/mod.rs` module turns a [`ProjectFetch`](./project-fetch.md) and a [`RawIssueEdges`](./issue-edges.md) into a typed graph of nodes and edges. It owns the `NodeId`, `Node`, `Edge`, and `Graph` types and applies every policy from [node model](./node-model.md) and [edge convention](./edge-convention.md). Error types (`BuildError`, `CycleReport`) live with the rest of the error hierarchy in `src/error/graph.rs`.

## Public entry points

```rust
impl Graph {
    pub fn from_fetch(
        project: ProjectFetch,
        edges: RawIssueEdges,
        config: &Config,
    ) -> Result<Graph, BuildError>;

    pub fn validate(&self) -> Result<(), CycleReport>;
}
```

Build is fallible only on self-edges; cycle detection is a separate call so a caller can hold a `Graph` value before validation (useful in tests and incremental tooling). The CLI runs `validate` after `from_fetch`.

## Node identity

```rust
pub enum NodeId {
    Issue   { owner: String, repo: String, number: u64 },
    Draft(String),     // GitHub node id, e.g. DI_xxx
    Redacted(String),  // project-item id, e.g. PVTI_xxx
    Ghost   { owner: String, repo: String, number: u64 },
}
```

The four variants correspond one-to-one with the node kinds in [node model](./node-model.md). `Display` produces the canonical rendered identifier: `<owner>/<repo>#<number>` for `Issue` and `Ghost`, raw id for `Draft` and `Redacted`. `Ord` matches the documented sort order — `Issue` and `Ghost` by `(owner, repo, number)`, then `Draft` by id, then `Redacted` by id.

PR project items use the `Issue` variant. PRs and Issues share the `<owner>/<repo>#<number>` namespace on GitHub; treating them as one identity kind avoids duplicate nodes when an issue and a PR share a number across repos and keeps sort stable.

## Graph shape

```rust
pub struct Graph {
    pub nodes: Vec<Node>,  // sorted by NodeId
    pub edges: Vec<Edge>,  // sorted by (source, kind, target)
}

pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub label: String,
    pub url: Option<String>,
    pub status: Option<String>,
    pub cluster: Option<String>,
    // payload retained for future render needs (tooltips, badges):
    pub body: Option<String>,
    pub state: Option<String>,
    pub assignees: Vec<String>,
}

pub enum NodeKind { Issue, PullRequest, DraftIssue, Redacted, Ghost }

pub struct Edge {
    pub kind: EdgeKind,
    pub source: NodeId,
    pub target: NodeId,
}

pub enum EdgeKind { SubIssue, Blocks, CrossReference, SeeAlso }
```

Both vectors are sorted at build time. Render walks them in stored order; `Graph` is byte-stable for fixed inputs. Adjacency lookups for `validate` and `unblocked` are derived on demand — the value type carries no redundant indices.

## Policy application

`from_fetch` applies, in order:

1. **Node materialization.** Every `ProjectItem` becomes one node by identity. Redacted items become `NodeId::Redacted`; drafts become `Draft`; issues and PRs become `Issue`. A sub-issue that also appears as its own project item resolves to one node, not two.
2. **Body front-matter parse.** Each `NodeId::Issue` body is run through `graph::see_also::parse`. The metadata pipe-table is stripped from `Node.body` (so it never reaches the tooltip); recognised `See also` rows are stashed for step 5b below. Malformed rows print one `eprintln!` per row to stderr. See [see-also edges](./see-also.md).
3. **On-board set.** A `HashSet<NodeId>` over the materialized nodes, consulted for every endpoint check below.
4. **Sub-issue edges.** Walk `IssueContent.sub_issues.nodes` for each Issue node. Off-board targets become **ghost nodes** (added to the node set) per [node model](./node-model.md). Self-edges produce `BuildError::SelfEdge`.
5. **Blocking edges.** Walk `RawIssueEdges.issues[].blocking` per source issue. Off-board targets become ghost nodes. Self-edges error.
5b. **See-also edges.** Drain the targets stashed in step 2. Off-board targets become ghost nodes, matching the sub-issue and blocking convention. Self-edges error.
6. **Cross-reference edges.** Walk `RawIssueEdges.issues[].cross_references` per target issue. **Both endpoints must be on board** — off-board sources drop silently per [edge convention](./edge-convention.md). Self-edges error. `[edges.cross-ref] require-labels` is permissive: an empty list (the default) includes every cross-reference; a non-empty list narrows to sources whose inlined `labels` contain at least one listed name (exact match).
7. **Sort.** `nodes` by `Ord`; `edges` walked by source in node order, then by `(kind, target)`.

## Errors

```rust
pub enum BuildError {
    SelfEdge { node: NodeId, kind: EdgeKind },
}

pub struct CycleReport {
    pub cycle: Vec<NodeId>,    // first node repeated at end: [A, B, C, A]
    pub kinds: Vec<EdgeKind>,  // len = cycle.len() - 1
}
```

`BuildError` is small on purpose:

- **Duplicate `NodeId`** is not modeled. Items normalize by identity; a duplicate from GitHub would be a fetch-layer bug, not a config bug.
- **Dangling targets** are not modeled. Off-board endpoints become ghost nodes (for `SubIssue`, `Blocks`, and `SeeAlso`) or drop silently (for `CrossReference`). An unrepresentable reference — e.g. GitHub returned a target with no parseable number — is a parse failure surfaced by `github/issues.rs` as `GitHubError`, not by `from_fetch`.

## Validation

`Graph::validate` runs an iterative DFS over `SubIssue` and `Blocks` edges only. **Cross-references and see-also edges are excluded from cycle detection**: both are decorative pointers, not dependency relationships, and the render layer already treats them as decorative via `constraint=false`. Including them would reject benign boards as cyclic. A `SubIssue → Blocks → SubIssue → ...` round-trip is still a cycle; a path closed by a `CrossReference` or `SeeAlso` edge is not. The first detected back-edge produces a `CycleReport` containing the path from the back-edge target through the active DFS stack and back. Subcommands run validate after build; render aborts non-zero before emission.

The `validate` subcommand prints the cycle path; finding *all* simple cycles (Johnson's algorithm) is deferred to a future flag if real boards demand it.
