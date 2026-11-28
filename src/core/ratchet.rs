use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::{
    core::model::Implementation,
    policy::{Mode, Policy, Selection},
    report::{Finding, Report},
};

/// Auditable summary of a two-snapshot Git-reference Debt Envelope ratchet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RatchetSummary {
    pub reference_ref: String,
    pub reference_commit: String,
    pub reference_blockers: usize,
    pub candidate_blockers: usize,
    pub regressed_blockers: usize,
    pub legacy_within_envelope: usize,
    pub burndown_delta: isize,
    pub reference_suppressions: usize,
    pub candidate_suppressions: usize,
    pub new_suppressions: usize,
    pub suppression_guardrail_triggered: bool,
    pub policy_changed: bool,
    pub policy_weakened_rules: Vec<String>,
}

/// Default-on, machine-verifiable run completeness and anti-gaming attestation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct RunAttestation {
    pub complete: bool,
    pub exit_code: u8,
    pub rules_selected: usize,
    pub rules_measured: usize,
    pub rules_incomplete: usize,
    pub history_complete: bool,
    pub policy_sha256: String,
    pub suppressions_total: usize,
    pub ratchet_active: bool,
}

/// Immutable comparison context passed into the pure ratchet engine.
pub(crate) struct RatchetContext<'a> {
    pub(crate) reference_ref: &'a str,
    pub(crate) reference_commit: &'a str,
    pub(crate) reference_policy_sha256: Option<&'a str>,
    pub(crate) candidate_policy_sha256: &'a str,
    pub(crate) reference_policy: Option<&'a Policy>,
    pub(crate) candidate_policy: &'a Policy,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct EnvelopeScope {
    rule_id: String,
    implementations: Vec<String>,
}

type ReferenceEnvelope = BTreeMap<EnvelopeScope, Vec<f64>>;
type CandidateGroups = BTreeMap<EnvelopeScope, Vec<(usize, f64)>>;
/// Rule, path and symbol of a reference blocker, used only to decide which of
/// several equal-burden candidates the envelope covers.
type FindingKey = (String, String, String);

fn finding_key(finding: &Finding) -> FindingKey {
    (
        finding.rule_id.clone(),
        finding.location.path.clone(),
        finding.symbol.clone(),
    )
}

/// Extract a deterministic numeric burden for ordering findings within one rule.
pub(crate) fn finding_burden(finding: &Finding) -> f64 {
    value_burden(&finding.evaluation.observed)
}

fn value_burden(value: &serde_json::Value) -> f64 {
    if let Some(number) = value.as_f64() {
        return number;
    }
    if let Some(map) = value.as_object() {
        // Duplicate-code pairs observe `{intersection, union}`. Only the shared
        // token mass is debt: counting `union` would make diverging a copy look
        // worse, while growing both copies in lockstep must still regress.
        if let Some(shared) = map.get("intersection").and_then(serde_json::Value::as_f64) {
            return shared;
        }
        let sum: f64 = map.values().filter_map(serde_json::Value::as_f64).sum();
        if sum > 0.0 {
            return sum;
        }
    }
    1.0
}

/// Canonical set of manifest-owned implementations participating in a finding.
pub(crate) fn finding_implementation_scope(
    finding: &Finding,
    implementations: &[Implementation],
) -> Vec<String> {
    let mut owners = BTreeSet::new();
    for implementation in implementations {
        if finding_touches_implementation(finding, &implementation.source_files) {
            owners.insert(implementation.id.clone());
        }
    }
    owners.into_iter().collect()
}

fn finding_touches_implementation(finding: &Finding, source_files: &[String]) -> bool {
    source_files.binary_search(&finding.location.path).is_ok()
        || finding
            .related_locations
            .iter()
            .any(|location| source_files.binary_search(&location.path).is_ok())
}

/// Optimal two-pointer descending dominance match.
///
/// Both `reference` and `candidates` must be sorted in descending burden order.
/// Returns the candidate finding indices that cannot be covered by a distinct
/// equal-or-greater reference allowance.
pub(crate) fn envelope_regressed_indices(
    reference: &[f64],
    candidates: &[(usize, f64)],
) -> Vec<usize> {
    let mut ref_index = 0usize;
    let mut regressed = Vec::new();
    for &(finding_index, burden) in candidates {
        if ref_index < reference.len() && burden <= reference[ref_index] {
            ref_index += 1;
        } else {
            regressed.push(finding_index);
        }
    }
    regressed
}

