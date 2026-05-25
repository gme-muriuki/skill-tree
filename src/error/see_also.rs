//! Warnings emitted by the see-also metadata-table parser.
//!
//! Per `md/design/see-also.md`, parse failures are warnings — not errors:
//! an author's typo never blocks the render of an otherwise-valid
//! project. The graph layer turns each warning into one `eprintln!` line.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SeeAlsoWarning {
    /// Row's value cell did not parse as an issue reference. Carries the
    /// raw cell content for the warning text.
    #[error("unparseable see-also reference {0:?}")]
    UnparseableRef(String),

    /// Row matched the `See also` label but the value cell was empty.
    #[error("see-also row has empty value")]
    MissingValue,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unparseable_ref_displays_value() {
        let w = SeeAlsoWarning::UnparseableRef("xyz".to_owned());
        assert_eq!(w.to_string(), r#"unparseable see-also reference "xyz""#);
    }

    #[test]
    fn missing_value_displays_static_text() {
        let w = SeeAlsoWarning::MissingValue;
        assert_eq!(w.to_string(), "see-also row has empty value");
    }
}
