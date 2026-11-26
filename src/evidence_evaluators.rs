use crate::{policy::Policy, rule_runtime::RuleKind};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Evaluation {
    pub metric: &'static str,
    pub observed: Value,
    pub comparison: &'static str,
    pub threshold: Value,
    pub matched: bool,
}

type Evaluator = fn(&str, &BTreeMap<String, u64>, &Policy) -> Result<Evaluation, String>;

const EVALUATORS: &[(RuleKind, Evaluator)] = &[
    (RuleKind::AlternativeInterfaces, alternative_interfaces),
    (RuleKind::DependencyContract, dependency_contract),
    (RuleKind::DivergentChange, divergent_change),
    (RuleKind::ForeignAccesses, foreign_accesses),
    (RuleKind::ForwardingShare, forwarding_share),
    (RuleKind::FunctionCrap, function_crap),
    (RuleKind::LibraryCapabilities, library_capabilities),
    (RuleKind::NavigationChains, navigation_chains),
    (RuleKind::NominalSlotContract, nominal_slot_contract),
    (RuleKind::ParallelInheritance, parallel_inheritance),
    (RuleKind::PortConformance, port_conformance),
    (RuleKind::PrimitiveSlots, primitive_slots),
    (RuleKind::RefusedBequest, refused_bequest),
    (RuleKind::RepeatedDispatch, repeated_dispatch),
    (RuleKind::ShotgunSurgery, shotgun_surgery),
    (RuleKind::TemporaryFields, temporary_fields),
    (RuleKind::UnusedCode, unused_code_findings),
    (
        RuleKind::UnusedTypeParameters,
        unused_type_parameter_findings,
    ),
];

pub(crate) fn evaluate(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    let kind = RuleKind::from_id(id)?;
    let evaluator = EVALUATORS
        .iter()
        .find_map(|(registered, evaluator)| (*registered == kind).then_some(*evaluator))
        .ok_or_else(|| format!("no built-in evaluator for {id}"))?;
    evaluator(id, measurements, policy)
}

const ESTIMATED_CRAP: &str = "No coverage was supplied, so this score assumes 0% coverage and does not block; supply LCOV coverage to measure it.";
const WARNING_CRAP: &str =
    "The score is in the warning band, at most `maximum`, so it does not block.";

/// Why a matched observation is reported without blocking, or `None` when it may block.
/// Only CRAP has a warning band: a score blocks when it exceeds `maximum` and was
/// measured against supplied coverage, never when it is the zero-coverage upper bound.
pub(crate) fn advisory(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    evidence: &Value,
    policy: &Policy,
) -> Option<&'static str> {
    match RuleKind::from_id(id) {
        Ok(RuleKind::FunctionCrap) => {}
        Ok(RuleKind::UnusedCode) => {
            return (evidence.get("test_code") == Some(&json!(true)))
                .then_some(crate::core::test_suite::TEST_CODE_ADVISORY);
        }
        _ => return None,
    }
    if evidence.get("coverage_assumption_percent") == Some(&json!(0)) {
        return Some(ESTIMATED_CRAP);
    }
    let (numerator, denominator) = (measurements["numerator"], measurements["denominator"]);
    let maximum = policy.parameter(id, "maximum");
    (u128::from(numerator) <= u128::from(maximum) * u128::from(denominator)).then_some(WARNING_CRAP)
}

fn values(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    keys: &[&str],
) -> Result<Vec<u64>, String> {
    let expected = keys.iter().copied().collect::<BTreeSet<_>>();
    let actual = measurements.keys().map(String::as_str).collect();
    if expected != actual {
        return Err(format!("provider measurement keys mismatch: {id}"));
    }
    Ok(keys.iter().map(|key| measurements[*key]).collect())
}

fn primitive_slots(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    let measured = values(id, measurements, &["raw_slots", "total_slots"])?;
    let (raw, total) = (measured[0], measured[1]);
    if total == 0 || raw > total {
        return Err(format!("invalid primitive slot evidence: {id}"));
    }
    let minimum = policy.parameter(id, "minimum_raw_slots");
    let share = policy.parameter(id, "minimum_share_percent");
    Ok(Evaluation {
        metric: "primitive slots",
        observed: json!({"raw_slots":raw,"total_slots":total}),
        comparison: "count and share >=",
        threshold: json!({"minimum_raw_slots":minimum,"minimum_share_percent":share}),
        matched: raw >= minimum && u128::from(raw) * 100 >= u128::from(share) * u128::from(total),
    })
}

