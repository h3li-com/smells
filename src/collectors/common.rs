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
