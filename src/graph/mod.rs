//! Typed graph of nodes and edges. Built from a [`crate::github::projects::ProjectFetch`]
//! plus a [`crate::github::issues::RawIssueEdges`] by [`Graph::from_fetch`],
//! validated for cycles by [`Graph::validate`].
//!
//! See `md/design/graph-build.md` for the design.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::config::Config;
use crate::error::BuildError;
use crate::github::issues::{CrossReferenceSource, RawIssueEdges};
use crate::github::projects::{ItemContent, ProjectFetch, ProjectItem, RepositoryRef};

mod validate;

// ---------------------------------------------------------------------------
// NodeId
// ---------------------------------------------------------------------------

/// Identity of a graph node. Four kinds, one variant each.
///
/// `Issue` covers both GitHub Issues and Pull Requests — they share the
/// `<owner>/<repo>#<number>` namespace, so a single identity variant
/// avoids duplicate nodes when an Issue and a PR happen to share a number
/// across repos.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeId {
    /// On-board Issue or Pull Request, identified by `<owner>/<repo>#<number>`.
    Issue {
        owner: String,
        repo: String,
        number: u64,
    },
    /// Draft issue. Carries GitHub's project-scoped node id (`DI_xxx`).
    Draft(String),
    /// Item the token cannot read (lost permission, deleted, or unknown
    /// content type). Carries the project-item id (`PVTI_xxx`).
    Redacted(String),
    /// Off-board endpoint of a sub-issue or blocking edge, materialized
    /// so the edge can render. Identified by `<owner>/<repo>#<number>`.
    Ghost {
        owner: String,
        repo: String,
        number: u64,
    },
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeId::Issue {
                owner,
                repo,
                number,
            }
            | NodeId::Ghost {
                owner,
                repo,
                number,
            } => write!(f, "{owner}/{repo}#{number}"),
            NodeId::Draft(id) | NodeId::Redacted(id) => f.write_str(id),
        }
    }
}

impl Ord for NodeId {
    fn cmp(&self, other: &Self) -> Ordering {
        sort_key(self).cmp(&sort_key(other))
    }
}

impl PartialOrd for NodeId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Sort key tuple. Categories order Issue+Ghost first, then Draft, then
/// Redacted; within Issue+Ghost the primary key is `(owner, repo, number)`
/// and `Issue` precedes `Ghost` only as a defensive tiebreak — by
/// construction the two cannot share the same tuple (Ghosts materialize
/// only for off-board endpoints).
fn sort_key(id: &NodeId) -> (u8, &str, &str, u64, u8, &str) {
    match id {
        NodeId::Issue {
            owner,
            repo,
            number,
        } => (0, owner, repo, *number, 0, ""),
        NodeId::Ghost {
            owner,
            repo,
            number,
        } => (0, owner, repo, *number, 1, ""),
        NodeId::Draft(id) => (1, "", "", 0, 0, id),
        NodeId::Redacted(id) => (2, "", "", 0, 0, id),
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// One node in the rendered graph.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub label: String,
    pub url: Option<String>,
    /// `display_string()` of the configured `[colors]` field.
    pub status: Option<String>,
    /// `display_string()` of the configured `[cluster]` field.
    pub cluster: Option<String>,
    /// Issue/PR body. Retained for future tooltip rendering.
    pub body: Option<String>,
    /// Issue/PR state (`OPEN`, `CLOSED`, etc.). Retained for future
    /// tooltip rendering.
    pub state: Option<String>,
    /// Logins of users assigned to this node.
    pub assignees: Vec<String>,
}

/// Kind of underlying GitHub object, for render-time styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Issue,
    PullRequest,
    DraftIssue,
    Redacted,
    Ghost,
}

// ---------------------------------------------------------------------------
// Edge
// ---------------------------------------------------------------------------

/// One directed edge in the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub kind: EdgeKind,
    pub source: NodeId,
    pub target: NodeId,
}

/// Kind of relationship. See `md/design/edge-convention.md` for direction
/// and visual-style mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeKind {
    /// `child → parent` from `Issue.subIssues`.
    SubIssue,
    /// `blocker → blocked` from `Issue.trackedIssues`.
    Blocks,
    /// `mentioner → mentioned` from `Issue.timelineItems` filtered to
    /// `CROSS_REFERENCED_EVENT`.
    CrossReference,
}

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

