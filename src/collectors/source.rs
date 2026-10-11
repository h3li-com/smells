use super::{
    common::{location_at, observation, primitive_type, simple_names},
    model::SourceModel,
};
use crate::{core::model::Input, evidence::Observation, policy::Rule, report::Location};
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

pub(super) fn visit_navigation_chains(
    model: &SourceModel,
    input: &Input,
    mut emit: impl FnMut(Observation),
) -> Result<(), String> {
    for path in input.files.keys() {
        let code = model
            .code_by_path
            .get(path)
            .ok_or_else(|| format!("missing authored code for {path}"))?;
        for (line_index, line) in code.lines().enumerate() {
            for candidate in navigation_expression().find_iter(line) {
                let transitions = candidate
                    .as_str()
                    .bytes()
                    .filter(|byte| *byte == b'.')
                    .count();
                emit(Observation {
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
    Ok(())
}

pub(super) fn foreign_accesses(
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    let mut observations = Vec::new();
    for (path, source) in &input.files {
        let code = model
            .code_by_path
            .get(path)
            .ok_or_else(|| format!("missing authored code for {path}"))?;
        let accesses = member_access().captures_iter(code).collect::<Vec<_>>();
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
        let code = model
            .code_by_path
            .get(path)
            .ok_or_else(|| format!("missing authored code for {path}"))?;
        let mut matches = Vec::new();
        for (receiver, field) in &private_fields {
            let access = Regex::new(&format!(
                r"\b{}\s*\.\s*{}\b",
                regex::escape(receiver),
                regex::escape(field)
            ))
            .expect("escaped Rust private-field expression");
            matches.extend(access.find_iter(code).map(|found| found.start()));
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
        let code = model
            .code_by_path
            .get(path)
            .ok_or_else(|| format!("missing authored code for {path}"))?;
        let accesses = private_member_access()
            .captures_iter(code)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        collectors::model::{ClassFact, FieldFact},
        core::model::History,
        policy::{Limits, Policy, ResolvedSelection},
    };
    use std::sync::Arc;

    fn rule(id: &str) -> Rule {
        Rule {
            id: id.into(),
            version: 1,
            smell: "test".into(),
            kind: "threshold".into(),
            implementation: "test".into(),
            inputs: Vec::new(),
            parameters: BTreeMap::new(),
            contract: "test".into(),
        }
    }

    fn input(rule_pack: &str, files: &[(&str, &str)]) -> Input {
        Input {
            files: files
                .iter()
                .map(|(path, source)| ((*path).into(), (*source).into()))
                .collect(),
            manifests: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            artifact_candidates: BTreeMap::new(),
            implementations: Vec::new(),
            source_roles: Vec::new(),
            history: History::default(),
            policy: Policy {
                schema_version: 1,
                rule_pack: rule_pack.into(),
                scanner_version: env!("CARGO_PKG_VERSION").into(),
                scope: "repository".into(),
                default_groups: vec!["all".into()],
                exclude_directories: Vec::new(),
                source_roles: Vec::new(),
                limits: Limits {
                    maximum_files: 10,
                    maximum_pairs: 10,
                    maximum_group_combinations: 10,
                },
                rules: BTreeMap::new(),
                exceptions: Vec::new(),
                resolved: ResolvedSelection::default(),
            },
            digest: "0".repeat(64),
            mode: "test",
        }
    }

    fn model_with_code(path: &str, source: &str) -> SourceModel {
        SourceModel {
            code_by_path: BTreeMap::from([(path.into(), Arc::from(source))]),
            ..SourceModel::default()
        }
    }

    #[test]
    fn navigation_chains_count_transitions_and_preserve_one_based_locations() {
        let source = "let value = root.first().second.third;\n  other?.child.run();\n";
        let input = input("typescript-v1", &[("sample.ts", source)]);
        let model = model_with_code("sample.ts", source);
        let mut observations = Vec::new();
        visit_navigation_chains(&model, &input, |observation| observations.push(observation))
            .unwrap();

        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].measurements["transitions"], 3);
        assert_eq!(observations[0].location.line, 1);
        assert_eq!(observations[0].location.column, 13);
        assert_eq!(observations[1].measurements["transitions"], 2);
        assert_eq!(observations[1].location.line, 2);
        assert_eq!(observations[1].location.column, 3);
    }

    #[test]
    fn foreign_accesses_exclude_each_owned_receiver_but_keep_total_accesses() {
        let source = concat!(
            "self.owned\n",
            "service.remote\n",
            "this.owned\n",
            "object.other\n",
            "super.owned\n",
        );
        let observations = foreign_accesses(
            &model_with_code("sample.py", source),
            &input("python-v1", &[("sample.py", source)]),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["foreign_accesses"], 2);
        assert_eq!(observations[0].measurements["total_accesses"], 5);
        assert_eq!(observations[0].location.line, 2);
        assert_eq!(observations[0].location.column, 1);
    }

    #[test]
    fn dependency_contract_counts_only_foreign_private_accesses() {
        let source = concat!(
            "self._owned\n",
            "service._token\n",
            "service._other\n",
            "this.#owned\n",
        );
        let observations = dependency_contract(
            &rule("python.dependency_contract"),
            &model_with_code("sample.py", source),
            &input("python-v1", &[("sample.py", source)]),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["forbidden_accesses"], 2);
        assert_eq!(observations[0].location.line, 2);
        assert_eq!(observations[0].location.column, 1);
    }

    #[test]
    fn rust_dependency_contract_uses_snake_case_private_field_owners() {
        let source = concat!(
            "fn inspect(user_account: UserAccount, other: UserAccount) {\n",
            "  let first = user_account.secret;\n",
            "  let second = other.secret;\n",
            "  let third = user_account.secret;\n",
            "  let public = user_account.public_value;\n",
            "  let nested = user_account.nested.secret;\n",
            "}\n",
        );
        let mut model = model_with_code("src/check.rs", source);
        model.classes.push(ClassFact {
            name: "UserAccount".into(),
            symbol: "src/domain.rs::UserAccount".into(),
            location: Location {
                path: "src/domain.rs".into(),
                line: 1,
                column: 1,
            },
            type_parameters: Vec::new(),
            source: Arc::from(""),
            bases: Vec::new(),
            interface: false,
            private: false,
            fields: vec![
                FieldFact {
                    name: "secret".into(),
                    authored_type: "String".into(),
                    temporary: false,
                    private: true,
                },
                FieldFact {
                    name: "public_value".into(),
                    authored_type: "String".into(),
                    temporary: false,
                    private: false,
                },
                FieldFact {
                    name: "nested.secret".into(),
                    authored_type: "String".into(),
                    temporary: false,
                    private: true,
                },
            ],
            methods: Vec::new(),
            range: (0, 0),
        });
        let observations = dependency_contract(
            &rule("rust.dependency_contract"),
            &model,
            &input("rust-v1", &[("src/check.rs", source)]),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["forbidden_accesses"], 2);
        assert_eq!(observations[0].location.line, 2);
        assert_eq!(observations[0].location.column, 15);
    }
}
