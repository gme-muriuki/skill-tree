//! GitHub API error types.
//!
//! All failures from GitHub requests are translated into structured
//! `GitHubError` variants. The transport layer does not know about
//! higher-level concerns like which project or owner triggered a call;
//! callers that want that context should wrap these errors at their
//! call site.

use crate::error::ConfigIssue;
use crate::github::projects::OwnerKind;
use std::fmt;

/// Error returned by the GitHub GraphQL client.
#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    /// No token found in --token flag or GITHUB_TOKEN environment variable.
    #[error("no GitHub token found. Set GITHUB_TOKEN or use --token flag")]
    MissingToken,

    /// HTTP client could not be constructed (TLS backend, proxy config, etc.).
    #[error("failed to initialize HTTP client: {0}")]
    ClientInit(String),

    /// Network-level failure: timeout, DNS, TLS, connection refused, etc.
    #[error("network error ({kind}): {message}")]
    Network {
        /// Category of network failure.
        kind: NetworkErrorKind,
        /// Human-readable description.
        message: String,
    },

    /// HTTP response with error status code (4xx or 5xx).
    #[error("HTTP {status}: {body}")]
    HttpError {
        /// HTTP status code.
        status: u16,
        /// Full response body.
        body: String,
    },

    /// GraphQL response contained errors in the `errors` field.
    #[error("GraphQL error: {0}")]
    GraphQLError(String),

    /// GitHub returned a body we could not interpret: malformed JSON, or a
    /// well-formed envelope with neither `data` nor `errors`.
    #[error("invalid response body: {0}")]
    InvalidResponse(String),

    /// GitHub rate limit exceeded. Caller should wait before retrying.
    #[error("rate limit exceeded, retry after {retry_after}s")]
    RateLimited {
        /// Seconds to wait before retrying.
        retry_after: u64,
    },

    /// Request exceeded the configured timeout.
    #[error("request timeout after {0}s")]
    Timeout(u64),

    /// Neither `organization(login: $owner)` nor `user(login: $owner)`
    /// returned a non-null node. The owner does not exist, or the token
    /// lacks the scope to see it (GitHub returns null in both cases).
    #[error(
        "no organization or user named '{owner}' visible to this token \
         (it may not exist or your token may lack access)"
    )]
    OwnerUnreachable { owner: String },

    /// Owner exists but has no project with the given number.
    #[error("project #{number} not found under {owner_kind} '{owner}'")]
    ProjectNotFound {
        owner: String,
        number: u64,
        owner_kind: OwnerKind,
    },

    /// `.skill-tree.toml` references fields or option values that do not
    /// match the project's metadata. All issues found are reported together.
    #[error("{}", format_config_mismatch(.issues))]
    ConfigMismatch { issues: Vec<ConfigIssue> },
}

fn format_config_mismatch(issues: &[ConfigIssue]) -> String {
    use std::fmt::Write;
    let mut out = format!(
        "{} config issue{}:",
        issues.len(),
        if issues.len() == 1 { "" } else { "s" }
    );
    for issue in issues {
        write!(out, "\n  {issue}").expect("writing to a String never fails");
    }
    out
}

/// Category of network-level failure.
#[derive(Debug, Clone)]
pub enum NetworkErrorKind {
    /// Request timeout (socket, DNS, or connection timeout).
    Timeout,
    /// Connection refused, reset, or closed unexpectedly.
    Connection,
    /// Other network error not categorized above.
    Other(String),
}

impl fmt::Display for NetworkErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkErrorKind::Timeout => write!(f, "timeout"),
            NetworkErrorKind::Connection => write!(f, "connection refused"),
            NetworkErrorKind::Other(s) => write!(f, "{s}"),
        }
    }
}

impl GitHubError {
    /// Return the process exit code for this error.
    ///
    /// - 1: malformed response (likely a bug or upstream regression)
    /// - 3: GitHub API errors (network, HTTP, GraphQL, rate limit, timeout)
    /// - 4: configuration errors (missing token, client init failure)
    pub fn exit_code(&self) -> u8 {
        match self {
            GitHubError::MissingToken | GitHubError::ClientInit(_) => 4,
            GitHubError::Network { .. }
            | GitHubError::HttpError { .. }
            | GitHubError::GraphQLError(_)
            | GitHubError::RateLimited { .. }
            | GitHubError::Timeout(_)
            | GitHubError::OwnerUnreachable { .. }
            | GitHubError::ProjectNotFound { .. } => 3,
            GitHubError::ConfigMismatch { .. } => 4,
            GitHubError::InvalidResponse(_) => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_token_exit_code() {
        assert_eq!(GitHubError::MissingToken.exit_code(), 4);
    }

    #[test]
    fn client_init_exit_code() {
        assert_eq!(GitHubError::ClientInit("tls".into()).exit_code(), 4);
    }

    #[test]
    fn network_error_exit_code() {
        let err = GitHubError::Network {
            kind: NetworkErrorKind::Timeout,
            message: "timeout waiting for response".to_string(),
        };
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn http_error_exit_code() {
        let err = GitHubError::HttpError {
            status: 500,
            body: "Internal Server Error".to_string(),
        };
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn graphql_error_exit_code() {
        let err = GitHubError::GraphQLError("Field not found".to_string());
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn rate_limited_exit_code() {
        let err = GitHubError::RateLimited { retry_after: 3600 };
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn timeout_exit_code() {
        let err = GitHubError::Timeout(30);
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn invalid_response_exit_code() {
        let err = GitHubError::InvalidResponse("no data, no errors".into());
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn owner_unreachable_exit_code() {
        let err = GitHubError::OwnerUnreachable {
            owner: "rust-lan".into(),
        };
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn project_not_found_exit_code() {
        let err = GitHubError::ProjectNotFound {
            owner: "rust-lang".into(),
            number: 4242,
            owner_kind: OwnerKind::Organization,
        };
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn config_mismatch_exit_code() {
        let err = GitHubError::ConfigMismatch {
            issues: vec![ConfigIssue::FieldNotFound {
                section: "colors",
                name: "Statu".into(),
            }],
        };
        assert_eq!(err.exit_code(), 4);
    }

    #[test]
    fn config_mismatch_display_lists_each_issue() {
        let err = GitHubError::ConfigMismatch {
            issues: vec![
                ConfigIssue::FieldNotFound {
                    section: "colors",
                    name: "Statu".into(),
                },
                ConfigIssue::OptionNotFound {
                    field: "Status".into(),
                    value: "Don done".into(),
                },
            ],
        };
        let msg = err.to_string();
        assert!(msg.contains("2 config issues"));
        assert!(msg.contains("Statu"));
        assert!(msg.contains("Don done"));
    }

    #[test]
    fn project_not_found_display_includes_owner_kind() {
        let err = GitHubError::ProjectNotFound {
            owner: "rust-lang".into(),
            number: 42,
            owner_kind: OwnerKind::Organization,
        };
        let msg = err.to_string();
        assert!(msg.contains("organization"));
        assert!(msg.contains("rust-lang"));
        assert!(msg.contains("42"));
    }

    #[test]
    fn network_error_kind_display() {
        assert_eq!(NetworkErrorKind::Timeout.to_string(), "timeout");
        assert_eq!(
            NetworkErrorKind::Connection.to_string(),
            "connection refused"
        );
        assert_eq!(
            NetworkErrorKind::Other("custom error".to_string()).to_string(),
            "custom error"
        );
    }
}
