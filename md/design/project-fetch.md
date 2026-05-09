# Project fetching

The `github/projects.rs` module fetches a single GitHub Projects V2 board and returns it as a typed Rust value.

It owns the GraphQL queries for project metadata, project items, and sub-issue overflow; the response types (`ProjectMeta`, `ProjectItem`, `FieldValue`, `ItemContent`); and the pagination loop. Transport, auth, and retries live in `github/mod.rs`. GitHub-native blocking edges will live in `github/issues.rs` once the edge convention is settled. Graph construction lives in `graph/mod.rs`, which consumes a `ProjectFetch` and decides what becomes a node, what becomes an edge, and what to filter.

## Public entry point

```rust
pub async fn fetch_project(
    client: &GitHubClient,
    config: &Config,
) -> Result<ProjectFetch, GitHubError>;
```

`ProjectFetch` carries the project metadata and the full item list. Pagination is internal — the caller never sees a cursor.

`fetch_project()` runs the metadata query first, validates `Config` against the returned field definitions, and only then issues the items query. A typo in `.skill-tree.toml` fails fast after one cheap call instead of after the full paginated item fetch.

## Three queries

Project metadata, project items, and sub-issue overflow are three separate GraphQL documents.

The metadata query runs once. It returns project title, field definitions, and field options. `Config` validation depends on this data.

The items query is paginated at `first: 100`. For each item it returns `fieldValues`, the underlying `content` (Issue, PullRequest, DraftIssue), light per-item metadata, and an inline `subIssues(first: 50)` connection.

The sub-issue overflow query runs per-issue when an inline sub-issue connection reports `hasNextPage: true`. `fetch_project()` resolves it before returning, so the graph layer always receives complete sub-issue lists.

## Field values

`fieldValues.nodes` is a polymorphic union. It deserializes into a `FieldValue` enum with one variant per GitHub field type — `Text`, `Number`, `SingleSelect`, `Date`, `Iteration` — plus an `Unknown` variant via `#[serde(other)]` so new GitHub types do not crash the fetch.

`FieldValue::display_string()` returns the value as a string for `[colors.values]` lookup and CLI output. It returns `None` for `Unknown`.

## Item content

Project items expose their underlying GitHub object through an `ItemContent` enum with four variants: `Issue`, `PullRequest`, `DraftIssue`, and `Redacted` (the token has lost permission to read the content, or the content was deleted).

All four travel through the transport layer unfiltered. The graph layer decides what to render. Each content variant carries fields the graph does not render today (assignees, labels, state, body).

`DraftIssue` items have no underlying GitHub Issue and therefore no sub-issues or blocking edges. They are leaf nodes.

## Sub-issue fetching

The inline `first: 50` covers most projects. When an issue exceeds it, `fetch_project()` issues per-issue follow-up queries internally and returns the complete list. The graph layer never sees pagination state.

GitHub caps `first` at 100, so an issue with 200 sub-issues costs two follow-up calls. Overflow is a long-tail event — most projects never trigger it.

## Errors

`fetch_project()` returns `GitHubError` from `github/mod.rs`. If the project does not exist or the token cannot see it, the metadata query returns a `GitHubError::GraphQLError` with GitHub's upstream message. Config-vs-metadata mismatches surface as `GitHubError::ConfigMismatch`, a new variant carrying the offending field or option name.
