# Node model

Each item on the project board becomes one node in the rendered graph. This document specifies how a `ProjectItem` (see [project-fetch.md](./project-fetch.md)) maps to a graph vertex: identity, payload, and clustering.

## Node identity

A graph node's identity is derived from the underlying content:

- **Issue** / **Pull request**: `<repository.nameWithOwner>#<number>` (e.g., `rust-lang/rust#12345`).
- **Draft issue**: GitHub's project-scoped node id (`DI_xxx`).
- **Redacted**: GitHub's project-item id (`PVTI_xxx`).

Identity is stable across runs. The same issue surfaced both as a project item and as a sub-issue of another item resolves to one node, not two.

Nodes emit in deterministic order: issues and pull requests sort by `(repository, number)`; draft issues follow, sorted by `(creation timestamp, id)`. Render output is byte-identical run-to-run for a fixed `ProjectFetch`.

## Node payload

What every node carries into the graph layer:

- **id** — the identity above.
- **kind** — `Issue`, `PullRequest`, `DraftIssue`, or `Redacted`.
- **label** — `#<number>: <title>` for issues and pull requests on single-repo boards; `<repo>#<number>: <title>` when the board contains items from multiple repositories. Draft issues render as `draft: <title>` with a `note` shape. Redacted nodes use a placeholder.
- **url** — the issue/PR URL. `None` for drafts and redacted nodes.
- **status** — the `display_string()` of the `[colors] field`, or `None`.
- **cluster** — the `display_string()` of the `[cluster] field`, or `None`.

Additional data is available on the node but is not rendered in v3: `body`, `assignees`, `state`, and the inline sub-issue list. The render layer can surface any of it later (tooltips, badges, sub-task summaries) without a fetch-layer change. Labels are not on the slice 1 fetch — they enter via `github/issues.rs` when the cross-reference label filter lands.

## Clusters

A cluster is a named swimlane in the rendered graph. Cluster name comes from a designated SingleSelect field on the project, configured via:

```toml
[cluster]
field = "Track"
```

Each option of the cluster field becomes one swimlane. Items whose cluster field is absent or unset render outside any cluster — no synthetic "uncategorized" bucket. The cluster field may be the same SingleSelect used by `[colors]`, though typically it is a separate field.

## Special node kinds

**Redacted** nodes are first-class. The token has lost permission to read the underlying content, or the content was deleted. They render with a placeholder label.

**Draft issues** are leaf nodes by construction. They have no underlying GitHub Issue, no sub-issues, and no blocking relationships. They render with a `note` shape and a `draft: <title>` label to mark them visually distinct from real issues.

**Off-board references** — a sub-issue, blocker, or cross-reference pointing at an issue not on the project board — become *ghost nodes* with `<repo>#<number>` as the label. Visual styling for ghosts is a render-layer call.
