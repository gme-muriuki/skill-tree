//! Integration tests for `GitHubClient` against a mock GraphQL endpoint.
//!
//! Only imports the public API of `skill_tree` and the test infrastructure
//! exposed by `skill_tree_testlib`. The wiremock plumbing lives in the
//! testlib so individual tests stay focused on the scenario.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use skill_tree::error::GitHubError;
use skill_tree_testlib::MockGitHub;

#[derive(Serialize)]
struct EmptyVars {}

#[derive(Debug, Deserialize, PartialEq)]
struct Hello {
    hello: String,
}

#[tokio::test]
async fn retries_5xx_then_succeeds_and_returns_data() {
    let gh = MockGitHub::start().await;
    gh.status(503).up_to_n_times(1).mount(&gh.server).await;
    gh.ok_data(json!({ "hello": "world" }))
        .mount(&gh.server)
        .await;

    let client = gh.client(Duration::from_secs(10));
    let resp: Hello = client
        .query("query Q { hello }", EmptyVars {})
        .await
        .unwrap();
    assert_eq!(
        resp,
        Hello {
            hello: "world".into()
        }
    );
}

#[tokio::test]
async fn gives_up_after_max_retries_returning_last_real_error() {
    let gh = MockGitHub::start().await;
    gh.status_with_body(500, "boom")
        .expect(3) // MAX_ATTEMPTS
        .mount(&gh.server)
        .await;

    let client = gh.client(Duration::from_secs(30));
    let err = client
        .query::<_, Hello>("query Q { hello }", EmptyVars {})
        .await
        .unwrap_err();

    match err {
        GitHubError::HttpError { status, body } => {
            assert_eq!(status, 500);
            assert_eq!(body, "boom");
        }
        other => panic!("expected HttpError(500), got {other:?}"),
    }
}

#[tokio::test]
async fn rate_limit_within_budget_waits_and_retries() {
    let gh = MockGitHub::start().await;
    gh.rate_limited(1).up_to_n_times(1).mount(&gh.server).await;
    gh.ok_data(json!({ "hello": "world" }))
        .mount(&gh.server)
        .await;

    let client = gh.client(Duration::from_secs(10));
    let resp: Hello = client
        .query("query Q { hello }", EmptyVars {})
        .await
        .unwrap();
    assert_eq!(
        resp,
        Hello {
            hello: "world".into()
        }
    );
}

#[tokio::test]
async fn rate_limit_outside_budget_surfaces_to_caller() {
    let gh = MockGitHub::start().await;
    gh.rate_limited(60).mount(&gh.server).await;

    // Tight timeout so a 60s wait is outside the budget.
    let client = gh.client(Duration::from_secs(2));
    let err = client
        .query::<_, Hello>("query Q { hello }", EmptyVars {})
        .await
        .unwrap_err();

    match err {
        GitHubError::RateLimited { retry_after } => {
            assert!(retry_after >= 58, "expected ~60s, got {retry_after}");
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[tokio::test]
async fn graphql_errors_are_returned_without_retry() {
    let gh = MockGitHub::start().await;
    gh.graphql_error("Field 'oops' not found")
        .expect(1) // not retried
        .mount(&gh.server)
        .await;

    let client = gh.client(Duration::from_secs(10));
    let err = client
        .query::<_, Hello>("query Q { oops }", EmptyVars {})
        .await
        .unwrap_err();

    match err {
        GitHubError::GraphQLError(msg) => assert!(msg.contains("oops")),
        other => panic!("expected GraphQLError, got {other:?}"),
    }
}

#[tokio::test]
async fn invalid_response_when_envelope_has_neither_data_nor_errors() {
    let gh = MockGitHub::start().await;
    gh.empty_envelope()
        .expect(1) // not retried
        .mount(&gh.server)
        .await;

    let client = gh.client(Duration::from_secs(10));
    let err = client
        .query::<_, Hello>("query Q { hello }", EmptyVars {})
        .await
        .unwrap_err();

    assert!(
        matches!(err, GitHubError::InvalidResponse(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn sends_api_version_and_authorization_headers() {
    let gh = MockGitHub::start().await;
    gh.ok_data_with_headers(
        json!({ "hello": "world" }),
        &[
            ("X-GitHub-Api-Version", "2022-11-28"),
            ("Authorization", "Bearer test-token"),
        ],
    )
    .expect(1)
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let _: Hello = client
        .query("query Q { hello }", EmptyVars {})
        .await
        .unwrap();
    // Mock's `.expect(1)` is verified on drop — if headers were wrong, no
    // mock would have matched and the assertion would fail there.
}
