//! GitHub GraphQL API client.
//!
//! This module is the only place in skill-tree that talks to GitHub.
//! Everything else works with the typed structs from [`projects`] and [`issues`].
//!
//! See `md/design/github_client.md` for the design.

pub mod issues;
pub mod projects;

use crate::error::{GitHubError, NetworkErrorKind};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// GraphQL primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct GraphQLRequest<'a, V: Serialize> {
    pub query: &'a str,
    pub variables: V,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GraphQLResponse<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<GraphQLErrorResponse>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GraphQLErrorResponse {
    pub message: String,
}

// ---------------------------------------------------------------------------
// Pagination types
// ---------------------------------------------------------------------------
//
// GitHub's GraphQL API uses cursor-based pagination on every list ("connection").
// The transport does not paginate — callers loop, using `Connection<T>` in
// their response types and reading `page_info` to drive the loop.

/// Page metadata returned by every GitHub GraphQL connection.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

/// A list of `nodes` plus its `page_info`. Embed in your response struct
/// to get the standard pagination shape.
#[derive(Debug, Clone, Deserialize)]
pub struct Connection<T> {
    pub nodes: Vec<T>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// A configured GitHub GraphQL client with built-in retry and rate limit handling.
///
/// Handles network errors, transient failures, rate limiting, and timeouts.
/// Pass `&GitHubClient` to [`projects`] and [`issues`] functions.
pub struct GitHubClient {
    client: Client,
    endpoint: String,
    token: String,
    timeout: Duration,
}

impl GitHubClient {
    const DEFAULT_ENDPOINT: &'static str = "https://api.github.com/graphql";
    const API_VERSION: &'static str = "2022-11-28";

    /// Maximum number of HTTP requests per `query()` call: one initial
    /// attempt plus `MAX_ATTEMPTS - 1` retries.
    const MAX_ATTEMPTS: u32 = 3;

    /// Wait used when GitHub returns a 429 with no parseable
    /// `X-RateLimit-Reset` header. One minute is GitHub's documented
    /// minimum reset window for secondary rate limits.
    const RATE_LIMIT_FALLBACK_SECS: u64 = 60;

    /// Create a new client targeting `https://api.github.com/graphql`,
    /// reading the token from the parameter or the `GITHUB_TOKEN` env var.
    ///
    /// Fails immediately with [`GitHubError::MissingToken`] if neither is present,
    /// before any network I/O occurs.
    pub fn new(token: Option<String>, timeout: Duration) -> Result<Self, GitHubError> {
        Self::with_endpoint(Self::DEFAULT_ENDPOINT.to_string(), token, timeout)
    }

    /// Like [`Self::new`] but targets the supplied GraphQL endpoint URL.
    /// Used by integration tests against a mock server; also the foundation
    /// for any future GitHub Enterprise support.
    pub fn with_endpoint(
        endpoint: String,
        token: Option<String>,
        timeout: Duration,
    ) -> Result<Self, GitHubError> {
        let token = token
            .or_else(|| std::env::var("GITHUB_TOKEN").ok())
            .ok_or(GitHubError::MissingToken)?;

        // Per-request timeouts are set in `query_once` from the *remaining*
        // budget, so a single hung request can't consume the whole timeout.
        let client = Client::builder()
            .user_agent("skill-tree")
            .build()
            .map_err(|e| GitHubError::ClientInit(e.to_string()))?;

        Ok(Self {
            client,
            endpoint,
            token,
            timeout,
        })
    }

    /// Send a GraphQL query with automatic retry and rate limit handling.
    ///
    /// Makes one initial HTTP request plus up to 2 retries on transient
    /// failures, with exponential backoff between attempts. Detects rate
    /// limits and waits before retrying when the timeout budget allows.
    /// Fails with [`GitHubError::Timeout`] if the entire operation exceeds
    /// the configured timeout.
    pub async fn query<V, T>(&self, query: &str, variables: V) -> Result<T, GitHubError>
    where
        V: Serialize,
        T: DeserializeOwned,
    {
        let start = Instant::now();

        for attempt in 1..=Self::MAX_ATTEMPTS {
            if start.elapsed() >= self.timeout {
                return Err(GitHubError::Timeout(self.timeout.as_secs()));
            }

            let err = match self.query_once(query, &variables, start).await {
                Ok(response) => return Ok(response),
                Err(err) => err,
            };

            // Last attempt: surface whatever we got, no more retries.
            if attempt == Self::MAX_ATTEMPTS {
                return Err(err);
            }

            // Rate limit: wait if the remaining budget covers it, else fail now.
            if let GitHubError::RateLimited { retry_after } = &err {
                let wait_secs = *retry_after;
                let remaining = self
                    .timeout
                    .as_secs()
                    .saturating_sub(start.elapsed().as_secs());

                if remaining > wait_secs {
                    eprintln!("Rate limited, waiting {wait_secs} seconds...");
                    tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                    continue;
                }
                return Err(err);
            }

            // Transient: back off and retry.
            if Self::is_transient(&err) {
                let backoff = Self::backoff_duration(attempt);
                eprintln!(
                    "Transient error (attempt {}/{}), retrying in {:?}...",
                    attempt,
                    Self::MAX_ATTEMPTS,
                    backoff
                );
                tokio::time::sleep(backoff).await;
                continue;
            }

            // Non-transient: fail fast.
            return Err(err);
        }

        // Loop body always returns or `continue`s on attempts < MAX_ATTEMPTS,
        // and always returns on attempt == MAX_ATTEMPTS.
        unreachable!("retry loop exited without returning")
    }

