use crate::{
    collectors,
    evidence_evaluators::evaluate,
    input::Input,
    policy::{Registry, Rule},
    report::{FindingRelations, Location, Report},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBundle {
    schema_version: u32,
    scanner_version: String,
    rule_pack: String,
    input_sha256: String,
    providers: Vec<RuleEvidence>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleEvidence {
    rule_id: String,
    provider: Provider,
    complete: bool,
    observations: Vec<Observation>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Provider {
    name: String,
    version: String,
    configuration_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Observation {
    pub(crate) symbol: String,
    pub(crate) location: Location,
    pub(crate) related_symbols: Vec<String>,
    pub(crate) related_locations: Vec<Location>,
    pub(crate) measurements: BTreeMap<String, u64>,
    pub(crate) evidence: Value,
}

pub fn parse(bytes: &[u8]) -> Result<EvidenceBundle, String> {
    serde_json::from_slice(bytes).map_err(|error| format!("invalid provider evidence: {error}"))
}

pub fn digest(bytes: &[u8]) -> String {
    crate::input::digest(&[b"provider-evidence-v1", bytes])
}

fn collector_rule(rule: &Rule) -> bool {
    !rule.inputs.iter().any(|input| input == "authored_source")
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_location(location: &Location, input: &Input) -> bool {
    let Some(source) = input.files.get(&location.path) else {
        return false;
    };
    if location.line == 0 || location.column == 0 {
        return false;
    }
    source
        .split('\n')
        .nth(location.line - 1)
        .is_some_and(|line| location.column <= line.chars().count() + 1)
}

fn validate_header(
    bundle: &EvidenceBundle,
    input: &Input,
    registry: &Registry,
) -> Result<(), String> {
    if bundle.schema_version != 1
        || bundle.scanner_version != env!("CARGO_PKG_VERSION")
        || bundle.rule_pack != registry.rule_pack
        || bundle.input_sha256 != input.digest
    {
        return Err("provider evidence header does not match the captured scan input".into());
    }
    Ok(())
}

fn ensure_unique<'a>(seen: &mut BTreeSet<&'a str>, rule_id: &'a str) -> Result<(), String> {
    if seen.insert(rule_id) {
        return Ok(());
    }
    Err(format!("duplicate provider evidence for {rule_id}"))
}

fn ensure_registered(registered: &BTreeSet<&str>, rule_id: &str) -> Result<(), String> {
    if registered.contains(rule_id) {
        return Ok(());
    }
    Err(format!(
        "provider evidence names an unknown or source-owned rule: {rule_id}"
    ))
}

fn validate_provider_identity(rule: &RuleEvidence) -> Result<(), String> {
    if rule.provider.name.is_empty()
        || rule.provider.version.is_empty()
        || !valid_sha256(&rule.provider.configuration_sha256)
    {
        return Err(format!("invalid provider identity for {}", rule.rule_id));
    }
    Ok(())
}

fn validate_related(observation: &Observation, input: &Input) -> bool {
    observation.related_symbols.len() == observation.related_locations.len()
        && observation
            .related_locations
            .iter()
            .all(|location| valid_location(location, input))
}

fn validate_observation(
    rule_id: &str,
    observation: &Observation,
    input: &Input,
) -> Result<(), String> {
    if observation.symbol.is_empty()
        || !valid_location(&observation.location, input)
        || !validate_related(observation, input)
    {
        return Err(format!("invalid provider observation for {rule_id}"));
    }
    evaluate(rule_id, &observation.measurements, &input.policy)?;
    Ok(())
}

fn validate_rule_evidence(rule: &RuleEvidence, input: &Input) -> Result<(), String> {
    if !rule.complete {
        return Err(format!(
            "provider evidence is incomplete for {}",
            rule.rule_id
        ));
    }
    validate_provider_identity(rule)?;
    for observation in &rule.observations {
        validate_observation(&rule.rule_id, observation, input)?;
    }
    Ok(())
}

pub fn validate(bundle: &EvidenceBundle, input: &Input, registry: &Registry) -> Result<(), String> {
    validate_header(bundle, input, registry)?;
    let registered: BTreeMap<_, _> = registry
        .rules
        .iter()
        .filter(|rule| collector_rule(rule))
        .map(|rule| (rule.id.as_str(), rule))
        .collect();
    let registered = registered.keys().copied().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for supplied in &bundle.providers {
        ensure_unique(&mut seen, &supplied.rule_id)?;
        ensure_registered(&registered, &supplied.rule_id)?;
        validate_rule_evidence(supplied, input)?;
    }
    Ok(())
}

pub fn apply(
    bundle: Option<&EvidenceBundle>,
    input: &Input,
    registry: &Registry,
    report: &mut Report,
) {
    let supplied = bundle
        .map(|bundle| {
            bundle
                .providers
                .iter()
                .map(|provider| (provider.rule_id.as_str(), provider))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let built_in_collectors = collectors::BuiltInCollectors::new(input);
    for rule in registry.rules.iter().filter(|rule| collector_rule(rule)) {
        if !input.policy.enabled(&rule.id) {
            continue;
        }
        let built_in;
        let (provider, complete, observations) =
            if let Some(provider) = supplied.get(rule.id.as_str()) {
                (
                    json!(provider.provider),
                    provider.complete,
                    provider.observations.as_slice(),
                )
            } else {
                built_in = match built_in_collectors.collect(rule) {
                    Ok(observations) => observations,
                    Err(error) => {
                        report.rule_error(&rule.id, error);
                        continue;
                    }
                };
                (
                    json!({
                        "name": "smells-built-in",
                        "version": env!("CARGO_PKG_VERSION"),
                        "configuration_sha256": crate::input::digest(&[
                            b"smells-built-in-collectors-v1",
                            rule.id.as_bytes(),
                        ]),
                    }),
                    true,
                    built_in.as_slice(),
                )
            };
        for observation in observations {
            let evaluation = match evaluate(&rule.id, &observation.measurements, &input.policy) {
                Ok(evaluation) => evaluation,
                Err(error) => {
                    report.rule_error(&rule.id, error);
                    continue;
                }
            };
            report.finding_with_relations(
                &input.policy,
                &rule.id,
                &observation.symbol,
                &observation.location,
                evaluation.metric,
                evaluation.observed,
                evaluation.comparison,
                evaluation.threshold,
                evaluation.matched,
                FindingRelations {
                    evidence: json!({
                        "provider": provider,
                        "measurements": observation.measurements,
                        "provider_evidence": observation.evidence,
                        "complete": complete
                    }),
                    symbols: observation.related_symbols.clone(),
                    locations: observation.related_locations.clone(),
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measurements(values: &[(&str, u64)]) -> BTreeMap<String, u64> {
        values
            .iter()
            .map(|(key, value)| ((*key).to_string(), *value))
            .collect()
    }

    #[test]
    fn every_registered_provider_rule_has_a_matching_threshold_evaluator() {
        let cases = [
            (
                "primitive_slots",
                measurements(&[("raw_slots", 5), ("total_slots", 5)]),
            ),
            (
                "alternative_interfaces",
                measurements(&[
                    ("first_tokens", 20),
                    ("intersection", 82),
                    ("second_tokens", 20),
                    ("union", 100),
                ]),
            ),
            (
                "repeated_dispatch",
                measurements(&[("arms_per_site", 4), ("sites", 3)]),
            ),
            (
                "temporary_fields",
                measurements(&[
                    ("fields", 3),
                    ("methods", 4),
                    ("possible_uses", 4),
                    ("uses", 1),
                ]),
            ),
            (
                "forwarding_share",
                measurements(&[("forwarders", 4), ("methods", 5)]),
            ),
            (
                "function_crap",
                measurements(&[("denominator", 1), ("numerator", 6)]),
            ),
            ("unused_code", measurements(&[("findings", 1)])),
            ("unused_type_parameters", measurements(&[("findings", 1)])),
            ("nominal_slot_contract", measurements(&[("mismatches", 1)])),
            ("port_conformance", measurements(&[("failures", 1)])),
            (
                "refused_bequest",
                measurements(&[("inherited_members", 5), ("unused_members", 4)]),
            ),
            (
                "divergent_change",
                measurements(&[("changes", 3), ("responsibilities", 3)]),
            ),
            (
                "parallel_inheritance",
                measurements(&[("parallel_pairs", 3)]),
            ),
            ("shotgun_surgery", measurements(&[("owners", 4)])),
            (
                "foreign_accesses",
                measurements(&[("foreign_accesses", 7), ("total_accesses", 10)]),
            ),
            (
                "dependency_contract",
                measurements(&[("forbidden_accesses", 1)]),
            ),
            ("library_capabilities", measurements(&[("failures", 1)])),
            ("navigation_chains", measurements(&[("transitions", 3)])),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>();
        for (pack, policy_text) in [
            ("rust-v1", include_str!("../examples/quality-policy.json")),
            (
                "python-v1",
                include_str!("../examples/python-quality-policy.json"),
            ),
            (
                "typescript-v1",
                include_str!("../examples/typescript-quality-policy.json"),
            ),
        ] {
            let registry = crate::policy::registry(pack).unwrap();
            let policy = crate::policy::parse(policy_text, &registry).unwrap();
            for rule in registry.rules.iter().filter(|rule| collector_rule(rule)) {
                let suffix = rule.id.split_once('.').unwrap().1;
                let values = cases
                    .get(suffix)
                    .unwrap_or_else(|| panic!("missing provider evaluator fixture: {}", rule.id));
                assert!(
                    evaluate(&rule.id, values, &policy).unwrap().matched,
                    "{}",
                    rule.id
                );
            }
        }
    }

    #[test]
    fn strict_and_inclusive_provider_boundaries_are_exact() {
        let registry = crate::policy::registry("python-v1").unwrap();
        let policy = crate::policy::parse(
            include_str!("../examples/python-quality-policy.json"),
            &registry,
        )
        .unwrap();
        assert!(
            !evaluate(
                "python.function_crap",
                &measurements(&[("denominator", 1), ("numerator", 5)]),
                &policy,
            )
            .unwrap()
            .matched
        );
        assert!(
            !evaluate(
                "python.foreign_accesses",
                &measurements(&[("foreign_accesses", 6), ("total_accesses", 10)]),
                &policy,
            )
            .unwrap()
            .matched
        );
    }
}
