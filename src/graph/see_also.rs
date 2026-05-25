//! Parser for the `See also` rows in an issue body's front-matter
//! metadata pipe-table.
//!
//! See `md/design/see-also.md`. Pure: in goes a body and the origin
//! issue's `(owner, repo)`; out come typed targets, any warnings worth
//! logging, and the body minus the metadata table (the residue that
//! downstream consumers — tooltip cleanup — should operate on).

use std::sync::LazyLock;

use regex::Regex;

use crate::error::see_also::SeeAlsoWarning;

/// A fully-resolved see-also target. Relative `#NN` refs in the source
/// row have been expanded against the origin issue's `(owner, repo)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeeAlsoTarget {
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

/// Result of [`parse`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedBody {
    pub targets: Vec<SeeAlsoTarget>,
    pub warnings: Vec<SeeAlsoWarning>,
    /// Body with the leading metadata table removed. Equals the input
    /// body when no table was detected.
    pub rest: String,
}

const SEE_ALSO_LABEL: &str = "see also";

/// One pass: leading whitespace/comments, contiguous pipe-rows, then
/// the rest of the body.
static FRONT_MATTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)\A(?:[ \t\r\n]+|<!--.*?-->)*(?P<table>(?:[ \t]*\|[^\n]*(?:\n|$))+)(?P<rest>.*)\z",
    )
    .unwrap()
});

/// Two-cell row: label, value. Additional cells are tolerated.
static ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[ \t]*\|?\s*(?P<label>[^|\n]+?)\s*\|\s*(?P<value>[^|\n]*?)\s*(?:\|.*)?$").unwrap()
});

/// Issue reference: optional `owner/repo`, then `#NN`.
static ISSUE_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:(?P<owner>[A-Za-z0-9][A-Za-z0-9._-]*)/(?P<repo>[A-Za-z0-9][A-Za-z0-9._-]*))?#(?P<num>\d+)$",
    )
    .unwrap()
});

/// Wrapping `[anchor](url)` markdown link — capture the anchor.
static MD_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[(?P<inner>[^\]\n]+)\]\([^)\n]+\)$").unwrap());

/// Parse the metadata pipe-table at the top of `body` and extract every
/// `See also` row. Returns the typed targets, any warnings, and the body
/// with the metadata block stripped.
pub fn parse(body: &str, origin_owner: &str, origin_repo: &str) -> ParsedBody {
    let Some(caps) = FRONT_MATTER.captures(body) else {
        return ParsedBody {
            rest: body.to_owned(),
            ..Default::default()
        };
    };
    let table = caps.name("table").unwrap().as_str();
    let rest = caps.name("rest").unwrap().as_str().to_owned();

    let mut targets = Vec::new();
    let mut warnings = Vec::new();
    for line in table.lines() {
        let Some(row) = ROW.captures(line) else {
            continue;
        };
        if !row["label"].eq_ignore_ascii_case(SEE_ALSO_LABEL) {
            continue;
        }
        match resolve_ref(&row["value"], origin_owner, origin_repo) {
            Ok(t) => targets.push(t),
            Err(w) => warnings.push(w),
        }
    }

    ParsedBody {
        targets,
        warnings,
        rest,
    }
}

