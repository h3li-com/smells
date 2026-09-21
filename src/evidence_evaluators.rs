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
        matched: raw >= minimum && raw as u128 * 100 >= share as u128 * total as u128,
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
            && intersection as u128 * 10_000 >= similarity as u128 * union as u128,
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
            && uses as u128 * 100 <= maximum_share as u128 * possible as u128,
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
        matched: methods >= minimum && forwarders as u128 * 100 >= share as u128 * methods as u128,
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
    let maximum = policy.parameter(id, "maximum");
    Ok(Evaluation {
        metric: "CRAP score",
        observed: json!({"numerator":numerator,"denominator":denominator}),
        comparison: ">",
        threshold: json!({"maximum":maximum}),
        matched: numerator as u128 > maximum as u128 * denominator as u128,
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
            && unused as u128 * 100 >= unused_share as u128 * inherited as u128,
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
        matched: foreign >= minimum && foreign as u128 * 100 > share as u128 * total as u128,
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

    #[test]
    fn every_rule_kind_has_exactly_one_evaluator() {
        let registered = EVALUATORS
            .iter()
            .map(|(kind, _)| *kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(registered.len(), EVALUATORS.len());
        assert_eq!(registered, RuleKind::all().collect::<BTreeSet<RuleKind>>());
    }
}