    /// Send a single GraphQL request without retry logic. The per-request
    /// timeout is the *remaining* budget so a single hung request cannot
    /// consume the whole `query()`-level timeout.
    async fn query_once<V, T>(
        &self,
        query: &str,
        variables: &V,
        start: Instant,
    ) -> Result<T, GitHubError>
    where
        V: Serialize,
        T: DeserializeOwned,
    {
        let request = GraphQLRequest { query, variables };
        let remaining = self.timeout.saturating_sub(start.elapsed());

        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .header("X-GitHub-Api-Version", Self::API_VERSION)
            .timeout(remaining)
            .json(&request)
            .send()
            .await
            .map_err(Self::classify_reqwest_error)?;

        let status = response.status();
        if !status.is_success() {
            if status.as_u16() == 429 {
                let retry_after = response
                    .headers()
                    .get("X-RateLimit-Reset")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .and_then(|reset_time| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()?
                            .as_secs();
                        Some(reset_time.saturating_sub(now))
                    });

                return Err(GitHubError::RateLimited {
                    retry_after: retry_after.unwrap_or(Self::RATE_LIMIT_FALLBACK_SECS),
                });
            }

            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read body: {e}>"));
            return Err(GitHubError::HttpError {
                status: status.as_u16(),
                body,
            });
        }

        let body: GraphQLResponse<T> = response
            .json()
            .await
            .map_err(Self::classify_reqwest_error)?;

        // GitHub's GraphQL spec says `errors` must contain at least one entry
        // when present. Treat an empty array the same as no errors so we don't
        // surface a useless `GraphQLError("")`.
        if let Some(errors) = body.errors.filter(|e| !e.is_empty()) {
            let message = errors
                .into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; ");
            return Err(GitHubError::GraphQLError(message));
        }

        body.data.ok_or_else(|| {
            GitHubError::InvalidResponse(
                "GraphQL response had neither `data` nor `errors`".to_string(),
            )
        })
    }

    /// Classify a reqwest error. JSON decode failures are reported as
    /// `InvalidResponse`; everything else is a `Network` error.
    fn classify_reqwest_error(err: reqwest::Error) -> GitHubError {
        if err.is_decode() {
            return GitHubError::InvalidResponse(err.to_string());
        }

        let kind = if err.is_timeout() {
            NetworkErrorKind::Timeout
        } else if err.is_connect() {
            NetworkErrorKind::Connection
        } else {
            NetworkErrorKind::Other(err.to_string())
        };

        GitHubError::Network {
            kind,
            message: err.to_string(),
        }
    }

    /// Check if an error is transient and worth retrying.
    fn is_transient(err: &GitHubError) -> bool {
        match err {
            GitHubError::Network { .. } => true,
            GitHubError::HttpError { status, .. } => *status >= 500,
            _ => false,
        }
    }

    /// Delay before retry, with ±20% jitter to avoid thundering herd.
    /// Called after a failed `attempt` when more retries remain, so for
    /// `MAX_ATTEMPTS = 3` the inputs are 1 (~1s) and 2 (~2s).
    fn backoff_duration(attempt: u32) -> Duration {
        let base_millis = 1000_u64 * 2_u64.pow(attempt - 1);
        let jitter_pct = rand::random::<u64>() % 21; // 0..=20
        let signed = if rand::random::<bool>() {
            base_millis + base_millis * jitter_pct / 100
        } else {
            base_millis - base_millis * jitter_pct / 100
        };
        Duration::from_millis(signed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    struct Issue {
        number: u64,
    }

    #[test]
    fn connection_deserializes_from_github_shape() {
        let json = r#"{
            "nodes": [{"number": 1}, {"number": 2}],
            "pageInfo": {
                "hasNextPage": true,
                "endCursor": "Y3Vyc29yOjEw"
            }
        }"#;

        let conn: Connection<Issue> = serde_json::from_str(json).unwrap();
        assert_eq!(conn.nodes.len(), 2);
        assert_eq!(conn.nodes[0].number, 1);
        assert!(conn.page_info.has_next_page);
        assert_eq!(conn.page_info.end_cursor.as_deref(), Some("Y3Vyc29yOjEw"));
    }

    #[test]
    fn page_info_handles_null_end_cursor_on_last_page() {
        let json = r#"{"hasNextPage": false, "endCursor": null}"#;
        let info: PageInfo = serde_json::from_str(json).unwrap();
        assert!(!info.has_next_page);
        assert!(info.end_cursor.is_none());
    }
}
