//! Per-node label assembly.
//!
//! Plain DOT labels (not HTML-like) — see `md/design/node-display.md`
//! for the rationale. The label has two parts: a wrapped title and an
//! optional meta line carrying state and assignees.

use crate::graph::{Node, NodeKind};

const TITLE_WRAP_WIDTH: usize = 40;
const TITLE_MAX_LINES: usize = 2;
const ASSIGNEE_CAP: usize = 3;

/// Build the full multi-line label string. Newlines in the result are
/// plain `\n`; the caller's escape pass (`render::quote`) turns them
/// into the literal two-character escape Graphviz reads as a line break.
pub(super) fn format_label(node: &Node) -> String {
    match node.kind {
        NodeKind::Ghost | NodeKind::Redacted => node.label.clone(),
        NodeKind::Issue | NodeKind::PullRequest | NodeKind::DraftIssue => {
            let title = wrap_title(&node.label, TITLE_WRAP_WIDTH, TITLE_MAX_LINES);
            let meta = format_meta(node);
            if meta.is_empty() {
                title
            } else {
                format!("{title}\n{meta}")
            }
        }
    }
}

/// Greedy word-wrap on whitespace. A word longer than `width` is kept
/// whole and overflows the column — readable mid-word breaks are not
/// worth the parsing effort.
///
/// Overflow past `max_lines` is signaled by appending `…` to the last
/// visible line.
fn wrap_title(s: &str, width: usize, max_lines: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();

    for word in s.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }

    if lines.len() > max_lines {
        let mut kept: Vec<String> = lines.into_iter().take(max_lines).collect();
        if let Some(last) = kept.last_mut() {
            last.push('…');
        }
        kept.join("\n")
    } else {
        lines.join("\n")
    }
}

/// Meta line: `STATE · alice, bob`, either half optional. Empty when
/// the node has no state and no assignees — the caller then omits the
/// line entirely.
fn format_meta(node: &Node) -> String {
    let assignees = format_assignees(&node.assignees);
    let state = node.state.as_deref().unwrap_or("");
    [state, assignees.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// First [`ASSIGNEE_CAP`] logins comma-separated, with `+N` suffix when
/// more are present.
fn format_assignees(logins: &[String]) -> String {
    match logins.len() {
        0 => String::new(),
        n if n <= ASSIGNEE_CAP => logins.join(", "),
        n => format!(
            "{} +{}",
            logins[..ASSIGNEE_CAP].join(", "),
            n - ASSIGNEE_CAP
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::NodeId;

    fn issue(number: u64, title: &str) -> Node {
        Node {
            id: NodeId::Issue {
                owner: "o".into(),
                repo: "r".into(),
                number,
            },
            kind: NodeKind::Issue,
            label: format!("#{number}: {title}"),
            url: None,
            status: None,
            cluster: None,
            body: None,
            state: Some("OPEN".into()),
            assignees: vec![],
        }
    }

    fn draft(title: &str) -> Node {
        Node {
            id: NodeId::Draft("DI".into()),
            kind: NodeKind::DraftIssue,
            label: title.into(),
            url: None,
            status: None,
            cluster: None,
            body: None,
            state: None,
            assignees: vec![],
        }
    }

    #[test]
    fn wrap_keeps_single_short_line() {
        assert_eq!(wrap_title("short title", 40, 2), "short title");
    }

    #[test]
    fn wrap_breaks_on_whitespace_boundary() {
        let out = wrap_title("#289: Convert check_trait to a judgment function", 40, 2);
        assert_eq!(out.matches('\n').count(), 1, "expected one break: {out:?}");
    }

    #[test]
    fn wrap_appends_ellipsis_when_over_max_lines() {
        let very_long = "one two three four five six seven eight nine ten eleven twelve \
                         thirteen fourteen fifteen sixteen seventeen eighteen nineteen";
        let out = wrap_title(very_long, 40, 2);
        assert_eq!(out.lines().count(), 2);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn wrap_keeps_a_long_word_intact() {
        let out = wrap_title("supercalifragilisticexpialidocious", 10, 2);
        assert!(!out.contains('\n'));
        assert_eq!(out, "supercalifragilisticexpialidocious");
    }

    #[test]
    fn assignees_below_cap_render_inline() {
        let logins = vec!["alice".into(), "bob".into()];
        assert_eq!(format_assignees(&logins), "alice, bob");
    }

    #[test]
    fn assignees_above_cap_get_overflow_suffix() {
        let logins: Vec<String> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| (*s).into())
            .collect();
        assert_eq!(format_assignees(&logins), "a, b, c +2");
    }

    #[test]
    fn assignees_empty_is_empty_string() {
        assert_eq!(format_assignees(&[]), "");
    }

    #[test]
    fn meta_empty_when_no_state_and_no_assignees() {
        let n = draft("Draft title");
        assert_eq!(format_meta(&n), "");
    }

    #[test]
    fn meta_state_only_when_no_assignees() {
        let n = issue(1, "T");
        assert_eq!(format_meta(&n), "OPEN");
    }

    #[test]
    fn meta_joins_state_and_assignees_with_dot_separator() {
        let mut n = issue(1, "T");
        n.assignees = vec!["alice".into(), "bob".into()];
        assert_eq!(format_meta(&n), "OPEN · alice, bob");
    }

    #[test]
    fn meta_assignees_only_for_drafts_with_assignees() {
        let mut n = draft("Draft title");
        n.assignees = vec!["alice".into()];
        assert_eq!(format_meta(&n), "alice");
    }

    #[test]
    fn full_label_for_issue_includes_title_and_meta() {
        let mut n = issue(289, "Convert check_trait to a judgment function");
        n.assignees = vec!["alice".into()];
        let out = format_label(&n);
        assert!(out.starts_with("#289:"));
        assert!(out.ends_with("OPEN · alice"));
        assert!(out.contains('\n'));
    }

    #[test]
    fn full_label_for_ghost_is_node_label_verbatim() {
        let n = Node {
            id: NodeId::Ghost {
                owner: "ext".into(),
                repo: "lib".into(),
                number: 99,
            },
            kind: NodeKind::Ghost,
            label: "ext/lib#99".into(),
            url: None,
            status: None,
            cluster: None,
            body: None,
            state: None,
            assignees: vec![],
        };
        assert_eq!(format_label(&n), "ext/lib#99");
    }

    #[test]
    fn full_label_for_redacted_is_node_label_verbatim() {
        let n = Node {
            id: NodeId::Redacted("PVTI".into()),
            kind: NodeKind::Redacted,
            label: "[redacted]".into(),
            url: None,
            status: None,
            cluster: None,
            body: None,
            state: None,
            assignees: vec![],
        };
        assert_eq!(format_label(&n), "[redacted]");
    }
}
