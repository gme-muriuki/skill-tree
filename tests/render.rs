//! Integration tests for the `render` subcommand orchestrator.
//!
//! Drives [`skill_tree::cli::render::render_to_bytes`] against a mocked
//! GitHub endpoint and asserts on the produced DOT bytes. The SVG path
//! is not covered here — it shells out to the system `dot` binary which
//! may not be present in CI.

use std::time::Duration;

use serde_json::{Value, json};
use skill_tree::cli::render::{Format, RenderArgs, render_to_bytes};
use skill_tree::config::Config;
use skill_tree::error::CliError;
use skill_tree_testlib::MockGitHub;

fn parse_config(toml: &str) -> Config {
    toml::from_str(toml).expect("test TOML should parse")
}

fn empty_connection() -> Value {
    json!({ "nodes": [], "pageInfo": { "hasNextPage": false, "endCursor": null } })
}

fn metadata_with_status_field() -> Value {
    json!({
        "organization": {
            "projectV2": {
                "id": "PVT_1",
                "title": "demo project",
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
    })
}

fn base_config() -> Config {
    parse_config(
        r##"
            [github]
            owner   = "o"
            project = 1

            [colors]
            github-name = "Status"

            [colors.values]
            "Done" = "#57a85a"
        "##,
    )
}

#[tokio::test]
async fn empty_project_renders_empty_dot_graph() {
    let gh = MockGitHub::start().await;

    gh.ok_data(metadata_with_status_field())
        .up_to_n_times(1)
        .mount(&gh.server)
        .await;

    // Items query — zero items, no overflow.
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

    // No issue-edges call: with zero issue IDs, `fetch_issue_edges` makes
    // no network requests.

    let client = gh.client(Duration::from_secs(10));
    let config = base_config();
    let args = RenderArgs {
        format: Some(Format::Dot),
        ..Default::default()
    };

    let bytes = render_to_bytes(&client, &config, &args).await.unwrap();
    let dot = String::from_utf8(bytes).expect("DOT is UTF-8");

    assert!(dot.contains("digraph SkillTree"), "DOT body: {dot}");
    assert!(dot.contains("rankdir = \"LR\""));
    // No issue node identifiers should appear.
    assert!(!dot.contains("\"o/r#"));
}

#[tokio::test]
async fn small_project_renders_nodes_and_sub_issue_edge() {
    let gh = MockGitHub::start().await;

    gh.ok_data(metadata_with_status_field())
        .up_to_n_times(1)
        .mount(&gh.server)
        .await;

    // Two on-board issues: #1 has #2 as a sub-issue (so #2 → #1 is the
    // resulting edge per the child-parent direction in graph-build.md).
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
                            "title": "Parent",
                            "url": "https://github.com/o/r/issues/1",
                            "state": "OPEN",
                            "body": "",
                            "repository": { "nameWithOwner": "o/r" },
                            "assignees": { "nodes": [] },
                            "subIssues": {
                                "nodes": [
                                    {
                                        "id": "I_2",
                                        "number": 2,
                                        "title": "Child",
                                        "url": "https://github.com/o/r/issues/2",
                                        "state": "OPEN",
                                        "repository": { "nameWithOwner": "o/r" }
                                    }
                                ],
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
                            "title": "Child",
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
    .up_to_n_times(1)
    .mount(&gh.server)
    .await;

    // Issue-edges batch: both issues, no blocking / cross-ref edges.
    gh.ok_data(json!({
        "nodes": [
            {
                "__typename": "Issue",
                "id": "I_1",
                "trackedIssues": empty_connection(),
                "timelineItems": empty_connection()
            },
            {
                "__typename": "Issue",
                "id": "I_2",
                "trackedIssues": empty_connection(),
                "timelineItems": empty_connection()
            }
        ]
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let config = base_config();
    let args = RenderArgs {
        format: Some(Format::Dot),
        ..Default::default()
    };

    let bytes = render_to_bytes(&client, &config, &args).await.unwrap();
    let dot = String::from_utf8(bytes).expect("DOT is UTF-8");

    // Both nodes appear, quoted by their `<owner>/<repo>#<number>` id.
    assert!(dot.contains("\"o/r#1\""), "expected #1 node: {dot}");
    assert!(dot.contains("\"o/r#2\""), "expected #2 node: {dot}");

    // Sub-issue edge: child #2 → parent #1.
    assert!(
        dot.contains("\"o/r#2\" -> \"o/r#1\""),
        "expected sub-issue edge: {dot}"
    );
}

#[tokio::test]
async fn cycle_between_two_issues_surfaces_as_cli_error_cycle() {
    let gh = MockGitHub::start().await;

    gh.ok_data(metadata_with_status_field())
        .up_to_n_times(1)
        .mount(&gh.server)
        .await;

    // Two on-board issues, no sub-issue relationship — the cycle comes
    // from blocking edges below.
    let bare_issue = |id: &str, number: u64, title: &str| {
        json!({
            "id": format!("PVTI_{id}"),
            "fieldValues": { "nodes": [] },
            "content": {
                "__typename": "Issue",
                "id": id,
                "number": number,
                "title": title,
                "url": format!("https://github.com/o/r/issues/{number}"),
                "state": "OPEN",
                "body": "",
                "repository": { "nameWithOwner": "o/r" },
                "assignees": { "nodes": [] },
                "subIssues": empty_connection()
            }
        })
    };
    gh.ok_data(json!({
        "node": {
            "items": {
                "nodes": [
                    bare_issue("I_1", 1, "Alpha"),
                    bare_issue("I_2", 2, "Beta")
                ],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            }
        }
    }))
    .up_to_n_times(1)
    .mount(&gh.server)
    .await;

    // Each issue lists the other as a tracked-issue (blocker).
    // EdgeKind::Blocks direction is `blocker → blocked`, derived from
    // `Issue.trackedIssues`. So #1.tracked = [#2] yields the edge
    // #2 → #1, and #2.tracked = [#1] yields #1 → #2 — a 2-cycle.
    let tracked_by = |target_id: &str, number: u64| {
        json!({
            "nodes": [{ "id": target_id, "number": number, "repository": { "nameWithOwner": "o/r" } }],
            "pageInfo": { "hasNextPage": false, "endCursor": null }
        })
    };
    gh.ok_data(json!({
        "nodes": [
            {
                "__typename": "Issue",
                "id": "I_1",
                "trackedIssues": tracked_by("I_2", 2),
                "timelineItems": empty_connection()
            },
            {
                "__typename": "Issue",
                "id": "I_2",
                "trackedIssues": tracked_by("I_1", 1),
                "timelineItems": empty_connection()
            }
        ]
    }))
    .mount(&gh.server)
    .await;

    let client = gh.client(Duration::from_secs(10));
    let config = base_config();
    let args = RenderArgs {
        format: Some(Format::Dot),
        ..Default::default()
    };

    let err = render_to_bytes(&client, &config, &args)
        .await
        .expect_err("a 2-cycle should surface as CliError::Cycle");

    assert_eq!(
        err.exit_code(),
        1,
        "cycle errors share the unrenderable-output exit code"
    );
    match err {
        CliError::Cycle(report) => {
            let rendered = report.to_string();
            assert!(rendered.contains("o/r#1"), "cycle text: {rendered}");
            assert!(rendered.contains("o/r#2"), "cycle text: {rendered}");
        }
        other => panic!("expected CliError::Cycle, got {other:?}"),
    }
}
