//! Integration tests for the `embed` subcommand orchestrator.
//!
//! Drives [`skill_tree::cli::embed::embed_to_html`] against a mocked
//! GitHub endpoint. `embed` always shells out to `dot -Tsvg`, so these
//! tests skip (with a printed note) when the `dot` binary is absent,
//! matching the unit tests in `src/render/svg.rs`.

use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use skill_tree::cli::embed::{EmbedArgs, embed_to_html};
use skill_tree::config::Config;
use skill_tree_testlib::MockGitHub;

fn dot_available() -> bool {
    Command::new("dot")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

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

/// One on-board issue with a markdown body, so the produced HTML carries
/// a data record and rendered body.
async fn mount_single_issue(gh: &MockGitHub) {
    gh.ok_data(metadata_with_status_field())
        .up_to_n_times(1)
        .mount(&gh.server)
        .await;

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
                            "title": "Parser rewrite",
                            "url": "https://github.com/o/r/issues/1",
                            "state": "OPEN",
                            "body": "Needs **work** before merge.",
                            "repository": { "nameWithOwner": "o/r" },
                            "assignees": { "nodes": [ { "login": "octocat" } ] },
                            "subIssues": empty_connection()
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

    gh.ok_data(json!({
        "nodes": [
            {
                "__typename": "Issue",
                "id": "I_1",
                "trackedIssues": empty_connection(),
                "timelineItems": empty_connection()
            }
        ]
    }))
    .mount(&gh.server)
    .await;
}

#[tokio::test]
async fn standalone_embed_wraps_svg_with_data_and_panel() {
    if !dot_available() {
        println!("skipping: `dot` not on PATH");
        return;
    }

    let gh = MockGitHub::start().await;
    mount_single_issue(&gh).await;

    let client = gh.client(Duration::from_secs(10));
    let args = EmbedArgs::default();
    let html = embed_to_html(&client, &base_config(), &args)
        .await
        .expect("embed should produce HTML");

    // Full document with the standalone chrome.
    assert!(html.starts_with("<!doctype html>"), "not a full doc");
    assert!(html.contains("class=\"theme-dark\""));
    // The rendered SVG was inlined.
    assert!(html.contains("<svg"), "svg not inlined");
    // The issue is in the embedded data map, with rendered body markdown.
    // (Closing tags appear as `<\/strong>` in the JSON — `</` is escaped so
    // the embedded data cannot close the <script> early — so match the
    // opening tag, which is left intact.)
    assert!(html.contains("o/r#1"), "issue id missing from data");
    assert!(html.contains("<strong>work"), "body markdown not rendered");
    // The page title comes from the project metadata.
    assert!(html.contains("demo project"));
    // No template sentinels survive.
    assert!(!html.contains("__SVG__") && !html.contains("__DATA__"));
}

#[tokio::test]
async fn fragment_embed_is_a_scoped_div() {
    if !dot_available() {
        println!("skipping: `dot` not on PATH");
        return;
    }

    let gh = MockGitHub::start().await;
    mount_single_issue(&gh).await;

    let client = gh.client(Duration::from_secs(10));
    let args = EmbedArgs {
        fragment: true,
        ..Default::default()
    };
    let html = embed_to_html(&client, &base_config(), &args)
        .await
        .expect("embed should produce HTML");

    assert!(
        html.trim_start()
            .starts_with("<div class=\"st-widget st-embed"),
        "fragment should be a scoped div, got: {}",
        &html[..html.len().min(80)]
    );
    assert!(
        !html.contains("<!doctype"),
        "fragment must not be a full doc"
    );
    assert!(html.contains("<svg"), "svg not inlined");
}