fn alternative_interfaces(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    let measured = values(
        id,
        measurements,
        &["first_tokens", "intersection", "second_tokens", "union"],
    )?;
    let (first, intersection, second, union) = (measured[0], measured[1], measured[2], measured[3]);
    if union == 0 || intersection > union {
        return Err(format!("invalid interface similarity evidence: {id}"));
    }
    let minimum = policy.parameter(id, "minimum_tokens");
    let similarity = policy.parameter(id, "minimum_similarity_basis_points");
    Ok(Evaluation {
        metric: "class behavior similarity",
        observed: json!({"first_tokens":first,"second_tokens":second,"intersection":intersection,"union":union}),
        comparison: "tokens and Jaccard >=",
        threshold: json!({"minimum_tokens":minimum,"minimum_similarity_basis_points":similarity}),
        matched: first >= minimum
            && second >= minimum
            && u128::from(intersection) * 10_000 >= u128::from(similarity) * u128::from(union),
    })
}

fn repeated_dispatch(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    let measured = values(id, measurements, &["arms_per_site", "sites"])?;
    let minimum_sites = policy.parameter(id, "minimum_sites");
    let minimum_arms = policy.parameter(id, "minimum_arms");
    Ok(Evaluation {
        metric: "repeated dispatch sites",
        observed: json!({"sites":measured[1],"arms_per_site":measured[0]}),
        comparison: "both >=",
        threshold: json!({"minimum_sites":minimum_sites,"minimum_arms":minimum_arms}),
        matched: measured[1] >= minimum_sites && measured[0] >= minimum_arms,
    })
}

fn temporary_fields(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    let measured = values(
        id,
        measurements,
        &["fields", "methods", "possible_uses", "uses"],
    )?;
    let (fields, methods, possible, uses) = (measured[0], measured[1], measured[2], measured[3]);
    if possible == 0 || uses > possible {
        return Err(format!("invalid temporary field evidence: {id}"));
    }
    let minimum_fields = policy.parameter(id, "minimum_fields");
    let minimum_methods = policy.parameter(id, "minimum_methods");
    let maximum_share = policy.parameter(id, "maximum_use_percent");
    Ok(Evaluation {
        metric: "field-use share",
        observed: json!({"fields":fields,"methods":methods,"uses":uses,"possible_uses":possible}),
        comparison: "counts >= and use share <=",
        threshold: json!({"minimum_fields":minimum_fields,"minimum_methods":minimum_methods,"maximum_use_percent":maximum_share}),
        matched: fields >= minimum_fields
            && methods >= minimum_methods
            && u128::from(uses) * 100 <= u128::from(maximum_share) * u128::from(possible),
    })
}

fn forwarding_share(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    let measured = values(id, measurements, &["forwarders", "methods"])?;
    let (forwarders, methods) = (measured[0], measured[1]);
    if methods == 0 || forwarders > methods {
        return Err(format!("invalid forwarding evidence: {id}"));
    }
    let minimum = policy.parameter(id, "minimum_methods");
    let share = policy.parameter(id, "minimum_share_percent");
    Ok(Evaluation {
        metric: "forwarding methods",
        observed: json!({"forwarders":forwarders,"methods":methods}),
        comparison: "methods and share >=",
        threshold: json!({"minimum_methods":minimum,"minimum_share_percent":share}),
        matched: methods >= minimum
            && u128::from(forwarders) * 100 >= u128::from(share) * u128::from(methods),
    })
}

fn function_crap(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    let measured = values(id, measurements, &["denominator", "numerator"])?;
    let (denominator, numerator) = (measured[0], measured[1]);
    if denominator == 0 {
        return Err(format!("invalid CRAP evidence: {id}"));
    }
    let warning = policy.parameter(id, "warning_maximum");
    let maximum = policy.parameter(id, "maximum");
    Ok(Evaluation {
        metric: "CRAP score",
        observed: json!({"numerator":numerator,"denominator":denominator}),
        comparison: ">",
        threshold: json!({"warning_maximum":warning,"maximum":maximum}),
        matched: u128::from(numerator) > u128::from(warning.min(maximum)) * u128::from(denominator),
    })
}

fn unused_code_findings(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    count_maximum(
        id,
        measurements,
        policy,
        "findings",
        "maximum_findings",
        "unused-code findings",
    )
}

fn unused_type_parameter_findings(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    count_maximum(
        id,
        measurements,
        policy,
        "findings",
        "maximum_findings",
        "unused type-parameter findings",
    )
}