/// Typed graph of nodes and edges. Both vectors are sorted at build time
/// (see `md/design/node-model.md` and `md/design/edge-convention.md`);
/// the value is byte-stable for fixed inputs.
///
/// Adjacency lookups for [`Graph::validate`] and the `unblocked`
/// subcommand are derived on demand — `Graph` carries no redundant
/// indices.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

// ---------------------------------------------------------------------------
// from_fetch
// ---------------------------------------------------------------------------

impl Graph {
    /// Turn a [`ProjectFetch`] and a [`RawIssueEdges`] into a typed
    /// [`Graph`]. Applies the six-step policy from
    /// `md/design/graph-build.md`: node materialization, on-board
    /// snapshot, sub-issue edges, blocking edges, cross-reference edges,
    /// then a deterministic sort.
    ///
    /// Self-edges on any kind return [`BuildError::SelfEdge`]. Off-board
    /// endpoints of sub-issue and blocking edges become ghost nodes;
    /// off-board endpoints of cross-references drop silently per
    /// `md/design/edge-convention.md`.
    pub fn from_fetch(
        project: ProjectFetch,
        edges: RawIssueEdges,
        config: &Config,
    ) -> Result<Graph, BuildError> {
        let colors_field = config.colors.github_name.as_str();
        let cluster_field = config.cluster.github_name.as_str();

        // Step 1: materialize one node per project item.
        let mut nodes: Vec<Node> = Vec::with_capacity(project.items.len());
        let mut node_set: HashSet<NodeId> = HashSet::with_capacity(project.items.len());
        let mut gh_id_to_node_id: HashMap<String, NodeId> = HashMap::new();

        for item in &project.items {
            let Some(node) = materialize_node(item, colors_field, cluster_field) else {
                continue;
            };
            if !node_set.insert(node.id.clone()) {
                continue;
            }
            if let Some(gh_id) = github_node_id_of(&item.content) {
                gh_id_to_node_id.insert(gh_id.to_owned(), node.id.clone());
            }
            nodes.push(node);
        }

        // Step 2: on-board snapshot — frozen before any ghosts get added.
        let on_board: HashSet<NodeId> = node_set.clone();

        let mut edges_out: Vec<Edge> = Vec::new();

        // Step 3: sub-issue edges (child → parent). Off-board → ghost.
        for item in &project.items {
            let ItemContent::Issue(parent) = &item.content else {
                continue;
            };
            let Some(parent_id) = issue_node_id(&parent.repository, parent.number) else {
                continue;
            };
            for sub in &parent.sub_issues.nodes {
                let Some(child_issue_id) = issue_node_id(&sub.repository, sub.number) else {
                    continue;
                };
                if child_issue_id == parent_id {
                    return Err(BuildError::SelfEdge {
                        node: child_issue_id,
                        kind: EdgeKind::SubIssue,
                    });
                }
                let child_endpoint =
                    resolve_endpoint(child_issue_id, &on_board, &mut nodes, &mut node_set);
                edges_out.push(Edge {
                    kind: EdgeKind::SubIssue,
                    source: child_endpoint,
                    target: parent_id.clone(),
                });
            }
        }

        // Step 4: blocking edges (blocker → blocked). Off-board → ghost.
        for record in &edges.issues {
            let Some(blocked_id) = gh_id_to_node_id.get(&record.id).cloned() else {
                continue;
            };
            for blocker in &record.tracked_issues.nodes {
                let Some(blocker_issue_id) = issue_node_id(&blocker.repository, blocker.number)
                else {
                    continue;
                };
                if blocker_issue_id == blocked_id {
                    return Err(BuildError::SelfEdge {
                        node: blocker_issue_id,
                        kind: EdgeKind::Blocks,
                    });
                }
                let blocker_endpoint =
                    resolve_endpoint(blocker_issue_id, &on_board, &mut nodes, &mut node_set);
                edges_out.push(Edge {
                    kind: EdgeKind::Blocks,
                    source: blocker_endpoint,
                    target: blocked_id.clone(),
                });
            }
        }

        // Step 5: cross-reference edges (mentioner → mentioned). Both
        // endpoints must be on the on-board snapshot — ghosts added in
        // steps 3/4 do not count as "on-board" for cross-refs. Restrictive
        // label filter: empty `require_labels` drops every cross-reference.
        let require_labels = &config.edges.cross_ref.require_labels;
        for record in &edges.issues {
            let Some(target_id) = gh_id_to_node_id.get(&record.id).cloned() else {
                continue;
            };
            if !on_board.contains(&target_id) {
                continue;
            }
            for event in &record.timeline_items.nodes {
                let (repository, number, labels) = match &event.source {
                    CrossReferenceSource::Issue {
                        repository,
                        number,
                        labels,
                        ..
                    }
                    | CrossReferenceSource::PullRequest {
                        repository,
                        number,
                        labels,
                        ..
                    } => (repository, *number, labels),
                    CrossReferenceSource::Unknown => continue,
                };
                let Some(source_id) = issue_node_id(repository, number) else {
                    continue;
                };
                if source_id == target_id {
                    return Err(BuildError::SelfEdge {
                        node: source_id,
                        kind: EdgeKind::CrossReference,
                    });
                }
                if !on_board.contains(&source_id) {
                    continue;
                }
                if require_labels.is_empty() {
                    continue;
                }
                let matches = labels
                    .nodes
                    .iter()
                    .any(|l| require_labels.iter().any(|r| r == &l.name));
                if !matches {
                    continue;
                }
                edges_out.push(Edge {
                    kind: EdgeKind::CrossReference,
                    source: source_id,
                    target: target_id.clone(),
                });
            }
        }

        // Step 6: sort.
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        edges_out.sort_by(|a, b| {
            a.source
                .cmp(&b.source)
                .then_with(|| a.kind.cmp(&b.kind))
                .then_with(|| a.target.cmp(&b.target))
        });

        Ok(Graph {
            nodes,
            edges: edges_out,
        })
    }
}

