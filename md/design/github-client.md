# GitHub client

The `github/mod.rs` module owns all communication with the GitHub API. Other modules import typed structs from `github/projects.rs` and `github/issues.rs` and never construct URLs, handle HTTP errors, or parse JSON directly.

The module's responsibilities are authentication, transport, and error translation. The actual GraphQL queries live in `projects.rs` and `issues.rs`; those modules call back into this module for transport.

## Public API

```rust
pub struct GitHubClient { ... }

impl GitHubClient {
    pub fn new(token: Option<String>, timeout: Duration) -> Result<Self, GitHubError>;

    pub async fn query<V: Serialize, T: DeserializeOwned>(
        &self,
        query: &str,
        variables: V,
    ) -> Result<T, GitHubError>;
}
```

`new()` is synchronous and does no I/O. The client owns the HTTP connection pool, the auth token, and the timeout. `query()` sends one GraphQL request and returns the typed `data` field. Pagination is not handled by `query()` — see [Pagination](#pagination).

## Authentication

The token comes from `--token` (CLI flag, takes precedence) or `GITHUB_TOKEN` (environment variable). If neither is set, `GitHubClient::new()` returns `GitHubError::MissingToken` before any network I/O.

Required scopes: `read:project` for GitHub Projects V2, and `repo` for issue content and blocking relationships on private repositories. `public_repo` is sufficient for public repositories.

## Transport

`reqwest` for HTTP, `tokio` for async runtime. Each `query()` call serializes variables to JSON, POSTs to `https://api.github.com/graphql` with the `Authorization` header, parses the response body, checks HTTP status, checks for `errors` in the body, and returns `data` on success. The timeout applies to the entire request including retry backoff.

## Retry

Transient errors retry with exponential backoff and jitter: up to three attempts at ~1s, ~2s, ~4s with ±20% jitter, never exceeding the overall timeout. Retried conditions: network failures, HTTP 5xx, and HTTP 429.

Non-transient errors (4xx except 429, GraphQL validation, auth failures) fail immediately.

## Rate limiting

On HTTP 429, the client parses `X-RateLimit-Reset` to calculate the wait. If the wait fits within the remaining timeout the client logs the wait and sleeps. Otherwise it returns `GitHubError::RateLimited { retry_after }` so the caller can decide whether to wait or fail.

## Pagination

GitHub's GraphQL API uses cursor-based pagination. The transport sends one request and returns one response. Pagination loops live in the caller (`projects.rs`, `issues.rs`) where the query and response shape are known.

The transport provides two reusable types:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(Deserialize)]
pub struct Connection<T> {
    pub nodes: Vec<T>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}
```

Callers embed `Connection<T>` in their response types and loop on `pageInfo.has_next_page`. The query must declare `first: N` and `after: $after`.

A generic `paginate(...)` helper is not provided. Two callers exist (`projects.rs`, eventually `issues.rs`) and the loop pattern is short.

## Errors

`GitHubError` covers missing token, client-init failures, network-level failures, HTTP errors, GraphQL errors, malformed responses, rate-limit exhaustion, and overall timeout.

Exit codes via `GitHubError::exit_code()`:

- `1` — malformed upstream body (likely a regression).
- `3` — network, HTTP, GraphQL, rate-limit, or timeout failures.
- `4` — configuration failures (`MissingToken`, `ClientInit`).

Errors do not carry caller context. Callers wrap with context at their call sites.

## Configuration

The timeout is set globally via `--timeout` (seconds) or the `GITHUB_TIMEOUT` environment variable. Default is 30 seconds. Per-request overrides are not supported.

## Module structure

```
github/
  mod.rs         GitHubClient, transport, auth, retry
  projects.rs    project fetching (see project-fetch.md)
  issues.rs      GitHub-native blocking edges (deferred)
```
