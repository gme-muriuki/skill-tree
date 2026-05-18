//! Per-node color derivations and cross-reference edge coloring.
//!
//! See `md/design/node-display.md` for the chosen luma formula and the
//! border-darkening factor. See `md/design/render.md` for the
//! per-source cross-reference palette.

/// Qualitative palette for cross-reference edges. Tableau-10 inspired —
/// 10 hues chosen for categorical distinction on a light background.
/// Edges are grouped by source issue, so each source draws all its
/// outgoing cross-refs in one color and the eye can trace them.
const CROSS_REF_PALETTE: &[&str] = &[
    "#4e79a7", "#f28e2c", "#e15759", "#76b7b2", "#59a14f", "#edc948", "#b07aa1", "#ff9da7",
    "#9c755f", "#bab0ac",
];

/// Pick a stable color for a cross-reference based on its source. Same
/// source string always picks the same color across runs (determinism)
/// and across edges from that source (visual grouping).
pub(super) fn cross_ref_color(source: &str) -> &'static str {
    let hash = source.bytes().fold(0u32, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(u32::from(b))
    });
    CROSS_REF_PALETTE[(hash as usize) % CROSS_REF_PALETTE.len()]
}

/// Parse `#RRGGBB` (case-insensitive). Returns `None` on malformed input
/// so callers can fall back to a sensible default.
pub(super) fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let body = s.strip_prefix('#')?;
    if body.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&body[0..2], 16).ok()?;
    let g = u8::from_str_radix(&body[2..4], 16).ok()?;
    let b = u8::from_str_radix(&body[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Pick `black` or `white` for legible text on a `fill_hex` background
/// using ITU-R BT.601 luma. Falls back to `black` (the safer default for
/// our light chrome) when `fill_hex` is unparseable.
pub(super) fn pick_text_color(fill_hex: &str) -> &'static str {
    let Some((r, g, b)) = parse_hex(fill_hex) else {
        return "black";
    };
    let luma = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    if luma > 128.0 { "black" } else { "white" }
}

/// Multiply each RGB channel of `fill_hex` by `factor` (clamped to
/// `[0.0, 1.0]`) and return the result as `#rrggbb`. Used to derive a
/// border tone slightly darker than the fill. Unparseable input is
/// returned unchanged so callers do not have to special-case it.
pub(super) fn darken_hex(fill_hex: &str, factor: f32) -> String {
    let Some((r, g, b)) = parse_hex(fill_hex) else {
        return fill_hex.to_owned();
    };
    let factor = factor.clamp(0.0, 1.0);
    let dim = |c: u8| (f32::from(c) * factor).round() as u8;
    format!("#{:02x}{:02x}{:02x}", dim(r), dim(g), dim(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_accepts_lower_and_upper() {
        assert_eq!(parse_hex("#abcdef"), Some((0xAB, 0xCD, 0xEF)));
        assert_eq!(parse_hex("#ABCDEF"), Some((0xAB, 0xCD, 0xEF)));
    }

    #[test]
    fn parse_hex_rejects_malformed() {
        assert_eq!(parse_hex("abcdef"), None);
        assert_eq!(parse_hex("#abc"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn pick_text_color_picks_black_on_light_fills() {
        assert_eq!(pick_text_color("#ffffff"), "black");
        assert_eq!(pick_text_color("#d9b04a"), "black");
    }

    #[test]
    fn pick_text_color_picks_white_on_dark_fills() {
        assert_eq!(pick_text_color("#000000"), "white");
        assert_eq!(pick_text_color("#005599"), "white");
    }

    #[test]
    fn pick_text_color_defaults_to_black_on_garbage() {
        assert_eq!(pick_text_color("not a color"), "black");
    }

    #[test]
    fn darken_hex_scales_each_channel() {
        assert_eq!(darken_hex("#ffffff", 0.5), "#808080");
        assert_eq!(darken_hex("#57a85a", 0.8), "#468648");
    }

    #[test]
    fn darken_hex_clamps_factor() {
        assert_eq!(darken_hex("#ffffff", 1.5), "#ffffff");
        assert_eq!(darken_hex("#ffffff", -1.0), "#000000");
    }

    #[test]
    fn darken_hex_returns_input_unchanged_on_garbage() {
        assert_eq!(darken_hex("garbage", 0.5), "garbage");
    }

    #[test]
    fn cross_ref_color_is_deterministic_per_source() {
        let a = cross_ref_color("rust-lang/a-mir-formality#265");
        let b = cross_ref_color("rust-lang/a-mir-formality#265");
        assert_eq!(a, b);
    }

    #[test]
    fn cross_ref_color_distinguishes_different_sources() {
        let a = cross_ref_color("o/r#1");
        let b = cross_ref_color("o/r#2");
        let c = cross_ref_color("o/r#3");
        // Not a strict guarantee for any specific pair, but on three
        // simple inputs the palette is large enough that at least two
        // should differ — catches a regression that collapses to one
        // color.
        assert!(
            a != b || b != c || a != c,
            "expected some variation across sources"
        );
    }

    #[test]
    fn cross_ref_color_always_in_palette() {
        for source in ["x", "y", "z", "rust-lang/foo#1", "rust-lang/bar#999"] {
            let picked = cross_ref_color(source);
            assert!(
                CROSS_REF_PALETTE.contains(&picked),
                "picked color {picked} not in palette"
            );
        }
    }
}
