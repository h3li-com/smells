use crate::report::Location;

/// Contract version string for supported `test_suite` frameworks.
pub(crate) fn contract_version(framework: &str) -> Option<&'static str> {
    match framework {
        "pytest" => Some("pytest:test-function-v1"),
        "unittest" => Some("unittest:test-method-v1"),
        "vitest" => Some("vitest:test-block-v1"),
        "jest" => Some("jest:test-block-v1"),
        "playwright" => Some("playwright:test-block-v1"),
        "cargo_test" => Some("cargo-test:test-function-v1"),
        _ => None,
    }
}

/// Classify a declaration inside a declared `test_suite` source file.
///
/// Only canonical test entrypoints receive a declaration role (`test_case` or
/// `test_block`). Helper functions and fixtures return `None` so they retain
/// normal rule enforcement.
pub(crate) fn declaration_role(
    framework: &str,
    source: &str,
    symbol: &str,
    location: &Location,
) -> Option<&'static str> {
    match framework {
        "pytest" | "unittest" => python_test_role(symbol),
        "vitest" | "jest" | "playwright" => typescript_test_role(source, symbol, location),
        "cargo_test" => rust_test_role(source, location),
        _ => None,
    }
}

fn leaf_symbol(symbol: &str) -> &str {
    symbol
        .rsplit("::")
        .next()
        .and_then(|segment| segment.rsplit('.').next())
        .unwrap_or(symbol)
}

fn python_test_role(symbol: &str) -> Option<&'static str> {
    leaf_symbol(symbol)
        .strip_prefix("test_")
        .filter(|suffix| !suffix.is_empty())
        .map(|_| "test_case")
}

fn typescript_test_role(source: &str, symbol: &str, location: &Location) -> Option<&'static str> {
    if symbol != "<anonymous>" || location.line == 0 {
        return None;
    }
    let lines: Vec<&str> = source.lines().collect();
    let index = location.line.checked_sub(1)?;
    let current = lines.get(index)?.trim_start();
    if let Some(role) = classify_js_test_call(current) {
        return Some(role);
    }
    let previous = index
        .checked_sub(1)
        .and_then(|prev| lines.get(prev))?
        .trim();
    if previous.ends_with('{') || previous.ends_with(';') {
        return None;
    }
    classify_js_test_call(previous)
}

fn classify_js_test_call(context: &str) -> Option<&'static str> {
    if contains_call_prefix(context, "describe") || contains_call_prefix(context, "test.describe") {
        Some("test_block")
    } else if contains_call_prefix(context, "it") || contains_call_prefix(context, "test") {
        Some("test_case")
    } else {
        None
    }
}

fn contains_call_prefix(context: &str, name: &str) -> bool {
    let bytes = context.as_bytes();
    for (start, _) in context.match_indices(name) {
        let before_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        let after = start + name.len();
        let after_ok = matches!(bytes.get(after), Some(b'(' | b'.' | b' ' | b'\t'));
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn rust_test_role(source: &str, location: &Location) -> Option<&'static str> {
    let lines: Vec<&str> = source.lines().collect();
    let index = location.line.checked_sub(1)?;
    if line_has_rust_test_attr(lines.get(index)?) {
        return Some("test_case");
    }
    for line in lines[..index].iter().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if line_has_rust_test_attr(trimmed) {
            return Some("test_case");
        }
        if !trimmed.starts_with("#[") {
            break;
        }
    }
    None
}