fn nominal_slot_contract(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    count_maximum(
        id,
        measurements,
        policy,
        "mismatches",
        "maximum_mismatches",
        "nominal slot mismatches",
    )
}

fn port_conformance(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    count_maximum(
        id,
        measurements,
        policy,
        "failures",
        "maximum_failures",
        "port conformance failures",
    )
}

fn refused_bequest(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    let measured = values(id, measurements, &["inherited_members", "unused_members"])?;
    let (inherited, unused) = (measured[0], measured[1]);
    if inherited == 0 || unused > inherited {
        return Err(format!("invalid inheritance evidence: {id}"));
    }
    let minimum = policy.parameter(id, "minimum_inherited_members");
    let unused_share = policy.parameter(id, "minimum_unused_percent");
    Ok(Evaluation {
        metric: "resolved unused inheritance",
        observed: json!({"inherited_members":inherited,"unused_members":unused}),
        comparison: "count and unused share >=",
        threshold: json!({"minimum_inherited_members":minimum,"minimum_unused_percent":unused_share}),
        matched: inherited >= minimum
            && u128::from(unused) * 100 >= u128::from(unused_share) * u128::from(inherited),
    })
}

fn divergent_change(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    let measured = values(id, measurements, &["changes", "responsibilities"])?;
    let changes = policy.parameter(id, "minimum_changes");
    let responsibilities = policy.parameter(id, "minimum_responsibilities");
    Ok(Evaluation {
        metric: "owner change responsibilities",
        observed: json!({"changes":measured[0],"responsibilities":measured[1]}),
        comparison: "both >=",
        threshold: json!({"minimum_changes":changes,"minimum_responsibilities":responsibilities}),
        matched: measured[0] >= changes && measured[1] >= responsibilities,
    })
}

fn parallel_inheritance(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    count_minimum(
        id,
        measurements,
        policy,
        "parallel_pairs",
        "minimum_parallel_pairs",
        "paired inheritance additions",
    )
}

fn shotgun_surgery(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    count_maximum(
        id,
        measurements,
        policy,
        "owners",
        "maximum_owners",
        "owners changed together",
    )
}

fn foreign_accesses(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    let measured = values(id, measurements, &["foreign_accesses", "total_accesses"])?;
    let (foreign, total) = (measured[0], measured[1]);
    if total == 0 || foreign > total {
        return Err(format!("invalid foreign access evidence: {id}"));
    }
    let minimum = policy.parameter(id, "minimum_foreign_accesses");
    let share = policy.parameter(id, "minimum_share_percent_exclusive");
    Ok(Evaluation {
        metric: "foreign member accesses",
        observed: json!({"foreign_accesses":foreign,"total_accesses":total}),
        comparison: "count >= and share >",
        threshold: json!({"minimum_foreign_accesses":minimum,"minimum_share_percent_exclusive":share}),
        matched: foreign >= minimum
            && u128::from(foreign) * 100 > u128::from(share) * u128::from(total),
    })
}

fn dependency_contract(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    count_maximum(
        id,
        measurements,
        policy,
        "forbidden_accesses",
        "maximum_forbidden_accesses",
        "forbidden dependency accesses",
    )
}

fn library_capabilities(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    count_maximum(
        id,
        measurements,
        policy,
        "failures",
        "maximum_failures",
        "library capability failures",
    )
}

fn navigation_chains(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
) -> Result<Evaluation, String> {
    count_minimum(
        id,
        measurements,
        policy,
        "transitions",
        "minimum_transitions",
        "navigation transitions",
    )
}

fn count_maximum(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
    observed_key: &str,
    threshold_key: &str,
    metric: &'static str,
) -> Result<Evaluation, String> {
    let measured = values(id, measurements, &[observed_key])?[0];
    let maximum = policy.parameter(id, threshold_key);
    Ok(Evaluation {
        metric,
        observed: json!(measured),
        comparison: ">",
        threshold: json!(maximum),
        matched: measured > maximum,
    })
}

