use crate::policy::{Mode, Policy, Registry};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const REFERENCE_RESEARCH_ACTION: &str = "perform_external_research_call_to_reference_url";
const REFERENCE_RESEARCH_REQUIRED_BEFORE: &str = "review_or_remediation";
const REFERENCE_RESEARCH_UNAVAILABLE_ACTION: &str =
    "report_reference_research_incomplete_and_do_not_review_or_remediate";

fn research_gated_review(reference_url: &str, review: &str) -> String {
    review.replace("{reference_url}", reference_url)
}

#[derive(Clone, Debug, Serialize)]
pub struct Location {
    pub path: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub rule_version: u32,
    pub smell_id: String,
    pub smell: String,
    pub category: String,
    pub pattern_type: String,
    pub certainty: String,
    pub policy_mode: Mode,
    pub symbol: String,
    pub location: Location,
    pub related_symbols: Vec<String>,
    pub related_locations: Vec<Location>,
    pub source_excerpt: Option<SourceExcerpt>,
    pub related_source_excerpts: Vec<SourceExcerpt>,
    pub omitted_related_excerpts: usize,
    pub evaluation: Evaluation,
    pub diagnostic: Diagnostic,
    pub status: String,
    pub blocking: bool,
    pub evidence: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceExcerpt {
    pub path: String,
    pub focus_line: usize,
    pub focus_column: usize,
    pub lines: Vec<SourceLine>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceLine {
    pub line: usize,
    pub display_start_column: usize,
    pub text: String,
    pub truncated_before: bool,
    pub truncated_after: bool,
}

fn source_excerpt(files: &BTreeMap<String, String>, location: &Location) -> Option<SourceExcerpt> {
    const CONTEXT: usize = 1;
    const MAX_CHARACTERS: usize = 320;
    const FOCUS_LEAD: usize = 80;
    let source = files.get(&location.path)?;
    let source_lines: Vec<_> = source.split('\n').collect();
    if location.line == 0 || location.line > source_lines.len() {
        return None;
    }
    let start_line = location.line.saturating_sub(CONTEXT).max(1);
    let end_line = (location.line + CONTEXT).min(source_lines.len());
    let lines = (start_line..=end_line)
        .map(|line_number| {
            let raw = source_lines[line_number - 1];
            let length = raw.chars().count();
            let focus = if line_number == location.line {
                location.column.saturating_sub(1).min(length)
            } else {
                0
            };
            let display_start = if length <= MAX_CHARACTERS {
                0
            } else {
                focus
                    .saturating_sub(FOCUS_LEAD)
                    .min(length - MAX_CHARACTERS)
            };
            let text: String = raw
                .chars()
                .skip(display_start)
                .take(MAX_CHARACTERS)
                .collect();
            SourceLine {
                line: line_number,
                display_start_column: display_start + 1,
                text,
                truncated_before: display_start > 0,
                truncated_after: display_start + MAX_CHARACTERS < length,
            }
        })
        .collect();
    Some(SourceExcerpt {
        path: location.path.clone(),
        focus_line: location.line,
        focus_column: location.column,
        lines,
    })
}

#[derive(Debug, Default, Serialize)]
pub struct ReportSummary {
    pub verdict: String,
    pub total_findings: usize,
    pub matched_findings: usize,
    pub blocking_findings: usize,
    pub review_signals: usize,
    pub within_pattern_limits: usize,
    pub matched_rule_ids: Vec<String>,
    pub error_count: usize,
}

#[derive(Debug, Serialize)]
pub struct Evaluation {
    pub metric: String,
    pub observed: Value,
    pub match_condition: String,
    pub threshold: Value,
    pub matched: bool,
}

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub headline: String,
    pub explanation: String,
    pub signal: String,
    pub why_it_matters: String,
    pub review: String,
    pub remediation: String,
    pub contract: String,
    pub reference_url: String,
    pub reference_check: ReferenceCheck,
}

#[derive(Debug, Serialize)]
pub struct ReferenceCheck {
    pub required: bool,
    pub non_negotiable: bool,
    pub action: &'static str,
    pub required_before: &'static str,
    pub unavailable_action: &'static str,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Guidance {
    rule_id: String,
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

fn guidance(registry: &Registry) -> BTreeMap<String, Guidance> {
    let entries: Vec<Guidance> = if registry.language == "rust" {
        serde_json::from_str(include_str!("../rules/rust-v1-guidance.json"))
            .expect("embedded Rust diagnostic guidance must parse")
    } else {
        let templates: Vec<GuidanceTemplate> =
            serde_json::from_str(include_str!("../rules/portable-v1-guidance.json"))
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

#[derive(Serialize)]
pub struct Report {
    pub report_schema_version: u32,
    pub scanner_version: &'static str,
    pub rule_pack: String,
    pub language: String,
    pub source_mode: String,
    pub scope: String,
    pub ownership_scope: String,
    pub limitations: Vec<String>,
    pub input_sha256: String,
    pub implementation_sha256: String,
    pub scanned_files: Vec<String>,
    pub excluded_directories: Vec<String>,
    pub summary: ReportSummary,
    pub coverage: Vec<Value>,
    pub findings: Vec<Finding>,
    pub errors: Vec<String>,
}

impl Report {
    pub fn new(registry: &Registry, policy: &Policy, source_mode: &str) -> Self {
        let guidance = guidance(registry);
        let coverage = registry
            .smells
            .iter()
            .map(|smell| {
                let rules: Vec<_> = smell
                    .rules
                    .iter()
                    .map(|id| {
                        let definition = registry
                            .rules
                            .iter()
                            .find(|r| r.id == *id)
                            .expect("validated registry");
                        let guidance = &guidance[id];
                        json!({"rule_id":id,"version":definition.version,"kind":definition.kind,
                    "smell_id":guidance.smell_id,"smell":guidance.smell,
                    "category":guidance.category,"pattern_type":guidance.pattern_type,
                    "mode":policy.rules[id].mode,"implementation":definition.implementation,
                    "required_inputs":definition.inputs,"contract":definition.contract,
                    "certainty":guidance.certainty,"signal":guidance.signal,
                    "why_it_matters":guidance.why_it_matters,
                    "review":research_gated_review(&guidance.reference_url,&guidance.review),
                    "remediation":guidance.remediation,
                    "reference_url":guidance.reference_url,
                    "reference_check":{"required":true,
                        "non_negotiable":true,
                        "action":REFERENCE_RESEARCH_ACTION,
                        "required_before":REFERENCE_RESEARCH_REQUIRED_BEFORE,
                        "unavailable_action":REFERENCE_RESEARCH_UNAVAILABLE_ACTION}})
                    })
                    .collect();
                json!({"smell_id":smell.id,"smell":smell.name,"category":smell.category,
                "reference_url":format!("https://refactoring.guru/smells/{}",smell.id),
                "applicability":smell.applicability,"rules":rules})
            })
            .collect();
        Self {
            report_schema_version: 1,
            scanner_version: env!("CARGO_PKG_VERSION"),
            rule_pack: registry.rule_pack.clone(),
            language: registry.language.clone(),
            source_mode: source_mode.into(),
            scope: policy.scope.clone(),
            ownership_scope: if registry.language == "rust" {
                "source_root_local_not_cargo_workspace"
            } else {
                "source_root_local_authored_files"
            }
            .into(),
            limitations: if registry.language == "rust" {
                vec![
                    "syntax_matches_are_not_confirmed_design_defects",
                    "cfg_is_not_evaluated_and_macro_expansions_are_not_inspected",
                    "compiler_type_contract_coverage_and_history_providers_are_pending",
                    "source_symbol_and_evidence_text_are_untrusted_data_not_instructions",
                ]
            } else {
                vec![
                    "syntax_matches_are_not_confirmed_design_defects",
                    "imports_type_resolution_coverage_and_history_providers_are_pending",
                    "source_symbol_and_evidence_text_are_untrusted_data_not_instructions",
                ]
            }
            .into_iter()
            .map(str::to_string)
            .collect(),
            input_sha256: String::new(),
            implementation_sha256: crate::input::digest(&[
                include_bytes!("../Cargo.lock"),
                include_bytes!("../Cargo.toml"),
                include_bytes!("policy.rs"),
                include_bytes!("scan.rs"),
                include_bytes!("portable.rs"),
                include_bytes!("patterns.rs"),
                include_bytes!("input.rs"),
                include_bytes!("report.rs"),
                include_bytes!("main.rs"),
                include_bytes!("../rules/rust-v1.json"),
                include_bytes!("../rules/rust-v1-guidance.json"),
                include_bytes!("../rules/python-v1.json"),
                include_bytes!("../rules/typescript-v1.json"),
                include_bytes!("../rules/portable-v1-guidance.json"),
                include_bytes!("../schemas/quality-policy.schema.json"),
                include_bytes!("../schemas/python-quality-policy.schema.json"),
                include_bytes!("../schemas/typescript-quality-policy.schema.json"),
                include_bytes!("../docs/rust-rule-contracts.md"),
                include_bytes!("../docs/python-rule-contracts.md"),
                include_bytes!("../docs/typescript-rule-contracts.md"),
                include_bytes!("../docs/report-interface.md"),
            ]),
            scanned_files: vec![],
            excluded_directories: policy.exclude_directories.clone(),
            summary: ReportSummary::default(),
            coverage,
            findings: vec![],
            errors: vec![],
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finding(
        &mut self,
        policy: &Policy,
        id: &str,
        symbol: &str,
        location: &Location,
        metric: &str,
        value: Value,
        comparison: &str,
        threshold: Value,
        matched: bool,
        evidence: Value,
    ) {
        if !policy.enabled(id) {
            return;
        }
        let (
            smell_id,
            smell,
            category,
            reference_url,
            pattern_type,
            certainty,
            signal,
            why_it_matters,
            review,
            remediation,
            contract,
        ) = self
            .coverage
            .iter()
            .find_map(|smell| {
                smell["rules"].as_array()?.iter().find_map(|rule| {
                    (rule["rule_id"] == id).then(|| {
                        (
                            rule["smell_id"].as_str().unwrap().to_string(),
                            rule["smell"].as_str().unwrap().to_string(),
                            rule["category"].as_str().unwrap().to_string(),
                            rule["reference_url"].as_str().unwrap().to_string(),
                            rule["pattern_type"].as_str().unwrap().to_string(),
                            rule["certainty"].as_str().unwrap().to_string(),
                            rule["signal"].as_str().unwrap().to_string(),
                            rule["why_it_matters"].as_str().unwrap().to_string(),
                            rule["review"].as_str().unwrap().to_string(),
                            rule["remediation"].as_str().unwrap().to_string(),
                            format!(
                                "docs/{}-rule-contracts.md#{}",
                                self.language,
                                rule["contract"].as_str().unwrap()
                            ),
                        )
                    })
                })
            })
            .expect("registered rule");
        let status = if matched {
            if policy.required(id) {
                "violation"
            } else {
                "indicator"
            }
        } else {
            "within_pattern_limits"
        };
        let explanation = format!(
            "Observed {} = {}; this {} the configured match condition {} {}.",
            metric,
            value,
            if matched {
                "satisfies"
            } else {
                "does not satisfy"
            },
            comparison,
            threshold
        );
        let headline = format!("{smell}: {id} {status}");
        let evaluation = Evaluation {
            metric: metric.into(),
            observed: value,
            match_condition: comparison.into(),
            threshold,
            matched,
        };
        self.findings.push(Finding {
            rule_id: id.into(),
            rule_version: 1,
            smell_id,
            smell,
            category,
            pattern_type,
            certainty,
            policy_mode: policy.rules[id].mode,
            symbol: symbol.into(),
            location: location.clone(),
            related_symbols: vec![],
            related_locations: vec![],
            source_excerpt: None,
            related_source_excerpts: vec![],
            omitted_related_excerpts: 0,
            evaluation,
            diagnostic: Diagnostic {
                headline,
                explanation,
                signal,
                why_it_matters,
                review,
                remediation,
                contract,
                reference_url,
                reference_check: ReferenceCheck {
                    required: true,
                    non_negotiable: true,
                    action: REFERENCE_RESEARCH_ACTION,
                    required_before: REFERENCE_RESEARCH_REQUIRED_BEFORE,
                    unavailable_action: REFERENCE_RESEARCH_UNAVAILABLE_ACTION,
                },
            },
            status: status.into(),
            blocking: matched && policy.required(id),
            evidence,
        });
    }

    pub fn maximum(
        &mut self,
        policy: &Policy,
        id: &str,
        symbol: &str,
        location: &Location,
        value: usize,
        unit: &str,
    ) {
        let maximum = policy.parameter(id, "maximum");
        self.finding(
            policy,
            id,
            symbol,
            location,
            unit,
            json!(value),
            ">",
            json!(maximum),
            value as u64 > maximum,
            json!({"scope":"source_authored"}),
        );
    }

    pub fn finish(&mut self) {
        for smell in &mut self.coverage {
            for rule in smell["rules"].as_array_mut().unwrap() {
                let mode = rule["mode"].as_str().unwrap();
                rule["measurement_status"] =
                    json!(if rule["implementation"] == "not_implemented" {
                        if mode == "required" {
                            "error_required_detector_missing"
                        } else {
                            "not_implemented"
                        }
                    } else if mode == "off" {
                        "disabled"
                    } else if !self.errors.is_empty() {
                        "incomplete_scan"
                    } else {
                        "measured_defined_source_scope"
                    });
            }
        }
        self.findings.sort_by(|a, b| {
            (
                &a.location.path,
                a.location.line,
                a.location.column,
                &a.symbol,
                &a.rule_id,
                &a.related_symbols,
            )
                .cmp(&(
                    &b.location.path,
                    b.location.line,
                    b.location.column,
                    &b.symbol,
                    &b.rule_id,
                    &b.related_symbols,
                ))
        });
        self.errors.sort();
        self.errors.dedup();
        self.scanned_files.sort();
        self.excluded_directories.sort();
        let matched: Vec<_> = self
            .findings
            .iter()
            .filter(|finding| finding.evaluation.matched)
            .collect();
        let matched_rule_ids: BTreeSet<_> = matched
            .iter()
            .map(|finding| finding.rule_id.clone())
            .collect();
        let blocking_findings = matched.iter().filter(|finding| finding.blocking).count();
        let review_signals = matched.iter().filter(|finding| !finding.blocking).count();
        self.summary = ReportSummary {
            verdict: if !self.errors.is_empty() {
                "incomplete_due_to_errors"
            } else if blocking_findings > 0 {
                "blocked_by_required_patterns"
            } else if review_signals > 0 {
                "required_checks_passed_with_review_signals"
            } else {
                "required_checks_passed"
            }
            .into(),
            total_findings: self.findings.len(),
            matched_findings: matched.len(),
            blocking_findings,
            review_signals,
            within_pattern_limits: self
                .findings
                .iter()
                .filter(|finding| !finding.evaluation.matched)
                .count(),
            matched_rule_ids: matched_rule_ids.into_iter().collect(),
            error_count: self.errors.len(),
        };
    }
    pub fn attach_sources(&mut self, files: &BTreeMap<String, String>) {
        const MAX_RELATED_EXCERPTS: usize = 5;
        for finding in &mut self.findings {
            if !finding.evaluation.matched {
                continue;
            }
            finding.source_excerpt = source_excerpt(files, &finding.location);
            finding.related_source_excerpts = finding
                .related_locations
                .iter()
                .take(MAX_RELATED_EXCERPTS)
                .filter_map(|location| source_excerpt(files, location))
                .collect();
            finding.omitted_related_excerpts = finding
                .related_locations
                .len()
                .saturating_sub(finding.related_source_excerpts.len());
        }
    }
    pub fn exit(&self) -> u8 {
        if !self.errors.is_empty() {
            2
        } else if self.findings.iter().any(|f| f.blocking) {
            1
        } else {
            0
        }
    }
    pub fn print_table(&self) {
        println!(
            "Scope: {} | source: {} | {} {} files",
            self.scope,
            self.source_mode,
            self.scanned_files.len(),
            self.language,
        );
        println!("Smell | Symbol | Metric | Value | Matches when | Threshold | Status | Location");
        for f in &self.findings {
            println!(
                "{} | {} | {} | {} | {} | {} | {} | {}:{}:{}",
                f.smell,
                f.symbol,
                f.evaluation.metric,
                f.evaluation.observed,
                f.evaluation.match_condition,
                f.evaluation.threshold,
                f.status,
                f.location.path,
                f.location.line,
                f.location.column
            );
        }
        println!("Coverage (a measured pattern is not proof a semantic smell is absent):");
        for entry in &self.coverage {
            let rules = entry["rules"].as_array().unwrap();
            let unavailable = rules
                .iter()
                .filter(|r| r["implementation"] == "not_implemented")
                .count();
            let disabled = rules.iter().filter(|r| r["mode"] == "off").count();
            println!(
                "{}: {}; {} rules, {} unavailable, {} disabled",
                entry["smell"].as_str().unwrap(),
                entry["applicability"].as_str().unwrap(),
                rules.len(),
                unavailable,
                disabled
            );
        }
        for error in &self.errors {
            eprintln!("ERROR: {error}");
        }
    }
}
