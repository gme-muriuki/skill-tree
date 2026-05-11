//! Integration tests for `fetch_issue_edges`. Covers the batched
//! `nodes(ids:)` pass, per-issue overflow drains on both connections,
//! and defensive handling of null / non-Issue nodes.

use std::time::Duration;

use serde_json::{Value, json};
use skill_tree::github::issues::{CrossReferenceSource, fetch_issue_edges};
use skill_tree_testlib::MockGitHub;

fn empty_connection() -> Value {
    json!({ "nodes": [], "pageInfo": { "hasNextPage": false, "endCursor": null } })
}

fn issue_node(id: &str, tracked: Value, timeline: Value) -> Value {
    json!({
        "__typename": "Issue",
        "id": id,
        "trackedIssues": tracked,
        "timelineItems": timeline,
    })
}

#[tokio::test]
async fn happy_path_one_batch_no_overflow() {
    let gh = MockGitHub::start().await;

    gh.ok_data(json!({
        "nodes": [
            issue_node("I_1", empty_connection(), empty_connection()),
            issue_node("I_2",
                json!({
                    "nodes": [{ "id": "I_99", "number": 99, "repository": { "nameWithOwner": "o/r" } }],
                    "pageInfo": { "hasNextPage": false, "endCursor": null }
                }),
                json!({
                    "nodes": [{
                        "source": {
                            "__typename": "PullRequest",
                            "id": "PR_7", "number": 7,
                            "repository": { "nameWithOwner": "o/r" },
                            "labels": { "nodes": [{ "name": "ship" }] }
                        }
                    }],
                    "pageInfo": { "hasNextPage": false, "endCursor": null }
                }),
            ),
        ]
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let result = fetch_issue_edges(&client, &["I_1".into(), "I_2".into()])
        .await
        .unwrap();

    assert_eq!(result.issues.len(), 2);
    assert!(result.issues[0].tracked_issues.nodes.is_empty());
    assert!(result.issues[0].timeline_items.nodes.is_empty());
    assert_eq!(result.issues[1].tracked_issues.nodes[0].number, 99);
    match &result.issues[1].timeline_items.nodes[0].source {
        CrossReferenceSource::PullRequest { number, labels, .. } => {
            assert_eq!(*number, 7);
            assert_eq!(labels.nodes[0].name, "ship");
        }
        other => panic!("expected PullRequest, got {other:?}"),
    }
}

#[tokio::test]
async fn batches_at_100_ids_per_request() {
    let gh = MockGitHub::start().await;

    let batch_nodes = |start: u32, end: u32| -> Value {
        let nodes: Vec<Value> = (start..end)
            .map(|i| issue_node(&format!("I_{i}"), empty_connection(), empty_connection()))
            .collect();
        json!({ "nodes": nodes })
    };

    // First batch: 100 issues.
    gh.ok_data(batch_nodes(0, 100))
        .up_to_n_times(1)
        .mount(&gh.server)
        .await;
    // Second batch: 50 issues.
    gh.ok_data(batch_nodes(100, 150)).mount(&gh.server).await;

    let client = gh.client(Duration::from_secs(10));
    let ids: Vec<String> = (0..150).map(|i| format!("I_{i}")).collect();
    let result = fetch_issue_edges(&client, &ids).await.unwrap();

    assert_eq!(result.issues.len(), 150);
    assert_eq!(result.issues[0].id, "I_0");
    assert_eq!(result.issues[149].id, "I_149");
}

#[tokio::test]
async fn tracked_issues_overflow_is_drained() {
    let gh = MockGitHub::start().await;

    // Inline page reports overflow.
    gh.ok_data(json!({
        "nodes": [issue_node(
            "I_1",
            json!({
                "nodes": [
                    { "id": "I_10", "number": 10, "repository": { "nameWithOwner": "o/r" } }
                ],
                "pageInfo": { "hasNextPage": true, "endCursor": "c1" }
            }),
            empty_connection(),
        )]
    }))
    .up_to_n_times(1)
    .mount(&gh.server)
    .await;

    // Overflow drain.
    gh.ok_data(json!({
        "node": {
            "trackedIssues": {
                "nodes": [
                    { "id": "I_11", "number": 11, "repository": { "nameWithOwner": "o/r" } },
                    { "id": "I_12", "number": 12, "repository": { "nameWithOwner": "o/r" } }
                ],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let result = fetch_issue_edges(&client, &["I_1".into()]).await.unwrap();

    let issue = &result.issues[0];
    let numbers: Vec<u64> = issue
        .tracked_issues
        .nodes
        .iter()
        .map(|t| t.number)
        .collect();
    assert_eq!(numbers, vec![10, 11, 12]);
    assert!(!issue.tracked_issues.page_info.has_next_page);
    assert!(issue.tracked_issues.page_info.end_cursor.is_none());
}

#[tokio::test]
async fn timeline_items_overflow_is_drained() {
    let gh = MockGitHub::start().await;

    let xref_node = |source_number: u64| -> Value {
        json!({
            "source": {
                "__typename": "Issue",
                "id": format!("I_{source_number}"),
                "number": source_number,
                "repository": { "nameWithOwner": "o/r" },
                "labels": { "nodes": [] }
            }
        })
    };

    gh.ok_data(json!({
        "nodes": [issue_node(
            "I_1",
            empty_connection(),
            json!({
                "nodes": [xref_node(20)],
                "pageInfo": { "hasNextPage": true, "endCursor": "c1" }
            }),
        )]
    }))
    .up_to_n_times(1)
    .mount(&gh.server)
    .await;

    gh.ok_data(json!({
        "node": {
            "timelineItems": {
                "nodes": [xref_node(21), xref_node(22)],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let result = fetch_issue_edges(&client, &["I_1".into()]).await.unwrap();

    let issue = &result.issues[0];
    assert_eq!(issue.timeline_items.nodes.len(), 3);
    assert!(!issue.timeline_items.page_info.has_next_page);
}

#[tokio::test]
async fn both_connections_overflow_on_same_issue() {
    let gh = MockGitHub::start().await;

    gh.ok_data(json!({
        "nodes": [issue_node(
            "I_1",
            json!({
                "nodes": [{ "id": "I_10", "number": 10, "repository": { "nameWithOwner": "o/r" } }],
                "pageInfo": { "hasNextPage": true, "endCursor": "c1" }
            }),
            json!({
                "nodes": [{
                    "source": {
                        "__typename": "Issue",
                        "id": "I_20", "number": 20,
                        "repository": { "nameWithOwner": "o/r" },
                        "labels": { "nodes": [] }
                    }
                }],
                "pageInfo": { "hasNextPage": true, "endCursor": "c2" }
            }),
        )]
    }))
    .up_to_n_times(1)
    .mount(&gh.server)
    .await;

    // Blocking overflow drains first (drain_blocking is called before drain_cross_refs).
    gh.ok_data(json!({
        "node": {
            "trackedIssues": {
                "nodes": [{ "id": "I_11", "number": 11, "repository": { "nameWithOwner": "o/r" } }],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    }))
    .up_to_n_times(1)
    .mount(&gh.server)
    .await;

    gh.ok_data(json!({
        "node": {
            "timelineItems": {
                "nodes": [{
                    "source": {
                        "__typename": "Issue",
                        "id": "I_21", "number": 21,
                        "repository": { "nameWithOwner": "o/r" },
                        "labels": { "nodes": [] }
                    }
                }],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let result = fetch_issue_edges(&client, &["I_1".into()]).await.unwrap();

    let issue = &result.issues[0];
    assert_eq!(issue.tracked_issues.nodes.len(), 2);
    assert_eq!(issue.timeline_items.nodes.len(), 2);
    assert!(!issue.tracked_issues.page_info.has_next_page);
    assert!(!issue.timeline_items.page_info.has_next_page);
}

#[tokio::test]
async fn null_and_non_issue_nodes_are_skipped() {
    let gh = MockGitHub::start().await;

    gh.ok_data(json!({
        "nodes": [
            issue_node("I_1", empty_connection(), empty_connection()),
            null,
            { "__typename": "Repository" },
            issue_node("I_2", empty_connection(), empty_connection()),
        ]
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let result = fetch_issue_edges(
        &client,
        &[
            "I_1".into(),
            "I_missing".into(),
            "I_wrongtype".into(),
            "I_2".into(),
        ],
    )
    .await
    .unwrap();

    let ids: Vec<&str> = result.issues.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["I_1", "I_2"]);
}

#[tokio::test]
async fn empty_input_makes_no_requests() {
    let gh = MockGitHub::start().await;
    let client = gh.client(Duration::from_secs(10));
    let result = fetch_issue_edges(&client, &[]).await.unwrap();
    assert!(result.issues.is_empty());
}
