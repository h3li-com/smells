use crate::policy::Registry;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const REFERENCE_RESEARCH_ACTION: &str =
    "perform_external_research_call_to_reference_url";
pub(super) const REFERENCE_RESEARCH_REQUIRED_BEFORE: &str = "review_or_remediation";
pub(super) const REFERENCE_RESEARCH_UNAVAILABLE_ACTION: &str =
    "report_reference_research_incomplete_and_do_not_review_or_remediate";

pub(super) fn research_gated_review(reference_url: &str, review: &str) -> String {
    review.replace("{reference_url}", reference_url)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Guidance {
    pub(super) rule_id: String,
    pub(super) smell_id: String,
    pub(super) smell: String,
    pub(super) category: String,
    pub(super) pattern_type: String,
    pub(super) reference_url: String,
    pub(super) certainty: String,
    pub(super) signal: String,
    pub(super) why_it_matters: String,
    pub(super) review: String,
    pub(super) remediation: String,
}

#[derive(Clone)]
pub(super) struct RuleMetadata {
    pub(super) guidance: Guidance,
    pub(super) contract: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuidanceTemplate {
    rule_suffix: String,
    smell_id: String,
    smell: String,
    category: String,
    pattern_type: String,
    reference_url: String,
    certainty: String,
    signal: String,
    why_it_matters: String,
    review: String,
    remediation: String,
}

pub(super) fn guidance(registry: &Registry) -> BTreeMap<String, Guidance> {
    let entries: Vec<Guidance> = if registry.language == "rust" {
        serde_json::from_str(include_str!("../../rules/rust-v1-guidance.json"))
            .expect("embedded Rust diagnostic guidance must parse")
    } else {
        let templates: Vec<GuidanceTemplate> =
            serde_json::from_str(include_str!("../../rules/portable-v1-guidance.json"))
                .expect("embedded portable diagnostic guidance must parse");
        templates
            .into_iter()
            .map(|template| Guidance {
                rule_id: format!("{}.{}", registry.language, template.rule_suffix),
                smell_id: template.smell_id,
                smell: template.smell,
                category: template.category,
                pattern_type: template.pattern_type,
                reference_url: template.reference_url,
                certainty: template.certainty,
                signal: template.signal,
                why_it_matters: template.why_it_matters,
                review: template.review,
                remediation: template.remediation,
            })
            .collect()
    };
    let expected: BTreeSet<_> = registry.rules.iter().map(|rule| rule.id.as_str()).collect();
    let actual: BTreeSet<_> = entries.iter().map(|entry| entry.rule_id.as_str()).collect();
    assert_eq!(entries.len(), actual.len(), "duplicate diagnostic guidance");
    assert_eq!(
        expected, actual,
        "diagnostic guidance must cover every rule"
    );
    assert!(entries.iter().all(|entry| {
        matches!(
            entry.certainty.as_str(),
            "exact_source_metric"
                | "structural_indicator"
                | "provider_evidence"
                | "typed_indicator"
                | "project_contract"
                | "conservative_source_indicator"
                | "conservative_or_external_evidence"
                | "history_signal"
        ) && entry
            .reference_url
            .starts_with("https://refactoring.guru/smells/")
            && !entry.smell_id.is_empty()
            && !entry.smell.is_empty()
            && !entry.category.is_empty()
            && !entry.pattern_type.is_empty()
            && !entry.signal.is_empty()
            && !entry.why_it_matters.is_empty()
            && entry.review.starts_with("NON-NEGOTIABLE RESEARCH:")
            && entry.review.contains("{reference_url}")
            && !entry.remediation.is_empty()
    }));
    for smell in &registry.smells {
        let expected_url = format!("https://refactoring.guru/smells/{}", smell.id);
        for rule_id in &smell.rules {
            let entry = entries
                .iter()
                .find(|entry| entry.rule_id == *rule_id)
                .expect("guidance coverage checked above");
            assert_eq!(
                entry.reference_url, expected_url,
                "diagnostic guidance URL must match its catalog smell for {rule_id}"
            );
            assert_eq!(
                entry.smell_id, smell.id,
                "diagnostic guidance smell ID must match the catalog for {rule_id}"
            );
            assert_eq!(
                entry.smell, smell.name,
                "diagnostic guidance smell name must match the catalog for {rule_id}"
            );
            assert_eq!(
                entry.category, smell.category,
                "diagnostic guidance category must match the catalog for {rule_id}"
            );
            let definition = registry
                .rules
                .iter()
                .find(|definition| definition.id == *rule_id)
                .expect("validated registry");
            assert_eq!(
                entry.pattern_type, definition.kind,
                "diagnostic guidance pattern type must match the rule for {rule_id}"
            );
        }
    }
    entries
        .into_iter()
        .map(|entry| (entry.rule_id.clone(), entry))
        .collect()
}
