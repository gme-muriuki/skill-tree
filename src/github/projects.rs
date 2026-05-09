//! GitHub Projects V2 queries: response types for the items connection.
//!
//! See `md/design/project-fetch.md` for the design.

use serde::{Deserialize, Deserializer};

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
    pub sub_issues: NodeList<SubIssueRef>,
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
        let FieldValue::SingleSelect { field, name, option_id } = v else {
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
        let FieldValue::Iteration { field, title, start_date } = v else {
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
            field: FieldRef { name: "Status".into() },
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
                "subIssues": { "nodes": [] }
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
                    "subIssues": { "nodes": [] }
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
}
