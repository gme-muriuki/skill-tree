//! Integration tests for `fetch_project_items` and
//! `resolve_sub_issue_overflow` against a mock GitHub GraphQL endpoint.
//! Covers single-page and multi-page item fetches and the sub-issue
//! overflow follow-up query.

use std::time::Duration;

use serde_json::json;
use skill_tree::github::projects::{ItemContent, fetch_project_items, resolve_sub_issue_overflow};
use skill_tree_testlib::MockGitHub;

#[tokio::test]
async fn empty_project_returns_no_items() {
    let gh = MockGitHub::start().await;
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
    let items = fetch_project_items(&client, "PVT_1").await.unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn single_page_returns_items_in_order() {
    let gh = MockGitHub::start().await;
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
                            "title": "First",
                            "url": "https://github.com/o/r/issues/1",
                            "state": "OPEN",
                            "body": "",
                            "repository": { "nameWithOwner": "o/r" },
                            "assignees": { "nodes": [] },
                            "subIssues": {
                                "nodes": [],
                                "pageInfo": { "hasNextPage": false, "endCursor": null }
                            }
                        }
                    },
                    {
                        "id": "PVTI_2",
                        "fieldValues": { "nodes": [] },
                        "content": {
                            "__typename": "Issue",
                            "id": "I_2",
                            "number": 2,
                            "title": "Second",
                            "url": "https://github.com/o/r/issues/2",
                            "state": "OPEN",
                            "body": "",
                            "repository": { "nameWithOwner": "o/r" },
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
    let items = fetch_project_items(&client, "PVT_1").await.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, "PVTI_1");
    assert_eq!(items[1].id, "PVTI_2");
}

