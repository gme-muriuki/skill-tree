# See-also edges

skill-tree's edge model already covers GitHub-native relationships (sub-issue, blocking, cross-reference; see [edge convention](./edge-convention.md)). One soft relationship — *see also* — has no native source and is read from a metadata pipe-table at the top of the issue body.

## Why a body convention

GitHub-native blocking covers hard prerequisites. Cross-reference timeline events surface every `#NN` mention as an untyped soft edge; an author cannot opt in to a deliberate "this is relevant" signal distinct from a drive-by mention.

`rust-lang/rust-project-goals` already addresses this with a fixed pipe-table at the top of every goal doc. Their tooling walks `See also` rows recursively. skill-tree adopts the same format verbatim so existing goal docs feed in without rewriting.

Principle: anything GitHub models natively is fetched via the API. Anything it does not is read from the body. `Depends on`, `Tracking issue`, and richer relation kinds either already map to native features (blocking, sub-issues) or stay deferred until v1 proves out.

## Authoring format

A pipe-table at the top of the issue body, before any prose. Each row is `(label, value)`. The `See also` label declares a soft edge.

```markdown
| Tracking issue | rust-lang/rust#123         |
| See also       | rust-lang/rust#456         |
| See also       | rust-lang/rfcs#789         |
```

- **Label match**: `See also`, case-insensitive. All other labels parse silently as metadata and produce no edges in v1.
- **Value**: one issue reference per row. `#NN` resolves to the authoring issue's repo; `owner/repo#NN` is cross-repo. Wrapping square brackets (markdown link syntax, `[rust-lang/rust#123]`) are stripped.
- **No table / no See also row / unrecognised value**: no edges, no error.

Multiple `See also` rows produce multiple edges from the same source.

## Vocabulary

v1 supports one label: **`See also`**.

`Depends on` is **deliberately absent**. Hard prerequisites go through GitHub-native blocking — round-trips to the GitHub UI, surfaces in issue sidebars, feeds the existing `Blocks` edge. Two parallel authoring paths for the same relation kind would create dedup churn.

Industry tools converge on `blocks` + `relates to` as the canonical hard/soft pair (Linear, Jira, GitHub native, UML, mermaid). `See also` aligns close enough to `relates to` semantically and matches the `rust-project-goals` dialect exactly.

Future labels (richer relation kinds, `Tracking issue` → sub-issue parent binding) stay deferred until v1 sees use.

## Body splitting

The metadata table is detected and removed before any other body parsing:

```
clean_body(raw) -> body
split_front_matter(body) -> (metadata_rows, rest)
```

`metadata_rows` feeds the see-also extractor; `rest` is the issue prose, which continues to existing consumers (tooltip generation). The two paths produce disjoint edge sets by construction — no post-hoc dedup.

Splitter heuristic: walk from the start of `body`, skipping leading whitespace and HTML comments, and treat the first contiguous block of pipe-delimited lines as the metadata table. Anything after that block is `rest`.

## Model deltas

One new `EdgeKind` variant:

- `EdgeKind::SeeAlso` — soft pointer. Not cycle-relevant. Renders as a data edge (`constraint=false`).

Off-board endpoints become ghost nodes per the existing convention (see [node model](./node-model.md)). Self-edges are rejected at validation, same as other kinds. Cross-repo references go through the existing `RepositoryRef` parsing path.

## Module layout

- `src/graph/see_also.rs` — front-matter splitter, table parser, label matching. Returns typed edges plus the residual body that flows to existing consumers.
- `src/error/see_also.rs` — `thiserror` enum. Malformed rows print a single-line warning to stderr (matching the `eprintln!` pattern in `src/main.rs` and `src/github/mod.rs`) and drop from the edge set. The parser does not return errors to the graph layer; an author's typo never blocks the render of an otherwise-valid project.

## Rendering

`SeeAlso` is a data edge (`constraint=false`). Visual distinction from existing edges:

- `Blocks` — solid red (existing).
- `CrossRef` — dashed gray, directional arrowhead (existing).
- `SeeAlso` — dashed gray, **arrowless** (`dir=none`). The dir=none distinguishes the author-opt-in soft pointer from the auto-generated cross-reference.

Edge tooltip: `"see also"`. Exact hex values stay in render code; the structural decision is that the three soft/hard kinds remain visually distinguishable at a glance.

## Out of scope

- **Synthetic "design nodes"** — reifying "design needed" as a phantom prerequisite node. Deferred until see-also sees real use.
- **Configurable vocabulary** — a `.skill-tree.toml` extension that lets users declare additional row labels. Deferred until one fixed label proves insufficient.
- **Other rust-project-goals labels** — `Tracking issue`, `Point of contact`, `Status` rows are parsed and ignored. Future chapters may add edges or metadata bindings for them.
- **Edge-kind colour/style configuration** — render polish; the three built-in styles ship hardcoded for v1.
