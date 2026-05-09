//! Integration tests for the public `fetch_project` entry point. Covers
//! the happy path end-to-end and the fail-fast behavior of config
//! validation: a typo in `.skill-tree.toml` errors after the cheap
//! metadata query and never issues the paginated items query.

use std::time::Duration;

use serde_json::json;
use skill_tree::config::Config;
use skill_tree::error::GitHubError;
use skill_tree::github::projects::{ItemContent, fetch_project};
use skill_tree_testlib::MockGitHub;

fn parse_config(toml: &str) -> Config {
    toml::from_str(toml).expect("test TOML should be valid")
}

#[tokio::test]
async fn happy_path_fetches_meta_and_items() {
    let gh = MockGitHub::start().await;

    // Metadata response.
    gh.ok_data(json!({
        "organization": {
            "projectV2": {
                "id": "PVT_1",
                "title": "rust-lang skill tree",
                "fields": {
                    "nodes": [
                        {
                            "__typename": "ProjectV2SingleSelectField",
                            "id": "F_status",
                            "name": "Status",
                            "options": [
                                { "id": "o1", "name": "Done" },
                                { "id": "o2", "name": "In Progress" }
                            ]
                        }
                    ]
                }
            }
        },
        "user": null
    }))
    .up_to_n_times(1)
    .mount(&gh.server)
    .await;

    // Items response — one issue, no sub-issue overflow.
    gh.ok_data(json!({
        "node": {
            "items": {
                "nodes": [
                    {
                        "id": "PVTI_1",
                        "fieldValues": { "nodes": [] },
                        "content": {
                            "__typename": "Issue",
                            "id": "I_1",
                            "number": 1,
                            "title": "First issue",
                            "url": "https://github.com/rust-lang/r/issues/1",
                            "state": "OPEN",
                            "body": "",
                            "repository": { "nameWithOwner": "rust-lang/r" },
                            "assignees": { "nodes": [] },
                            "subIssues": {
                                "nodes": [],
                                "pageInfo": { "hasNextPage": false, "endCursor": null }
                            }
                        }
                    }
                ],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let config = parse_config(
        r##"
            [github]
            owner   = "rust-lang"
            project = 42

            [colors]
            github-name = "Status"

            [colors.values]
            "Done" = "#57a85a"
        "##,
    );

    let fetch = fetch_project(&client, &config).await.unwrap();
    assert_eq!(fetch.meta.id, "PVT_1");
    assert_eq!(fetch.meta.title, "rust-lang skill tree");
    assert_eq!(fetch.items.len(), 1);
    let ItemContent::Issue(ref issue) = fetch.items[0].content else {
        panic!("expected Issue");
    };
    assert_eq!(issue.number, 1);
}

#[tokio::test]
async fn config_typo_fails_fast_after_metadata_before_items() {
    let gh = MockGitHub::start().await;

    // Only the metadata mock is mounted. If validation fails fast (as it
    // should), we never call the items endpoint. If we accidentally do,
    // the request hits no matcher and the test fails on a transport-level
    // error — proving the items query is reachable but never reached.
    gh.ok_data(json!({
        "organization": {
            "projectV2": {
                "id": "PVT_1",
                "title": "rust-lang skill tree",
                "fields": {
                    "nodes": [
                        {
                            "__typename": "ProjectV2SingleSelectField",
                            "id": "F_status",
                            "name": "Status",
                            "options": [
                                { "id": "o1", "name": "Done" }
                            ]
                        }
                    ]
                }
            }
        },
        "user": null
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let config = parse_config(
        r##"
            [github]
            owner   = "rust-lang"
            project = 42

            [colors]
            github-name = "Statu"
        "##,
    );

    let err = fetch_project(&client, &config).await.unwrap_err();
    match err {
        GitHubError::ConfigMismatch { issues } => {
            assert_eq!(issues.len(), 1);
        }
        other => panic!("expected ConfigMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn bare_project_only_config_succeeds() {
    let gh = MockGitHub::start().await;

    gh.ok_data(json!({
        "organization": {
            "projectV2": {
                "id": "PVT_1",
                "title": "Bare project",
                "fields": { "nodes": [] }
            }
        },
        "user": null
    }))
    .up_to_n_times(1)
    .mount(&gh.server)
    .await;

    gh.ok_data(json!({
        "node": {
            "items": {
                "nodes": [],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let config = parse_config(
        r##"
            [github]
            owner   = "rust-lang"
            project = 42
        "##,
    );

    let fetch = fetch_project(&client, &config).await.unwrap();
    assert_eq!(fetch.meta.title, "Bare project");
    assert!(fetch.items.is_empty());
}

#[tokio::test]
async fn config_mismatch_carries_all_issues_at_once() {
    let gh = MockGitHub::start().await;

    gh.ok_data(json!({
        "organization": {
            "projectV2": {
                "id": "PVT_1",
                "title": "Project",
                "fields": {
                    "nodes": [
                        {
                            "__typename": "ProjectV2SingleSelectField",
                            "id": "F_status",
                            "name": "Status",
                            "options": [
                                { "id": "o1", "name": "Done" }
                            ]
                        }
                    ]
                }
            }
        },
        "user": null
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let config = parse_config(
        r##"
            [github]
            owner   = "rust-lang"
            project = 42

            [[field]]
            display-name = "prio"
            github-name  = "Priorityy"

            [colors]
            github-name = "Status"

            [colors.values]
            "Don done"  = "#e05252"
            "Off Track" = "#4a90d9"
        "##,
    );

    let err = fetch_project(&client, &config).await.unwrap_err();
    match err {
        GitHubError::ConfigMismatch { issues } => {
            // 1 missing field declaration + 2 unknown option values = 3
            assert_eq!(issues.len(), 3, "got: {issues:#?}");
        }
        other => panic!("expected ConfigMismatch, got {other:?}"),
    }
}
