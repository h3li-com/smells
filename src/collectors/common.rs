use crate::{evidence::Observation, report::Location};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

pub(super) fn primitive_type(language: &str, authored_type: &str) -> bool {
    let authored_type = authored_type.trim().replace(' ', "");
    match language {
        "python" => matches!(
            authored_type.as_str(),
            "str" | "int" | "float" | "bool" | "bytes"
        ),
        "typescript" => matches!(
            authored_type.as_str(),
            "string" | "number" | "boolean" | "bigint" | "symbol"
        ),
        "rust" => matches!(
            authored_type.as_str(),
            "bool"
                | "char"
                | "str"
                | "String"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
        ),
        _ => false,
    }
}

pub(super) fn location_at(path: &str, source: &str, offset: usize) -> Location {
    let prefix = &source[..offset];
    Location {
        path: path.into(),
        line: prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
        column: prefix
            .rsplit_once('\n')
            .map_or(prefix, |(_, suffix)| suffix)
            .chars()
            .count()
            + 1,
    }
}

pub(super) fn simple_names(value: &str) -> Vec<String> {
    static NAME: OnceLock<Regex> = OnceLock::new();
    let name = NAME.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").expect("valid name"));
    name.find_iter(value)
        .map(|matched| matched.as_str().to_string())
        .collect()
}

pub(super) fn observation(
    symbol: String,
    location: Location,
    measurements: impl IntoIterator<Item = (&'static str, u64)>,
    evidence: Value,
) -> Observation {
    Observation {
        symbol,
        location,
        related_symbols: Vec::new(),
        related_locations: Vec::new(),
        measurements: measurements
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn primitive_types_are_language_specific_and_do_not_accept_wrappers() {
        for (language, accepted) in [
            (
                "python",
                ["str", "int", "float", "bool", "bytes"].as_slice(),
            ),
            (
                "typescript",
                ["string", "number", "boolean", "bigint", "symbol"].as_slice(),
            ),
            (
                "rust",
                ["bool", "char", "String", "i32", "u64", "usize", "f64"].as_slice(),
            ),
        ] {
            for authored in accepted {
                assert!(primitive_type(language, authored), "{language}: {authored}");
            }
        }
        for (language, rejected) in [
            ("python", "Optional[str]"),
            ("typescript", "string | null"),
            ("rust", "Option<String>"),
            ("unknown", "string"),
        ] {
            assert!(!primitive_type(language, rejected));
        }
    }

    #[test]
    fn source_locations_are_one_based_and_unicode_columns_count_characters() {
        let source = "αβ\nvalue";
        assert_eq!(location_at("sample.py", source, 0).line, 1);
        assert_eq!(location_at("sample.py", source, 0).column, 1);
        assert_eq!(location_at("sample.py", source, "α".len()).column, 2);
        let second_line = location_at("sample.py", source, "αβ\n".len());
        assert_eq!(second_line.line, 2);
        assert_eq!(second_line.column, 1);
    }

    #[test]
    fn observations_keep_measurement_and_evidence_contracts() {
        let location = Location {
            path: "sample.rs".into(),
            line: 2,
            column: 3,
        };
        let observed = observation(
            "sample::symbol".into(),
            location.clone(),
            [("count", 2), ("total", 3)],
            json!({"collector": "test"}),
        );
        assert_eq!(observed.symbol, "sample::symbol");
        assert_eq!(observed.location.path, location.path);
        assert_eq!(observed.location.line, location.line);
        assert_eq!(observed.location.column, location.column);
        assert_eq!(observed.measurements["count"], 2);
        assert_eq!(observed.measurements["total"], 3);
        assert_eq!(observed.evidence["collector"], "test");
        assert!(observed.related_symbols.is_empty());
        assert_eq!(
            simple_names("crate::domain.Account<T>"),
            ["crate", "domain", "Account", "T"]
        );
    }
}
