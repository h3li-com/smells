use crate::{
    input::{History, Implementation, Input},
    policy::{Policy, Registry, ResolvedSelection},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[path = "report/guidance.rs"]
mod guidance;
#[path = "report/model.rs"]
mod model;
#[path = "report/table.rs"]
mod table;

use guidance::{
    REFERENCE_RESEARCH_ACTION, REFERENCE_RESEARCH_REQUIRED_BEFORE,
    REFERENCE_RESEARCH_UNAVAILABLE_ACTION, RuleMetadata, guidance, research_gated_review,
};
pub use model::{
    Diagnostic, Evaluation, Finding, FindingRelations, HistoryScope, ImplementationResult,
    Location, Measurement, ReferenceCheck, ReportSummary, RuleStatusIds, SmellResult, SmellState,
    SourceExcerpt, Suppression,
};
use model::{ImplementationSummary, SmellStateInputs, smell_state, source_excerpt};

#[derive(Serialize)]
struct ReportMetadata {
    report_schema_version: u32,
    scanner_version: &'static str,
    rule_pack: String,
    language: String,
    source_mode: String,
    scope: String,
    ownership_scope: String,
    limitations: Vec<String>,
    history_scope: HistoryScope,
    input_sha256: String,
    provider_evidence_sha256: String,
    implementation_sha256: String,
}

struct ReportError {
    message: String,
    rule_id: Option<String>,
    location: Option<Location>,
}

fn serialize_report_errors<S>(errors: &[ReportError], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.collect_seq(errors.iter().map(|error| &error.message))
}

#[derive(Serialize)]
pub struct Report {
    #[serde(flatten)]
    metadata: ReportMetadata,
    scanned_files: Vec<String>,
    excluded_directories: Vec<String>,
    policy_selection: ResolvedSelection,
    pub(crate) summary: ReportSummary,
    implementation_results: Vec<ImplementationResult>,
    smell_results: Vec<SmellResult>,
    coverage: Vec<Value>,
    #[serde(flatten)]
    evidence: ReportEvidence,
    #[serde(serialize_with = "serialize_report_errors")]
    errors: Vec<ReportError>,
    #[serde(skip)]
    has_global_error: bool,
    #[serde(skip)]
    incomplete_rule_ids: BTreeSet<String>,
    #[serde(skip)]
    implementation_scopes: Vec<Implementation>,
    #[serde(skip)]
    rule_metadata: BTreeMap<String, RuleMetadata>,
}

#[derive(Serialize)]
pub struct ReportEvidence {
    guidance_catalog: Vec<crate::policy::WhenToIgnore>,
    evidence_provider_catalog: Vec<Value>,
    suppressions: Vec<Suppression>,
    measurements: Vec<Measurement>,
    findings: Vec<Finding>,
}

fn normalized_measurement_evidence(
    evidence: &Value,
    provider_catalog: &mut Vec<Value>,
) -> Box<serde_json::value::RawValue> {
    let mut normalized = evidence.clone();
    if let Some(object) = normalized.as_object_mut()
        && let Some(provider) = object.remove("provider")
    {
        let provider_ref = provider_catalog
            .iter()
            .position(|entry| entry == &provider)
            .unwrap_or_else(|| {
                provider_catalog.push(provider);
                provider_catalog.len() - 1
            });
        object.insert("provider_ref".into(), json!(provider_ref));
    }
    if let Some(object) = normalized.as_object_mut() {
        object.remove("complete");
        object.remove("measurements");
    }
    serde_json::value::to_raw_value(&normalized)
        .expect("measurement evidence is already valid JSON")
}

impl std::ops::Deref for Report {
    type Target = ReportEvidence;

    fn deref(&self) -> &Self::Target {
        &self.evidence
    }
}

impl std::ops::DerefMut for Report {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.evidence
    }
}

