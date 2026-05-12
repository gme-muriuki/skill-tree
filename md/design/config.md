# Configuration

skill-tree reads configuration from a `.skill-tree.toml` file in the current directory. The file identifies which GitHub Project to fetch and how to display what comes back. Configuration is read once at startup; changes take effect on the next invocation.

## File format

```toml
[github]
owner   = "rust-lang"
project = 42

[[field]]
display-name = "status"
github-name  = "Status"

[colors]
github-name = "Status"

[colors.values]
"In Progress" = "#4a90d9"
"Blocked"     = "#e05252"
"Complete"    = "#57a85a"
"Not Started" = "#888888"

[cluster]
github-name = "Area"

[cluster.values]
"compiler-frontend" = "Frontend"
"compiler-mir"      = "MIR"

[edges.cross-ref]
require-labels = ["tracking-issue", "meta"]
```

## Sections

### `[github]`

Identifies the GitHub Project to fetch. `owner` is the organization or user (string); `project` is the project number from the URL (integer). For `github.com/orgs/rust-lang/projects/42`, `owner = "rust-lang"` and `project = 42`. Both fields are required.

### `[[field]]`

Gives a GitHub Projects custom field a friendly display name for CLI output. `display-name` is how skill-tree refers to the field; `github-name` must match GitHub's field header character for character, including case and spacing. Optional — skill-tree fetches all fields regardless of whether they appear in `[[field]]`. Unknown keys in a `[[field]]` entry are rejected at parse time.

### `[colors]`

Controls node color in the rendered graph. Optional. `github-name` selects which GitHub field drives color. `values` maps that field's option names to hex colors (`#rgb` or `#rrggbb`, leading `#` required).

`github-name` does not need a corresponding `[[field]]` entry — it refers directly to the GitHub field name. Keys in `[colors.values]` must match GitHub's option names exactly, including case and spacing. Nodes whose value is not in the map render with the default gray (`#dddddd`).

### `[cluster]`

Groups nodes into Graphviz subgraphs by the value of a GitHub SingleSelect field. Optional. `github-name` selects the field; `values` maps option names to friendly display labels for the cluster box. Unmapped options render with the raw option name. The field must be SingleSelect; mismatches surface during validation against project metadata.

### `[edges.cross-ref]`

Configures which GitHub cross-references become graph edges. `require-labels` is a list of label names. A cross-reference renders only if its source carries at least one listed label (exact-name, any-of). The default is the empty list — every cross-reference drops. Cross-refs are noisy by nature; users opt in by listing the labels that mark "this mention is a real dependency".

## Field auto-discovery

skill-tree fetches all custom fields GitHub returns for every project item, regardless of what is declared in `[[field]]`. Display declarations are not a fetch filter — they only affect CLI labeling. Adding a new `[[field]]` entry or a new `[colors.values]` key later does not require changing what gets fetched.

## Application context

The parsed `Config` is wrapped in a `SkillTree` struct that also carries the directory containing the config file. The rest of the pipeline takes `&SkillTree` rather than `&Config` directly, which keeps configuration threading explicit and avoids global state.

Two constructors:

- `SkillTree::from_dir(dir)` — load from a directory (production and tests).
- `SkillTree::from_path(path)` — load from an explicit file path.

## Validation

Parse-time failures:

- Missing `[github]` or its required keys.
- A `[[field]]` entry with unknown keys.
- Type mismatches on any field.

Validation-time failures (exit code 4):

- Any value in `[colors.values]` is not a valid hex color.

## Minimal config

```toml
[github]
owner   = "your-org"
project = 1
```

skill-tree fetches all fields and renders nodes in the default gray.
