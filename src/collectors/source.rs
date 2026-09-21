use super::{
    common::{location_at, observation, primitive_type, simple_names},
    model::SourceModel,
    syntax::code_only,
};
use crate::{evidence::Observation, input::Input, policy::Rule, report::Location};
use regex::Regex;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

fn navigation_expression() -> &'static Regex {
    static EXPRESSION: OnceLock<Regex> = OnceLock::new();
    EXPRESSION.get_or_init(|| {
        Regex::new(r"[A-Za-z_][A-Za-z0-9_]*(?:\??\.[A-Za-z_][A-Za-z0-9_]*(?:\([^()\n]*\))?){1,}")
            .expect("valid built-in navigation expression")
    })
}

fn member_access() -> &'static Regex {
    static ACCESS: OnceLock<Regex> = OnceLock::new();
    ACCESS.get_or_init(|| {
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\??\.")
            .expect("valid built-in member-access expression")
    })
}

fn private_member_access() -> &'static Regex {
    static ACCESS: OnceLock<Regex> = OnceLock::new();
    ACCESS.get_or_init(|| {
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\??\.(?:#|_)[A-Za-z_][A-Za-z0-9_]*")
            .expect("valid built-in private-member expression")
    })
}

pub(super) fn navigation_chains(input: &Input) -> Result<Vec<Observation>, String> {
    let mut observations = Vec::new();
    for (path, source) in &input.files {
        let code = code_only(path, source)?;
        for (line_index, line) in code.lines().enumerate() {
            for candidate in navigation_expression().find_iter(line) {
                let transitions = candidate
                    .as_str()
                    .bytes()
                    .filter(|byte| *byte == b'.')
                    .count();
                observations.push(Observation {
                    symbol: format!("{path}::navigation@{}", line_index + 1),
                    location: Location {
                        path: path.clone(),
                        line: line_index + 1,
                        column: line[..candidate.start()].chars().count() + 1,
                    },
                    related_symbols: Vec::new(),
                    related_locations: Vec::new(),
                    measurements: BTreeMap::from([("transitions".into(), transitions as u64)]),
                    evidence: json!({
                        "collector": "source_navigation_chain_v1",
                        "expression": candidate.as_str(),
                    }),
                });
            }
        }
    }
    Ok(observations)
}

pub(super) fn foreign_accesses(input: &Input) -> Result<Vec<Observation>, String> {
    let mut observations = Vec::new();
    for (path, source) in &input.files {
        let code = code_only(path, source)?;
        let accesses = member_access().captures_iter(&code).collect::<Vec<_>>();
        if accesses.is_empty() {
            continue;
        }
        let foreign = accesses
            .iter()
            .filter(|capture| !matches!(&capture[1], "self" | "cls" | "this" | "Self" | "super"))
            .count();
        let first = accesses
            .iter()
            .find(|capture| !matches!(&capture[1], "self" | "cls" | "this" | "Self" | "super"))
            .unwrap_or(&accesses[0]);
        let start = first.get(0).expect("member access match").start();
        let prefix = &source[..start];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix
            .rsplit_once('\n')
            .map_or(prefix, |(_, suffix)| suffix)
            .chars()
            .count()
            + 1;
        observations.push(Observation {
            symbol: format!("{path}::foreign-member-accesses"),
            location: Location {
                path: path.clone(),
                line,
                column,
            },
            related_symbols: Vec::new(),
            related_locations: Vec::new(),
            measurements: BTreeMap::from([
                ("foreign_accesses".into(), foreign as u64),
                ("total_accesses".into(), accesses.len() as u64),
            ]),
            evidence: json!({
                "collector": "source_member_ownership_v1",
                "owned_receivers": ["self", "cls", "this", "Self", "super"],
            }),
        });
    }
    Ok(observations)
}

fn rust_dependency_contract(
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    let private_fields = model
        .classes
        .iter()
        .flat_map(|class| {
            let mut receiver = String::new();
            for (index, character) in class.name.chars().enumerate() {
                if index > 0 && character.is_ascii_uppercase() {
                    receiver.push('_');
                }
                receiver.push(character.to_ascii_lowercase());
            }
            class
                .fields
                .iter()
                .filter(|field| field.private && simple_names(&field.name).len() == 1)
                .map(move |field| (receiver.clone(), field.name.clone()))
        })
        .collect::<BTreeSet<_>>();
    let mut observations = Vec::new();
    for (path, source) in &input.files {
        let code = code_only(path, source)?;
        let mut matches = Vec::new();
        for (receiver, field) in &private_fields {
            let access = Regex::new(&format!(
                r"\b{}\s*\.\s*{}\b",
                regex::escape(receiver),
                regex::escape(field)
            ))
            .expect("escaped Rust private-field expression");
            matches.extend(access.find_iter(&code).map(|found| found.start()));
        }
        matches.sort_unstable();
        let Some(first) = matches.first() else {
            continue;
        };
        observations.push(observation(
            format!("{path}::foreign-private-field-accesses"),
            location_at(path, source, *first),
            [("forbidden_accesses", matches.len() as u64)],
            json!({
                "collector": "rust_private_field_name_access_v1",
                "scope": "dotted access whose receiver name matches the snake-case private-field owner",
                "private_owner_fields": private_fields,
            }),
        ));
    }
    Ok(observations)
}

pub(super) fn dependency_contract(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    if rule.id.starts_with("rust.") {
        return rust_dependency_contract(model, input);
    }
    let mut observations = Vec::new();
    for (path, source) in &input.files {
        let code = code_only(path, source)?;
        let accesses = private_member_access()
            .captures_iter(&code)
            .filter(|capture| !matches!(&capture[1], "self" | "cls" | "this" | "Self" | "super"))
            .collect::<Vec<_>>();
        let Some(first) = accesses.first() else {
            continue;
        };
        let start = first.get(0).expect("private member match").start();
        let prefix = &source[..start];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix
            .rsplit_once('\n')
            .map_or(prefix, |(_, suffix)| suffix)
            .chars()
            .count()
            + 1;
        observations.push(Observation {
            symbol: format!("{path}::foreign-private-accesses"),
            location: Location {
                path: path.clone(),
                line,
                column,
            },
            related_symbols: Vec::new(),
            related_locations: Vec::new(),
            measurements: BTreeMap::from([("forbidden_accesses".into(), accesses.len() as u64)]),
            evidence: json!({
                "collector": "source_private_member_access_v1",
                "contract": "language_private_naming_or_syntax",
            }),
        });
    }
    Ok(observations)
}

pub(super) fn primitive_slots(rule: &Rule, model: &SourceModel) -> Vec<Observation> {
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    model
        .classes
        .iter()
        .filter(|class| !class.fields.is_empty())
        .map(|class| {
            let raw = class
                .fields
                .iter()
                .filter(|field| primitive_type(language, &field.authored_type))
                .count();
            observation(
                class.symbol.clone(),
                class.location.clone(),
                [
                    ("raw_slots", raw as u64),
                    ("total_slots", class.fields.len() as u64),
                ],
                json!({
                    "collector": "authored_primitive_annotations_v1",
                    "scope": "annotated class slots",
                    "language": language,
                }),
            )
        })
        .collect()
}
