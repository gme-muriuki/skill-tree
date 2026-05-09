//! GitHub Projects V2 queries: response types for the items connection
//! and the project metadata fetch.
//!
//! See `md/design/project-fetch.md` for the design.

use crate::error::GitHubError;
use crate::github::{Connection, GitHubClient};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Connection helpers
// ---------------------------------------------------------------------------

/// A GraphQL `nodes`-only list. Used for short, unpaginated inner connections
/// such as `assignees(first: 10)`. Use [`crate::github::Connection<T>`] when
/// pagination is required.
#[derive(Debug, Clone, Deserialize)]
pub struct NodeList<T> {
    pub nodes: Vec<T>,
}

impl<T> Default for NodeList<T> {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

/// Reference to a project field, just enough to look it up by name.
#[derive(Debug, Clone, Deserialize)]
pub struct FieldRef {
    pub name: String,
}

// ---------------------------------------------------------------------------
// FieldValue
// ---------------------------------------------------------------------------
//
// `items[].fieldValues.nodes` is a polymorphic union. Variants we recognize
// expose typed data; anything else lands in `Unknown` via `#[serde(other)]`
// so a new GitHub field type does not crash the fetch.

/// A single field value attached to a project item.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "__typename")]
pub enum FieldValue {
    #[serde(rename = "ProjectV2ItemFieldTextValue")]
    Text { field: FieldRef, text: String },

    #[serde(rename = "ProjectV2ItemFieldNumberValue")]
    Number { field: FieldRef, number: f64 },

    #[serde(rename = "ProjectV2ItemFieldSingleSelectValue")]
    SingleSelect {
        field: FieldRef,
        name: String,
        #[serde(rename = "optionId")]
        option_id: String,
    },

    #[serde(rename = "ProjectV2ItemFieldDateValue")]
    Date { field: FieldRef, date: String },

    #[serde(rename = "ProjectV2ItemFieldIterationValue")]
    Iteration {
        field: FieldRef,
        title: String,
        #[serde(rename = "startDate")]
        start_date: String,
    },

    /// Forward-compat: any `__typename` not matched above.
    #[serde(other)]
    Unknown,
}

impl FieldValue {
    /// String form for `[colors.values]` lookup and CLI display.
    /// Returns `None` for `Unknown`.
    pub fn display_string(&self) -> Option<String> {
        match self {
            FieldValue::Text { text, .. } => Some(text.clone()),
            FieldValue::Number { number, .. } => Some(number.to_string()),
            FieldValue::SingleSelect { name, .. } => Some(name.clone()),
            FieldValue::Date { date, .. } => Some(date.clone()),
            FieldValue::Iteration { title, .. } => Some(title.clone()),
            FieldValue::Unknown => None,
        }
    }

