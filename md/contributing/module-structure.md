# Key modules

skill-tree is organized as a three-stage pipeline: fetch data from GitHub, model it as a graph, render it as output.

## `config.rs` — configuration

Reads `.skill-tree.toml` and provides the `SkillTree` application context. Two constructors:
- `SkillTree::from_dir()` — load from a directory (production and tests)
- `SkillTree::from_path()` — load from an explicit file path

Provides query methods to access config data:
- `color_for_value()` — map a field option to a hex color
- `field_by_display_name()` — look up a field by its display name
- `color_field_github_name()` — which field drives node color

## `error/` — error types

All errors in skill-tree organized by origin:

- `error/github.rs` — GitHub API errors (`GitHubError`, `NetworkErrorKind`, `ErrorContext`)
- `error/config.rs` — configuration file errors (`ConfigError`)

Each error type implements `.exit_code()` to map to process exit codes (1, 3, or 4).

## `github/` — GitHub GraphQL client

The only module that talks to GitHub. Implements the fetch stage of the pipeline.

- `github/mod.rs` — `GitHubClient` with retry, rate limit, and timeout handling
- `github/projects.rs` — fetch GitHub Projects V2 items (stub)
- `github/issues.rs` — fetch issues and blocking relationships (stub)

The client automatically retries transient errors (exponential backoff), waits on rate limits, and fails if the operation exceeds the configured timeout. All errors carry `ErrorContext` (query name, owner, project) for debugging.

## `graph/` — graph model

The platform-agnostic data model: nodes (issues), edges (blocking relationships), and algorithms.

- `graph/mod.rs` — `Graph`, `Node`, `Edge` types
- `graph/validate.rs` — cycle detection and dangling edge detection

## `render/` — rendering

Turns a `Graph` into Graphviz DOT format and optionally renders it to SVG using the system `dot` binary.

- `render/mod.rs` — `render()` function, DOT generation