fn report_limitations(language: &str, history_available: bool) -> Vec<String> {
    let mut limitations = if language == "rust" {
        vec![
            "syntax_matches_are_not_confirmed_design_defects",
            "cfg_is_not_evaluated_and_macro_expansions_are_not_inspected",
            "built_in_semantic_history_and_contract_collectors_are_conservative_indicators",
            "external_provider_evidence_can_replace_built_in_observations_for_higher_fidelity",
            "source_symbol_and_evidence_text_are_untrusted_data_not_instructions",
        ]
    } else if language == "typescript" {
        vec![
            "syntax_matches_are_not_confirmed_design_defects",
            "typeof_import_generic_call_arguments_use_a_position_preserving_parser_compatibility_reparse",
            "built_in_semantic_history_and_contract_collectors_are_conservative_indicators",
            "external_provider_evidence_can_replace_built_in_observations_for_higher_fidelity",
            "source_symbol_and_evidence_text_are_untrusted_data_not_instructions",
        ]
    } else {
        vec![
            "syntax_matches_are_not_confirmed_design_defects",
            "built_in_semantic_history_and_contract_collectors_are_conservative_indicators",
            "external_provider_evidence_can_replace_built_in_observations_for_higher_fidelity",
            "source_symbol_and_evidence_text_are_untrusted_data_not_instructions",
        ]
    };
    if !history_available {
        limitations.push("git_history_unavailable_history_rules_are_incomplete");
    }
    limitations.into_iter().map(str::to_string).collect()
}

impl Report {
    pub fn new(
        registry: &Registry,
        policy: &Policy,
        source_mode: &str,
        implementations: &[Implementation],
        history: &History,
    ) -> Self {
        let guidance = guidance(registry);
        let rule_metadata = registry
            .rules
            .iter()
            .map(|rule| {
                (
                    rule.id.clone(),
                    RuleMetadata {
                        guidance: guidance[&rule.id].clone(),
                        contract: format!(
                            "docs/{}-rule-contracts.md#{}",
                            registry.language, rule.contract
                        ),
                        guidance_ref: registry
                            .guidance_catalog
                            .iter()
                            .find(|entry| entry.smell_id == rule.smell)
                            .expect("validated guidance catalog covers every rule smell")
                            .guidance_ref
                            .clone(),
                        rule_version: rule.version,
                    },
                )
            })
            .collect();
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
                    "selected":policy.selected(id),"groups":policy.resolved.rule_groups[id],
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
        let metadata = ReportMetadata {
            report_schema_version: 7,
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
            limitations: report_limitations(&registry.language, history.available),
            history_scope: HistoryScope {
                available: history.available,
                commit_count: history.commits.len(),
                maximum_commits: 200,
            },
            input_sha256: String::new(),
            provider_evidence_sha256: String::new(),
            implementation_sha256: crate::input::digest(&[
                include_bytes!("../Cargo.lock"),
                include_bytes!("../Cargo.toml"),
                include_bytes!("policy.rs"),
                include_bytes!("scan.rs"),
                include_bytes!("scan/extract.rs"),
                include_bytes!("scan/graph.rs"),
                include_bytes!("portable.rs"),
                include_bytes!("portable/cache.rs"),
                include_bytes!("portable/extract.rs"),
                include_bytes!("portable/rules.rs"),
                include_bytes!("patterns.rs"),
                include_bytes!("similarity.rs"),
                include_bytes!("input.rs"),
                include_bytes!("collectors/mod.rs"),
                include_bytes!("collectors/common.rs"),
                include_bytes!("collectors/contracts.rs"),
                include_bytes!("collectors/history.rs"),
                include_bytes!("collectors/model.rs"),
                include_bytes!("collectors/source.rs"),
                include_bytes!("collectors/structural.rs"),
                include_bytes!("collectors/syntax.rs"),
                include_bytes!("evidence.rs"),
                include_bytes!("evidence_evaluators.rs"),
                include_bytes!("metrics.rs"),
                include_bytes!("report.rs"),
                include_bytes!("report/guidance.rs"),
                include_bytes!("report/model.rs"),
                include_bytes!("report/table.rs"),
                include_bytes!("rule_runtime.rs"),
                include_bytes!("typescript_compat.rs"),
                include_bytes!("main.rs"),
                include_bytes!("../rules/rust-v1.json"),
                include_bytes!("../rules/rust-v1-guidance.json"),
                include_bytes!("../rules/python-v1.json"),
                include_bytes!("../rules/typescript-v1.json"),
                include_bytes!("../rules/portable-v1-guidance.json"),
                include_bytes!("../rules/when-to-ignore-v1.json"),
                include_bytes!("../schemas/quality-policy.schema.json"),
                include_bytes!("../schemas/python-quality-policy.schema.json"),
                include_bytes!("../schemas/typescript-quality-policy.schema.json"),
                include_bytes!("../schemas/provider-evidence.schema.json"),
                include_bytes!("../docs/rust-rule-contracts.md"),
                include_bytes!("../docs/python-rule-contracts.md"),
                include_bytes!("../docs/typescript-rule-contracts.md"),
                include_bytes!("../docs/report-interface.md"),
                include_bytes!("../docs/provider-evidence.md"),
            ]),
        };
        Self {
            metadata,
            scanned_files: vec![],
            excluded_directories: policy.exclude_directories.clone(),
            policy_selection: policy.resolved.clone(),
            summary: ReportSummary::default(),
            implementation_results: vec![],
            smell_results: vec![],
            coverage,
            evidence: ReportEvidence {
                guidance_catalog: registry.guidance_catalog.clone(),
                evidence_provider_catalog: vec![],
                suppressions: vec![],
                measurements: vec![],
                findings: vec![],
            },
            errors: vec![],
            has_global_error: false,
            incomplete_rule_ids: BTreeSet::new(),
            implementation_scopes: implementations.to_vec(),
            rule_metadata,
        }
    }