    /// The GitHub field name this value belongs to, or `None` for `Unknown`.
    pub fn field_name(&self) -> Option<&str> {
        match self {
            FieldValue::Text { field, .. }
            | FieldValue::Number { field, .. }
            | FieldValue::SingleSelect { field, .. }
            | FieldValue::Date { field, .. }
            | FieldValue::Iteration { field, .. } => Some(&field.name),
            FieldValue::Unknown => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Item content helpers
// ---------------------------------------------------------------------------

/// Repository this content belongs to. The `nameWithOwner` form
/// (`octocat/Hello-World`) is sufficient for display and linking.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryRef {
    pub name_with_owner: String,
}

/// A user mentioned by an item (assignee, reviewer, etc.).
#[derive(Debug, Clone, Deserialize)]
pub struct UserRef {
    pub login: String,
}

/// Reference to a sub-issue. Carries enough to render a graph node;
/// the full sub-issue is fetched separately if the graph layer expands it.
#[derive(Debug, Clone, Deserialize)]
pub struct SubIssueRef {
    pub id: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub repository: RepositoryRef,
}

// ---------------------------------------------------------------------------
// ItemContent
// ---------------------------------------------------------------------------
//
// `items[].content` is a union of `Issue | PullRequest | DraftIssue`. The
// GraphQL field is nullable: when the token cannot read the underlying
// content (lost permission, deleted), GitHub returns `null`. Both the
// unrecognized-typename path and the `null` path map to `Redacted`.

/// The underlying GitHub object behind a project item.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "__typename")]
pub enum ItemContent {
    #[serde(rename = "Issue")]
    Issue(IssueContent),

    #[serde(rename = "PullRequest")]
    PullRequest(PullRequestContent),

    #[serde(rename = "DraftIssue")]
    DraftIssue(DraftIssueContent),

    /// Token cannot read the underlying content, the content was deleted,
    /// or `__typename` is a variant skill-tree does not recognize.
    #[serde(other)]
    Redacted,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueContent {
    pub id: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub body: String,
    pub repository: RepositoryRef,
    pub assignees: NodeList<UserRef>,
    /// Sub-issues from the inline `subIssues(first: 50)` connection on
    /// the items query. After [`fetch_project`], `nodes` is the complete
    /// list and `page_info.has_next_page` is `false` — overflow has been
    /// resolved by [`resolve_sub_issue_overflow`].
    pub sub_issues: Connection<SubIssueRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestContent {
    pub id: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub body: String,
    pub repository: RepositoryRef,
    pub assignees: NodeList<UserRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftIssueContent {
    pub id: String,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub assignees: NodeList<UserRef>,
}

// ---------------------------------------------------------------------------
// ProjectItem
// ---------------------------------------------------------------------------

/// A single row on the project board.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectItem {
    pub id: String,
    pub field_values: NodeList<FieldValue>,
    /// `null` content (lost permission, deleted) is mapped to
    /// [`ItemContent::Redacted`].
    #[serde(deserialize_with = "deserialize_content_or_redacted")]
    pub content: ItemContent,
}

fn deserialize_content_or_redacted<'de, D>(d: D) -> Result<ItemContent, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<ItemContent> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(ItemContent::Redacted))
}

// ---------------------------------------------------------------------------
// Project metadata
// ---------------------------------------------------------------------------
//
// `ProjectMeta` is the result of a single GraphQL document that probes both
// `organization(login: $owner)` and `user(login: $owner)` in one round-trip
// (the namespaces are disjoint, so at most one branch is non-null). The
// `owner_kind` recorded on `ProjectMeta` lets later queries — items, issue
// detail — pick the correct root selector without re-discovering it.

/// Whether a project's owner is an organization or a personal user account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerKind {
    Organization,
    User,
}

impl fmt::Display for OwnerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OwnerKind::Organization => write!(f, "organization"),
            OwnerKind::User => write!(f, "user"),
        }
    }
}

/// One option of a SingleSelect field.
#[derive(Debug, Clone, Deserialize)]
pub struct FieldOption {
    pub id: String,
    pub name: String,
}

/// One iteration of an Iteration field.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Iteration {
    pub id: String,
    pub title: String,
    pub start_date: String,
    pub duration: u32,
}

/// Type-specific payload of a project field. The header (`id`, `name`)
/// lives on [`ProjectField`]; only the type-discriminated extras live here.
#[derive(Debug, Clone)]
pub enum FieldKind {
    Text,
    Number,
    Date,
    SingleSelect {
        options: Vec<FieldOption>,
    },
    Iteration {
        iterations: Vec<Iteration>,
    },
    /// Any field type skill-tree does not recognize. Forward-compatible
    /// with future GitHub field kinds.
    Unknown,
}

/// One field defined on the project.
#[derive(Debug, Clone)]
pub struct ProjectField {
    pub id: String,
    pub name: String,
    pub kind: FieldKind,
}

/// Project-level metadata: identity, title, owner kind, and field defs.
/// Returned by [`fetch_project_meta`] and consumed by `Config::validate_against`.
#[derive(Debug, Clone)]
pub struct ProjectMeta {
    pub id: String,
    pub title: String,
    pub owner_kind: OwnerKind,
    pub fields: Vec<ProjectField>,
}

impl ProjectMeta {
    /// Find a field by its GitHub-side name. Used by config validation
    /// to resolve `[colors] field = "..."` and `[cluster] field = "..."`.
    pub fn field_by_name(&self, name: &str) -> Option<&ProjectField> {
        self.fields.iter().find(|f| f.name == name)
    }
}

// ---------------------------------------------------------------------------
// Metadata: raw deserialization shape
// ---------------------------------------------------------------------------
//
// GitHub's `ProjectV2FieldConfiguration` is a union with three concrete
// types. We deserialize a flat `RawProjectField` that captures the
// superset of fields, then collapse to the typed `ProjectField` /
// `FieldKind` shape in code. This keeps `Unknown` cheap (no data loss
// for `id` / `name`) and avoids fighting serde over a multi-tag union.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProjectField {
    #[serde(rename = "__typename")]
    typename: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    data_type: Option<String>,
    #[serde(default)]
    options: Option<Vec<FieldOption>>,
    #[serde(default)]
    configuration: Option<RawIterationConfiguration>,
}

