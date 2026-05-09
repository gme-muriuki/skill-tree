//! Integration tests for `fetch_project_meta` against a mock GitHub
//! GraphQL endpoint. Covers owner-kind detection (organization vs user),
//! the both-null `OwnerUnreachable` case, and the owner-found-but-no-
//! project `ProjectNotFound` case.

use std::time::Duration;

use serde_json::json;
use skill_tree::error::GitHubError;
use skill_tree::github::projects::{FieldKind, OwnerKind, fetch_project_meta};
use skill_tree_testlib::MockGitHub;

fn project_meta_fixture() -> serde_json::Value {
    json!({
        "id": "PVT_1",
        "title": "rust-lang skill tree",
        "fields": {
            "nodes": [
                {
                    "__typename": "ProjectV2SingleSelectField",
                    "id": "F_status",
                    "name": "Status",
                    "options": [
                        { "id": "o_done",        "name": "Done" },
                        { "id": "o_inprogress",  "name": "In progress" }
                    ]
                }
            ]
        }
    })
}

#[tokio::test]
async fn org_with_project_returns_meta_with_organization_kind() {
    let gh = MockGitHub::start().await;
    gh.ok_data(json!({
        "organization": { "projectV2": project_meta_fixture() },
        "user": null
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let meta = fetch_project_meta(&client, "rust-lang", 42).await.unwrap();

    assert_eq!(meta.id, "PVT_1");
    assert_eq!(meta.title, "rust-lang skill tree");
    assert_eq!(meta.owner_kind, OwnerKind::Organization);
    assert!(matches!(
        meta.field_by_name("Status").unwrap().kind,
        FieldKind::SingleSelect { .. }
    ));
}

#[tokio::test]
async fn user_with_project_returns_meta_with_user_kind() {
    let gh = MockGitHub::start().await;
    gh.ok_data(json!({
        "organization": null,
        "user": { "projectV2": project_meta_fixture() }
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let meta = fetch_project_meta(&client, "nikomatsakis", 7)
        .await
        .unwrap();

    assert_eq!(meta.owner_kind, OwnerKind::User);
}

#[tokio::test]
async fn both_null_returns_owner_unreachable() {
    let gh = MockGitHub::start().await;
    gh.ok_data(json!({
        "organization": null,
        "user": null
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let err = fetch_project_meta(&client, "rust-lan", 42)
        .await
        .unwrap_err();

    match err {
        GitHubError::OwnerUnreachable { owner } => assert_eq!(owner, "rust-lan"),
        other => panic!("expected OwnerUnreachable, got {other:?}"),
    }
}

#[tokio::test]
async fn org_without_project_returns_project_not_found_with_org_kind() {
    let gh = MockGitHub::start().await;
    gh.ok_data(json!({
        "organization": { "projectV2": null },
        "user": null
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let err = fetch_project_meta(&client, "rust-lang", 9999)
        .await
        .unwrap_err();

    match err {
        GitHubError::ProjectNotFound {
            owner,
            number,
            owner_kind,
        } => {
            assert_eq!(owner, "rust-lang");
            assert_eq!(number, 9999);
            assert_eq!(owner_kind, OwnerKind::Organization);
        }
        other => panic!("expected ProjectNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn user_without_project_returns_project_not_found_with_user_kind() {
    let gh = MockGitHub::start().await;
    gh.ok_data(json!({
        "organization": null,
        "user": { "projectV2": null }
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let err = fetch_project_meta(&client, "nikomatsakis", 9999)
        .await
        .unwrap_err();

    match err {
        GitHubError::ProjectNotFound { owner_kind, .. } => {
            assert_eq!(owner_kind, OwnerKind::User);
        }
        other => panic!("expected ProjectNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn meta_carries_text_number_date_and_iteration_field_kinds() {
    let gh = MockGitHub::start().await;
    gh.ok_data(json!({
        "organization": {
            "projectV2": {
                "id": "PVT_1",
                "title": "Mixed fields",
                "fields": {
                    "nodes": [
                        { "__typename": "ProjectV2Field", "id": "f1", "name": "Notes",    "dataType": "TEXT" },
                        { "__typename": "ProjectV2Field", "id": "f2", "name": "Priority", "dataType": "NUMBER" },
                        { "__typename": "ProjectV2Field", "id": "f3", "name": "Due",      "dataType": "DATE" },
                        {
                            "__typename": "ProjectV2IterationField",
                            "id": "f4",
                            "name": "Sprint",
                            "configuration": {
                                "iterations": [
                                    { "id": "i1", "title": "Sprint 1", "startDate": "2026-05-01", "duration": 14 }
                                ]
                            }
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
    let meta = fetch_project_meta(&client, "rust-lang", 42).await.unwrap();

    assert!(matches!(meta.field_by_name("Notes").unwrap().kind, FieldKind::Text));
    assert!(matches!(meta.field_by_name("Priority").unwrap().kind, FieldKind::Number));
    assert!(matches!(meta.field_by_name("Due").unwrap().kind, FieldKind::Date));

    let FieldKind::Iteration { iterations } = &meta.field_by_name("Sprint").unwrap().kind else {
        panic!("expected Iteration field");
    };
    assert_eq!(iterations.len(), 1);
    assert_eq!(iterations[0].title, "Sprint 1");
    assert_eq!(iterations[0].duration, 14);
}