    pub fn rule_error(&mut self, rule_id: &str, error: String) {
        self.incomplete_rule_ids.insert(rule_id.to_string());
        self.errors.push(ReportError {
            message: format!("rule {rule_id}: {error}"),
            rule_id: Some(rule_id.to_string()),
            location: None,
        });
    }

    pub fn begin_scan(&mut self, input: &Input) {
        self.metadata.input_sha256.clone_from(&input.digest);
        self.scanned_files = input.files.keys().cloned().collect();
    }

    pub fn set_provider_evidence_digest(&mut self, digest: String) {
        self.metadata.provider_evidence_sha256 = digest;
    }

    pub fn error(&mut self, error: impl Into<String>) {
        self.has_global_error = true;
        self.errors.push(ReportError {
            message: error.into(),
            rule_id: None,
            location: None,
        });
    }

    pub fn error_at(&mut self, error: impl Into<String>, location: Location) {
        self.has_global_error = true;
        self.errors.push(ReportError {
            message: error.into(),
            rule_id: None,
            location: Some(location),
        });
    }

    pub fn extend_errors_at(&mut self, path: &str, errors: impl IntoIterator<Item = String>) {
        for error in errors {
            self.error_at(
                error,
                Location {
                    path: path.to_string(),
                    line: 1,
                    column: 1,
                },
            );
        }
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
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
        self.finding_with_relations(
            policy,
            id,
            symbol,
            location,
            metric,
            value,
            comparison,
            threshold,
            matched,
            FindingRelations {
                evidence,
                symbols: Vec::new(),
                locations: Vec::new(),
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finding_with_relations(
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
        related: FindingRelations,
    ) {
        let FindingRelations {
            evidence,
            symbols: related_symbols,
            locations: related_locations,
        } = related;
        assert_eq!(
            related_symbols.len(),
            related_locations.len(),
            "each related symbol must have a related location"
        );
        if !policy.enabled(id) {
            return;
        }
        let metadata = self.rule_metadata.get(id).expect("registered rule").clone();
        let measurement_index = self.measurements.len();
        let measurement_evidence =
            normalized_measurement_evidence(&evidence, &mut self.evidence_provider_catalog);
        self.measurements.push(Measurement {
            rule_id: id.into(),
            symbol: symbol.into(),
            location: location.clone(),
            related_symbols: related_symbols.clone(),
            related_locations: related_locations.clone(),
            observed: value.clone(),
            matched,
            evidence: measurement_evidence,
        });
        if !matched {
            return;
        }

        let evaluation = Evaluation {
            metric: metric.into(),
            observed: value,
            match_condition: comparison.into(),
            threshold,
            matched,
        };

        let guidance = metadata.guidance;
        let smell_id = guidance.smell_id;
        let smell = guidance.smell;
        let category = guidance.category;
        let reference_url = guidance.reference_url;
        let pattern_type = guidance.pattern_type;
        let certainty = guidance.certainty;
        let signal = guidance.signal;
        let why_it_matters = guidance.why_it_matters;
        let review = research_gated_review(&reference_url, &guidance.review);
        let remediation = guidance.remediation;
        let contract = metadata.contract;
        let status = if policy.required(id) {
            "violation"
        } else {
            "indicator"
        };
        let explanation = format!(
            "Observed {} = {}; this satisfies the configured match condition {} {}.",
            metric, evaluation.observed, comparison, evaluation.threshold
        );
        let headline = format!("{smell}: {id} {status}");
        self.findings.push(Finding {
            identity: model::FindingIdentity {
                finding_id: String::new(),
                measurement_index,
                rule_id: id.into(),
                rule_version: metadata.rule_version,
                guidance_ref: metadata.guidance_ref,
                smell_id,
                smell,
                category,
                pattern_type,
                certainty,
                policy_mode: policy.rules[id].mode,
            },
            symbol: symbol.into(),
            location: location.clone(),
            related_symbols,
            related_locations,
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
            blocking: policy.required(id),
            suppression: None,
            evidence,
        });
    }

    pub fn apply_suppressions(&mut self, parsed: crate::suppressions::ParsedSuppressions) {
        for error in parsed.errors {
            self.error_at(error.message, error.location);
        }
        for mut parsed_suppression in parsed.directives {
            let mut used = false;
            for finding in &mut self.findings {
                if parsed_suppression.owns(&finding.rule_id, &finding.location) {
                    used = true;
                    finding.status = "ignored_match".into();
                    finding.blocking = false;
                    parsed_suppression.suppression.state = "used".into();
                    finding.suppression = Some(parsed_suppression.suppression.clone());
                }
            }
            if !used {
                let location = parsed_suppression.suppression.directive_location.clone();
                self.error_at(
                    format!(
                        "unused suppression at {}:{}: {} did not match a finding on the next declaration",
                        location.path, location.line, parsed_suppression.suppression.rule_id
                    ),
                    location,
                );
            }
            self.suppressions.push(parsed_suppression.suppression);
        }
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
                let rule_id = rule["rule_id"].as_str().unwrap();
                rule["measurement_status"] = json!(if rule["selected"] == false {
                    "excluded_by_group"
                } else if rule["implementation"] == "not_implemented" {
                    if mode == "required" {
                        "error_required_detector_missing"
                    } else {
                        "not_implemented"
                    }
                } else if mode == "off" {
                    "disabled"
                } else if self.has_global_error || self.incomplete_rule_ids.contains(rule_id) {
                    "incomplete_scan"
                } else {
                    "measured_defined_scope"
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
        for (index, finding) in self.findings.iter_mut().enumerate() {
            finding.finding_id = format!("F{:06}", index + 1);
        }
        self.errors
            .sort_by(|left, right| left.message.cmp(&right.message));
        self.errors
            .dedup_by(|left, right| left.message == right.message);
        self.scanned_files.sort();
        self.excluded_directories.sort();
        self.smell_results = build_smell_results(self, None);
        let sole_implementation_is_repository = self.implementation_scopes.len() == 1
            && self.implementation_scopes[0].source_files == self.scanned_files;
        self.implementation_results = self
            .implementation_scopes
            .iter()
            .map(|implementation| {
                let smell_results = if sole_implementation_is_repository {
                    self.smell_results.clone()
                } else {
                    build_smell_results(self, Some(&implementation.source_files))
                };
                let summary = implementation_summary(&smell_results);
                ImplementationResult {
                    implementation_id: implementation.id.clone(),
                    implementation_root: implementation.root.clone(),
                    ownership: implementation.ownership,
                    implementation_types: implementation.implementation_types.clone(),
                    runtime_types: implementation.runtime_types.clone(),
                    runtime_manifests: implementation.runtime_manifests.clone(),
                    scanned_files: implementation.source_files.len(),
                    summary,
                    smell_results,
                }
            })
            .collect();
        let matched: Vec<_> = self.findings.iter().collect();
        let matched_rule_ids: BTreeSet<_> = matched
            .iter()
            .map(|finding| finding.rule_id.clone())
            .collect();
        let blocking_findings = matched.iter().filter(|finding| finding.blocking).count();
        let ignored_findings = matched
            .iter()
            .filter(|finding| finding.status == "ignored_match")
            .count();
        let review_signals = matched
            .iter()
            .filter(|finding| finding.status == "indicator")
            .count();
        let matched_smell_ids: Vec<_> = self
            .smell_results
            .iter()
            .filter(|result| {
                matches!(
                    result.state.as_str(),
                    "blocking_match" | "ignored_match" | "review_match"
                )
            })
            .map(|result| result.smell_id.clone())
            .collect();
        self.summary = ReportSummary {
            verdict: if !self.errors.is_empty() {
                "incomplete_due_to_errors"
            } else if blocking_findings > 0 {
                "blocked_by_required_patterns"
            } else if review_signals > 0 {
                "required_checks_passed_with_review_signals"
            } else if ignored_findings > 0 {
                "passed_with_ignored_findings"
            } else {
                "required_checks_passed"
            }
            .into(),
            implementations: self.implementation_results.len(),
            unowned_files: self
                .implementation_scopes
                .iter()
                .find(|implementation| implementation.ownership == "unowned_source")
                .map_or(0, |implementation| implementation.source_files.len()),
            total_smell_patterns: self.smell_results.len(),
            matched_smell_patterns: matched_smell_ids.len(),
            blocking_smell_patterns: self
                .smell_results
                .iter()
                .filter(|result| result.state == SmellState::BlockingMatch)
                .count(),
            review_smell_patterns: self
                .smell_results
                .iter()
                .filter(|result| result.state == SmellState::ReviewMatch)
                .count(),
            error_smell_patterns: self
                .smell_results
                .iter()
                .filter(|result| result.state == SmellState::Error)
                .count(),
            matched_smell_ids,
            total_measurements: self.measurements.len(),
            total_findings: self.findings.len(),
            matched_findings: matched.len(),
            blocking_findings,
            ignored_findings,
            review_signals,
            within_pattern_limits: self
                .measurements
                .iter()
                .filter(|measurement| !measurement.matched)
                .count(),
            matched_rule_ids: matched_rule_ids.into_iter().collect(),
            error_count: self.errors.len(),
        };
    }
}

fn implementation_summary(smell_results: &[SmellResult]) -> ImplementationSummary {
    let matched_smell_ids: Vec<_> = smell_results
        .iter()
        .filter(|result| {
            matches!(
                result.state.as_str(),
                "blocking_match" | "ignored_match" | "review_match"
            )
        })
        .map(|result| result.smell_id.clone())
        .collect();
    let blocking_smell_patterns = smell_results
        .iter()
        .filter(|result| result.state == SmellState::BlockingMatch)
        .count();
    let review_smell_patterns = smell_results
        .iter()
        .filter(|result| result.state == SmellState::ReviewMatch)
        .count();
    let ignored_smell_patterns = smell_results
        .iter()
        .filter(|result| result.state == SmellState::IgnoredMatch)
        .count();
    let error_smell_patterns = smell_results
        .iter()
        .filter(|result| result.state == SmellState::Error)
        .count();
    ImplementationSummary {
        verdict: if error_smell_patterns > 0 {
            "incomplete_due_to_errors"
        } else if blocking_smell_patterns > 0 {
            "blocked_by_required_patterns"
        } else if review_smell_patterns > 0 {
            "required_checks_passed_with_review_signals"
        } else if ignored_smell_patterns > 0 {
            "passed_with_ignored_findings"
        } else {
            "required_checks_passed"
        }
        .into(),
        total_smell_patterns: smell_results.len(),
        matched_smell_patterns: matched_smell_ids.len(),
        blocking_smell_patterns,
        ignored_smell_patterns,
        review_smell_patterns,
        error_smell_patterns,
        matched_smell_ids,
    }
}

fn finding_in_source_files(finding: &Finding, source_files: &[String]) -> bool {
    source_files.binary_search(&finding.location.path).is_ok()
        || finding
            .related_locations
            .iter()
            .any(|location| source_files.binary_search(&location.path).is_ok())
}

fn measurement_in_source_files(measurement: &Measurement, source_files: &[String]) -> bool {
    source_files
        .binary_search(&measurement.location.path)
        .is_ok()
        || measurement
            .related_locations
            .iter()
            .any(|location| source_files.binary_search(&location.path).is_ok())
}

fn path_in_source_files(path: &str, source_files: Option<&[String]>) -> bool {
    source_files.is_none_or(|files| {
        files
            .binary_search_by(|file| file.as_str().cmp(path))
            .is_ok()
    })
}

fn build_smell_results(report: &Report, source_files: Option<&[String]>) -> Vec<SmellResult> {
    report
        .coverage
        .iter()
        .map(|smell| {
            let smell_id = smell["smell_id"].as_str().unwrap();
            let applicability = smell["applicability"].as_str().unwrap();
            let rules = smell["rules"].as_array().unwrap();
            let measured_rule_ids: Vec<_> = rules
                .iter()
                .filter(|rule| rule["measurement_status"] == "measured_defined_scope")
                .map(|rule| rule["rule_id"].as_str().unwrap().to_string())
                .collect();
            let pending_rule_ids: Vec<_> = rules
                .iter()
                .filter(|rule| rule["measurement_status"] == "not_implemented")
                .map(|rule| rule["rule_id"].as_str().unwrap().to_string())
                .collect();
            let disabled_rule_ids: Vec<_> = rules
                .iter()
                .filter(|rule| rule["measurement_status"] == "disabled")
                .map(|rule| rule["rule_id"].as_str().unwrap().to_string())
                .collect();
            let excluded_rule_ids: Vec<_> = rules
                .iter()
                .filter(|rule| rule["measurement_status"] == "excluded_by_group")
                .map(|rule| rule["rule_id"].as_str().unwrap().to_string())
                .collect();
            let incomplete_rule_ids: Vec<_> = rules
                .iter()
                .filter(|rule| {
                    matches!(
                        rule["measurement_status"].as_str(),
                        Some("incomplete_scan" | "error_required_detector_missing")
                    )
                })
                .map(|rule| rule["rule_id"].as_str().unwrap().to_string())
                .collect();
            let indexed_measurements: Vec<_> = report
                .measurements
                .iter()
                .enumerate()
                .filter(|(_, measurement)| {
                    report.rule_metadata[&measurement.rule_id].guidance.smell_id == smell_id
                        && source_files
                            .is_none_or(|files| measurement_in_source_files(measurement, files))
                })
                .collect();
            let matched_findings: Vec<_> = report
                .findings
                .iter()
                .enumerate()
                .filter(|(_, finding)| {
                    finding.smell_id == smell_id
                        && source_files.is_none_or(|files| finding_in_source_files(finding, files))
                })
                .collect();
            let blocking_findings = matched_findings
                .iter()
                .filter(|(_, finding)| finding.blocking)
                .count();
            let ignored_findings = matched_findings
                .iter()
                .filter(|(_, finding)| finding.status == "ignored_match")
                .count();
            let review_signals = matched_findings
                .iter()
                .filter(|(_, finding)| finding.status == "indicator")
                .count();
            let status = smell_state(SmellStateInputs {
                applicable: applicability == "applicable",
                incomplete: !incomplete_rule_ids.is_empty(),
                blocking_findings,
                ignored_findings,
                review_signals,
                measured: !measured_rule_ids.is_empty(),
                pending: !pending_rule_ids.is_empty(),
                excluded: !excluded_rule_ids.is_empty(),
            });
            let affected_files: BTreeSet<_> = matched_findings
                .iter()
                .flat_map(|(_, finding)| {
                    std::iter::once(finding.location.path.as_str())
                        .chain(
                            finding
                                .related_locations
                                .iter()
                                .map(|location| location.path.as_str()),
                        )
                        .filter(|path| path_in_source_files(path, source_files))
                })
                .collect();
            let mut affected_symbols = BTreeSet::new();
            for (_, finding) in &matched_findings {
                if path_in_source_files(&finding.location.path, source_files) {
                    affected_symbols.insert(finding.symbol.as_str());
                }
                for (symbol, location) in finding
                    .related_symbols
                    .iter()
                    .zip(&finding.related_locations)
                {
                    if path_in_source_files(&location.path, source_files) {
                        affected_symbols.insert(symbol.as_str());
                    }
                }
            }
            let matched_rule_ids: BTreeSet<_> = matched_findings
                .iter()
                .map(|(_, finding)| finding.rule_id.clone())
                .collect();
            SmellResult {
                smell_id: smell_id.to_string(),
                smell: smell["smell"].as_str().unwrap().to_string(),
                category: smell["category"].as_str().unwrap().to_string(),
                reference_url: smell["reference_url"].as_str().unwrap().to_string(),
                reference_check: ReferenceCheck {
                    required: true,
                    non_negotiable: true,
                    action: REFERENCE_RESEARCH_ACTION,
                    required_before: REFERENCE_RESEARCH_REQUIRED_BEFORE,
                    unavailable_action: REFERENCE_RESEARCH_UNAVAILABLE_ACTION,
                },
                applicability: applicability.to_string(),
                state: status.state,
                interpretation: status.state.interpretation(),
                coverage_status: status.coverage,
                evaluated_measurements: indexed_measurements.len(),
                matched_findings: matched_findings.len(),
                blocking_findings,
                ignored_findings,
                review_signals,
                within_pattern_limits: indexed_measurements.len() - matched_findings.len(),
                affected_files: affected_files.len(),
                affected_symbols: affected_symbols.len(),
                measured_rule_ids,
                matched_rule_ids: matched_rule_ids.into_iter().collect(),
                rule_status: RuleStatusIds {
                    pending_rule_ids,
                    disabled_rule_ids,
                    excluded_rule_ids,
                    incomplete_rule_ids,
                },
                matched_finding_indices: matched_findings.iter().map(|(index, _)| *index).collect(),
            }
        })
        .collect()
}

impl Report {
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
}

pub fn write_finding_log(report: &Report, output: impl std::io::Write) -> std::io::Result<()> {
    table::write_finding_log(report, output)
}

#[cfg(test)]
mod tests {
    use super::{
        Location, Report, SmellState,
        model::{CoverageStatus, SmellStateInputs},
        smell_state, write_finding_log,
    };
    use crate::input::History;
    use serde_json::json;

    #[test]
    fn typed_result_states_match_the_report_schema_spellings() {
        let states = [
            SmellState::BlockingMatch,
            SmellState::ReviewMatch,
            SmellState::CheckedNoMatchInMeasuredScope,
            SmellState::Pending,
            SmellState::Excluded,
            SmellState::Disabled,
            SmellState::NotApplicable,
            SmellState::Error,
        ];
        for state in states {
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{}\"", state.as_str())
            );
            assert!(!state.interpretation().is_empty());
        }

        let coverage = [
            CoverageStatus::MeasuredWithPendingRules,
            CoverageStatus::MeasuredDefinedScope,
            CoverageStatus::Pending,
            CoverageStatus::Excluded,
            CoverageStatus::Disabled,
            CoverageStatus::NotApplicable,
            CoverageStatus::Incomplete,
        ];
        for status in coverage {
            assert_eq!(
                serde_json::to_string(&status).unwrap(),
                format!("\"{}\"", status.as_str())
            );
        }
    }

    #[test]
    fn smell_state_precedence_preserves_fail_closed_reporting() {
        let cases = [
            (
                (false, true, 1, 1, 1, true, true, true),
                SmellState::NotApplicable,
                CoverageStatus::NotApplicable,
            ),
            (
                (true, true, 1, 1, 1, true, true, true),
                SmellState::Error,
                CoverageStatus::Incomplete,
            ),
            (
                (true, false, 1, 1, 1, true, true, false),
                SmellState::BlockingMatch,
                CoverageStatus::MeasuredWithPendingRules,
            ),
            (
                (true, false, 0, 0, 1, true, false, false),
                SmellState::ReviewMatch,
                CoverageStatus::MeasuredDefinedScope,
            ),
            (
                (true, false, 0, 1, 0, true, false, false),
                SmellState::IgnoredMatch,
                CoverageStatus::MeasuredDefinedScope,
            ),
            (
                (true, false, 0, 0, 0, true, false, false),
                SmellState::CheckedNoMatchInMeasuredScope,
                CoverageStatus::MeasuredDefinedScope,
            ),
            (
                (true, false, 0, 0, 0, false, true, true),
                SmellState::Pending,
                CoverageStatus::Pending,
            ),
            (
                (true, false, 0, 0, 0, false, false, true),
                SmellState::Excluded,
                CoverageStatus::Excluded,
            ),
            (
                (true, false, 0, 0, 0, false, false, false),
                SmellState::Disabled,
                CoverageStatus::Disabled,
            ),
        ];

        for (inputs, expected_state, expected_coverage) in cases {
            let (applicable, incomplete, blocking, ignored, review, measured, pending, excluded) =
                inputs;
            let actual = smell_state(SmellStateInputs {
                applicable,
                incomplete,
                blocking_findings: blocking,
                ignored_findings: ignored,
                review_signals: review,
                measured,
                pending,
                excluded,
            });
            assert_eq!(actual.state, expected_state);
            assert_eq!(actual.coverage.as_str(), expected_coverage.as_str());
        }
    }

    #[test]
    fn global_error_scope_does_not_depend_on_display_text() {
        let registry = crate::policy::registry("rust-v1").unwrap();
        let policy =
            crate::policy::parse(include_str!("../examples/quality-policy.json"), &registry)
                .unwrap();
        let mut report = Report::new(
            &registry,
            &policy,
            "working_tree",
            &[],
            &History {
                available: true,
                commits: Vec::new(),
            },
        );
        report.error("rule this is still a global failure");
        report.finish();

        for smell in &report.coverage {
            for rule in smell["rules"].as_array().unwrap() {
                if rule["selected"] == true {
                    assert_eq!(rule["measurement_status"], "incomplete_scan");
                }
            }
        }
    }

    #[test]
    fn finding_log_keeps_error_metadata_paired_while_json_errors_stay_strings() {
        let registry = crate::policy::registry("rust-v1").unwrap();
        let policy =
            crate::policy::parse(include_str!("../examples/quality-policy.json"), &registry)
                .unwrap();
        let mut report = Report::new(
            &registry,
            &policy,
            "working_tree",
            &[],
            &History {
                available: true,
                commits: Vec::new(),
            },
        );
        report.rule_error("rust.function_arguments", "z failure".into());
        report.error_at(
            "a failure",
            Location {
                path: "src/lib.rs".into(),
                line: 4,
                column: 2,
            },
        );
        report.finish();

        let serialized = serde_json::to_value(&report).unwrap();
        assert_eq!(
            serialized["errors"],
            json!(["a failure", "rule rust.function_arguments: z failure"])
        );

        let mut log = Vec::new();
        write_finding_log(&report, &mut log).unwrap();
        let log = String::from_utf8(log).unwrap();
        assert!(log.contains("E000001 | scanner_error | scanner | src/lib.rs:4:2 | detail line "));
        assert!(
            log.contains("E000002 | scanner_error | rust.function_arguments | - | detail line ")
        );
    }
}
