# Render

The `render/` module turns a [`Graph`](./graph-build.md) into a Graphviz DOT document. An optional pipe through the system `dot` binary produces an SVG with clickable nodes.

This document captures the goals that drive slice 3. Concrete DOT conventions, color/state mapping, CLI flag surface, and the SVG post-processing pipeline are still to be designed.

## Primary goal: legible diagrams on real boards

On boards with ~80 issues the naive rendering is hard to read: arrows overlap, spines fan out, cross-reference edges cross the body of the tree. A pipeline that produces unreadable SVGs is not useful.

Slice 3 aims to ship a layout that stays legible at that scale before any other render feature.

## Techniques

Five techniques apply independently and stack:

**Layout engine.** Pin `dot` (hierarchical) for the spine. `neato`/`sfdp` (force-directed) look messier on issue-dependency graphs because they optimize edge length rather than rank.

**Cluster subgraphs.** Group nodes by the `[cluster]` field into Graphviz `subgraph cluster_<id> { ... }` blocks. Graphviz lays clusters tight; cross-cluster edges pull to the periphery and stop crossing intra-cluster structure. The `[cluster]` configuration is already wired in (see [config](./config.md)).

**Edge constraint separation.** Add `constraint=false` to cross-reference edges. The blocking and sub-issue edges define the rank structure; cross-refs decorate without warping the spine. This is the single biggest lever on chaos: most "spaghetti" on a real board is cross-refs being treated as first-class layout edges.

**Edge style.** Solid for sub-issue and blocking, dashed for cross-reference (already in [edge convention](./edge-convention.md)). The eye latches onto the spine, treats dashed lines as soft signals.

**Transitive reduction (future flag).** If `A → B → C` and `A → C` both render, the direct `A → C` is implied and adds a crossing. `--transitive-reduce` removes such edges before render. Off by default — the user must opt in, because reduction discards data the user wrote.

## Adoption of petgraph

Transitive reduction is the moment to introduce [`petgraph`](https://crates.io/crates/petgraph) as a dependency. petgraph provides `dag_to_transitive_reduction`, `toposort`, and `tarjan_scc` — all useful in slice 3 and beyond. [`Graph::validate`](./graph-build.md) currently runs a custom DFS; when petgraph lands, the cycle path reconstruction migrates over so the codebase has one graph backing.

The exact migration plan, and whether render takes a borrow on a petgraph-backed copy or builds it on demand, is a slice-3 brainstorm question.
