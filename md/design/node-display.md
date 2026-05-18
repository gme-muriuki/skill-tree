# Node display

Each `Node` in the graph is rendered as a DOT node with a visible label and a hover tooltip. Slice 3 emitted a single-line label (`#NN: Title`) and dumped a 200-character body excerpt into the tooltip, mid-sentence. Slice 4 splits the surfaces:

- The **label** carries information the reader should see at a glance: title, GitHub state, assignees.
- The **tooltip** carries the cleaned issue body — markdown noise stripped, truncated at a sentence boundary.

Information is reachable from at least one of label / tooltip / click-through. No field is hidden.

## Label format

Plain DOT labels (not HTML-like). Plain labels serialize cleanly into draw.io for [[project-lcnr-use-cases|lcnr's editing workflow]] and avoid a second escape path on top of the existing DOT string rules.

```
#289: Convert check_trait to a
judgment function
OPEN · JojoFlex1
```

- **Line 1+**: title prefixed with `#NN: `, wrapped at column 40 on the last whitespace boundary, capped at two lines. If a third line would be needed the second line gets `…`.
- **Last line (meta)**: state + `·` separator + assignee list. Omitted entirely when both state and assignees are absent (Ghost, Redacted, some Drafts).
- **State**: GitHub lifecycle state in bare caps (`OPEN` / `CLOSED`). No brackets, no closed-issue dim/strikethrough — plain labels do not support per-line typography and the explicit `CLOSED` token is enough.
- **Assignees**: first three logins, comma-separated, with `+N` overflow suffix when more.

`status` (the custom field that drives fill color) is **not** repeated in the label — the fill color already encodes it. `labels` and `cluster` are **not** in the label — labels are project-specific noise, and cluster is already encoded by the surrounding subgraph box.

## Tooltip format

The tooltip is the cleaned issue body, nothing else. State and assignees already appear in the label, so the tooltip does not repeat them. This honors [[project-lcnr-use-cases|lcnr's request]] for hover-accessible state/assignees/body: state and assignees are reachable without hovering at all, body is reachable on hover.

The cleanup pipeline (Tier 2) operates on the raw body before truncation:

1. Strip HTML comments (`<!--…-->`).
2. Strip triple-backtick code fences; keep the content inside.
3. Strip inline markdown emphasis (`**x**`, `*x*`, `_x_`, `__x__`) to plain text.
4. Strip link syntax (`[text](url)`) to its text.
5. Strip leading heading markers (`#`, `##`, `###`).
6. Normalize whitespace: `\r\n` → `\n`, runs of three or more newlines collapse to two.
7. Truncate at the last sentence boundary (`. `, `! `, `? `, or `\n\n`) before 400 characters. Append `…` if truncated.

The cleanup is a pure function in `render/`. `Node.body` stays raw GitHub markdown; cleanup is a presentation concern and does not erase data other consumers (CLI text export, JSON dump, search) may need.

## Styling

Plain labels share one font and one size per node. Visual rhythm comes from layout and color, not per-line typography.

| Attribute | Value |
|---|---|
| `shape` | `box` |
| `style` | `"rounded,filled"` |
| `fontname` | `"Helvetica,Arial,sans-serif"` |
| `fontsize` | `11` |
| `margin` | `"0.18,0.08"` |
| `penwidth` | `1.5` |
| `color` (border) | each RGB channel of `fillcolor` clamped to 80% |
| `fontcolor` | black if `0.299·R + 0.587·G + 0.114·B > 128`, white otherwise (ITU-R BT.601 luma) |

`fontname` is a portable chain — Graphviz picks the first installed font. Helvetica covers macOS and modern Windows; Arial covers older Windows; `sans-serif` is the abstract fallback Graphviz resolves to whatever the system considers a default sans.

## Emission

Slice 3 emitted every attribute on every node line, which would balloon the DOT output 4–6× under this design. Slice 4 sets defaults once at the digraph level:

```
digraph SkillTree {
    rankdir = "LR";
    graph [fontname="Helvetica,Arial,sans-serif"];
    node  [shape=box, style="rounded,filled",
           fontname="Helvetica,Arial,sans-serif",
           fontsize=11, margin="0.18,0.08", penwidth=1.5];
    ...
}
```

Per-node lines emit only what differs: `fillcolor`, `fontcolor`, `color` (border), `label`, `URL`, `tooltip`. The `graph [fontname=…]` line also gives cluster labels the same typeface as nodes, fixing the Times-Roman inconsistency that would otherwise appear.

## Node kind variants

- **Issue / PullRequest**: full treatment per the rules above.
- **DraftIssue**: same label/tooltip rules, but state is always absent (drafts have no GitHub lifecycle). The meta line shows assignees only, or is omitted if there are none. Shape stays `note` to distinguish drafts visually.
- **Ghost** (off-board referent): label is the qualified `owner/repo#NN`, dashed border, no fill. No tooltip — there is no body to show.
- **Redacted** (permission lost or deleted): label is the literal `[redacted]`. No tooltip.

## Out of scope

- HTML-table labels matching the old TOML-first skill-tree's per-row styling. Deferred — the cost (new escape path, draw.io complications) is not justified by the visual delta from plain labels with the styling above.
- Configurable styling via `.skill-tree.toml`. Deferred — defaults need usage data before we commit to a TOML surface.
- Closed-issue visual disambiguation beyond the `CLOSED` token (strikethrough, opacity). Plain labels cannot strike text without HTML; the `CLOSED` token is sufficient.
- Edge styling changes. Edge tooltips (`sub-issue`, `blocks`, `cross-reference`) stay as-is.