fn line_has_rust_test_attr(line: &str) -> bool {
    line.contains("#[test]")
        || line.contains("::test]")
        || line.contains("::test(")
        || line.contains("#[rstest]")
        || line.contains("#[rstest(")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(line: usize) -> Location {
        Location {
            path: "test_file".into(),
            line,
            column: 1,
        }
    }

    #[test]
    fn contract_versions_cover_every_supported_test_framework() {
        assert_eq!(contract_version("pytest"), Some("pytest:test-function-v1"));
        assert_eq!(
            contract_version("unittest"),
            Some("unittest:test-method-v1")
        );
        assert_eq!(contract_version("vitest"), Some("vitest:test-block-v1"));
        assert_eq!(contract_version("jest"), Some("jest:test-block-v1"));
        assert_eq!(
            contract_version("playwright"),
            Some("playwright:test-block-v1")
        );
        assert_eq!(
            contract_version("cargo_test"),
            Some("cargo-test:test-function-v1")
        );
        assert_eq!(contract_version("unknown"), None);
    }

    #[test]
    fn python_classifies_only_nonempty_test_prefixed_callables() {
        assert_eq!(
            declaration_role("pytest", "", "tests/test_api.py::test_login", &loc(1)),
            Some("test_case")
        );
        assert_eq!(
            declaration_role("unittest", "", "TestApi.test_logout", &loc(1)),
            Some("test_case")
        );
        assert_eq!(
            declaration_role("pytest", "", "tests/test_api.py::test_", &loc(1)),
            None
        );
        assert_eq!(
            declaration_role("pytest", "", "tests/test_api.py::build_fixture", &loc(1)),
            None
        );
    }

    #[test]
    fn typescript_classifies_describe_it_and_test_callbacks_only() {
        let src = "describe('sidebar', () => {\n  it('renders', async () => {\n    expect(1).toBe(1);\n  });\n});\nfunction helper() {\n  return 1;\n}\n";
        assert_eq!(
            declaration_role("vitest", src, "<anonymous>", &loc(1)),
            Some("test_block")
        );
        assert_eq!(
            declaration_role("jest", src, "<anonymous>", &loc(2)),
            Some("test_case")
        );
        assert_eq!(declaration_role("vitest", src, "helper", &loc(6)), None);
        assert_eq!(
            declaration_role("vitest", src, "<anonymous>", &loc(0)),
            None
        );

        let playwright = "test.describe('flow', () => {\n  const inner = [1].map(() => 2);\n  test(\n    async () => {},\n  );\n});\nconst latest = () => {};\n";
        assert_eq!(
            declaration_role("playwright", playwright, "<anonymous>", &loc(1)),
            Some("test_block")
        );
        assert_eq!(
            declaration_role("playwright", playwright, "<anonymous>", &loc(2)),
            None
        );
        assert_eq!(
            declaration_role("playwright", playwright, "<anonymous>", &loc(4)),
            Some("test_case")
        );
        assert_eq!(
            declaration_role("playwright", playwright, "<anonymous>", &loc(7)),
            None
        );
    }

    #[test]
    fn typescript_call_prefixes_respect_identifier_boundaries() {
        assert_eq!(
            classify_js_test_call("await test('x', async () => {"),
            Some("test_case")
        );
        assert_eq!(
            classify_js_test_call("void it('x', () => {"),
            Some("test_case")
        );
        assert_eq!(classify_js_test_call("$test('x', () => {"), None);
        assert_eq!(classify_js_test_call("my_test('x', () => {"), None);
        assert_eq!(classify_js_test_call("latest('x', () => {"), None);
        assert!(is_ident_byte(b'_'));
        assert!(is_ident_byte(b'$'));
        assert!(is_ident_byte(b'a'));
        assert!(!is_ident_byte(b' '));
        assert!(!is_ident_byte(b'('));
    }

    #[test]
    fn typescript_named_declarations_on_test_lines_are_not_test_callbacks() {
        let src = "describe('sidebar', () => {\n  it('renders', function named() {});\n});\n";
        assert_eq!(declaration_role("vitest", src, "named", &loc(2)), None);
        assert_eq!(
            declaration_role("vitest", src, "<anonymous>", &loc(2)),
            Some("test_case")
        );
    }

    #[test]
    fn rust_test_attribute_detection_accepts_each_supported_form() {
        for line in [
            "#[test]",
            "#[tokio::test]",
            "#[tokio::test(flavor = \"multi_thread\")]",
            "#[rstest]",
            "#[rstest(case)]",
        ] {
            assert!(line_has_rust_test_attr(line), "{line}");
        }
        assert_eq!(
            declaration_role(
                "cargo_test",
                "#[test]\n// note\nfn noted() {}",
                "noted",
                &loc(3)
            ),
            Some("test_case")
        );
        assert_eq!(
            declaration_role("cargo_test", "#[test]\n\nfn spaced() {}", "spaced", &loc(3)),
            Some("test_case")
        );
        assert!(!line_has_rust_test_attr("#[allow(dead_code)]"));
        assert!(!line_has_rust_test_attr("fn test() {}"));
    }

    #[test]
    fn rust_classifies_attributed_test_functions_and_ignores_helpers() {
        let src = "fn helper() {\n    assert!(true);\n}\n\n#[test]\n#[allow(dead_code)]\nfn verifies_behavior() {\n    helper();\n}\n\n#[tokio::test(flavor = \"multi_thread\")]\nasync fn async_case() {}\n";
        assert_eq!(declaration_role("cargo_test", src, "helper", &loc(1)), None);
        assert_eq!(
            declaration_role("cargo_test", src, "verifies_behavior", &loc(7)),
            Some("test_case")
        );
        assert_eq!(
            declaration_role("cargo_test", src, "async_case", &loc(12)),
            Some("test_case")
        );
        assert_eq!(
            declaration_role("cargo_test", "#[test] fn inline() {}", "inline", &loc(1)),
            Some("test_case")
        );
        assert_eq!(declaration_role("unknown", src, "fn", &loc(1)), None);
    }
}
