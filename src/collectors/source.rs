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
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\??\s*\.\s*[A-Za-z_]")
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

fn navigation_hop() -> &'static Regex {
    static HOP: OnceLock<Regex> = OnceLock::new();
    HOP.get_or_init(|| {
        Regex::new(r"\??\.([A-Za-z_][A-Za-z0-9_]*)(\([^()\n]*\))?")
            .expect("valid built-in navigation hop expression")
    })
}

/// Standard-library adapters and conversions reshape a value without reaching
/// into another object, so they neither extend nor break a navigation run.
fn navigation_adapters(language: &str) -> &'static [&'static str] {
    match language {
        "rust" => &[
            "iter",
            "iter_mut",
            "into_iter",
            "clone",
            "cloned",
            "copied",
            "collect",
            "unwrap",
            "expect",
            "as_ref",
            "as_mut",
            "as_str",
            "as_slice",
            "as_deref",
            "as_bytes",
            "to_string",
            "to_owned",
            "to_vec",
            "into",
            "len",
            "is_empty",
            "borrow",
            "borrow_mut",
            "keys",
            "values",
            "rev",
            "chars",
            "bytes",
            "lines",
            "trim",
            "count",
            "sum",
            "first",
            "last",
            "unwrap_or_default",
            "ok",
            "some",
            "flatten",
            "peekable",
            "enumerate",
            "display",
        ],
        "python" => &[
            "items",
            "keys",
            "values",
            "copy",
            "strip",
            "lstrip",
            "rstrip",
            "lower",
            "upper",
            "encode",
            "decode",
            "split",
            "splitlines",
            "casefold",
            "title",
        ],
        "typescript" => &[
            "toString",
            "valueOf",
            "trim",
            "toLowerCase",
            "toUpperCase",
            "keys",
            "values",
            "entries",
            "length",
            "slice",
            "flat",
            "reverse",
            "toJSON",
        ],
        _ => &[],
    }
}

const NAVIGATION_SELF_ROOTS: [&str; 3] = ["self", "this", "Self"];

/// The longest run of consecutive navigation hops: field reads and zero-argument
/// calls. A call that takes arguments starts a new run, adapters are transparent,
/// and the first hop from the method's own receiver reads owned state.
fn navigation_run(expression: &str, adapters: &[&str]) -> u64 {
    let root_end = expression.find(['.', '?']).unwrap_or(expression.len());
    let mut skip_first = NAVIGATION_SELF_ROOTS.contains(&&expression[..root_end]);
    let (mut run, mut longest) = (0_u64, 0_u64);
    for hop in navigation_hop().captures_iter(&expression[root_end..]) {
        if std::mem::take(&mut skip_first) {
            continue;
        }
        let name = hop.get(1).expect("hop name group").as_str();
        let arguments = hop.get(2).map_or("()", |arguments| arguments.as_str());
        if arguments != "()" {
            run = 0;
        } else if !adapters.contains(&name) {
            run += 1;
            longest = longest.max(run);
        }
    }
    longest
}

pub(super) fn visit_navigation_chains(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
    mut emit: impl FnMut(Observation),
) -> Result<(), String> {
    let language = rule.id.split('.').next().unwrap_or_default();
    let adapters = navigation_adapters(language);
    for path in input.files.keys() {
        let code = model
            .code_by_path
            .get(path)
            .ok_or_else(|| format!("missing authored code for {path}"))?;
        for (line_index, line) in code.lines().enumerate() {
            for candidate in navigation_expression().find_iter(line) {
                let transitions = navigation_run(candidate.as_str(), adapters);
                emit(Observation {
                    symbol: format!("{path}::navigation@{}", line_index + 1),
                    location: Location {
                        path: path.clone(),
                        line: line_index + 1,
                        column: line[..candidate.start()].chars().count() + 1,
                    },
                    related_symbols: Vec::new(),
                    related_locations: Vec::new(),
                    measurements: BTreeMap::from([("transitions".into(), transitions)]),
                    evidence: json!({
                        "collector": "source_navigation_chain_v2",
                        "expression": candidate.as_str(),
                    }),
                });
            }
        }
    }
    Ok(())
}

const OWNED_RECEIVERS: [&str; 5] = ["self", "cls", "this", "Self", "super"];

