# Common issues

## Known v1 limitations

### Pagination not yet implemented

The GitHub client has the structure for automatic pagination (following `hasNextPage` cursors), but the actual implementation is stubbed. For projects with more than ~100 items, fetching will be incomplete.

**Status:** Ready to implement after `projects.rs` is written. The client already handles the retry/rate limit/timeout logic that pagination needs.

### PR node support deferred

Pull requests are not included in the graph. Only issues and their blocking relationships are fetched.

**Why:** PRs don't form dependency nodes in the skill tree model. A PR either resolves an issue (removes it) or doesn't. Blocking relationships are only tracked between issues.

**Status:** Deferred to v2 if users request it.

### Edge source: GitHub blocking only

The only blocking relationships fetched are those tracked natively in GitHub Projects (the "blocks" field). Relationships encoded in issue bodies (e.g., "blocked by #123") are not parsed.

**Why:** Text parsing is fragile and loses the explicit structure GitHub provides. Native blocking is more reliable.

**Status:** v2 may add opt-in body parsing if requested.

### Single color field in v1

Only one GitHub field can drive node color. Multiple color rules (e.g., Status drives fill, Priority drives border) are deferred.

**Why:** Keeping v1 simple. The infrastructure is designed to support multiple rules in v2.

**Status:** v2 will add `[[color-rule]]` with `attribute` field.

### Deterministic output only by issue number

Node and edge order in DOT output is deterministic (sorted by issue number) but not configurable. Custom sort orders are deferred.

**Status:** v2 may add sort options.

## Potential gotchas for contributors

### `ErrorContext` is metadata, not an error source

The `ErrorContext` struct carries debugging information (query name, owner, project) but is not part of the error chain. Don't use `#[source]` on ErrorContext fields.

### Config filename has a hyphen

The config file is `.skill-tree.toml` (hyphen), not `.skill_tree.toml` (underscore). This matters in tests and error messages.

### GitHub returns 200 for GraphQL errors

When GitHub encounters a GraphQL error, it returns HTTP 200 with an `errors` field in the JSON body. Always check `errors` even on successful HTTP status.

### Network errors during JSON parsing

When `response.json()` fails (bad JSON from GitHub), it returns a `reqwest::Error`, not `serde_json::Error`. Classify it as a network error, not a JSON error.