fn count_minimum(
    id: &str,
    measurements: &BTreeMap<String, u64>,
    policy: &Policy,
    observed_key: &str,
    threshold_key: &str,
    metric: &'static str,
) -> Result<Evaluation, String> {
    let measured = values(id, measurements, &[observed_key])?[0];
    let minimum = policy.parameter(id, threshold_key);
    Ok(Evaluation {
        metric,
        observed: json!(measured),
        comparison: ">=",
        threshold: json!(minimum),
        matched: measured >= minimum,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        let registry = crate::policy::registry("python-v1").unwrap();
        crate::policy::parse(
            include_str!("../examples/python-quality-policy.json"),
            &registry,
        )
        .unwrap()
    }

    fn measured(values: &[(&str, u64)]) -> BTreeMap<String, u64> {
        values
            .iter()
            .map(|(key, value)| ((*key).into(), *value))
            .collect()
    }

    fn matched(id: &str, values: &[(&str, u64)], policy: &Policy) -> bool {
        evaluate(id, &measured(values), policy).unwrap().matched
    }

    #[test]
    fn every_rule_kind_has_exactly_one_evaluator() {
        let registered = EVALUATORS
            .iter()
            .map(|(kind, _)| *kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(registered.len(), EVALUATORS.len());
        assert_eq!(registered, RuleKind::all().collect::<BTreeSet<RuleKind>>());
    }

    #[test]
    fn primitive_slot_count_and_share_boundaries_are_independent() {
        let policy = policy();
        assert!(matched(
            "python.primitive_slots",
            &[("raw_slots", 5), ("total_slots", 5)],
            &policy
        ));
        assert!(!matched(
            "python.primitive_slots",
            &[("raw_slots", 4), ("total_slots", 5)],
            &policy
        ));
        assert!(!matched(
            "python.primitive_slots",
            &[("raw_slots", 5), ("total_slots", 7)],
            &policy
        ));
        assert!(
            evaluate(
                "python.primitive_slots",
                &measured(&[("raw_slots", 1), ("total_slots", 0)]),
                &policy
            )
            .is_err()
        );
        assert!(
            evaluate(
                "python.primitive_slots",
                &measured(&[("raw_slots", 6), ("total_slots", 5)]),
                &policy
            )
            .is_err()
        );
    }

    #[test]
    fn interface_similarity_requires_both_token_counts_and_jaccard_share() {
        let policy = policy();
        let case = |first, intersection, second, union| {
            matched(
                "python.alternative_interfaces",
                &[
                    ("first_tokens", first),
                    ("intersection", intersection),
                    ("second_tokens", second),
                    ("union", union),
                ],
                &policy,
            )
        };
        assert!(case(20, 82, 20, 100));
        assert!(!case(19, 82, 20, 100));
        assert!(!case(20, 82, 19, 100));
        assert!(!case(20, 81, 20, 100));
        assert!(
            evaluate(
                "python.alternative_interfaces",
                &measured(&[
                    ("first_tokens", 20),
                    ("intersection", 0),
                    ("second_tokens", 20),
                    ("union", 0),
                ]),
                &policy
            )
            .is_err()
        );
        assert!(
            evaluate(
                "python.alternative_interfaces",
                &measured(&[
                    ("first_tokens", 20),
                    ("intersection", 101),
                    ("second_tokens", 20),
                    ("union", 100),
                ]),
                &policy
            )
            .is_err()
        );
    }

    #[test]
    fn composite_count_rules_require_every_threshold() {
        let policy = policy();
        for (id, matching, nonmatching) in [
            (
                "python.repeated_dispatch",
                vec![("arms_per_site", 4), ("sites", 3)],
                vec![
                    vec![("arms_per_site", 3), ("sites", 3)],
                    vec![("arms_per_site", 4), ("sites", 2)],
                ],
            ),
            (
                "python.divergent_change",
                vec![("changes", 3), ("responsibilities", 3)],
                vec![
                    vec![("changes", 2), ("responsibilities", 3)],
                    vec![("changes", 3), ("responsibilities", 2)],
                ],
            ),
        ] {
            assert!(matched(id, &matching, &policy));
            for values in nonmatching {
                assert!(!matched(id, &values, &policy));
            }
        }
    }

    #[test]
    fn temporary_field_counts_and_use_share_are_independent() {
        let policy = policy();
        let case = |fields, methods, possible, uses| {
            matched(
                "python.temporary_fields",
                &[
                    ("fields", fields),
                    ("methods", methods),
                    ("possible_uses", possible),
                    ("uses", uses),
                ],
                &policy,
            )
        };
        assert!(case(3, 4, 4, 1));
        assert!(!case(2, 4, 4, 1));
        assert!(!case(3, 3, 4, 1));
        assert!(!case(3, 4, 4, 2));
        assert!(!case(3, 4, 4, 4));
        for (possible, uses) in [(0, 0), (4, 5)] {
            assert!(
                evaluate(
                    "python.temporary_fields",
                    &measured(&[
                        ("fields", 3),
                        ("methods", 4),
                        ("possible_uses", possible),
                        ("uses", uses),
                    ]),
                    &policy
                )
                .is_err()
            );
        }
    }

    #[test]
    fn forwarding_and_refused_bequest_validate_totals_counts_and_shares() {
        let policy = policy();
        assert!(matched(
            "python.forwarding_share",
            &[("forwarders", 4), ("methods", 5)],
            &policy
        ));
        assert!(!matched(
            "python.forwarding_share",
            &[("forwarders", 4), ("methods", 4)],
            &policy
        ));
        assert!(!matched(
            "python.forwarding_share",
            &[("forwarders", 3), ("methods", 5)],
            &policy
        ));
        for values in [
            [("forwarders", 0), ("methods", 0)],
            [("forwarders", 6), ("methods", 5)],
        ] {
            assert!(evaluate("python.forwarding_share", &measured(&values), &policy).is_err());
        }

        assert!(matched(
            "python.refused_bequest",
            &[("inherited_members", 5), ("unused_members", 4)],
            &policy
        ));
        assert!(!matched(
            "python.refused_bequest",
            &[("inherited_members", 2), ("unused_members", 2)],
            &policy
        ));
        assert!(!matched(
            "python.refused_bequest",
            &[("inherited_members", 5), ("unused_members", 3)],
            &policy
        ));
        for values in [
            [("inherited_members", 0), ("unused_members", 0)],
            [("inherited_members", 5), ("unused_members", 6)],
        ] {
            assert!(evaluate("python.refused_bequest", &measured(&values), &policy).is_err());
        }
    }

    #[test]
    fn crap_bands_block_only_measured_scores_above_the_maximum() {
        let policy = policy();
        let crap = |numerator, denominator| {
            measured(&[("denominator", denominator), ("numerator", numerator)])
        };
        let measured_evidence = json!({"collector": "lcov_line_coverage_crap_v1"});
        let estimated = json!({"coverage_assumption_percent": 0});
        let reason = |values: &BTreeMap<String, u64>, evidence: &Value| {
            advisory("python.function_crap", values, evidence, &policy)
        };
        assert!(!matched(
            "python.function_crap",
            &[("denominator", 2), ("numerator", 10)],
            &policy
        ));
        assert!(matched(
            "python.function_crap",
            &[("denominator", 2), ("numerator", 11)],
            &policy
        ));
        assert_eq!(reason(&crap(20, 2), &measured_evidence), Some(WARNING_CRAP));
        assert_eq!(reason(&crap(21, 2), &measured_evidence), None);
        assert_eq!(reason(&crap(21, 2), &estimated), Some(ESTIMATED_CRAP));
        assert_eq!(
            reason(&crap(12, 1), &json!({"coverage_assumption_percent": 50})),
            None
        );
        assert_eq!(
            advisory("python.function_lines", &crap(21, 2), &estimated, &policy),
            None
        );
        let unused = |test_code: bool| {
            advisory(
                "python.unused_code",
                &measured(&[("findings", 1)]),
                &json!({ "test_code": test_code }),
                &policy,
            )
        };
        assert_eq!(
            unused(true),
            Some(crate::core::test_suite::TEST_CODE_ADVISORY)
        );
        assert_eq!(unused(false), None);

        let mut inverted = self::policy();
        let parameters = &mut inverted
            .rules
            .get_mut("python.function_crap")
            .unwrap()
            .parameters;
        parameters.insert("warning_maximum".into(), 10);
        parameters.insert("maximum".into(), 5);
        assert!(matched(
            "python.function_crap",
            &[("denominator", 1), ("numerator", 6)],
            &inverted
        ));
        assert_eq!(
            advisory(
                "python.function_crap",
                &crap(6, 1),
                &measured_evidence,
                &inverted
            ),
            None
        );
    }

    #[test]
    fn crap_and_foreign_access_ratios_use_exact_integer_boundaries() {
        let policy = policy();

        assert!(!matched(
            "python.foreign_accesses",
            &[("foreign_accesses", 6), ("total_accesses", 10)],
            &policy
        ));
        assert!(matched(
            "python.foreign_accesses",
            &[("foreign_accesses", 7), ("total_accesses", 10)],
            &policy
        ));
        assert!(!matched(
            "python.foreign_accesses",
            &[("foreign_accesses", 4), ("total_accesses", 5)],
            &policy
        ));
        for values in [
            [("foreign_accesses", 0), ("total_accesses", 0)],
            [("foreign_accesses", 6), ("total_accesses", 5)],
        ] {
            assert!(evaluate("python.foreign_accesses", &measured(&values), &policy).is_err());
        }
    }
}