/// Feature Envy is a property of a method: compare its accesses through foreign
/// receivers with those through its own. Free functions have no own data.
pub(super) fn foreign_accesses(model: &SourceModel) -> Vec<Observation> {
    model
        .functions
        .iter()
        .filter(|function| function.receiver)
        .filter_map(|function| {
            let body = function.body.as_deref()?;
            let receivers = member_access()
                .captures_iter(body)
                .map(|capture| capture.get(1).expect("receiver group").as_str())
                .collect::<Vec<_>>();
            if receivers.is_empty() {
                return None;
            }
            let foreign = receivers
                .iter()
                .filter(|receiver| !OWNED_RECEIVERS.contains(receiver))
                .count();
            Some(observation(
                function.symbol.clone(),
                function.location.clone(),
                [
                    ("foreign_accesses", foreign as u64),
                    ("total_accesses", receivers.len() as u64),
                ],
                json!({
                    "collector": "source_method_member_ownership_v2",
                    "owned_receivers": OWNED_RECEIVERS,
                }),
            ))
        })
        .collect()
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
            policy_sha256: String::new(),
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
    fn navigation_chains_count_the_longest_run_of_navigation_hops() {
        let source = concat!(
            "let value = root.first().second.third;\n",
            "  other?.child.run();\n",
            "  a.b.c.find(x).d.e;\n",
            "  self.config.items.iter().cloned().last_seen;\n",
            "  this.a.b.c;\n",
        );
        let input = input("typescript-v1", &[("sample.ts", source)]);
        let model = model_with_code("sample.ts", source);
        let mut observations = Vec::new();
        visit_navigation_chains(
            &rule("typescript.navigation_chains"),
            &model,
            &input,
            |observation| observations.push(observation),
        )
        .unwrap();

        let measured = observations
            .iter()
            .map(|observation| {
                (
                    observation.location.line,
                    observation.location.column,
                    observation.measurements["transitions"],
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            measured,
            [(1, 13, 3), (2, 3, 2), (3, 3, 2), (4, 3, 4), (5, 3, 2)]
        );
        assert_eq!(
            navigation_run(
                "self.config.items.iter().cloned().last_seen",
                navigation_adapters("rust")
            ),
            2
        );
        assert_eq!(navigation_run("Self.a.b.c.d", &[]), 3);
    }

    #[test]
    fn foreign_accesses_measure_each_receiver_method_against_its_own_accesses() {
        let source = concat!(
            "class Service:\n",
            "    def envy(self, order):\n",
            "        return order.a + order.b + order.c + self.x\n",
            "    def own(self):\n",
            "        return self.x + cls.y + super.z\n",
            "    @staticmethod\n",
            "    def helper(order):\n",
            "        return order.a + order.b\n",
            "def free(order):\n",
            "    return order.a + order.b\n",
        );
        let model = super::super::model::source_model(
            &rule("python.foreign_accesses"),
            &input("python-v1", &[("sample.py", source)]),
        )
        .unwrap();
        let observations = foreign_accesses(&model);
        let measured = observations
            .iter()
            .map(|observation| {
                (
                    observation.symbol.as_str(),
                    observation.location.line,
                    observation.measurements["foreign_accesses"],
                    observation.measurements["total_accesses"],
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            measured,
            [
                ("sample.py::Service.envy", 2, 3, 4),
                ("sample.py::Service.own", 4, 0, 3),
            ]
        );
    }

    #[test]
    fn foreign_accesses_read_token_stream_rust_method_bodies() {
        let source = concat!(
            "pub struct S { x: u8 }\n",
            "impl S {\n",
            "    pub fn envy(&self, o: &O) -> u8 { o.a + o.b + o.c + self.x + (0..o.d).len() as u8 }\n",
            "    pub fn new() -> Self { S { x: 0 } }\n",
            "}\n",
        );
        let model = super::super::model::source_model(
            &rule("rust.foreign_accesses"),
            &input("rust-v1", &[("src/lib.rs", source)]),
        )
        .unwrap();
        let observations = foreign_accesses(&model);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].symbol, "src/lib.rs::S.envy");
        assert_eq!(observations[0].measurements["foreign_accesses"], 4);
        assert_eq!(observations[0].measurements["total_accesses"], 5);
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