// ---------------------------------------------------------------------------
// from_fetch helpers
// ---------------------------------------------------------------------------

fn materialize_node(item: &ProjectItem, colors_field: &str, cluster_field: &str) -> Option<Node> {
    let status = field_value_display(item, colors_field);
    let cluster = field_value_display(item, cluster_field);

    match &item.content {
        ItemContent::Issue(c) => {
            let (owner, repo) = parse_owner_repo(&c.repository.name_with_owner)?;
            Some(Node {
                id: NodeId::Issue {
                    owner: owner.to_owned(),
                    repo: repo.to_owned(),
                    number: c.number,
                },
                kind: NodeKind::Issue,
                label: format!("#{}: {}", c.number, c.title),
                url: Some(c.url.clone()),
                status,
                cluster,
                body: Some(c.body.clone()),
                state: Some(c.state.clone()),
                assignees: c.assignees.nodes.iter().map(|u| u.login.clone()).collect(),
            })
        }
        ItemContent::PullRequest(c) => {
            let (owner, repo) = parse_owner_repo(&c.repository.name_with_owner)?;
            Some(Node {
                id: NodeId::Issue {
                    owner: owner.to_owned(),
                    repo: repo.to_owned(),
                    number: c.number,
                },
                kind: NodeKind::PullRequest,
                label: format!("#{}: {}", c.number, c.title),
                url: Some(c.url.clone()),
                status,
                cluster,
                body: Some(c.body.clone()),
                state: Some(c.state.clone()),
                assignees: c.assignees.nodes.iter().map(|u| u.login.clone()).collect(),
            })
        }
        ItemContent::DraftIssue(c) => Some(Node {
            id: NodeId::Draft(c.id.clone()),
            kind: NodeKind::DraftIssue,
            label: c.title.clone(),
            url: None,
            status,
            cluster,
            body: Some(c.body.clone()),
            state: None,
            assignees: c.assignees.nodes.iter().map(|u| u.login.clone()).collect(),
        }),
        ItemContent::Redacted => Some(Node {
            id: NodeId::Redacted(item.id.clone()),
            kind: NodeKind::Redacted,
            label: "[redacted]".to_owned(),
            url: None,
            status: None,
            cluster: None,
            body: None,
            state: None,
            assignees: vec![],
        }),
    }
}

fn github_node_id_of(content: &ItemContent) -> Option<&str> {
    match content {
        ItemContent::Issue(c) => Some(&c.id),
        ItemContent::PullRequest(c) => Some(&c.id),
        ItemContent::DraftIssue(_) | ItemContent::Redacted => None,
    }
}

