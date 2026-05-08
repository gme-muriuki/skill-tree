# Important flows

skill-tree is a three-stage pipeline. These are the major paths through the code.

## `render` command

Fetch GitHub Project → model as graph → render as DOT/SVG.

**Flow:**
1. Load config from `.skill-tree.toml`
2. Construct GitHub client (read token from `--token` or `GITHUB_TOKEN` env var)
3. Fetch project items and issues from GitHub GraphQL API
4. Build graph: create nodes for each issue, edges for blocking relationships
5. Render graph to Graphviz DOT format
6. If `--format svg` specified, invoke system `dot` binary to render SVG
7. Write output to file or stdout

## `validate` command

Load graph → check for cycles and dangling edges.

**Flow:**
1. Load config from `.skill-tree.toml`
2. Fetch and build graph (same as render command, steps 2-4)
3. Run cycle detection: depth-first search from each unvisited node
4. Check for dangling edges: edges that reference issues not in the project
5. Exit with code 0 if valid, 2 if cycles found, 3 if GitHub error, 4 if config error

## `unblocked` command

Load graph → find issues with no incoming blocking edges.

**Flow:**
1. Load config from `.skill-tree.toml`
2. Fetch and build graph (same as render command, steps 2-4)
3. Filter to issues with no incoming edges
4. Sort by issue number for deterministic output
5. Print each unblocked issue (or JSON if `--json` specified)