/// Detect whether any required rule in the reference policy was weakened in the
/// candidate policy (mode downgrade or looser numeric parameter threshold).
pub(crate) fn detect_policy_weakening(
    reference_policy: Option<&Policy>,
    candidate_policy: &Policy,
) -> Vec<String> {
    let Some(reference) = reference_policy else {
        return Vec::new();
    };
    let mut weakened = Vec::new();
    for (rule_id, ref_rule) in &reference.rules {
        if ref_rule.mode != Mode::Required {
            continue;
        }
        let Some(cand_rule) = candidate_policy.rules.get(rule_id) else {
            weakened.push(rule_id.clone());
            continue;
        };
        if rule_selection_weakened(ref_rule, cand_rule) {
            weakened.push(rule_id.clone());
        }
    }
    weakened
}

fn rule_selection_weakened(reference: &Selection, candidate: &Selection) -> bool {
    if candidate.mode != Mode::Required {
        return true;
    }
    reference.parameters.iter().any(|(name, ref_value)| {
        let Some(cand_value) = candidate.parameters.get(name) else {
            return true;
        };
        parameter_weakened(name, *ref_value, *cand_value)
    })
}

fn parameter_weakened(name: &str, reference: u64, candidate: u64) -> bool {
    (name.starts_with("maximum") || name.starts_with("minimum")) && candidate > reference
}

fn reference_envelope(
    reference_report: Option<&Report>,
) -> (ReferenceEnvelope, BTreeSet<FindingKey>, usize, usize) {
    let Some(report) = reference_report else {
        return (BTreeMap::new(), BTreeSet::new(), 0, 0);
    };
    let scopes = &report.implementation_scopes;
    let findings = &report.evidence.findings;
    let suppressions = findings
        .iter()
        .filter(|finding| finding.status == "ignored_match")
        .count();
    let mut envelope: BTreeMap<EnvelopeScope, Vec<f64>> = BTreeMap::new();
    let mut known = BTreeSet::new();
    for finding in findings.iter().filter(|finding| finding.blocking) {
        known.insert(finding_key(finding));
        let scope = EnvelopeScope {
            rule_id: finding.rule_id.clone(),
            implementations: finding_implementation_scope(finding, scopes),
        };
        envelope
            .entry(scope)
            .or_default()
            .push(finding_burden(finding));
    }
    let blockers: usize = envelope.values().map(Vec::len).sum();
    for burdens in envelope.values_mut() {
        burdens.sort_by(|left, right| right.total_cmp(left));
    }
    (envelope, known, blockers, suppressions)
}

/// Group candidate blockers by envelope scope in descending burden order.
///
/// Among equal burdens, findings already present in the reference sort first, so
/// the two-pointer match covers them and reports the newly added instance as the
/// regression. The tie-break never changes how many findings regress.
fn candidate_blocker_groups(
    candidate_report: &Report,
    known: &BTreeSet<FindingKey>,
) -> (CandidateGroups, usize, usize) {
    let scopes = &candidate_report.implementation_scopes;
    let findings = &candidate_report.evidence.findings;
    let suppressions = findings
        .iter()
        .filter(|finding| finding.status == "ignored_match")
        .count();
    let mut groups: BTreeMap<EnvelopeScope, Vec<(usize, f64)>> = BTreeMap::new();
    let is_new = |index: usize| !known.contains(&finding_key(&findings[index]));
    for (index, finding) in findings.iter().enumerate() {
        if !finding.blocking {
            continue;
        }
        let scope = EnvelopeScope {
            rule_id: finding.rule_id.clone(),
            implementations: finding_implementation_scope(finding, scopes),
        };
        groups
            .entry(scope)
            .or_default()
            .push((index, finding_burden(finding)));
    }
    let blockers: usize = groups.values().map(Vec::len).sum();
    for entries in groups.values_mut() {
        entries.sort_by(|(left_idx, left_burden), (right_idx, right_burden)| {
            right_burden
                .total_cmp(left_burden)
                .then_with(|| is_new(*left_idx).cmp(&is_new(*right_idx)))
                .then_with(|| left_idx.cmp(right_idx))
        });
    }
    (groups, blockers, suppressions)
}

