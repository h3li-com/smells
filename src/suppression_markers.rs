//! Comment markers that tell a linter, formatter, type checker or coverage tool
//! to skip code. Both history capture and source analysis read them, so they
//! live apart from either.

/// Comment markers that suppress a linter, formatter, type checker or coverage
/// tool for the line or block they annotate.
pub(crate) const SUPPRESSION_MARKERS: &[&str] = &[
    "eslint-disable",
    "eslint-enable",
    "oxlint-disable",
    "oxlint-enable",
    "biome-ignore",
    "prettier-ignore",
    "@ts-expect-error",
    "@ts-ignore",
    "@ts-nocheck",
    "istanbul ignore",
    "c8 ignore",
    "noqa",
    "nosec",
    "type: ignore",
    "pyright: ignore",
    "pylint: disable",
    "pylint: enable",
    "pragma: no cover",
    "fmt: off",
    "fmt: on",
    "fmt: skip",
];

/// Whether a comment is a tool directive: after its opener it starts with a
/// suppression marker, such as `// @ts-expect-error why` or `# type: ignore`.
pub(crate) fn directive_comment(comment: &str) -> bool {
    let text = comment
        .trim_start()
        .trim_start_matches(['/', '*', '#', '!'])
        .trim_start()
        .to_ascii_lowercase();
    SUPPRESSION_MARKERS
        .iter()
        .any(|marker| text.starts_with(marker))
}

#[cfg(test)]
mod tests {
    use super::directive_comment;

    #[test]
    fn directive_comments_start_with_a_suppression_marker() {
        for comment in [
            "// @ts-expect-error wrong overload",
            "/* eslint-disable no-console */",
            "# type: ignore[attr-defined]",
            "#noqa: E501",
            "// prettier-ignore",
        ] {
            assert!(directive_comment(comment), "{comment}");
        }
        for comment in [
            "// explain why",
            "// see eslint-disable above",
            "#: doc",
            "",
        ] {
            assert!(!directive_comment(comment), "{comment}");
        }
    }
}
