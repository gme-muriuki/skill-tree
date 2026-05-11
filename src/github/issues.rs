//! Issue-level edge fetching: blocking dependencies and cross-references.
//!
//! See `md/design/issue-edges.md` for the design.

use crate::error::GitHubError;
use crate::github::projects::{NodeList, RepositoryRef};
use crate::github::{Connection, GitHubClient};
use serde::{Deserialize, Serialize};

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
// fetch_issue_edges
// ---------------------------------------------------------------------------
//
// Two query documents back the second pass: a batched `nodes(ids:)` for
// the inline first-50 pages, and two per-issue overflow queries used when
// either inline connection reports `hasNextPage`. Splitting overflow into
// two queries keeps each focused on one connection — an issue that
// overflows only on trackedIssues does not pay for a wasted page on
// timelineItems.

const FETCH_ISSUE_EDGES_QUERY: &str = r#"
    query FetchIssueEdges($ids: [ID!]!) {
        nodes(ids: $ids) {
            __typename
            ... on Issue {
                id
                trackedIssues(first: 50) {
                    nodes {
                        id
                        number
                        repository { nameWithOwner }
                    }
                    pageInfo { hasNextPage endCursor }
                }
                timelineItems(itemTypes: CROSS_REFERENCED_EVENT, first: 50) {
                    nodes {
                        ... on CrossReferencedEvent {
                            source {
                                __typename
                                ... on Issue {
                                    id
                                    number
                                    repository { nameWithOwner }
                                    labels(first: 20) { nodes { name } }
                                }
                                ... on PullRequest {
                                    id
                                    number
                                    repository { nameWithOwner }
                                    labels(first: 20) { nodes { name } }
                                }
                            }
                        }
                    }
                    pageInfo { hasNextPage endCursor }
                }
            }
        }
    }
"#;

/// IDs per `nodes(ids:)` batch. GitHub's hard cap is 100.
const IDS_PER_BATCH: usize = 100;

/// Per-page size for overflow queries. GitHub caps `first` at 100.
const OVERFLOW_PAGE: u32 = 100;

#[derive(Serialize)]
struct IssueEdgesVariables<'a> {
    ids: &'a [&'a str],
}

#[derive(Debug, Deserialize)]
struct FetchIssueEdgesResponse {
    nodes: Vec<Option<RawIssueNode>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum RawIssueNode {
    Issue(IssueEdgeRecord),
    /// A non-Issue node — should not occur given we only pass Issue IDs,
    /// but kept for forward-compat. Dropped at the assembly step.
    #[serde(other)]
    Other,
}

/// Fetch blocking + cross-reference edge data for every Issue passed in.
///
/// Batches `issue_ids` at 100 per request via `nodes(ids:)`. Inline pages
/// are first-50 on both `trackedIssues` and the cross-reference timeline;
/// any issue whose inline connection reports `hasNextPage` is drained by
/// per-issue overflow queries before this function returns. Non-Issue
/// nodes and `null` entries (deleted between item fetch and this call)
/// are silently skipped.
///
/// After the call, every `IssueEdgeRecord` in the returned `RawIssueEdges`
/// has `tracked_issues.nodes` and `timeline_items.nodes` as complete
/// lists with `page_info.has_next_page == false`. See
/// `md/design/issue-edges.md`.
pub async fn fetch_issue_edges(
    client: &GitHubClient,
    issue_ids: &[String],
) -> Result<RawIssueEdges, GitHubError> {
    let mut records: Vec<IssueEdgeRecord> = Vec::with_capacity(issue_ids.len());

    for chunk in issue_ids.chunks(IDS_PER_BATCH) {
        let ids: Vec<&str> = chunk.iter().map(String::as_str).collect();
        let response: FetchIssueEdgesResponse = client
            .query(FETCH_ISSUE_EDGES_QUERY, IssueEdgesVariables { ids: &ids })
            .await?;
        for node in response.nodes.into_iter().flatten() {
            if let RawIssueNode::Issue(record) = node {
                records.push(record);
            }
        }
    }

    resolve_overflow(client, &mut records).await?;

    Ok(RawIssueEdges { issues: records })
}

// ---------------------------------------------------------------------------
// Overflow
// ---------------------------------------------------------------------------

const FETCH_REMAINING_BLOCKING_QUERY: &str = r#"
    query FetchRemainingBlocking($issueId: ID!, $first: Int!, $after: String!) {
        node(id: $issueId) {
            ... on Issue {
                trackedIssues(first: $first, after: $after) {
                    nodes {
                        id
                        number
                        repository { nameWithOwner }
                    }
                    pageInfo { hasNextPage endCursor }
                }
            }
        }
    }
"#;

const FETCH_REMAINING_CROSS_REFS_QUERY: &str = r#"
    query FetchRemainingCrossRefs($issueId: ID!, $first: Int!, $after: String!) {
        node(id: $issueId) {
            ... on Issue {
                timelineItems(itemTypes: CROSS_REFERENCED_EVENT, first: $first, after: $after) {
                    nodes {
                        ... on CrossReferencedEvent {
                            source {
                                __typename
                                ... on Issue {
                                    id
                                    number
                                    repository { nameWithOwner }
                                    labels(first: 20) { nodes { name } }
                                }
                                ... on PullRequest {
                                    id
                                    number
                                    repository { nameWithOwner }
                                    labels(first: 20) { nodes { name } }
                                }
                            }
                        }
                    }
                    pageInfo { hasNextPage endCursor }
                }
            }
        }
    }
"#;

#[derive(Serialize)]
struct OverflowVariables<'a> {
    #[serde(rename = "issueId")]
    issue_id: &'a str,
    first: u32,
    after: &'a str,
}