#[derive(Debug, Deserialize)]
struct RawIterationConfiguration {
    iterations: Vec<Iteration>,
}

impl ProjectField {
    fn from_raw(raw: RawProjectField) -> Self {
        let kind = match raw.typename.as_str() {
            "ProjectV2Field" => match raw.data_type.as_deref() {
                Some("TEXT") => FieldKind::Text,
                Some("NUMBER") => FieldKind::Number,
                Some("DATE") => FieldKind::Date,
                _ => FieldKind::Unknown,
            },
            "ProjectV2SingleSelectField" => FieldKind::SingleSelect {
                options: raw.options.unwrap_or_default(),
            },
            "ProjectV2IterationField" => FieldKind::Iteration {
                iterations: raw.configuration.map(|c| c.iterations).unwrap_or_default(),
            },
            _ => FieldKind::Unknown,
        };
        ProjectField {
            id: raw.id,
            name: raw.name,
            kind,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawProjectMeta {
    id: String,
    title: String,
    fields: NodeList<RawProjectField>,
}

impl ProjectMeta {
    fn from_raw(raw: RawProjectMeta, owner_kind: OwnerKind) -> Self {
        Self {
            id: raw.id,
            title: raw.title,
            owner_kind,
            fields: raw
                .fields
                .nodes
                .into_iter()
                .map(ProjectField::from_raw)
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OwnerProjectV2 {
    #[serde(rename = "projectV2")]
    project_v2: Option<RawProjectMeta>,
}

#[derive(Debug, Deserialize)]
struct FetchProjectMetaResponse {
    organization: Option<OwnerProjectV2>,
    user: Option<OwnerProjectV2>,
}

#[derive(Serialize)]
struct ProjectMetaVariables<'a> {
    owner: &'a str,
    number: u32,
}

// ---------------------------------------------------------------------------
// Metadata: fetch
// ---------------------------------------------------------------------------

/// GraphQL document for the project metadata query. Probes `organization`
/// and `user` in one round-trip; at most one is non-null.
const FETCH_PROJECT_META_QUERY: &str = r#"
    query FetchProjectMeta($owner: String!, $number: Int!) {
        organization(login: $owner) {
            projectV2(number: $number) {
                ...ProjectMetaFields
            }
        }
        user(login: $owner) {
            projectV2(number: $number) {
                ...ProjectMetaFields
            }
        }
    }

    fragment ProjectMetaFields on ProjectV2 {
        id
        title
        fields(first: 100) {
            nodes {
                __typename
                ... on ProjectV2Field {
                    id
                    name
                    dataType
                }
                ... on ProjectV2SingleSelectField {
                    id
                    name
                    options {
                        id
                        name
                    }
                }
                ... on ProjectV2IterationField {
                    id
                    name
                    configuration {
                        iterations {
                            id
                            title
                            startDate
                            duration
                        }
                    }
                }
            }
        }
    }
"#;

/// Fetch project-level metadata: title, field definitions, and owner kind.
///
/// Returns [`GitHubError::OwnerUnreachable`] when neither `organization`
/// nor `user` resolves the login (nonexistent owner, or owner private to
/// the token). Returns [`GitHubError::ProjectNotFound`] when the owner
/// resolves but has no project with the given number.
///
/// Marked `#[doc(hidden)]`: the user-facing API is `fetch_project()`,
/// which calls this internally. Public so integration tests can reach it.
#[doc(hidden)]
pub async fn fetch_project_meta(
    client: &GitHubClient,
    owner: &str,
    number: u32,
) -> Result<ProjectMeta, GitHubError> {
    let response: FetchProjectMetaResponse = client
        .query(
            FETCH_PROJECT_META_QUERY,
            ProjectMetaVariables { owner, number },
        )
        .await?;

    if let Some(org) = response.organization {
        return match org.project_v2 {
            Some(raw) => Ok(ProjectMeta::from_raw(raw, OwnerKind::Organization)),
            None => Err(GitHubError::ProjectNotFound {
                owner: owner.to_string(),
                number,
                owner_kind: OwnerKind::Organization,
            }),
        };
    }

    if let Some(user) = response.user {
        return match user.project_v2 {
            Some(raw) => Ok(ProjectMeta::from_raw(raw, OwnerKind::User)),
            None => Err(GitHubError::ProjectNotFound {
                owner: owner.to_string(),
                number,
                owner_kind: OwnerKind::User,
            }),
        };
    }

    Err(GitHubError::OwnerUnreachable {
        owner: owner.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Items: query and pagination
// ---------------------------------------------------------------------------

/// GraphQL document for the project items query. Paginated by `$first` /
/// `$after`. Each item carries its `fieldValues`, content (Issue,
/// PullRequest, DraftIssue), and an inline `subIssues(first: 50)`
/// connection. Issues with more than 50 sub-issues are resolved by a
/// follow-up query in [`resolve_sub_issue_overflow`].
const FETCH_PROJECT_ITEMS_QUERY: &str = r#"
    query FetchProjectItems($projectId: ID!, $first: Int!, $after: String) {
        node(id: $projectId) {
            ... on ProjectV2 {
                items(first: $first, after: $after) {
                    nodes { ...ProjectItemFields }
                    pageInfo { hasNextPage endCursor }
                }
            }
        }
    }

    fragment ProjectItemFields on ProjectV2Item {
        id
        fieldValues(first: 30) {
            nodes {
                __typename
                ... on ProjectV2ItemFieldTextValue {
                    field { ...FieldRefFields }
                    text
                }
                ... on ProjectV2ItemFieldNumberValue {
                    field { ...FieldRefFields }
                    number
                }
                ... on ProjectV2ItemFieldSingleSelectValue {
                    field { ...FieldRefFields }
                    name
                    optionId
                }
                ... on ProjectV2ItemFieldDateValue {
                    field { ...FieldRefFields }
                    date
                }
                ... on ProjectV2ItemFieldIterationValue {
                    field { ...FieldRefFields }
                    title
                    startDate
                }
            }
        }
        content {
            __typename
            ... on Issue {
                id
                number
                title
                url
                state
                body
                repository { nameWithOwner }
                assignees(first: 10) { nodes { login } }
                subIssues(first: 50) {
                    nodes {
                        id
                        number
                        title
                        url
                        state
                        repository { nameWithOwner }
                    }
                    pageInfo { hasNextPage endCursor }
                }
            }
            ... on PullRequest {
                id
                number
                title
                url
                state
                body
                repository { nameWithOwner }
                assignees(first: 10) { nodes { login } }
            }
            ... on DraftIssue {
                id
                title
                body
                createdAt
                assignees(first: 10) { nodes { login } }
            }
        }
    }

    fragment FieldRefFields on ProjectV2FieldConfiguration {
        ... on ProjectV2Field { name }
        ... on ProjectV2SingleSelectField { name }
        ... on ProjectV2IterationField { name }
    }
"#;

/// Items returned per request to GitHub. GitHub allows up to 100 per page.
const ITEMS_PER_PAGE: u32 = 100;

#[derive(Serialize)]
struct ProjectItemsVariables<'a> {
    #[serde(rename = "projectId")]
    project_id: &'a str,
    first: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    after: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct FetchProjectItemsResponse {
    node: Option<ProjectItemsNode>,
}

#[derive(Debug, Deserialize)]
struct ProjectItemsNode {
    items: Connection<ProjectItem>,
}

/// Fetch every item on the project. Pages internally at 100 per request
/// and stitches the results into a single flat `Vec<ProjectItem>`.
///
/// Sub-issues come back inline at `first: 50`. Overflow (issues with more
/// than 50 sub-issues) is **not** resolved here — call
/// [`resolve_sub_issue_overflow`] on the result before consuming.
///
/// Marked `#[doc(hidden)]`: the user-facing API is `fetch_project()`,
/// which calls this internally. Public so integration tests can reach it.
#[doc(hidden)]
pub async fn fetch_project_items(
    client: &GitHubClient,
    project_id: &str,
) -> Result<Vec<ProjectItem>, GitHubError> {
    let mut items = Vec::new();
    let mut after: Option<String> = None;

    loop {
        let response: FetchProjectItemsResponse = client
            .query(
                FETCH_PROJECT_ITEMS_QUERY,
                ProjectItemsVariables {
                    project_id,
                    first: ITEMS_PER_PAGE,
                    after: after.as_deref(),
                },
            )
            .await?;

        let node = response.node.ok_or_else(|| {
            GitHubError::InvalidResponse(format!(
                "items query returned null node for project '{project_id}'"
            ))
        })?;

        items.extend(node.items.nodes);

        if !node.items.page_info.has_next_page {
            break;
        }
        // GitHub guarantees end_cursor when has_next_page is true; defend
        // against a malformed response that breaks the invariant.
        after = node.items.page_info.end_cursor;
        if after.is_none() {
            return Err(GitHubError::InvalidResponse(
                "items pagination claimed has_next_page but returned no end_cursor".into(),
            ));
        }
    }

    Ok(items)
}

// ---------------------------------------------------------------------------
// Sub-issue overflow
// ---------------------------------------------------------------------------

/// GraphQL document for the per-issue sub-issue overflow query. Used when
/// an issue's inline `subIssues(first: 50)` connection has more pages.
const FETCH_REMAINING_SUB_ISSUES_QUERY: &str = r#"
    query FetchRemainingSubIssues($issueId: ID!, $first: Int!, $after: String!) {
        node(id: $issueId) {
            ... on Issue {
                subIssues(first: $first, after: $after) {
                    nodes {
                        id
                        number
                        title
                        url
                        state
                        repository { nameWithOwner }
                    }
                    pageInfo { hasNextPage endCursor }
                }
            }
        }
    }
"#;

/// Sub-issues per request when paginating overflow. GitHub's connection
/// `first` cap is 100; fetching the maximum minimizes round-trips on
/// issues with many children.
const SUB_ISSUES_PER_PAGE: u32 = 100;

#[derive(Serialize)]
struct SubIssuesVariables<'a> {
    #[serde(rename = "issueId")]
    issue_id: &'a str,
    first: u32,
    after: &'a str,
}

#[derive(Debug, Deserialize)]
struct FetchSubIssuesResponse {
    node: Option<IssueSubIssuesNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueSubIssuesNode {
    sub_issues: Connection<SubIssueRef>,
}

/// Resolve sub-issue overflow in place. For each `Issue` whose inline
/// connection reports `has_next_page: true`, follow up with the
/// per-issue overflow query and append the remaining nodes. Sets
/// `has_next_page: false` on every resolved issue so callers can rely
/// on the invariant that `nodes` is the complete list.
///
/// Marked `#[doc(hidden)]`: called internally by `fetch_project()`.
#[doc(hidden)]
pub async fn resolve_sub_issue_overflow(
    client: &GitHubClient,
    items: &mut [ProjectItem],
) -> Result<(), GitHubError> {
    for item in items.iter_mut() {
        if let ItemContent::Issue(ref mut issue) = item.content {
            if !issue.sub_issues.page_info.has_next_page {
                continue;
            }
            let Some(initial_cursor) = issue.sub_issues.page_info.end_cursor.clone() else {
                return Err(GitHubError::InvalidResponse(format!(
                    "issue '{}' inline sub-issues claimed has_next_page but returned no end_cursor",
                    issue.id
                )));
            };
            let mut after = Some(initial_cursor);
            while let Some(cursor) = after.take() {
                let response: FetchSubIssuesResponse = client
                    .query(
                        FETCH_REMAINING_SUB_ISSUES_QUERY,
                        SubIssuesVariables {
                            issue_id: &issue.id,
                            first: SUB_ISSUES_PER_PAGE,
                            after: &cursor,
                        },
                    )
                    .await?;

                let node = response.node.ok_or_else(|| {
                    GitHubError::InvalidResponse(format!(
                        "sub-issue overflow returned null node for issue '{}'",
                        issue.id
                    ))
                })?;

                issue.sub_issues.nodes.extend(node.sub_issues.nodes);

                if node.sub_issues.page_info.has_next_page {
                    after = node.sub_issues.page_info.end_cursor;
                    if after.is_none() {
                        return Err(GitHubError::InvalidResponse(
                            "sub-issue overflow claimed has_next_page but returned no end_cursor"
                                .into(),
                        ));
                    }
                }
            }
            // Resolved — graph layer can rely on the complete list.
            issue.sub_issues.page_info.has_next_page = false;
            issue.sub_issues.page_info.end_cursor = None;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    // -- FieldValue: known variants --

    #[test]
    fn field_value_text_deserializes() {
        let json = indoc! {r#"
            {
                "__typename": "ProjectV2ItemFieldTextValue",
                "field": { "name": "Notes" },
                "text": "ship it"
            }
        "#};
        let v: FieldValue = serde_json::from_str(json).unwrap();
        let FieldValue::Text { field, text } = v else {
            panic!("expected Text, got {v:?}");
        };
        assert_eq!(field.name, "Notes");
        assert_eq!(text, "ship it");
    }

    #[test]
    fn field_value_number_deserializes() {
        let json = indoc! {r#"
            {
                "__typename": "ProjectV2ItemFieldNumberValue",
                "field": { "name": "Priority" },
                "number": 1.5
            }
        "#};
        let v: FieldValue = serde_json::from_str(json).unwrap();
        let FieldValue::Number { field, number } = v else {
            panic!("expected Number, got {v:?}");
        };
        assert_eq!(field.name, "Priority");
        assert_eq!(number, 1.5);
    }

    #[test]
    fn field_value_single_select_deserializes_with_option_id_rename() {
        let json = indoc! {r#"
            {
                "__typename": "ProjectV2ItemFieldSingleSelectValue",
                "field": { "name": "Status" },
                "name": "In Progress",
                "optionId": "abc123"
            }
        "#};
        let v: FieldValue = serde_json::from_str(json).unwrap();
        let FieldValue::SingleSelect {
            field,
            name,
            option_id,
        } = v
        else {
            panic!("expected SingleSelect, got {v:?}");
        };
        assert_eq!(field.name, "Status");
        assert_eq!(name, "In Progress");
        assert_eq!(option_id, "abc123");
    }

    #[test]
    fn field_value_date_deserializes() {
        let json = indoc! {r#"
            {
                "__typename": "ProjectV2ItemFieldDateValue",
                "field": { "name": "Due" },
                "date": "2026-06-01"
            }
        "#};
        let v: FieldValue = serde_json::from_str(json).unwrap();
        assert!(matches!(v, FieldValue::Date { .. }));
    }

    #[test]
    fn field_value_iteration_deserializes_with_start_date_rename() {
        let json = indoc! {r#"
            {
                "__typename": "ProjectV2ItemFieldIterationValue",
                "field": { "name": "Sprint" },
                "title": "Sprint 12",
                "startDate": "2026-05-01"
            }
        "#};
        let v: FieldValue = serde_json::from_str(json).unwrap();
        let FieldValue::Iteration {
            field,
            title,
            start_date,
        } = v
        else {
            panic!("expected Iteration, got {v:?}");
        };
        assert_eq!(field.name, "Sprint");
        assert_eq!(title, "Sprint 12");
        assert_eq!(start_date, "2026-05-01");
    }

    // -- FieldValue: forward-compat --

    #[test]
    fn field_value_unknown_catches_new_typename() {
        let json = indoc! {r#"
            {
                "__typename": "ProjectV2ItemFieldFutureValue",
                "field": { "name": "Future" },
                "weirdNewProperty": 42
            }
        "#};
        let v: FieldValue = serde_json::from_str(json).unwrap();
        assert!(matches!(v, FieldValue::Unknown));
    }

    // -- FieldValue accessors --

    #[test]
    fn field_value_display_string_covers_known_variants() {
        let cases = [
            (
                FieldValue::Text {
                    field: FieldRef { name: "f".into() },
                    text: "hello".into(),
                },
                Some("hello".to_string()),
            ),
            (
                FieldValue::Number {
                    field: FieldRef { name: "f".into() },
                    number: 3.0,
                },
                Some("3".to_string()),
            ),
            (
                FieldValue::SingleSelect {
                    field: FieldRef { name: "f".into() },
                    name: "Done".into(),
                    option_id: "x".into(),
                },
                Some("Done".to_string()),
            ),
            (
                FieldValue::Date {
                    field: FieldRef { name: "f".into() },
                    date: "2026-01-01".into(),
                },
                Some("2026-01-01".to_string()),
            ),
            (
                FieldValue::Iteration {
                    field: FieldRef { name: "f".into() },
                    title: "Sprint 1".into(),
                    start_date: "2026-01-01".into(),
                },
                Some("Sprint 1".to_string()),
            ),
            (FieldValue::Unknown, None),
        ];

        for (input, expected) in cases {
            assert_eq!(input.display_string(), expected, "input was {input:?}");
        }
    }

    #[test]
    fn field_value_field_name_returns_name_for_known_variants_and_none_for_unknown() {
        let known = FieldValue::Text {
            field: FieldRef {
                name: "Status".into(),
            },
            text: "t".into(),
        };
        assert_eq!(known.field_name(), Some("Status"));
        assert_eq!(FieldValue::Unknown.field_name(), None);
    }

    // -- ItemContent: known variants --

    #[test]
    fn item_content_issue_deserializes() {
        let json = indoc! {r#"
            {
                "__typename": "Issue",
                "id": "I_1",
                "number": 12,
                "title": "Parser rewrite",
                "url": "https://github.com/o/r/issues/12",
                "state": "OPEN",
                "body": "details",
                "repository": { "nameWithOwner": "o/r" },
                "assignees": { "nodes": [{ "login": "octocat" }] },
                "subIssues": {
                    "nodes": [],
                    "pageInfo": { "hasNextPage": false, "endCursor": null }
                }
            }
        "#};
        let c: ItemContent = serde_json::from_str(json).unwrap();
        let ItemContent::Issue(issue) = c else {
            panic!("expected Issue, got {c:?}");
        };
        assert_eq!(issue.number, 12);
        assert_eq!(issue.title, "Parser rewrite");
        assert_eq!(issue.repository.name_with_owner, "o/r");
        assert_eq!(issue.assignees.nodes[0].login, "octocat");
        assert!(issue.sub_issues.nodes.is_empty());
        assert!(!issue.sub_issues.page_info.has_next_page);
    }

    #[test]
    fn item_content_pull_request_deserializes() {
        let json = indoc! {r#"
            {
                "__typename": "PullRequest",
                "id": "PR_1",
                "number": 7,
                "title": "Fix off-by-one",
                "url": "https://github.com/o/r/pull/7",
                "state": "MERGED",
                "body": "fixes",
                "repository": { "nameWithOwner": "o/r" },
                "assignees": { "nodes": [] }
            }
        "#};
        let c: ItemContent = serde_json::from_str(json).unwrap();
        assert!(matches!(c, ItemContent::PullRequest(_)));
    }

    #[test]
    fn item_content_draft_issue_deserializes() {
        let json = indoc! {r#"
            {
                "__typename": "DraftIssue",
                "id": "DI_1",
                "title": "Idea: cache layer",
                "body": "tbd",
                "createdAt": "2026-04-01T12:00:00Z",
                "assignees": { "nodes": [] }
            }
        "#};
        let c: ItemContent = serde_json::from_str(json).unwrap();
        let ItemContent::DraftIssue(draft) = c else {
            panic!("expected DraftIssue, got {c:?}");
        };
        assert_eq!(draft.title, "Idea: cache layer");
        assert_eq!(draft.created_at, "2026-04-01T12:00:00Z");
    }

    // -- ItemContent: redacted paths --

    #[test]
    fn item_content_redacted_catches_unknown_typename() {
        let json = r#"{ "__typename": "FutureContentType", "id": "x" }"#;
        let c: ItemContent = serde_json::from_str(json).unwrap();
        assert!(matches!(c, ItemContent::Redacted));
    }

    #[test]
    fn project_item_redacts_null_content() {
        let json = indoc! {r#"
            {
                "id": "PVTI_1",
                "fieldValues": { "nodes": [] },
                "content": null
            }
        "#};
        let item: ProjectItem = serde_json::from_str(json).unwrap();
        assert!(matches!(item.content, ItemContent::Redacted));
    }

    // -- ProjectItem: full-shape sanity --

    #[test]
    fn project_item_with_full_issue_content_round_trips() {
        let json = indoc! {r#"
            {
                "id": "PVTI_1",
                "fieldValues": {
                    "nodes": [
                        {
                            "__typename": "ProjectV2ItemFieldSingleSelectValue",
                            "field": { "name": "Status" },
                            "name": "In Progress",
                            "optionId": "opt_1"
                        }
                    ]
                },
                "content": {
                    "__typename": "Issue",
                    "id": "I_1",
                    "number": 12,
                    "title": "Parser rewrite",
                    "url": "https://github.com/o/r/issues/12",
                    "state": "OPEN",
                    "body": "details",
                    "repository": { "nameWithOwner": "o/r" },
                    "assignees": { "nodes": [] },
                    "subIssues": {
                        "nodes": [],
                        "pageInfo": { "hasNextPage": false, "endCursor": null }
                    }
                }
            }
        "#};
        let item: ProjectItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.id, "PVTI_1");
        assert_eq!(item.field_values.nodes.len(), 1);
        let FieldValue::SingleSelect { name, .. } = &item.field_values.nodes[0] else {
            panic!("expected SingleSelect");
        };
        assert_eq!(name, "In Progress");
        assert!(matches!(item.content, ItemContent::Issue(_)));
    }

    // -- ProjectField: from_raw routing --

    fn raw_field(typename: &str) -> RawProjectField {
        RawProjectField {
            typename: typename.into(),
            id: "F_1".into(),
            name: "field".into(),
            data_type: None,
            options: None,
            configuration: None,
        }
    }

    #[test]
    fn project_field_text_routes_from_data_type() {
        let raw = RawProjectField {
            data_type: Some("TEXT".into()),
            ..raw_field("ProjectV2Field")
        };
        let field = ProjectField::from_raw(raw);
        assert!(matches!(field.kind, FieldKind::Text));
        assert_eq!(field.id, "F_1");
        assert_eq!(field.name, "field");
    }

    #[test]
    fn project_field_number_routes_from_data_type() {
        let raw = RawProjectField {
            data_type: Some("NUMBER".into()),
            ..raw_field("ProjectV2Field")
        };
        assert!(matches!(
            ProjectField::from_raw(raw).kind,
            FieldKind::Number
        ));
    }

    #[test]
    fn project_field_date_routes_from_data_type() {
        let raw = RawProjectField {
            data_type: Some("DATE".into()),
            ..raw_field("ProjectV2Field")
        };
        assert!(matches!(ProjectField::from_raw(raw).kind, FieldKind::Date));
    }

    #[test]
    fn project_field_unknown_data_type_falls_through_to_unknown() {
        let raw = RawProjectField {
            data_type: Some("FUTURE_TYPE".into()),
            ..raw_field("ProjectV2Field")
        };
        assert!(matches!(
            ProjectField::from_raw(raw).kind,
            FieldKind::Unknown
        ));
    }

    #[test]
    fn project_field_single_select_carries_options() {
        let raw = RawProjectField {
            options: Some(vec![
                FieldOption {
                    id: "o1".into(),
                    name: "Done".into(),
                },
                FieldOption {
                    id: "o2".into(),
                    name: "In progress".into(),
                },
            ]),
            ..raw_field("ProjectV2SingleSelectField")
        };
        let field = ProjectField::from_raw(raw);
        let FieldKind::SingleSelect { options } = field.kind else {
            panic!("expected SingleSelect");
        };
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].name, "Done");
    }

    #[test]
    fn project_field_iteration_carries_iterations() {
        let raw = RawProjectField {
            configuration: Some(RawIterationConfiguration {
                iterations: vec![Iteration {
                    id: "i1".into(),
                    title: "Sprint 1".into(),
                    start_date: "2026-05-01".into(),
                    duration: 14,
                }],
            }),
            ..raw_field("ProjectV2IterationField")
        };
        let field = ProjectField::from_raw(raw);
        let FieldKind::Iteration { iterations } = field.kind else {
            panic!("expected Iteration");
        };
        assert_eq!(iterations.len(), 1);
        assert_eq!(iterations[0].title, "Sprint 1");
        assert_eq!(iterations[0].duration, 14);
    }

    #[test]
    fn project_field_unrecognized_typename_is_unknown() {
        let raw = raw_field("ProjectV2FutureField");
        assert!(matches!(
            ProjectField::from_raw(raw).kind,
            FieldKind::Unknown
        ));
    }

    // -- ProjectMeta: deserialization end-to-end --

    #[test]
    fn raw_project_meta_deserializes_with_mixed_field_kinds() {
        let json = indoc! {r#"
            {
                "id": "PVT_1",
                "title": "skill tree v3",
                "fields": {
                    "nodes": [
                        {
                            "__typename": "ProjectV2Field",
                            "id": "F_a",
                            "name": "Notes",
                            "dataType": "TEXT"
                        },
                        {
                            "__typename": "ProjectV2SingleSelectField",
                            "id": "F_b",
                            "name": "Status",
                            "options": [
                                { "id": "o1", "name": "Done" }
                            ]
                        },
                        {
                            "__typename": "ProjectV2IterationField",
                            "id": "F_c",
                            "name": "Sprint",
                            "configuration": {
                                "iterations": [
                                    {
                                        "id": "i1",
                                        "title": "Sprint 1",
                                        "startDate": "2026-05-01",
                                        "duration": 14
                                    }
                                ]
                            }
                        }
                    ]
                }
            }
        "#};
        let raw: RawProjectMeta = serde_json::from_str(json).unwrap();
        let meta = ProjectMeta::from_raw(raw, OwnerKind::Organization);

        assert_eq!(meta.id, "PVT_1");
        assert_eq!(meta.title, "skill tree v3");
        assert_eq!(meta.owner_kind, OwnerKind::Organization);
        assert_eq!(meta.fields.len(), 3);

        assert!(matches!(
            meta.field_by_name("Notes").unwrap().kind,
            FieldKind::Text
        ));
        assert!(matches!(
            meta.field_by_name("Status").unwrap().kind,
            FieldKind::SingleSelect { .. }
        ));
        assert!(matches!(
            meta.field_by_name("Sprint").unwrap().kind,
            FieldKind::Iteration { .. }
        ));
        assert!(meta.field_by_name("missing").is_none());
    }

    #[test]
    fn owner_kind_displays_lowercase_words() {
        assert_eq!(OwnerKind::Organization.to_string(), "organization");
        assert_eq!(OwnerKind::User.to_string(), "user");
    }
}
