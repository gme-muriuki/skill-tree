# Edge convention

skill-tree's purpose is to visualize dependencies between issues. This document specifies which GitHub relationships become edges, their direction, and how visual style maps to relationship kind.

## Edge sources

Three GitHub-native relationships produce graph edges. All three are queryable as structured data — no body parsing.

**Sub-issue** — from `Issue.subIssues.nodes`. Direction: child → parent ("doing the child enables the parent").

**Blocking** — GitHub's native issue dependencies. Direction: blocker → blocked ("blocker must finish before blocked can start").

**Cross-reference** — a `CROSS_REFERENCED_EVENT` timeline item recording that issue A's body, comment, or PR description mentioned issue B with `#<number>` syntax. From `Issue.timelineItems(itemTypes: CROSS_REFERENCED_EVENT)`. Direction: mentioner → mentioned.

Blocking and cross-reference data both live on the Issue, not on the project item, so they are fetched together by `github/issues.rs` as a second pass after the items query (see [issue edges](./issue-edges.md)). Sub-issues come back inline with the items query (see [project fetching](./project-fetch.md)).

## Edge styles

Style is determined by edge kind:

- **Solid** — sub-issue and blocking. GitHub's structured hard dependencies.
- **Dashed** — cross-reference. A softer signal: "A talks about B" without claiming B is a prerequisite.

Both styles render by default. No configuration is required to get the basic mix.

## Filtering cross-references

Every PR that mentions an issue creates a `CROSS_REFERENCED_EVENT`. Without filtering, the graph saturates with PR-to-issue mentions on busy projects. Two filters apply.

**Both endpoints must be on the project board.** Cross-references to issues outside the project drop silently. Always on; not configurable.

**Source-issue label filter (permissive default).** `[edges.cross-ref] require-labels` is a list of label names that narrows what renders. When non-empty, a cross-reference renders only if its source carries at least one listed label (exact-name match, any-of). The default is the empty list — every cross-reference renders. Users on noisy boards add labels here to cut PR-to-issue chatter; first-look renders see everything.

## Edge identity

Every edge in the model carries its kind tag — `SubIssue`, `Blocks`, or `CrossReference`. The render layer uses the tag to apply visual style. Future variants (e.g., a label-based "depends-on" convention) extend the enum without re-fetching.

Edges emit in deterministic order: walked by source node in node-sort order (see [node-model.md](./node-model.md)), then within each source by `(kind, target)`.

## Validation

**Cycles** — a path `A → B → ... → A` is a hard error. skill-tree reports the cycle path and exits non-zero. Cycles are almost always data bugs.

**Self-edges** — an issue listed as its own sub-issue, blocker, or cross-reference is rejected at validation.

**Off-board endpoints** — when an edge points at an issue not on the project board, the endpoint becomes a ghost node (see [node-model.md](./node-model.md)). The edge still renders.

## Transitive reduction

Off by default. If a user has stated `A → B`, `B → C`, and `A → C`, all three edges render. A `--transitive-reduce` flag may land later if asked for.
