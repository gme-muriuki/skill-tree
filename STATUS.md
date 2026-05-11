# skill-tree v3 — status & roadmap

A working note. Not for publication. Tracks what's shipped, what's pending,
and what "usable v1" means.

---

## What v1 is

A CLI that points at a GitHub Project (via `.skill-tree.toml`) and renders
the issues + their relationships as a directed dependency graph (DOT/SVG).

Three-stage pipeline:

```
.skill-tree.toml ─► fetch ─► model ─► render ─► graph.dot / graph.svg
```

Three subcommands:

```
skill-tree render --format svg -o graph.svg    # primary use
skill-tree validate                            # cycles, dangling refs
skill-tree unblocked                           # list issues with no incoming deps
```

Three GitHub-native edge sources (no body parsing):

- **Sub-issue** — solid arrow, child → parent
- **Blocking** (issue dependencies) — solid arrow, blocker → blocked
- **Cross-reference** (timeline `CROSS_REFERENCED_EVENT`) — dashed arrow, mentioner → mentioned

SVG output edits cleanly in draw.io (stable per-node/per-edge IDs in DOT)
and exposes hover tooltips for body / state / assignees (lcnr's workflow).

---

## What's shipped

### Branch: github-client (current open PR)

- Workspace + skill-tree-testlib crate
- `.skill-tree.toml` parsing + Config + SkillTree context
- `GitHubClient` GraphQL transport: auth (--token / GITHUB_TOKEN), retry
  with exponential backoff + jitter, rate-limit handling
  (X-RateLimit-Reset), per-request timeouts derived from remaining budget
- `Connection<T>` + `PageInfo` for cursor pagination (loop runs in caller)
- Structured `GitHubError` enum
- `MockGitHub` testlib for integration tests
- mdbook scaffolding + CI

### Branch: design/projects-fetch (next PR — slice 1)

- Project metadata fetch with owner-kind detection (single-document probe
  of `organization` and `user`)
- Items query, paginated at 100 per page
- Sub-issue overflow resolved eagerly inside fetch — graph layer never
  sees pagination state
- `Config::validate_against(&meta)` — collect-all validation; missing
  fields, wrong field types, unknown option values surface together
- Public `fetch_project(client, config) -> ProjectFetch` — fail-fast on
  config typo before the items query runs
- Errors: `OwnerUnreachable`, `ProjectNotFound { owner_kind }`,
  `ConfigMismatch { issues }`
- 83 tests (unit + integration via MockGitHub)
- Design docs: `project-fetch.md`, `node-model.md`, `edge-convention.md`

---

## What's remaining for usable v1

### Slice 2 — Graph layer

`src/graph/mod.rs` consumes `ProjectFetch` and builds nodes + edges per
`md/design/node-model.md` and `md/design/edge-convention.md`.

- Node identity (`<repo>#<number>` for issues/PRs, `DI_xxx` for drafts,
  `PVTI_xxx` for redacted, `ghost-...` for off-board references)
- Deterministic sort: `(repo, number)` for issues/PRs, `(created_at, id)`
  for drafts
- Cluster derivation from the configured SingleSelect field
- Off-board endpoints become ghost nodes
- `Graph::from_fetch(fetch, config) -> Result<Graph, BuildError>` —
  enforces self-edge / dangling-reference invariants
- `Graph::validate(&self) -> Result<(), CycleReport>` — full DFS for
  cycle detection
- `github/issues.rs` — fetches blocking edges (`trackedIssues`) +
  cross-references (`timelineItems(itemTypes: CROSS_REFERENCED_EVENT)`)
  per issue. Both filtered to "both endpoints on the project board"
- Optional `[edges.cross-ref] require-label = "..."` filter

Estimate: 3–4 commits, ~1 day.

### Slice 3 — Render layer

`src/render/mod.rs` writes DOT (and shells out to `dot` for SVG).

- DOT generator with stable per-element IDs (`issue-<owner>-<repo>-<num>`,
  `edge-<kind>-<src>-to-<tgt>`) — survives draw.io edits across regen
- Solid edges for sub-issue + blocking, dashed for cross-reference
- Drafts render with `shape=note` and `draft: <title>` label
- Cluster boxes for the SingleSelect-driven swimlanes; items with no
  cluster value render outside
- Auto repo prefix when the board spans multiple repos
- Default gray (`#888888`) for unset / unrecognized SingleSelect values
- Empty-project placeholder node + stderr note
- SVG path: `Command::new("dot").args(["-Tsvg"])...`; helpful error if
  graphviz not installed
- Deterministic emission order so committed `graph.dot` diffs cleanly

Estimate: 2–3 commits, ~1 day.

### Slice 4 — CLI subcommands

`src/cli/{render,validate,unblocked}.rs` already stubbed.

- `render` — fetch, build, validate, write DOT or SVG
- `validate` — fetch, build, run full validation, print "ok" or cycle path
- `unblocked` — fetch, build, list issues with no incoming `Blocks` /
  `SubIssue` edges that aren't already resolved
- `.skill-tree.toml` discovery — current dir, then walk up
- `--token` flag passes through to `GitHubClient::new`
- Exit codes: 0 success, 1 invalid response, 3 GitHub API errors,
  4 config errors

Estimate: 1–2 commits, ~half day.

### Slice 5 — Polish before tagging v1

- Stderr warnings: unrecognized SingleSelect values, off-board ghost
  references, label filter with no matches
- README install + usage section
- "skill-tree against itself" — point at a real project, render the SVG,
  open in draw.io, confirm the lcnr editing workflow works
- mdbook deploy via Pages (already wired in CI)

Estimate: ~half day.

---

## Order of branches to land

1. **github-client PR** (currently open) — the foundation
2. **design/projects-fetch** (next PR) — slice 1, fetch is done
3. Branch: graph + issues.rs (slice 2)
4. Branch: render (slice 3)
5. Branch: CLI (slice 4)
6. Tag v1, write the user-facing README, ship

Total to a usable v1: 4 more PRs after this one.

---

## What "usable" means

A Rust types team member can:

1. Drop a `.skill-tree.toml` in their repo pointing at a GitHub Project
2. Run `skill-tree render --format svg -o graph.svg`
3. Open the SVG and see their issues, sub-issues, blocking edges, and
   cross-references — colored by Status, clustered by Track
4. Hover a node and see body/state/assignees
5. Edit the SVG in draw.io and have edits survive `skill-tree render`
   re-runs (stable IDs)
6. Get clear errors when their config is wrong or their token is missing
7. Commit `graph.dot` to a repo and have it diff cleanly across regens

That's v1. Anything beyond — `.drawio` native export, transitive
reduction, label-based "depends-on" edges, multi-color rules,
incremental fetching — goes to v2.

---

## Open questions (non-blocking)

- `.skill-tree.toml` discovery path — CWD only, or walk up like cargo?
  (lean: walk up)
- `gh auth token` shell-out fallback — bundle in slice 4 or defer?
  (lean: defer, document `export GITHUB_TOKEN=$(gh auth token)`)
- Cross-reference noise on real boards — does the source-issue label
  filter suffice, or do we need a default-deny + opt-in?
  (lean: ship default-allow with the label filter, see what real users
  hit, adjust)

---

## Memory checkpoints

- User feedback rules: provide commit message + file list, never run
  `git commit`. Niko-style commit body length (1–3 dense paragraphs,
  no test enumeration).
- Workflow rules: brainstorm → design doc → code for substantive work.
- lcnr's use cases: editable SVG in draw.io, hover tooltips with
  body/state/assignees. Justifies keeping body/state/assignees on
  IssueContent / PullRequestContent.
- Testing: hybrid — unit fixtures inline in src/, integration via
  MockGitHub in tests/.