/// First field value whose GitHub name matches `field_name`. Empty
/// `field_name` (the section is unset in config) returns `None` without
/// scanning.
fn field_value_display(item: &ProjectItem, field_name: &str) -> Option<String> {
    if field_name.is_empty() {
        return None;
    }
    item.field_values.nodes.iter().find_map(|fv| {
        if fv.field_name() == Some(field_name) {
            fv.display_string()
        } else {
            None
        }
    })
}

/// Split `name_with_owner` on the first `/`. `None` on malformed input —
/// edges and nodes that depend on the split drop silently per
/// `md/design/graph-build.md`.
fn parse_owner_repo(name_with_owner: &str) -> Option<(&str, &str)> {
    let (owner, repo) = name_with_owner.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner, repo))
}

fn issue_node_id(repository: &RepositoryRef, number: u64) -> Option<NodeId> {
    let (owner, repo) = parse_owner_repo(&repository.name_with_owner)?;
    Some(NodeId::Issue {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number,
    })
}

fn ghost_node(id: NodeId) -> Node {
    let (owner, repo, number) = match &id {
        NodeId::Ghost {
            owner,
            repo,
            number,
        } => (owner.as_str(), repo.as_str(), *number),
        _ => unreachable!("ghost_node called with non-Ghost id"),
    };
    Node {
        id: id.clone(),
        kind: NodeKind::Ghost,
        label: format!("{owner}/{repo}#{number}"),
        url: Some(format!("https://github.com/{owner}/{repo}/issues/{number}")),
        status: None,
        cluster: None,
        body: None,
        state: None,
        assignees: vec![],
    }
}

