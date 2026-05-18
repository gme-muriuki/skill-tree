//! Issue-body cleanup for the node tooltip.
//!
//! The pipeline is sequential — each step operates on the output of the
//! previous one. See `md/design/node-display.md` for the rationale.

use std::sync::LazyLock;

use regex::Regex;

/// Maximum char count of the cleaned tooltip body before truncation.
pub(super) const BODY_TOOLTIP_LIMIT: usize = 400;

static HTML_COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
static CODE_FENCE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]*```.*$").unwrap());
static BOLD_STAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*([^*\n]+?)\*\*").unwrap());
static BOLD_UNDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"__([^_\n]+?)__").unwrap());
static ITALIC_STAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|(?P<lead>[\s(]))\*([^*\n]+?)\*").unwrap());
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]\n]+)\]\([^)\n]+\)").unwrap());
static HEADING_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]*#+[ \t]*").unwrap());
static MULTI_NEWLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

/// Run the full cleanup pipeline and truncate at the nearest sentence
/// boundary before `limit` chars.
pub(super) fn clean_body(raw: &str, limit: usize) -> String {
    let s = HTML_COMMENT.replace_all(raw, "");
    let s = CODE_FENCE_LINE.replace_all(&s, "");
    let s = BOLD_STAR.replace_all(&s, "$1");
    let s = BOLD_UNDER.replace_all(&s, "$1");
    let s = ITALIC_STAR.replace_all(&s, "$lead$2");
    let s = LINK.replace_all(&s, "$1");
    let s = HEADING_MARKER.replace_all(&s, "");
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let s = MULTI_NEWLINE.replace_all(&s, "\n\n");
    truncate_at_sentence(s.trim(), limit)
}

/// Truncate at the last sentence boundary (`. `, `! `, `? `, or
/// `\n\n`) inside the first `limit` chars. Falls back to a char-aware
/// hard cap with `…` when no boundary fits. Returns `s` unchanged when
/// `s` already fits.
fn truncate_at_sentence(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_owned();
    }

    let budget_end = s
        .char_indices()
        .nth(limit)
        .map(|(byte, _)| byte)
        .unwrap_or(s.len());
    let head = &s[..budget_end];

    let cut = [". ", "! ", "? ", "\n\n"]
        .iter()
        .filter_map(|boundary| head.rfind(boundary).map(|p| p + boundary.len()))
        .max();

    let body = match cut {
        Some(p) => head[..p].trim_end().to_owned(),
        None => head.chars().take(limit).collect(),
    };

    format!("{body}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html_comments_inline_and_multiline() {
        assert_eq!(
            clean_body("before <!-- noise --> after", 200),
            "before  after"
        );
        assert_eq!(clean_body("a<!--\nmulti\nline\n-->b", 200), "ab");
    }

    #[test]
    fn drops_code_fence_lines_keeps_code_text() {
        let raw = "intro\n```rust\nlet x = 1;\n```\noutro";
        let cleaned = clean_body(raw, 200);
        assert!(cleaned.contains("intro"));
        assert!(cleaned.contains("let x = 1;"));
        assert!(cleaned.contains("outro"));
        assert!(!cleaned.contains("```"));
    }

    #[test]
    fn strips_bold_with_either_marker() {
        assert_eq!(clean_body("a **bold** word", 200), "a bold word");
        assert_eq!(clean_body("a __bold__ word", 200), "a bold word");
    }

    #[test]
    fn strips_italic_star_without_eating_snake_case() {
        // `_` italics are intentionally not stripped — would mangle
        // snake_case identifiers and code references.
        assert_eq!(clean_body("a *word* here", 200), "a word here");
        assert_eq!(
            clean_body("variable foo_bar untouched", 200),
            "variable foo_bar untouched"
        );
    }

    #[test]
    fn strips_link_syntax_keeping_anchor_text() {
        assert_eq!(
            clean_body("see [the docs](https://example.com) here", 200),
            "see the docs here"
        );
    }

    #[test]
    fn strips_heading_markers_keeps_heading_text() {
        assert_eq!(clean_body("## Heading\nbody", 200), "Heading\nbody");
        assert_eq!(clean_body("# Title", 200), "Title");
    }

    #[test]
    fn collapses_runs_of_blank_lines() {
        assert_eq!(clean_body("a\n\n\n\nb", 200), "a\n\nb");
    }

    #[test]
    fn truncates_at_last_sentence_boundary() {
        let raw =
            "First sentence. Second sentence. Third sentence is long enough that it overflows.";
        let cleaned = clean_body(raw, 35);
        assert!(cleaned.ends_with('…'), "{cleaned:?}");
        assert!(
            cleaned.starts_with("First sentence."),
            "should cut at the first boundary inside budget: {cleaned:?}"
        );
    }

    #[test]
    fn returns_unchanged_when_input_fits_budget() {
        assert_eq!(clean_body("short", 200), "short");
    }

    #[test]
    fn falls_back_to_hard_cap_when_no_boundary_in_budget() {
        let raw = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let cleaned = clean_body(raw, 10);
        assert_eq!(cleaned.chars().count(), 11); // 10 chars + ellipsis
        assert!(cleaned.ends_with('…'));
    }

    #[test]
    fn truncate_does_not_split_utf8() {
        let raw = "日本語日本語日本語日本語日本語日本語"; // 18 chars, 54 bytes
        let cleaned = clean_body(raw, 5);
        assert_eq!(cleaned.chars().count(), 6);
        assert!(cleaned.ends_with('…'));
    }
}