#[tokio::test]
async fn two_pages_stitch_in_order() {
    let gh = MockGitHub::start().await;

    // Page 1: one item, hasNextPage true.
    gh.ok_data(json!({
        "node": {
            "items": {
                "nodes": [
                    {
                        "id": "PVTI_p1",
                        "fieldValues": { "nodes": [] },
                        "content": {
                            "__typename": "DraftIssue",
                            "id": "DI_p1",
                            "title": "page-one draft",
                            "body": "",
                            "createdAt": "2026-04-01T12:00:00Z",
                            "assignees": { "nodes": [] }
                        }
                    }
                ],
                "pageInfo": { "hasNextPage": true, "endCursor": "cursor-page-1" }
            }
        }
    }))
    .up_to_n_times(1)
    .mount(&gh.server)
    .await;

    // Page 2: one item, hasNextPage false.
    gh.ok_data(json!({
        "node": {
            "items": {
                "nodes": [
                    {
                        "id": "PVTI_p2",
                        "fieldValues": { "nodes": [] },
                        "content": {
                            "__typename": "DraftIssue",
                            "id": "DI_p2",
                            "title": "page-two draft",
                            "body": "",
                            "createdAt": "2026-04-02T12:00:00Z",
                            "assignees": { "nodes": [] }
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
    let items = fetch_project_items(&client, "PVT_1").await.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, "PVTI_p1");
    assert_eq!(items[1].id, "PVTI_p2");
}

#[tokio::test]
async fn null_node_surfaces_invalid_response() {
    let gh = MockGitHub::start().await;
    gh.ok_data(json!({ "node": null })).mount(&gh.server).await;

    let client = gh.client(Duration::from_secs(10));
    let err = fetch_project_items(&client, "PVT_missing")
        .await
        .unwrap_err();

    use skill_tree::error::GitHubError;
    match err {
        GitHubError::InvalidResponse(msg) => {
            assert!(msg.contains("PVT_missing"), "message was: {msg}");
        }
        other => panic!("expected InvalidResponse, got {other:?}"),
    }
}

#[tokio::test]
async fn sub_issue_overflow_appends_remaining_and_clears_flag() {
    let gh = MockGitHub::start().await;

    // Items query: one issue with 2 inline sub-issues and hasNextPage true.
    gh.ok_data(json!({
        "node": {
            "items": {
                "nodes": [
                    {
                        "id": "PVTI_1",
                        "fieldValues": { "nodes": [] },
                        "content": {
                            "__typename": "Issue",
                            "id": "I_parent",
                            "number": 100,
                            "title": "Big tracking issue",
                            "url": "https://github.com/o/r/issues/100",
                            "state": "OPEN",
                            "body": "",
                            "repository": { "nameWithOwner": "o/r" },
                            "assignees": { "nodes": [] },
                            "subIssues": {
                                "nodes": [
                                    { "id": "I_c1", "number": 1, "title": "child 1", "url": "https://github.com/o/r/issues/1", "state": "OPEN", "repository": { "nameWithOwner": "o/r" } },
                                    { "id": "I_c2", "number": 2, "title": "child 2", "url": "https://github.com/o/r/issues/2", "state": "OPEN", "repository": { "nameWithOwner": "o/r" } }
                                ],
                                "pageInfo": { "hasNextPage": true, "endCursor": "cursor-after-2" }
                            }
                        }
                    }
                ],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    }))
    .up_to_n_times(1)
    .mount(&gh.server)
    .await;

    // Overflow query: one more child, hasNextPage false.
    gh.ok_data(json!({
        "node": {
            "subIssues": {
                "nodes": [
                    { "id": "I_c3", "number": 3, "title": "child 3", "url": "https://github.com/o/r/issues/3", "state": "OPEN", "repository": { "nameWithOwner": "o/r" } }
                ],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let mut items = fetch_project_items(&client, "PVT_1").await.unwrap();
    resolve_sub_issue_overflow(&client, &mut items)
        .await
        .unwrap();

    let ItemContent::Issue(ref issue) = items[0].content else {
        panic!("expected Issue content");
    };
    assert_eq!(issue.sub_issues.nodes.len(), 3);
    assert_eq!(issue.sub_issues.nodes[0].number, 1);
    assert_eq!(issue.sub_issues.nodes[1].number, 2);
    assert_eq!(issue.sub_issues.nodes[2].number, 3);
    assert!(!issue.sub_issues.page_info.has_next_page);
    assert!(issue.sub_issues.page_info.end_cursor.is_none());
}

#[tokio::test]
async fn sub_issue_overflow_errors_when_initial_cursor_missing() {
    let gh = MockGitHub::start().await;

    // Items query: one issue claiming has_next_page true but end_cursor null
    // (a malformed GitHub response — defends the invariant).
    gh.ok_data(json!({
        "node": {
            "items": {
                "nodes": [
                    {
                        "id": "PVTI_1",
                        "fieldValues": { "nodes": [] },
                        "content": {
                            "__typename": "Issue",
                            "id": "I_parent",
                            "number": 1,
                            "title": "Issue with broken pagination",
                            "url": "https://github.com/o/r/issues/1",
                            "state": "OPEN",
                            "body": "",
                            "repository": { "nameWithOwner": "o/r" },
                            "assignees": { "nodes": [] },
                            "subIssues": {
                                "nodes": [],
                                "pageInfo": { "hasNextPage": true, "endCursor": null }
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
    let mut items = fetch_project_items(&client, "PVT_1").await.unwrap();
    let err = resolve_sub_issue_overflow(&client, &mut items)
        .await
        .unwrap_err();

    use skill_tree::error::GitHubError;
    match err {
        GitHubError::InvalidResponse(msg) => {
            assert!(msg.contains("I_parent"), "message was: {msg}");
            assert!(msg.contains("end_cursor"), "message was: {msg}");
        }
        other => panic!("expected InvalidResponse, got {other:?}"),
    }
}

#[tokio::test]
async fn sub_issue_overflow_no_op_when_inline_complete() {
    let gh = MockGitHub::start().await;

    // Items query: one issue with sub-issues and hasNextPage already false.
    gh.ok_data(json!({
        "node": {
            "items": {
                "nodes": [
                    {
                        "id": "PVTI_1",
                        "fieldValues": { "nodes": [] },
                        "content": {
                            "__typename": "Issue",
                            "id": "I_parent",
                            "number": 100,
                            "title": "Small issue",
                            "url": "https://github.com/o/r/issues/100",
                            "state": "OPEN",
                            "body": "",
                            "repository": { "nameWithOwner": "o/r" },
                            "assignees": { "nodes": [] },
                            "subIssues": {
                                "nodes": [
                                    { "id": "I_c1", "number": 1, "title": "child 1", "url": "https://github.com/o/r/issues/1", "state": "OPEN", "repository": { "nameWithOwner": "o/r" } }
                                ],
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
    let mut items = fetch_project_items(&client, "PVT_1").await.unwrap();
    // No second mock mounted — if resolve_sub_issue_overflow tries to
    // hit the API it will fail, proving the no-op behavior.
    resolve_sub_issue_overflow(&client, &mut items)
        .await
        .unwrap();

    let ItemContent::Issue(ref issue) = items[0].content else {
        panic!("expected Issue content");
    };
    assert_eq!(issue.sub_issues.nodes.len(), 1);
    assert!(!issue.sub_issues.page_info.has_next_page);
}
