use super::{
    common::observation,
    model::{FunctionFact, SourceModel},
};
use crate::{core::model::Input, evidence::Observation, policy::Rule, report::Location};
use regex::Regex;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

type SimilarityFeature = (String, u64);

fn normalized_tokens(source: &str) -> BTreeMap<String, u64> {
    static TOKEN: OnceLock<Regex> = OnceLock::new();
    let token = TOKEN.get_or_init(|| {
        Regex::new(r#"[A-Za-z_][A-Za-z0-9_]*|\d+(?:\.\d+)?|==|!=|<=|>=|=>|&&|\|\||[^\s]"#)
            .expect("valid normalized-token expression")
    });
    let keywords = [
        "if", "else", "elif", "for", "while", "match", "case", "switch", "return", "throw",
        "raise", "await", "yield", "and", "or", "not", "in", "is", "new",
    ];
    let mut result = BTreeMap::new();
    for found in token.find_iter(source) {
        let value = found.as_str();
        let normalized = if value.chars().next().is_some_and(char::is_alphabetic) {
            if keywords.contains(&value) {
                value
            } else {
                "identifier"
            }
        } else if value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        {
            "number"
        } else {
            value
        };
        *result.entry(normalized.into()).or_insert(0) += 1;
    }
    result
}

fn token_similarity(first: &BTreeMap<String, u64>, second: &BTreeMap<String, u64>) -> (u64, u64) {
    let keys = first.keys().chain(second.keys()).collect::<BTreeSet<_>>();
    keys.into_iter().fold((0, 0), |(intersection, union), key| {
        let first = first.get(key).copied().unwrap_or(0);
        let second = second.get(key).copied().unwrap_or(0);
        (intersection + first.min(second), union + first.max(second))
    })
}

fn class_interfaces(model: &SourceModel) -> Vec<BTreeSet<&str>> {
    model
        .classes
        .iter()
        .map(|class| {
            class
                .methods
                .iter()
                .map(|method| method.name.as_str())
                .collect()
        })
        .collect()
}

fn class_token_maps(model: &SourceModel) -> Vec<BTreeMap<String, u64>> {
    model
        .classes
        .iter()
        .map(|class| {
            let bodies = class
                .methods
                .iter()
                .filter_map(|method| method.body.as_deref())
                .collect::<Vec<_>>()
                .join("\n");
            normalized_tokens(&bodies)
        })
        .collect()
}

fn feature_frequencies(token_maps: &[BTreeMap<String, u64>]) -> BTreeMap<SimilarityFeature, usize> {
    let mut frequencies = BTreeMap::new();
    for tokens in token_maps {
        for (token, count) in tokens {
            for occurrence in 0..*count {
                *frequencies.entry((token.clone(), occurrence)).or_default() += 1;
            }
        }
    }
    frequencies
}

fn similarity_prefixes(
    interfaces: &[BTreeSet<&str>],
    token_maps: &[BTreeMap<String, u64>],
    token_counts: &[u64],
    minimum: u64,
    similarity: u64,
) -> Vec<Vec<SimilarityFeature>> {
    let frequencies = feature_frequencies(token_maps);
    token_maps
        .iter()
        .enumerate()
        .map(|(index, tokens)| {
            if interfaces[index].is_empty() || token_counts[index] < minimum {
                return Vec::new();
            }
            let mut features = tokens
                .iter()
                .flat_map(|(token, count)| {
                    (0..*count).map(|occurrence| (token.clone(), occurrence))
                })
                .collect::<Vec<_>>();
            features.sort_by_key(|feature| (frequencies[feature], feature.clone()));
            let required = (similarity as u128 * features.len() as u128).div_ceil(10_000) as usize;
            features.truncate((features.len() - required + 1).min(features.len()));
            features
        })
        .collect()
}

fn candidate_pairs(prefixes: &[Vec<SimilarityFeature>]) -> BTreeSet<(usize, usize)> {
    let mut postings = BTreeMap::<SimilarityFeature, Vec<usize>>::new();
    let mut candidates = BTreeSet::new();
    for (right, prefix) in prefixes.iter().enumerate() {
        for feature in prefix {
            for left in postings.get(feature).into_iter().flatten() {
                candidates.insert((*left, right));
            }
            postings.entry(feature.clone()).or_default().push(right);
        }
    }
    candidates
}

fn lengths_can_match(first: u64, second: u64, similarity: u64) -> bool {
    let (smaller, larger) = if first <= second {
        (first, second)
    } else {
        (second, first)
    };
    smaller as u128 * 10_000 >= similarity as u128 * larger as u128
}

fn interface_observation(
    model: &SourceModel,
    token_maps: &[BTreeMap<String, u64>],
    token_counts: &[u64],
    left: usize,
    right: usize,
) -> Option<Observation> {
    let (intersection, union) = token_similarity(&token_maps[left], &token_maps[right]);
    if union == 0 {
        return None;
    }
    let first = &model.classes[left];
    let second = &model.classes[right];
    let mut candidate = observation(
        first.symbol.clone(),
        first.location.clone(),
        [
            ("first_tokens", token_counts[left]),
            ("second_tokens", token_counts[right]),
            ("intersection", intersection),
            ("union", union),
        ],
        json!({
            "collector": "authored_class_behavior_similarity_v1",
            "normalization": "lexical token multiset; identifiers normalized",
            "candidate_filter": "multiset_jaccard_prefix_and_length_v1",
        }),
    );
    candidate.related_symbols.push(second.symbol.clone());
    candidate.related_locations.push(second.location.clone());
    Some(candidate)
}

pub(super) fn alternative_interfaces(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    let minimum = rule.parameters["minimum_tokens"];
    let similarity = rule.parameters["minimum_similarity_basis_points"];
    let interfaces = class_interfaces(model);
    let token_maps = class_token_maps(model);
    let token_counts = token_maps
        .iter()
        .map(|tokens| tokens.values().sum::<u64>())
        .collect::<Vec<_>>();
    let prefixes =
        similarity_prefixes(&interfaces, &token_maps, &token_counts, minimum, similarity);

    let mut observations = Vec::new();
    let mut pairs = 0usize;
    for (left, right) in candidate_pairs(&prefixes) {
        if interfaces[left] == interfaces[right] {
            continue;
        }
        if !lengths_can_match(token_counts[left], token_counts[right], similarity) {
            continue;
        }
        pairs += 1;
        if pairs > input.policy.limits.maximum_pairs {
            return Err(format!(
                "maximum_pairs budget exceeded: charged {pairs} exact comparisons; policy limit is {}",
                input.policy.limits.maximum_pairs
            ));
        }
        if let Some(candidate) =
            interface_observation(model, &token_maps, &token_counts, left, right)
        {
            observations.push(candidate);
        }
    }
    Ok(observations)
}

fn dispatch_labels(body: &str) -> Vec<String> {
    static CASE: OnceLock<Regex> = OnceLock::new();
    let case = CASE.get_or_init(|| {
        Regex::new(r"(?m)^\s*case\s+([^:\n=]+?)(?:\s*=>|\s*:)")
            .expect("valid dispatch-case expression")
    });
    case.captures_iter(body)
        .map(|capture| capture[1].split_whitespace().collect::<String>())
        .collect()
}

pub(super) fn repeated_dispatch(model: &SourceModel) -> Vec<Observation> {
    let mut grouped: BTreeMap<Vec<String>, Vec<&FunctionFact>> = BTreeMap::new();
    for function in &model.functions {
        let Some(body) = &function.body else {
            continue;
        };
        let labels = dispatch_labels(body);
        if !labels.is_empty() {
            grouped.entry(labels).or_default().push(function);
        }
    }
    grouped
        .into_iter()
        .filter_map(|(labels, functions)| {
            let first = functions.first()?;
            let mut candidate = observation(
                first.symbol.clone(),
                first.location.clone(),
                [
                    ("sites", functions.len() as u64),
                    ("arms_per_site", labels.len() as u64),
                ],
                json!({
                    "collector": "authored_repeated_case_labels_v1",
                    "case_labels": labels,
                }),
            );
            for related in functions.iter().skip(1) {
                candidate.related_symbols.push(related.symbol.clone());
                candidate.related_locations.push(related.location.clone());
            }
            Some(candidate)
        })
        .collect()
}

pub(super) fn temporary_fields(rule: &Rule, model: &SourceModel) -> Vec<Observation> {
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    let receiver = if language == "python" { "self" } else { "this" };
    model
        .classes
        .iter()
        .filter_map(|class| {
            let temporary = class
                .fields
                .iter()
                .filter(|field| field.temporary)
                .collect::<Vec<_>>();
            let methods = class
                .methods
                .iter()
                .filter(|method| method.body.is_some())
                .collect::<Vec<_>>();
            if temporary.is_empty() || methods.is_empty() {
                return None;
            }
            let uses = temporary
                .iter()
                .map(|field| {
                    let access = Regex::new(&format!(
                        r"\b{}\s*\.\s*{}\b",
                        regex::escape(receiver),
                        regex::escape(&field.name)
                    ))
                    .expect("escaped temporary-field access expression");
                    methods
                        .iter()
                        .filter(|method| {
                            method
                                .body
                                .as_deref()
                                .is_some_and(|body| access.is_match(body))
                        })
                        .count()
                })
                .sum::<usize>();
            Some(observation(
                class.symbol.clone(),
                class.location.clone(),
                [
                    ("fields", temporary.len() as u64),
                    ("methods", methods.len() as u64),
                    ("uses", uses as u64),
                    ("possible_uses", (temporary.len() * methods.len()) as u64),
                ],
                json!({
                    "collector": "authored_nullable_field_use_v1",
                    "fields": temporary.iter().map(|field| &field.name).collect::<Vec<_>>(),
                }),
            ))
        })
        .collect()
}

fn forwarding_method(body: &str, language: &str) -> bool {
    let receiver = if language == "python" { "self" } else { "this" };
    let compact = body
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim()
        .trim_end_matches(';')
        .trim();
    let compact = compact.split_whitespace().collect::<Vec<_>>().join(" ");
    let expression = Regex::new(&format!(
        r"(?s)^(?:return\s+)?{receiver}\s*\.\s*[A-Za-z_][A-Za-z0-9_]*\s*\.\s*[A-Za-z_][A-Za-z0-9_]*\([^;{{}}]*\)$"
    ))
    .expect("valid forwarding expression");
    expression.is_match(&compact)
}

pub(super) fn forwarding_share(rule: &Rule, model: &SourceModel) -> Vec<Observation> {
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    model
        .classes
        .iter()
        .filter_map(|class| {
            let methods = class
                .methods
                .iter()
                .filter(|method| {
                    method.body.is_some()
                        && !matches!(method.name.as_str(), "__init__" | "constructor")
                })
                .collect::<Vec<_>>();
            if methods.is_empty() {
                return None;
            }
            let forwarders = methods
                .iter()
                .filter(|method| {
                    method
                        .body
                        .as_deref()
                        .is_some_and(|body| forwarding_method(body, language))
                })
                .count();
            Some(observation(
                class.symbol.clone(),
                class.location.clone(),
                [
                    ("methods", methods.len() as u64),
                    ("forwarders", forwarders as u64),
                ],
                json!({"collector": "authored_single_delegate_method_v1"}),
            ))
        })
        .collect()
}

fn complexity(body: &str, language: &str) -> u64 {
    static DECISION: OnceLock<Regex> = OnceLock::new();
    let decision = DECISION.get_or_init(|| {
        Regex::new(r"\b(?:if|elif|for|while|case|catch)\b|&&|\|\|")
            .expect("valid complexity expression")
    });
    let rust_match_arms = if language == "rust" {
        body.match_indices("=>").count() as u64
    } else {
        0
    };
    1 + decision.find_iter(body).count() as u64 + rust_match_arms
}

pub(super) fn function_crap(rule: &Rule, model: &SourceModel) -> Vec<Observation> {
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    model
        .functions
        .iter()
        .filter_map(|function| {
            let complexity = complexity(function.body.as_deref()?, language);
            observation(
                function.symbol.clone(),
                function.location.clone(),
                [
                    ("numerator", complexity * complexity + complexity),
                    ("denominator", 1),
                ],
                json!({
                    "collector": "authored_zero_coverage_crap_upper_bound_v1",
                    "cyclomatic_complexity": complexity,
                    "coverage_assumption_percent": 0,
                    "interpretation": "conservative upper bound when no complete coverage artifact is supplied",
                }),
            )
            .into()
        })
        .collect()
}

fn corpus_occurrences(model: &SourceModel, name: &str) -> usize {
    let expression = Regex::new(&format!(r"\b{}\b", regex::escape(name)))
        .expect("escaped identifier expression");
    model
        .code_by_path
        .values()
        .map(|source| expression.find_iter(source).count())
        .sum()
}

pub(super) fn unused_code(model: &SourceModel) -> Vec<Observation> {
    let mut by_file: BTreeMap<String, (Location, Vec<String>)> = BTreeMap::new();
    for function in &model.functions {
        if function.private
            && !function.name.starts_with("__")
            && corpus_occurrences(model, &function.name) == 1
        {
            by_file
                .entry(function.location.path.clone())
                .or_insert_with(|| (function.location.clone(), Vec::new()))
                .1
                .push(function.symbol.clone());
        }
    }
    for class in &model.classes {
        if class.private
            && !class.name.starts_with("__")
            && corpus_occurrences(model, &class.name) == 1
        {
            by_file
                .entry(class.location.path.clone())
                .or_insert_with(|| (class.location.clone(), Vec::new()))
                .1
                .push(class.symbol.clone());
        }
    }
    by_file
        .into_iter()
        .map(|(path, (location, symbols))| {
            observation(
                format!("{path}::unused-private-declarations"),
                location,
                [("findings", symbols.len() as u64)],
                json!({
                    "collector": "authored_private_declaration_references_v1",
                    "symbols": symbols,
                    "scope": "single-underscore private declarations with no corpus reference",
                }),
            )
        })
        .collect()
}

pub(super) fn unused_type_parameters(model: &SourceModel) -> Vec<Observation> {
    let mut observations = Vec::new();
    for function in &model.functions {
        let unused = function
            .type_parameters
            .iter()
            .filter(|parameter| {
                Regex::new(&format!(r"\b{}\b", regex::escape(parameter)))
                    .expect("escaped type parameter")
                    .find_iter(&function.source)
                    .count()
                    == 1
            })
            .cloned()
            .collect::<Vec<_>>();
        if !unused.is_empty() {
            observations.push(observation(
                function.symbol.clone(),
                function.location.clone(),
                [("findings", unused.len() as u64)],
                json!({
                    "collector": "authored_generic_parameter_use_v1",
                    "unused_parameters": unused,
                }),
            ));
        }
    }
    for class in &model.classes {
        let unused = class
            .type_parameters
            .iter()
            .filter(|parameter| {
                Regex::new(&format!(r"\b{}\b", regex::escape(parameter)))
                    .expect("escaped type parameter")
                    .find_iter(&class.source)
                    .count()
                    == 1
            })
            .cloned()
            .collect::<Vec<_>>();
        if !unused.is_empty() {
            observations.push(observation(
                class.symbol.clone(),
                class.location.clone(),
                [("findings", unused.len() as u64)],
                json!({
                    "collector": "authored_generic_parameter_use_v1",
                    "unused_parameters": unused,
                }),
            ));
        }
    }
    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::model::{ClassFact, FieldFact};
    use proptest::prelude::*;
    use std::sync::Arc;

    fn location(line: usize) -> Location {
        Location {
            path: "src/domain.rs".into(),
            line,
            column: 1,
        }
    }

    fn function(name: &str, body: Option<&str>) -> FunctionFact {
        FunctionFact {
            name: name.into(),
            symbol: format!("src/domain.rs::{name}"),
            location: location(1),
            type_parameters: Vec::new(),
            body: body.map(Arc::from),
            source: Arc::from(body.unwrap_or_default()),
            private: false,
        }
    }

    fn class(name: &str, methods: Vec<FunctionFact>) -> ClassFact {
        ClassFact {
            name: name.into(),
            symbol: format!("src/domain.rs::{name}"),
            location: location(1),
            type_parameters: Vec::new(),
            source: Arc::from(""),
            bases: Vec::new(),
            interface: false,
            private: false,
            fields: Vec::new(),
            methods,
            range: (0, 0),
        }
    }

    #[test]
    fn token_multisets_preserve_control_flow_and_count_normalized_values() {
        let first = normalized_tokens("if account == 42 { return account; }");
        let second = normalized_tokens("if customer == 7 { return other; }");
        assert_eq!(first, second);
        assert_eq!(first["identifier"], 2);
        assert_eq!(first["number"], 1);
        assert_eq!(first["if"], 1);

        let different = normalized_tokens("while account != 1 { account = account + 1; }");
        assert_eq!(token_similarity(&first, &first), (9, 9));
        let (intersection, union) = token_similarity(&first, &different);
        assert!(intersection > 0);
        assert!(union > intersection);
    }

    #[test]
    fn prefix_filter_counts_repeated_features_and_keeps_exact_threshold_boundaries() {
        let maps = vec![
            BTreeMap::from([("identifier".into(), 2), ("return".into(), 1)]),
            BTreeMap::from([("identifier".into(), 1), ("number".into(), 1)]),
        ];
        assert_eq!(
            feature_frequencies(&maps),
            BTreeMap::from([
                (("identifier".into(), 0), 2),
                (("identifier".into(), 1), 1),
                (("number".into(), 0), 1),
                (("return".into(), 0), 1),
            ])
        );
        let interfaces = vec![BTreeSet::from(["run"]), BTreeSet::from(["execute"])];
        let prefixes = similarity_prefixes(&interfaces, &maps, &[3, 2], 2, 5_000);
        assert_eq!(prefixes[0].len(), 2);
        assert_eq!(prefixes[1].len(), 2);
        assert!(candidate_pairs(&prefixes).is_empty());

        let complete_prefixes = similarity_prefixes(&interfaces, &maps, &[3, 2], 2, 0);
        assert_eq!(
            candidate_pairs(&complete_prefixes),
            BTreeSet::from([(0, 1)])
        );
    }

    #[test]
    fn forwarding_and_complexity_contracts_reject_near_misses() {
        assert!(forwarding_method(
            "return self.client.load(value)",
            "python"
        ));
        assert!(forwarding_method(
            "{ return this.client.load(value); }",
            "typescript"
        ));
        assert!(!forwarding_method("return client.load(value)", "python"));
        assert!(!forwarding_method(
            "return self.client.load(value); self.audit()",
            "python"
        ));
        assert_eq!(
            complexity("if a && b { for value in values {} }", "rust"),
            4
        );
        assert_eq!(complexity("match value { A => 1, B => 2 }", "rust"), 3);
        assert_eq!(complexity("match value { A => 1 }", "typescript"), 1);
    }

    #[test]
    fn forwarding_share_counts_only_concrete_non_constructor_methods() {
        let model = SourceModel {
            classes: vec![class(
                "Gateway",
                vec![
                    function("constructor", Some("this.client.load(value)")),
                    function("declared_only", None),
                    function("load", Some("this.client.load(value)")),
                ],
            )],
            ..SourceModel::default()
        };
        let rule = Rule {
            id: "typescript.forwarding_share".into(),
            version: 1,
            smell: "middle_man".into(),
            kind: "ratio".into(),
            implementation: "forwarding_share".into(),
            inputs: Vec::new(),
            parameters: BTreeMap::new(),
            contract: "test".into(),
        };

        let observations = forwarding_share(&rule, &model);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["methods"], 1);
        assert_eq!(observations[0].measurements["forwarders"], 1);
    }

    #[test]
    fn temporary_field_observation_counts_the_full_use_matrix() {
        let mut account = class(
            "Account",
            vec![
                function("first", Some("self.pending = 1")),
                function("second", Some("return self.other")),
                function("abstract", None),
            ],
        );
        account.fields = vec![
            FieldFact {
                name: "pending".into(),
                authored_type: "Optional[int]".into(),
                temporary: true,
                private: false,
            },
            FieldFact {
                name: "other".into(),
                authored_type: "int".into(),
                temporary: false,
                private: false,
            },
        ];
        let model = SourceModel {
            classes: vec![account],
            ..SourceModel::default()
        };
        let rule = Rule {
            id: "python.temporary_fields".into(),
            version: 1,
            smell: "temporary_field".into(),
            kind: "ratio".into(),
            implementation: "temporary_fields".into(),
            inputs: Vec::new(),
            parameters: BTreeMap::new(),
            contract: "test".into(),
        };
        let observations = temporary_fields(&rule, &model);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["fields"], 1);
        assert_eq!(observations[0].measurements["methods"], 2);
        assert_eq!(observations[0].measurements["uses"], 1);
        assert_eq!(observations[0].measurements["possible_uses"], 2);
    }

    #[test]
    fn unused_declarations_and_type_parameters_require_exactly_one_corpus_occurrence() {
        let mut hidden = function("_hidden", Some("return 1"));
        hidden.private = true;
        hidden.source = Arc::from("fn _hidden<T>() { return 1; }");
        hidden.type_parameters = vec!["T".into()];
        let mut used = function("_used", Some("return 2"));
        used.private = true;
        let model = SourceModel {
            functions: vec![hidden, used],
            code_by_path: BTreeMap::from([(
                "src/domain.rs".into(),
                Arc::from("fn _hidden() {} fn _used() {} fn call() { _used(); }") as Arc<str>,
            )]),
            ..SourceModel::default()
        };

        let unused = unused_code(&model);
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].measurements["findings"], 1);
        assert_eq!(unused[0].evidence["symbols"][0], "src/domain.rs::_hidden");
        let unused_parameters = unused_type_parameters(&model);
        assert_eq!(unused_parameters.len(), 1);
        assert_eq!(unused_parameters[0].evidence["unused_parameters"][0], "T");
    }

    #[test]
    fn unused_code_applies_the_same_privacy_name_and_reference_contract_to_classes() {
        let mut unused = class("_Unused", Vec::new());
        unused.private = true;
        let mut referenced = class("_Referenced", Vec::new());
        referenced.private = true;
        let mut public = class("Public", Vec::new());
        public.private = false;
        let mut dunder = class("__Generated", Vec::new());
        dunder.private = true;
        let model = SourceModel {
            classes: vec![unused, referenced, public, dunder],
            code_by_path: BTreeMap::from([(
                "src/domain.rs".into(),
                Arc::from(concat!(
                    "class _Unused: pass\n",
                    "class _Referenced: pass\n",
                    "use(_Referenced)\n",
                    "class Public: pass\n",
                    "class __Generated: pass\n",
                )) as Arc<str>,
            )]),
            ..SourceModel::default()
        };

        let observations = unused_code(&model);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["findings"], 1);
        assert_eq!(
            observations[0].evidence["symbols"],
            json!(["src/domain.rs::_Unused"])
        );
    }

    #[test]
    fn unused_type_parameters_are_reported_for_classes_only_when_unreferenced() {
        let mut generic = class("Envelope", Vec::new());
        generic.type_parameters = vec!["Unused".into(), "Value".into()];
        generic.source = Arc::from("class Envelope<Unused, Value> { value: Value }");
        let mut complete = class("Pair", Vec::new());
        complete.type_parameters = vec!["Left".into(), "Right".into()];
        complete.source = Arc::from("class Pair<Left, Right> { left: Left; right: Right }");
        let model = SourceModel {
            classes: vec![generic, complete],
            ..SourceModel::default()
        };

        let observations = unused_type_parameters(&model);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].symbol, "src/domain.rs::Envelope");
        assert_eq!(observations[0].measurements["findings"], 1);
        assert_eq!(
            observations[0].evidence["unused_parameters"],
            json!(["Unused"])
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 96,
            max_shrink_iters: 128,
            ..ProptestConfig::default()
        })]

        #[test]
        fn length_filter_matches_the_exact_ratio_contract(
            first in 0_u64..10_000,
            second in 0_u64..10_000,
            similarity in 0_u64..=10_000,
        ) {
            let smaller = first.min(second) as u128;
            let larger = first.max(second) as u128;
            prop_assert_eq!(
                lengths_can_match(first, second, similarity),
                smaller * 10_000 >= similarity as u128 * larger,
            );
            prop_assert_eq!(
                lengths_can_match(first, second, similarity),
                lengths_can_match(second, first, similarity),
            );
        }
    }
}