/// Compare candidate against reference Debt Envelope and enforce default-on
/// anti-gaming guardrails (burndown lock, suppression growth, policy weakening).
pub(crate) fn apply_ratchet(
    reference_report: Option<&Report>,
    candidate_report: &mut Report,
    context: &RatchetContext<'_>,
) {
    if let Some(reference) = reference_report
        && reference.summary.error_count != 0
    {
        candidate_report.error(format!(
            "reference snapshot {} ({}) failed closed with {} error(s)",
            context.reference_ref, context.reference_commit, reference.summary.error_count
        ));
    }

    let (envelope, known, reference_blockers, reference_suppressions) =
        reference_envelope(reference_report);
    let (candidate_groups, candidate_blockers, candidate_suppressions) =
        candidate_blocker_groups(candidate_report, &known);

    let mut regressed = BTreeSet::new();
    for (scope, candidates) in &candidate_groups {
        let allowed = envelope.get(scope).map_or(&[][..], Vec::as_slice);
        for index in envelope_regressed_indices(allowed, candidates) {
            regressed.insert(index);
        }
    }

    for (index, finding) in candidate_report.evidence.findings.iter_mut().enumerate() {
        if finding.blocking && !regressed.contains(&index) {
            finding.blocking = false;
            finding.status = "legacy_within_envelope".into();
        }
    }
    let legacy_within_envelope = candidate_blockers.saturating_sub(regressed.len());

    let new_suppressions = candidate_suppressions.saturating_sub(reference_suppressions);
    let policy_changed = context
        .reference_policy_sha256
        .is_some_and(|ref_sha| ref_sha != context.candidate_policy_sha256);
    let policy_weakened_rules =
        detect_policy_weakening(context.reference_policy, context.candidate_policy);

    let summary = RatchetSummary {
        reference_ref: context.reference_ref.to_string(),
        reference_commit: context.reference_commit.to_string(),
        reference_blockers,
        candidate_blockers,
        regressed_blockers: regressed.len(),
        legacy_within_envelope,
        burndown_delta: candidate_blockers.cast_signed() - reference_blockers.cast_signed(),
        reference_suppressions,
        candidate_suppressions,
        new_suppressions,
        suppression_guardrail_triggered: new_suppressions > 0,
        policy_changed,
        policy_weakened_rules,
    };

    candidate_report.ratchet = Some(summary);
    candidate_report.finish();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy;
    use serde_json::json;

    #[test]
    fn two_pointer_dominance_allows_equal_or_better_and_blocks_worsened_or_extra() {
        assert_eq!(
            envelope_regressed_indices(&[150.0, 120.0], &[(0, 140.0), (1, 110.0)]),
            Vec::<usize>::new()
        );
        assert_eq!(
            envelope_regressed_indices(&[150.0, 120.0], &[(0, 160.0), (1, 140.0)]),
            vec![0]
        );
        assert_eq!(
            envelope_regressed_indices(&[150.0, 120.0], &[(0, 150.0), (1, 125.0)]),
            vec![1]
        );
        assert_eq!(
            envelope_regressed_indices(&[150.0], &[(0, 110.0), (1, 105.0)]),
            vec![1]
        );
        assert_eq!(envelope_regressed_indices(&[], &[(0, 101.0)]), vec![0]);
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "burdens built from small integers are exact in f64"
    )]
    fn value_burden_handles_scalars_objects_and_non_numeric_values() {
        assert_eq!(value_burden(&json!(113)), 113.0);
        assert_eq!(
            value_burden(&json!({"changes": 5, "responsibilities": 3})),
            8.0
        );
        assert_eq!(
            value_burden(&json!({"intersection": 40, "union": 44})),
            40.0
        );
        assert_eq!(value_burden(&json!({"flag": true})), 1.0);
        assert_eq!(value_burden(&json!("match")), 1.0);
    }

    #[test]
    fn policy_weakening_detects_mode_downgrade_and_looser_thresholds() {
        let registry = policy::registry("rust-v1").unwrap();
        let raw = include_str!("../../examples/quality-policy.json");
        let reference = policy::parse(raw, &registry).unwrap();
        let mut weakened = policy::parse(raw, &registry).unwrap();
        assert!(detect_policy_weakening(Some(&reference), &weakened).is_empty());
        assert!(detect_policy_weakening(None, &weakened).is_empty());

        weakened
            .rules
            .get_mut("rust.function_lines")
            .unwrap()
            .parameters
            .insert("maximum".into(), 200);
        weakened
            .rules
            .get_mut("rust.function_arguments")
            .unwrap()
            .mode = Mode::Report;

        let detected = detect_policy_weakening(Some(&reference), &weakened);
        assert_eq!(
            detected,
            vec!["rust.function_arguments", "rust.function_lines"]
        );
    }

    #[test]
    fn parameter_weakening_distinguishes_tighter_from_looser_bounds() {
        assert!(parameter_weakened("maximum", 100, 120));
        assert!(!parameter_weakened("maximum", 100, 100));
        assert!(!parameter_weakened("maximum", 100, 80));
        assert!(parameter_weakened("minimum_tokens", 20, 30));
        assert!(!parameter_weakened("minimum_tokens", 20, 15));
        assert!(!parameter_weakened("other", 10, 20));
    }
}
