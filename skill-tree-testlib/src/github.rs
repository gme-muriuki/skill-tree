//! Mock GitHub GraphQL endpoint for integration tests.
//!
//! `MockGitHub` wraps a `wiremock::MockServer` configured to look like
//! `https://api.github.com/graphql`, plus response builders for the
//! shapes `GitHubClient` needs to handle: 200-with-data, 5xx, 429 with
//! `X-RateLimit-Reset`, GraphQL `errors` envelopes, and malformed bodies.
//!
//! Tests only import this module — they should not pull in `wiremock`
//! directly. `Mock` is re-exported so callers can chain `.expect(N)` /
//! `.up_to_n_times(N)` and call `.mount(&gh.server).await` without a
//! `wiremock` dependency line.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use skill_tree::github::GitHubClient;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockBuilder, MockServer, ResponseTemplate};

pub use wiremock::Mock as MockHandle;

/// A wiremock server preconfigured to look like GitHub's GraphQL endpoint.
pub struct MockGitHub {
    pub server: MockServer,
}

impl MockGitHub {
    pub async fn start() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    /// A `GitHubClient` pointed at this mock with the given timeout.
    /// The token is a non-empty placeholder so `with_endpoint` does not
    /// fall back to `GITHUB_TOKEN`.
    pub fn client(&self, timeout: Duration) -> GitHubClient {
        GitHubClient::with_endpoint(
            format!("{}/graphql", self.server.uri()),
            Some("test-token".into()),
            timeout,
        )
        .expect("token is supplied directly")
    }

    /// `POST /graphql` matcher base. Useful when a test needs an extra
    /// matcher like a header check.
    pub fn matcher(&self) -> MockBuilder {
        Mock::given(method("POST")).and(path("/graphql"))
    }

    /// 200 response wrapping `body` in a GraphQL `data` envelope.
    pub fn ok_data(&self, body: Value) -> MockHandle {
        self.matcher().respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "data": body })),
        )
    }

    /// Like `ok_data`, but the mock only matches requests carrying every
    /// `(name, value)` header pair. Used to assert that the client sent
    /// the headers it was supposed to.
    pub fn ok_data_with_headers(&self, body: Value, headers: &[(&str, &str)]) -> MockHandle {
        let mut builder = self.matcher();
        for (name, value) in headers {
            builder = builder.and(header(*name, *value));
        }
        builder.respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "data": body })),
        )
    }

    /// 200 response with a non-empty GraphQL `errors` array.
    pub fn graphql_error(&self, message: &str) -> MockHandle {
        self.matcher()
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": null,
                "errors": [{ "message": message }],
            })))
    }

    /// 200 response with neither `data` nor `errors` — exercises
    /// `GitHubError::InvalidResponse`.
    pub fn empty_envelope(&self) -> MockHandle {
        self.matcher()
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
    }

    /// Response with the given HTTP status and an empty body.
    pub fn status(&self, status: u16) -> MockHandle {
        self.matcher().respond_with(ResponseTemplate::new(status))
    }

    /// Response with the given HTTP status and a string body.
    pub fn status_with_body(&self, status: u16, body: &str) -> MockHandle {
        self.matcher()
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
    }

    /// 429 response with `X-RateLimit-Reset` set to `now + secs_until_reset`,
    /// matching the format GitHub returns.
    pub fn rate_limited(&self, secs_until_reset: u64) -> MockHandle {
        let reset = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is past UNIX_EPOCH")
            .as_secs()
            + secs_until_reset;
        self.matcher().respond_with(
            ResponseTemplate::new(429)
                .insert_header("X-RateLimit-Reset", reset.to_string().as_str()),
        )
    }
}