fn resolve_ref(
    value: &str,
    origin_owner: &str,
    origin_repo: &str,
) -> Result<SeeAlsoTarget, SeeAlsoWarning> {
    if value.is_empty() {
        return Err(SeeAlsoWarning::MissingValue);
    }

    let inner = if let Some(caps) = MD_LINK.captures(value) {
        caps.name("inner").unwrap().as_str()
    } else if let Some(stripped) = value.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        stripped
    } else {
        value
    }
    .trim();

    let Some(caps) = ISSUE_REF.captures(inner) else {
        return Err(SeeAlsoWarning::UnparseableRef(value.to_owned()));
    };
    let number: u64 = caps["num"]
        .parse()
        .map_err(|_| SeeAlsoWarning::UnparseableRef(value.to_owned()))?;
    let (owner, repo) = match (caps.name("owner"), caps.name("repo")) {
        (Some(o), Some(r)) => (o.as_str().to_owned(), r.as_str().to_owned()),
        _ => (origin_owner.to_owned(), origin_repo.to_owned()),
    };
    Ok(SeeAlsoTarget {
        owner,
        repo,
        number,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    fn target(owner: &str, repo: &str, number: u64) -> SeeAlsoTarget {
        SeeAlsoTarget {
            owner: owner.into(),
            repo: repo.into(),
            number,
        }
    }

    #[test]
    fn empty_body_yields_nothing() {
        let p = parse("", "o", "r");
        assert!(p.targets.is_empty());
        assert!(p.warnings.is_empty());
        assert_eq!(p.rest, "");
    }

    #[test]
    fn body_without_table_returns_body_as_rest() {
        let body = "Just prose here.\n\nNo metadata.\n";
        let p = parse(body, "o", "r");
        assert!(p.targets.is_empty());
        assert!(p.warnings.is_empty());
        assert_eq!(p.rest, body);
    }

    #[test]
    fn absolute_ref_resolves_to_owner_repo() {
        let body = "| See also | rust-lang/rust#42 |\n";
        let p = parse(body, "o", "r");
        assert_eq!(p.targets, vec![target("rust-lang", "rust", 42)]);
    }

    #[test]
    fn relative_ref_inherits_origin_owner_repo() {
        let body = "| See also | #99 |\n";
        let p = parse(body, "rust-lang", "rust");
        assert_eq!(p.targets, vec![target("rust-lang", "rust", 99)]);
    }

    #[test]
    fn multiple_see_also_rows_collect_all() {
        let body = indoc! {"
            | See also | rust-lang/rust#1 |
            | See also | rust-lang/rfcs#2 |
            | See also | #3 |
        "};
        let p = parse(body, "o", "r");
        assert_eq!(
            p.targets,
            vec![
                target("rust-lang", "rust", 1),
                target("rust-lang", "rfcs", 2),
                target("o", "r", 3),
            ]
        );
    }

    #[test]
    fn other_labels_silently_ignored() {
        let body = indoc! {"
            | Tracking issue | rust-lang/rust#1 |
            | Point of contact | someone |
            | See also | rust-lang/rust#2 |
        "};
        let p = parse(body, "o", "r");
        assert_eq!(p.targets, vec![target("rust-lang", "rust", 2)]);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn header_and_separator_rows_silently_skipped() {
        let body = indoc! {"
            | Field | Value |
            | --- | --- |
            | See also | #1 |
        "};
        let p = parse(body, "o", "r");
        assert_eq!(p.targets, vec![target("o", "r", 1)]);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn markdown_link_value_extracts_anchor_text() {
        let body =
            "| See also | [rust-lang/rust#42](https://github.com/rust-lang/rust/issues/42) |\n";
        let p = parse(body, "o", "r");
        assert_eq!(p.targets, vec![target("rust-lang", "rust", 42)]);
    }

    #[test]
    fn square_bracketed_value_strips_brackets() {
        let body = "| See also | [rust-lang/rust#42] |\n";
        let p = parse(body, "o", "r");
        assert_eq!(p.targets, vec![target("rust-lang", "rust", 42)]);
    }

    #[test]
    fn malformed_value_emits_unparseable_warning() {
        let body = "| See also | not-a-ref |\n";
        let p = parse(body, "o", "r");
        assert!(p.targets.is_empty());
        assert_eq!(
            p.warnings,
            vec![SeeAlsoWarning::UnparseableRef("not-a-ref".into())]
        );
    }

    #[test]
    fn empty_value_emits_missing_value_warning() {
        let body = "| See also |  |\n";
        let p = parse(body, "o", "r");
        assert!(p.targets.is_empty());
        assert_eq!(p.warnings, vec![SeeAlsoWarning::MissingValue]);
    }

    #[test]
    fn label_match_is_case_insensitive() {
        let body = indoc! {"
            | SEE ALSO | #1 |
            | see also | #2 |
            | See Also | #3 |
        "};
        let p = parse(body, "o", "r");
        assert_eq!(
            p.targets,
            vec![
                target("o", "r", 1),
                target("o", "r", 2),
                target("o", "r", 3)
            ]
        );
    }

    #[test]
    fn leading_html_comment_skipped() {
        let body = indoc! {"
            <!-- planning note -->
            | See also | #1 |
        "};
        let p = parse(body, "o", "r");
        assert_eq!(p.targets, vec![target("o", "r", 1)]);
    }

    #[test]
    fn leading_blank_lines_skipped() {
        let body = "\n\n| See also | #1 |\n";
        let p = parse(body, "o", "r");
        assert_eq!(p.targets, vec![target("o", "r", 1)]);
    }

    #[test]
    fn prose_before_table_means_no_table_detected() {
        let body = indoc! {"
            Some prose.
            | See also | #1 |
        "};
        let p = parse(body, "o", "r");
        assert!(p.targets.is_empty());
        assert_eq!(p.rest, body);
    }

    #[test]
    fn rest_returns_body_after_table() {
        let body = indoc! {"
            | See also | #1 |

            Issue prose continues here.
        "};
        let p = parse(body, "o", "r");
        assert_eq!(p.targets, vec![target("o", "r", 1)]);
        assert_eq!(p.rest, "\nIssue prose continues here.\n");
    }

    #[test]
    fn second_table_block_after_blank_is_part_of_rest() {
        let body = indoc! {"
            | See also | #1 |

            | See also | #2 |
        "};
        let p = parse(body, "o", "r");
        assert_eq!(p.targets, vec![target("o", "r", 1)]);
        assert_eq!(p.rest, "\n| See also | #2 |\n");
    }

    #[test]
    fn unterminated_html_comment_bails_to_no_table() {
        let body = "<!-- never closed\n| See also | #1 |\n";
        let p = parse(body, "o", "r");
        assert!(p.targets.is_empty());
        assert_eq!(p.rest, body);
    }
}
