//! Issue-level edge fetching: blocking dependencies and cross-references.
//!
//! See `md/design/issue-edges.md` for the design.

use crate::github::Connection;
use crate::github::projects::{NodeList, RepositoryRef};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// RawIssueEdges
// ---------------------------------------------------------------------------

/// The full result of an issue-edge fetch: one record per Issue passed in.
/// After [`fetch_issue_edges`], both inner connections on every record are
/// fully drained — `page_info.has_next_page` is `false`.
#[derive(Debug, Clone)]
pub struct RawIssueEdges {
    pub issues: Vec<IssueEdgeRecord>,
}

/// One issue's worth of raw edge data, as returned by GitHub.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueEdgeRecord {
    /// GitHub node id of the source issue.
    pub id: String,
    /// Blocking targets — issues this one tracks as dependencies.
    pub tracked_issues: Connection<BlockingTarget>,
    /// Cross-reference events on this issue's timeline. The query filters
    /// to `CROSS_REFERENCED_EVENT` only.
    pub timeline_items: Connection<CrossReferenceEvent>,
}

// ---------------------------------------------------------------------------
// Blocking
// ---------------------------------------------------------------------------

/// A target of a blocking edge: an issue that the source issue depends on.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockingTarget {
    pub id: String,
    pub number: u64,
    pub repository: RepositoryRef,
}

// ---------------------------------------------------------------------------
// Cross-references
// ---------------------------------------------------------------------------
//
// `CrossReferencedEvent.source` is a `ReferencedSubject` union of `Issue`
// and `PullRequest`. We keep them as distinct variants on
// `CrossReferenceSource` because the graph layer treats them uniformly
// (both share the `<owner>/<repo>#<number>` identity namespace), but the
// `__typename` is preserved for diagnostics. `Unknown` is defensive
// forward-compat: the schema does not currently produce other variants.

/// A `CROSS_REFERENCED_EVENT` on the target issue's timeline. Carries the
/// source (the issue or PR that mentioned the target) with labels inlined
/// so the `[edges.cross-ref]` require-label filter needs no third pass.
#[derive(Debug, Clone, Deserialize)]
pub struct CrossReferenceEvent {
    pub source: CrossReferenceSource,
}

/// The mentioning side of a cross-reference.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "__typename")]
pub enum CrossReferenceSource {
    Issue {
        id: String,
        number: u64,
        repository: RepositoryRef,
        labels: NodeList<Label>,
    },
    PullRequest {
        id: String,
        number: u64,
        repository: RepositoryRef,
        labels: NodeList<Label>,
    },
    /// Any `__typename` the schema may introduce that skill-tree does not
    /// model. The graph layer treats this as "drop silently".
    #[serde(other)]
    Unknown,
}

/// One label on the source issue or PR of a cross-reference.
#[derive(Debug, Clone, Deserialize)]
pub struct Label {
    pub name: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    // -- BlockingTarget --

