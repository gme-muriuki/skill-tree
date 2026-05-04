# Key modules

skill-tree is a Rust workspace with a library crate (`skill-tree/src/lib.rs`),
a binary (`skill-tree/src/main.rs`), and a test infrastructure crate
(`skill-tree-testlib`).

### `config.rs` — configuration

Reads and validates `.skill-tree.toml`. Owns four structs:

- `GitHubConfig` — `owner` and `project` number
- `FieldConfig` — one entry per `[[field]]` block, with `display_name`
  (how skill-tree refers to the field internally) and `github_name`
  (how the field appears in GitHub Projects)
- `ColorsConfig` — `github_name` identifying which field drives node color,
  and a `values` map from that field's option strings to hex colors
- `Config` — the top-level struct holding all of the above

`Config::load()` reads the file, deserializes it, and validates it.
`Config::color_for_value()` looks up a hex color by field option value.
`Config::github_name_for_display()` resolves a display name to its GitHub
field name. Called by every CLI subcommand before the pipeline starts.

### `github/mod.rs` — GraphQL client

The only module that talks to GitHub. Owns `GitHubClient`, which wraps
`reqwest::Client` with bearer token auth. Exposes a single generic
`client.query()` method that all fetch functions call. Reads `GITHUB_TOKEN`
from the environment at construction time and fails fast if it is missing.

### `github/projects.rs` — project item fetching

Fetches GitHub Projects V2 items via GraphQL. For each item, reads the
values of every field declared in `[[field]]` blocks in the config.
Handles pagination by following `hasNextPage` cursors until all items
are collected. Returns `Vec<ProjectItem>` with number, title, url, and
a `HashMap<String, String>` of github-name → field value. Knows nothing
about the graph model.

### `github/issues.rs` — issue relationship fetching

Fetches sub-issues and blocking relationships for a list of issue numbers.
Returns `Vec<IssueRelationships>` with sub-issue numbers and blocked issue
numbers. Exposes `parse_owner_repo()` to extract owner and repo from a
GitHub issue URL.

### `graph/mod.rs` — graph data model

Defines `Node`, `Edge`, and `Graph`. A `Node` carries a `fields` map of
github-name → value for all fetched field values, not just status.
`Graph::build()` takes `Vec<ProjectItem>` and `Vec<IssueRelationships>`
and produces a `Graph`. Exposes `Graph::unblocked_nodes()` for the
unblocked subcommand. Knows nothing about GitHub or Graphviz.

### `graph/validate.rs` — graph validation

Cycle detection via depth-first search with path tracking. Dangling edge
detection. Orphaned node warnings. All validation collects every problem
before returning rather than stopping at the first error. Called by
`Graph::validate()` and the validate subcommand.

### `render/mod.rs` — DOT and SVG output

Renders a `Graph` as Graphviz DOT. Determines node color by looking up
the value of the field named in `colors.github-name` from each node's
`fields` map, then resolving it through `Config::color_for_value()`.
Sorts nodes by issue number before iterating to guarantee deterministic
output. Pipes DOT through the system `dot` binary to produce SVG. Every
node carries a `URL` attribute linking to its GitHub issue. Gives a
precise install message if `dot` is not found.

### `cli/` — subcommand implementations

Three files, one per subcommand. `render.rs` runs the full pipeline and
writes DOT or SVG. `unblocked.rs` runs fetch and model then prints issues
with indegree zero. `validate.rs` runs fetch and model then reports
problems without producing any rendered output. `mod.rs` owns the clap
definitions.

### `error.rs` — error types and exit codes

Defines exit codes: 0 success, 1 general error, 2 cycle detected,
3 GitHub API error, 4 configuration error. Each error variant carries
a message that names the specific thing that went wrong.