#[derive(Debug, Deserialize)]
struct OverflowBlockingResponse {
    node: Option<BlockingOverflowNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockingOverflowNode {
    tracked_issues: Connection<BlockingTarget>,
}

#[derive(Debug, Deserialize)]
struct OverflowCrossRefsResponse {
    node: Option<CrossRefsOverflowNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CrossRefsOverflowNode {
    timeline_items: Connection<CrossReferenceEvent>,
}

async fn resolve_overflow(
    client: &GitHubClient,
    records: &mut [IssueEdgeRecord],
) -> Result<(), GitHubError> {
    for record in records.iter_mut() {
        if record.tracked_issues.page_info.has_next_page {
            drain_blocking_overflow(client, record).await?;
        }
        if record.timeline_items.page_info.has_next_page {
            drain_cross_refs_overflow(client, record).await?;
        }
    }
    Ok(())
}

async fn drain_blocking_overflow(
    client: &GitHubClient,
    record: &mut IssueEdgeRecord,
) -> Result<(), GitHubError> {
    let Some(initial_cursor) = record.tracked_issues.page_info.end_cursor.clone() else {
        return Err(GitHubError::InvalidResponse(format!(
            "issue '{}' inline trackedIssues claimed has_next_page but returned no end_cursor",
            record.id
        )));
    };
    let mut after = Some(initial_cursor);
    while let Some(cursor) = after.take() {
        let response: OverflowBlockingResponse = client
            .query(
                FETCH_REMAINING_BLOCKING_QUERY,
                OverflowVariables {
                    issue_id: &record.id,
                    first: OVERFLOW_PAGE,
                    after: &cursor,
                },
            )
            .await?;
        let node = response.node.ok_or_else(|| {
            GitHubError::InvalidResponse(format!(
                "trackedIssues overflow returned null node for issue '{}'",
                record.id
            ))
        })?;
        record
            .tracked_issues
            .nodes
            .extend(node.tracked_issues.nodes);
        if node.tracked_issues.page_info.has_next_page {
            after = node.tracked_issues.page_info.end_cursor;
            if after.is_none() {
                return Err(GitHubError::InvalidResponse(
                    "trackedIssues overflow claimed has_next_page but returned no end_cursor"
                        .into(),
                ));
            }
        }
    }
    record.tracked_issues.page_info.has_next_page = false;
    record.tracked_issues.page_info.end_cursor = None;
    Ok(())
}

async fn drain_cross_refs_overflow(
    client: &GitHubClient,
    record: &mut IssueEdgeRecord,
) -> Result<(), GitHubError> {
    let Some(initial_cursor) = record.timeline_items.page_info.end_cursor.clone() else {
        return Err(GitHubError::InvalidResponse(format!(
            "issue '{}' inline timelineItems claimed has_next_page but returned no end_cursor",
            record.id
        )));
    };
    let mut after = Some(initial_cursor);
    while let Some(cursor) = after.take() {
        let response: OverflowCrossRefsResponse = client
            .query(
                FETCH_REMAINING_CROSS_REFS_QUERY,
                OverflowVariables {
                    issue_id: &record.id,
                    first: OVERFLOW_PAGE,
                    after: &cursor,
                },
            )
            .await?;
        let node = response.node.ok_or_else(|| {
            GitHubError::InvalidResponse(format!(
                "timelineItems overflow returned null node for issue '{}'",
                record.id
            ))
        })?;
        record
            .timeline_items
            .nodes
            .extend(node.timeline_items.nodes);
        if node.timeline_items.page_info.has_next_page {
            after = node.timeline_items.page_info.end_cursor;
            if after.is_none() {
                return Err(GitHubError::InvalidResponse(
                    "timelineItems overflow claimed has_next_page but returned no end_cursor"
                        .into(),
                ));
            }
        }
    }
    record.timeline_items.page_info.has_next_page = false;
    record.timeline_items.page_info.end_cursor = None;
    Ok(())
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
