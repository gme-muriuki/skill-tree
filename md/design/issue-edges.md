# Issue edges

The `github/issues.rs` module fetches per-issue edge data for every Issue on the project board: blocking relationships and cross-reference timeline events. It runs as a second pass after [project fetching](./project-fetch.md), gated on `ItemContent::Issue` — pull requests, draft issues, and redacted items are excluded.

It owns the GraphQL document for the second pass, the response types (`RawIssueEdges`, `BlockingTarget`, `CrossReferenceEvent`), and the pagination loops. Edge construction, ghost-node creation, and policy decisions live in [graph build](./graph-build.md).

## Public entry point

```rust
pub async fn fetch_issue_edges(
    client: &GitHubClient,
    issue_ids: &[String],
) -> Result<RawIssueEdges, GitHubError>;
```

Input is the list of issue GraphQL node IDs collected by walking `ProjectFetch.items` for `ItemContent::Issue`. Pagination of the batched query and per-issue overflow follow-ups are internal — the caller receives the complete data set.

## Why a second pass

`trackedIssues` and `timelineItems` live on the `Issue` object, not on `ProjectV2Item`. They cannot be co-fetched with the items query without nesting three paginated connections (`subIssues`, `trackedIssues`, `timelineItems`) inside an already-paginated items connection. Splitting the fetch:

- Leaves the items query unchanged — slice 1's pagination and overflow logic stay sealed.
- Restricts the second-pass input to `Issue` content. PRs, drafts, and redacted items have no `trackedIssues` connection and contribute no relevant timeline events under skill-tree's edge model (see [edge convention](./edge-convention.md)).
- Costs one extra batched round-trip per 100 issues — ~2 requests for a 200-issue board, plus any per-issue overflow.

## Batched query

Issue IDs are paginated at 100 (GitHub's cap on `nodes(ids:)`). For each batch:

```graphql
query IssueEdges($ids: [ID!]!) {
  nodes(ids: $ids) {
    ... on Issue {
      id
      trackedIssues(first: 50) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          number
          repository { nameWithOwner }
        }
      }
      timelineItems(itemTypes: CROSS_REFERENCED_EVENT, first: 50) {
        pageInfo { hasNextPage endCursor }
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
      }
    }
  }
}
```

`nodes(ids:)` returns a heterogeneous list; the inline `... on Issue` fragment skips any non-Issue items GitHub returns (PR IDs in the batch, deleted nodes). Drafts have no node id reachable this way and are filtered upstream.

## Overflow

Either inline `first: 50` connection may report `hasNextPage: true`. `fetch_issue_edges()` resolves each overflowing issue with a per-issue follow-up that drains the offending connection — mirroring the sub-issue overflow pattern documented in [project fetching](./project-fetch.md). Overflow is a long-tail event; most projects have neither 50+ blockers nor 50+ cross-references on a single issue.

## Inline source labels

The cross-reference label filter — `[edges.cross-ref] require-label = "..."` — needs the source issue's labels. Rather than a third pass, labels are inlined on the `CrossReferencedEvent.source` selection, capped at 20 per source. The label set ships into graph construction with no extra round-trip; the filter is exact-name match.

## Returned shape

```rust
pub struct RawIssueEdges {
    pub issues: Vec<IssueEdgeRecord>,
}

pub struct IssueEdgeRecord {
    pub id: String,                        // source issue's GitHub node id
    pub blocking: Vec<BlockingTarget>,
    pub cross_references: Vec<CrossReferenceEvent>,
}

pub struct BlockingTarget {
    pub id: String,
    pub number: u64,
    pub repository: RepositoryRef,
}

pub struct CrossReferenceEvent {
    pub source: CrossReferenceSource,
}

pub enum CrossReferenceSource {
    Issue {
        id: String,
        number: u64,
        repository: RepositoryRef,
        labels: Vec<String>,
    },
    PullRequest {
        id: String,
        number: u64,
        repository: RepositoryRef,
        labels: Vec<String>,
    },
}
```

The fetch layer applies no policy: no membership filter, no self-edge rejection, no source-label matching. Everything GitHub returned is preserved verbatim. The graph layer decides what becomes an edge, what becomes a ghost node, and what to drop. See [graph build](./graph-build.md).

## Errors

`fetch_issue_edges()` returns `GitHubError` from `github/mod.rs`. Standard transport, retry, and rate-limit behavior applies. GraphQL `errors` arrays surface as `GitHubError::GraphQLError` with GitHub's upstream message.