/// Pick the right endpoint for an edge targeting `issue_id`. If the
/// issue is on-board, the endpoint stays `NodeId::Issue`. Otherwise the
/// endpoint becomes `NodeId::Ghost`, and the ghost node is materialized
/// on first encounter.
fn resolve_endpoint(
    issue_id: NodeId,
    on_board: &HashSet<NodeId>,
    nodes: &mut Vec<Node>,
    node_set: &mut HashSet<NodeId>,
) -> NodeId {
    if on_board.contains(&issue_id) {
        return issue_id;
    }
    let NodeId::Issue {
        owner,
        repo,
        number,
    } = issue_id
    else {
        // `issue_node_id` only produces `NodeId::Issue`; defensive.
        return issue_id;
    };
    let ghost_id = NodeId::Ghost {
        owner,
        repo,
        number,
    };
    if node_set.insert(ghost_id.clone()) {
        nodes.push(ghost_node(ghost_id.clone()));
    }
    ghost_id
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(owner: &str, repo: &str, number: u64) -> NodeId {
        NodeId::Issue {
            owner: owner.into(),
            repo: repo.into(),
            number,
        }
    }

    fn ghost(owner: &str, repo: &str, number: u64) -> NodeId {
        NodeId::Ghost {
            owner: owner.into(),
            repo: repo.into(),
            number,
        }
    }

    // -- Display --

    #[test]
    fn display_issue_uses_owner_slash_repo_hash_number() {
        assert_eq!(issue("o", "r", 42).to_string(), "o/r#42");
    }

    #[test]
    fn display_ghost_uses_owner_slash_repo_hash_number() {
        assert_eq!(ghost("o", "r", 7).to_string(), "o/r#7");
    }

    #[test]
    fn display_draft_uses_raw_id() {
        assert_eq!(NodeId::Draft("DI_xyz".into()).to_string(), "DI_xyz");
    }

    #[test]
    fn display_redacted_uses_raw_id() {
        assert_eq!(NodeId::Redacted("PVTI_abc".into()).to_string(), "PVTI_abc");
    }

    // -- Equality --

    #[test]
    fn issue_and_ghost_with_same_tuple_are_not_equal() {
        // Same tuple but different variants must be distinct identities.
        assert_ne!(issue("o", "r", 1), ghost("o", "r", 1));
    }

    #[test]
    fn issue_equality_matches_on_all_fields() {
        assert_eq!(issue("o", "r", 1), issue("o", "r", 1));
        assert_ne!(issue("o", "r", 1), issue("o", "r", 2));
        assert_ne!(issue("o", "r", 1), issue("o", "s", 1));
    }

    // -- Ord --

    #[test]
    fn sort_orders_issues_by_owner_then_repo_then_number() {
        let mut v = vec![
            issue("o", "r", 2),
            issue("o", "r", 1),
            issue("a", "z", 9),
            issue("o", "q", 5),
        ];
        v.sort();
        assert_eq!(
            v,
            vec![
                issue("a", "z", 9),
                issue("o", "q", 5),
                issue("o", "r", 1),
                issue("o", "r", 2),
            ]
        );
    }

    #[test]
    fn sort_groups_categories_issue_ghost_then_draft_then_redacted() {
        let mut v = vec![
            NodeId::Redacted("PVTI_2".into()),
            NodeId::Draft("DI_b".into()),
            issue("o", "r", 1),
            NodeId::Redacted("PVTI_1".into()),
            ghost("o", "r", 5),
            NodeId::Draft("DI_a".into()),
        ];
        v.sort();
        // Issues + ghosts first, ordered by (owner, repo, number);
        // then drafts by id; then redacted by id.
        assert_eq!(
            v,
            vec![
                issue("o", "r", 1),
                ghost("o", "r", 5),
                NodeId::Draft("DI_a".into()),
                NodeId::Draft("DI_b".into()),
                NodeId::Redacted("PVTI_1".into()),
                NodeId::Redacted("PVTI_2".into()),
            ]
        );
    }

    #[test]
    fn sort_tiebreaks_issue_before_ghost_on_same_tuple() {
        let mut v = vec![ghost("o", "r", 1), issue("o", "r", 1)];
        v.sort();
        assert_eq!(v, vec![issue("o", "r", 1), ghost("o", "r", 1)]);
    }

    // -- from_fetch -----------------------------------------------------------

    use crate::config::Config;
    use crate::github::issues::{
        BlockingTarget, CrossReferenceEvent, CrossReferenceSource, IssueEdgeRecord, Label,
        RawIssueEdges,
    };
    use crate::github::projects::{
        DraftIssueContent, IssueContent, ItemContent, NodeList, OwnerKind, ProjectFetch,
        ProjectItem, ProjectMeta, RepositoryRef, SubIssueRef,
    };
    use crate::github::{Connection, PageInfo};

    fn page<T>(nodes: Vec<T>) -> Connection<T> {
        Connection {
            nodes,
            page_info: PageInfo {
                has_next_page: false,
                end_cursor: None,
            },
        }
    }

    fn issue_content(
        gh_id: &str,
        owner: &str,
        repo: &str,
        number: u64,
        title: &str,
        sub_issues: Vec<SubIssueRef>,
    ) -> IssueContent {
        IssueContent {
            id: gh_id.into(),
            number,
            title: title.into(),
            url: format!("https://github.com/{owner}/{repo}/issues/{number}"),
            state: "OPEN".into(),
            body: String::new(),
            repository: RepositoryRef {
                name_with_owner: format!("{owner}/{repo}"),
            },
            assignees: NodeList::default(),
            sub_issues: page(sub_issues),
        }
    }

    fn sub_issue_ref(gh_id: &str, owner: &str, repo: &str, number: u64) -> SubIssueRef {
        SubIssueRef {
            id: gh_id.into(),
            number,
            title: format!("sub {number}"),
            url: format!("https://github.com/{owner}/{repo}/issues/{number}"),
            state: "OPEN".into(),
            repository: RepositoryRef {
                name_with_owner: format!("{owner}/{repo}"),
            },
        }
    }

    fn issue_item(content: IssueContent) -> ProjectItem {
        ProjectItem {
            id: format!("PVTI_{}", content.id),
            field_values: NodeList::default(),
            content: ItemContent::Issue(content),
        }
    }

    fn draft_item(gh_id: &str, title: &str) -> ProjectItem {
        ProjectItem {
            id: format!("PVTI_{gh_id}"),
            field_values: NodeList::default(),
            content: ItemContent::DraftIssue(DraftIssueContent {
                id: gh_id.into(),
                title: title.into(),
                body: String::new(),
                created_at: "2026-01-01T00:00:00Z".into(),
                assignees: NodeList::default(),
            }),
        }
    }

    fn redacted_item(item_id: &str) -> ProjectItem {
        ProjectItem {
            id: item_id.into(),
            field_values: NodeList::default(),
            content: ItemContent::Redacted,
        }
    }

    fn project(items: Vec<ProjectItem>) -> ProjectFetch {
        ProjectFetch {
            meta: ProjectMeta {
                id: "PVT_test".into(),
                title: "Test".into(),
                owner_kind: OwnerKind::Organization,
                fields: vec![],
            },
            items,
        }
    }

    fn blocking_target(owner: &str, repo: &str, number: u64) -> BlockingTarget {
        BlockingTarget {
            id: format!("I_blocker_{number}"),
            number,
            repository: RepositoryRef {
                name_with_owner: format!("{owner}/{repo}"),
            },
        }
    }

    fn cross_ref_source(
        owner: &str,
        repo: &str,
        number: u64,
        labels: &[&str],
    ) -> CrossReferenceEvent {
        CrossReferenceEvent {
            source: CrossReferenceSource::Issue {
                id: format!("I_xref_{number}"),
                number,
                repository: RepositoryRef {
                    name_with_owner: format!("{owner}/{repo}"),
                },
                labels: NodeList {
                    nodes: labels.iter().map(|n| Label { name: (*n).into() }).collect(),
                },
            },
        }
    }

    fn issue_edge_record(
        gh_id: &str,
        blockers: Vec<BlockingTarget>,
        xrefs: Vec<CrossReferenceEvent>,
    ) -> IssueEdgeRecord {
        IssueEdgeRecord {
            id: gh_id.into(),
            tracked_issues: page(blockers),
            timeline_items: page(xrefs),
        }
    }

    fn config_with_cross_ref_labels(labels: &[&str]) -> Config {
        let toml_text = if labels.is_empty() {
            String::from("[github]\nowner = \"o\"\nproject = 1\n")
        } else {
            let list = labels
                .iter()
                .map(|l| format!("\"{l}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "[github]\nowner = \"o\"\nproject = 1\n\n[edges.cross-ref]\nrequire-labels = [{list}]\n"
            )
        };
        toml::from_str(&toml_text).expect("test config TOML should parse")
    }

    fn empty_edges() -> RawIssueEdges {
        RawIssueEdges { issues: vec![] }
    }

    // -- Step 1: node materialization --

    #[test]
    fn from_fetch_materializes_one_node_per_item() {
        let p = project(vec![
            issue_item(issue_content("I_a", "o", "r", 1, "first", vec![])),
            draft_item("DI_a", "drafty"),
            redacted_item("PVTI_redacted"),
        ]);
        let graph =
            Graph::from_fetch(p, empty_edges(), &config_with_cross_ref_labels(&[])).unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 0);
    }

    #[test]
    fn from_fetch_assigns_node_kinds_per_content_variant() {
        let p = project(vec![
            issue_item(issue_content("I_a", "o", "r", 1, "i", vec![])),
            draft_item("DI_a", "d"),
            redacted_item("PVTI_z"),
        ]);
        let graph =
            Graph::from_fetch(p, empty_edges(), &config_with_cross_ref_labels(&[])).unwrap();
        let kinds: Vec<_> = graph.nodes.iter().map(|n| n.kind).collect();
        // Sort order: Issue+Ghost first, then Draft, then Redacted.
        assert_eq!(
            kinds,
            vec![NodeKind::Issue, NodeKind::DraftIssue, NodeKind::Redacted]
        );
    }

    // -- Step 3: sub-issue edges --

    #[test]
    fn sub_issue_edge_child_to_parent_when_both_on_board() {
        let child = issue_item(issue_content("I_b", "o", "r", 2, "child", vec![]));
        let parent = issue_item(issue_content(
            "I_a",
            "o",
            "r",
            1,
            "parent",
            vec![sub_issue_ref("I_b", "o", "r", 2)],
        ));
        let p = project(vec![child, parent]);
        let graph =
            Graph::from_fetch(p, empty_edges(), &config_with_cross_ref_labels(&[])).unwrap();
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].kind, EdgeKind::SubIssue);
        assert_eq!(graph.edges[0].source, issue("o", "r", 2));
        assert_eq!(graph.edges[0].target, issue("o", "r", 1));
    }

    #[test]
    fn sub_issue_off_board_target_becomes_ghost() {
        let parent = issue_item(issue_content(
            "I_a",
            "o",
            "r",
            1,
            "parent",
            vec![sub_issue_ref("I_z", "ext", "lib", 99)],
        ));
        let p = project(vec![parent]);
        let graph =
            Graph::from_fetch(p, empty_edges(), &config_with_cross_ref_labels(&[])).unwrap();
        // 1 on-board issue + 1 ghost
        assert_eq!(graph.nodes.len(), 2);
        assert!(graph.nodes.iter().any(|n| n.kind == NodeKind::Ghost));
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].source, ghost("ext", "lib", 99));
        assert_eq!(graph.edges[0].target, issue("o", "r", 1));
    }

    #[test]
    fn self_sub_issue_edge_returns_error() {
        let parent = issue_item(issue_content(
            "I_a",
            "o",
            "r",
            1,
            "loop",
            vec![sub_issue_ref("I_a", "o", "r", 1)],
        ));
        let err = Graph::from_fetch(
            project(vec![parent]),
            empty_edges(),
            &config_with_cross_ref_labels(&[]),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            BuildError::SelfEdge {
                kind: EdgeKind::SubIssue,
                ..
            }
        ));
    }

    // -- Step 4: blocking edges --

    #[test]
    fn blocking_edge_blocker_to_blocked_when_both_on_board() {
        let blocker = issue_item(issue_content("I_b", "o", "r", 5, "blocker", vec![]));
        let blocked = issue_item(issue_content("I_a", "o", "r", 1, "blocked", vec![]));
        let raw = RawIssueEdges {
            issues: vec![issue_edge_record(
                "I_a",
                vec![blocking_target("o", "r", 5)],
                vec![],
            )],
        };
        let graph = Graph::from_fetch(
            project(vec![blocker, blocked]),
            raw,
            &config_with_cross_ref_labels(&[]),
        )
        .unwrap();
        let blocks: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Blocks)
            .collect();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].source, issue("o", "r", 5));
        assert_eq!(blocks[0].target, issue("o", "r", 1));
    }

    #[test]
    fn blocking_off_board_blocker_becomes_ghost() {
        let blocked = issue_item(issue_content("I_a", "o", "r", 1, "blocked", vec![]));
        let raw = RawIssueEdges {
            issues: vec![issue_edge_record(
                "I_a",
                vec![blocking_target("ext", "lib", 77)],
                vec![],
            )],
        };
        let graph = Graph::from_fetch(
            project(vec![blocked]),
            raw,
            &config_with_cross_ref_labels(&[]),
        )
        .unwrap();
        assert!(graph.nodes.iter().any(|n| n.kind == NodeKind::Ghost));
        let edge = &graph.edges[0];
        assert_eq!(edge.kind, EdgeKind::Blocks);
        assert_eq!(edge.source, ghost("ext", "lib", 77));
        assert_eq!(edge.target, issue("o", "r", 1));
    }

    // -- Step 5: cross-reference edges --

    #[test]
    fn cross_ref_drops_all_when_require_labels_empty() {
        let source = issue_item(issue_content("I_b", "o", "r", 2, "mentioner", vec![]));
        let target = issue_item(issue_content("I_a", "o", "r", 1, "target", vec![]));
        let raw = RawIssueEdges {
            issues: vec![issue_edge_record(
                "I_a",
                vec![],
                vec![cross_ref_source("o", "r", 2, &["tracking"])],
            )],
        };
        let graph = Graph::from_fetch(
            project(vec![source, target]),
            raw,
            &config_with_cross_ref_labels(&[]),
        )
        .unwrap();
        assert!(
            graph
                .edges
                .iter()
                .all(|e| e.kind != EdgeKind::CrossReference)
        );
    }

    #[test]
    fn cross_ref_renders_when_source_carries_required_label() {
        let source = issue_item(issue_content("I_b", "o", "r", 2, "mentioner", vec![]));
        let target = issue_item(issue_content("I_a", "o", "r", 1, "target", vec![]));
        let raw = RawIssueEdges {
            issues: vec![issue_edge_record(
                "I_a",
                vec![],
                vec![cross_ref_source("o", "r", 2, &["tracking"])],
            )],
        };
        let graph = Graph::from_fetch(
            project(vec![source, target]),
            raw,
            &config_with_cross_ref_labels(&["tracking"]),
        )
        .unwrap();
        let xrefs: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::CrossReference)
            .collect();
        assert_eq!(xrefs.len(), 1);
        assert_eq!(xrefs[0].source, issue("o", "r", 2));
        assert_eq!(xrefs[0].target, issue("o", "r", 1));
    }

    #[test]
    fn cross_ref_drops_when_source_only_present_as_ghost() {
        // Issue B exists only as a ghost (added by a sub-issue edge), so it
        // is NOT on the on-board snapshot. A cross-ref from B → A must drop.
        let parent = issue_item(issue_content(
            "I_a",
            "o",
            "r",
            1,
            "parent",
            vec![sub_issue_ref("I_b", "o", "r", 2)], // B is off-board → ghost
        ));
        let raw = RawIssueEdges {
            issues: vec![issue_edge_record(
                "I_a",
                vec![],
                vec![cross_ref_source("o", "r", 2, &["tracking"])],
            )],
        };
        let graph = Graph::from_fetch(
            project(vec![parent]),
            raw,
            &config_with_cross_ref_labels(&["tracking"]),
        )
        .unwrap();
        assert!(
            graph
                .edges
                .iter()
                .all(|e| e.kind != EdgeKind::CrossReference)
        );
    }

    #[test]
    fn cross_ref_unknown_source_variant_drops_silently() {
        let target = issue_item(issue_content("I_a", "o", "r", 1, "target", vec![]));
        let raw = RawIssueEdges {
            issues: vec![issue_edge_record(
                "I_a",
                vec![],
                vec![CrossReferenceEvent {
                    source: CrossReferenceSource::Unknown,
                }],
            )],
        };
        let graph = Graph::from_fetch(
            project(vec![target]),
            raw,
            &config_with_cross_ref_labels(&["x"]),
        )
        .unwrap();
        assert!(
            graph
                .edges
                .iter()
                .all(|e| e.kind != EdgeKind::CrossReference)
        );
    }

    // -- Step 6: deterministic sort --

    #[test]
    fn nodes_are_sorted_by_node_id_and_edges_by_source_kind_target() {
        // Inputs intentionally out of order.
        let b = issue_item(issue_content("I_b", "o", "r", 2, "B", vec![]));
        let a = issue_item(issue_content(
            "I_a",
            "o",
            "r",
            1,
            "A",
            vec![sub_issue_ref("I_b", "o", "r", 2)],
        ));
        let graph = Graph::from_fetch(
            project(vec![b, a]),
            empty_edges(),
            &config_with_cross_ref_labels(&[]),
        )
        .unwrap();
        // Issue#1 sorts before Issue#2.
        assert_eq!(graph.nodes[0].id, issue("o", "r", 1));
        assert_eq!(graph.nodes[1].id, issue("o", "r", 2));
    }

    // -- Field-value lookup --

    #[test]
    fn status_and_cluster_are_pulled_from_matching_field_values() {
        use crate::github::projects::{FieldRef, FieldValue};
        let mut content = issue_content("I_a", "o", "r", 1, "x", vec![]);
        let mut item = ProjectItem {
            id: "PVTI_a".into(),
            field_values: NodeList {
                nodes: vec![
                    FieldValue::SingleSelect {
                        field: FieldRef {
                            name: "Status".into(),
                        },
                        name: "In Progress".into(),
                        option_id: "opt1".into(),
                    },
                    FieldValue::Text {
                        field: FieldRef {
                            name: "Area".into(),
                        },
                        text: "compiler-frontend".into(),
                    },
                ],
            },
            content: ItemContent::Issue(content.clone()),
        };
        // (re-borrow content for the assignees clone above)
        content.assignees = NodeList::default();
        item.content = ItemContent::Issue(content);

        let toml_text = "[github]\nowner=\"o\"\nproject=1\n\n[colors]\ngithub-name=\"Status\"\n\n[cluster]\ngithub-name=\"Area\"\n";
        let cfg: Config = toml::from_str(toml_text).unwrap();

        let graph = Graph::from_fetch(project(vec![item]), empty_edges(), &cfg).unwrap();
        assert_eq!(graph.nodes[0].status.as_deref(), Some("In Progress"));
        assert_eq!(graph.nodes[0].cluster.as_deref(), Some("compiler-frontend"));
    }
}