    #[test]
    fn blocking_target_deserializes() {
        let json = indoc! {r#"
            {
                "id": "I_42",
                "number": 42,
                "repository": { "nameWithOwner": "o/r" }
            }
        "#};
        let v: BlockingTarget = serde_json::from_str(json).unwrap();
        assert_eq!(v.id, "I_42");
        assert_eq!(v.number, 42);
        assert_eq!(v.repository.name_with_owner, "o/r");
    }

    // -- CrossReferenceSource --

    #[test]
    fn cross_reference_source_issue_deserializes() {
        let json = indoc! {r#"
            {
                "__typename": "Issue",
                "id": "I_7",
                "number": 7,
                "repository": { "nameWithOwner": "o/r" },
                "labels": { "nodes": [{ "name": "tracking" }, { "name": "epic" }] }
            }
        "#};
        let v: CrossReferenceSource = serde_json::from_str(json).unwrap();
        let CrossReferenceSource::Issue {
            id,
            number,
            repository,
            labels,
        } = v
        else {
            panic!("expected Issue, got {v:?}");
        };
        assert_eq!(id, "I_7");
        assert_eq!(number, 7);
        assert_eq!(repository.name_with_owner, "o/r");
        assert_eq!(labels.nodes.len(), 2);
        assert_eq!(labels.nodes[0].name, "tracking");
        assert_eq!(labels.nodes[1].name, "epic");
    }

    #[test]
    fn cross_reference_source_pull_request_deserializes() {
        let json = indoc! {r#"
            {
                "__typename": "PullRequest",
                "id": "PR_3",
                "number": 3,
                "repository": { "nameWithOwner": "o/r" },
                "labels": { "nodes": [] }
            }
        "#};
        let v: CrossReferenceSource = serde_json::from_str(json).unwrap();
        let CrossReferenceSource::PullRequest {
            id, number, labels, ..
        } = v
        else {
            panic!("expected PullRequest, got {v:?}");
        };
        assert_eq!(id, "PR_3");
        assert_eq!(number, 3);
        assert!(labels.nodes.is_empty());
    }

    #[test]
    fn cross_reference_source_unknown_catches_new_typename() {
        let json = indoc! {r#"
            {
                "__typename": "FutureSourceType",
                "id": "X_1"
            }
        "#};
        let v: CrossReferenceSource = serde_json::from_str(json).unwrap();
        assert!(matches!(v, CrossReferenceSource::Unknown));
    }

    // -- CrossReferenceEvent --

    #[test]
    fn cross_reference_event_wraps_source() {
        let json = indoc! {r#"
            {
                "source": {
                    "__typename": "Issue",
                    "id": "I_9",
                    "number": 9,
                    "repository": { "nameWithOwner": "o/r" },
                    "labels": { "nodes": [] }
                }
            }
        "#};
        let v: CrossReferenceEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(v.source, CrossReferenceSource::Issue { .. }));
    }

    // -- IssueEdgeRecord --

    #[test]
    fn issue_edge_record_empty_connections_deserialize() {
        let json = indoc! {r#"
            {
                "id": "I_1",
                "trackedIssues": {
                    "nodes": [],
                    "pageInfo": { "hasNextPage": false, "endCursor": null }
                },
                "timelineItems": {
                    "nodes": [],
                    "pageInfo": { "hasNextPage": false, "endCursor": null }
                }
            }
        "#};
        let v: IssueEdgeRecord = serde_json::from_str(json).unwrap();
        assert_eq!(v.id, "I_1");
        assert!(v.tracked_issues.nodes.is_empty());
        assert!(!v.tracked_issues.page_info.has_next_page);
        assert!(v.timeline_items.nodes.is_empty());
    }

    #[test]
    fn issue_edge_record_populated_connections_deserialize() {
        let json = indoc! {r#"
            {
                "id": "I_1",
                "trackedIssues": {
                    "nodes": [
                        {
                            "id": "I_2",
                            "number": 2,
                            "repository": { "nameWithOwner": "o/r" }
                        }
                    ],
                    "pageInfo": { "hasNextPage": false, "endCursor": "c1" }
                },
                "timelineItems": {
                    "nodes": [
                        {
                            "source": {
                                "__typename": "PullRequest",
                                "id": "PR_5",
                                "number": 5,
                                "repository": { "nameWithOwner": "o/r" },
                                "labels": { "nodes": [{ "name": "ship" }] }
                            }
                        }
                    ],
                    "pageInfo": { "hasNextPage": true, "endCursor": "c2" }
                }
            }
        "#};
        let v: IssueEdgeRecord = serde_json::from_str(json).unwrap();
        assert_eq!(v.tracked_issues.nodes.len(), 1);
        assert_eq!(v.tracked_issues.nodes[0].number, 2);
        assert_eq!(v.timeline_items.nodes.len(), 1);
        assert!(v.timeline_items.page_info.has_next_page);
        assert_eq!(v.timeline_items.page_info.end_cursor.as_deref(), Some("c2"));
        match &v.timeline_items.nodes[0].source {
            CrossReferenceSource::PullRequest { number, labels, .. } => {
                assert_eq!(*number, 5);
                assert_eq!(labels.nodes[0].name, "ship");
            }
            other => panic!("expected PullRequest, got {other:?}"),
        }
    }
}
