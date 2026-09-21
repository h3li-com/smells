use crate::policy::Mode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
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

pub struct FindingRelations {
    pub evidence: Value,
    pub symbols: Vec<String>,
    pub locations: Vec<Location>,
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

pub(super) fn source_excerpt(
    files: &BTreeMap<String, String>,
    location: &Location,
) -> Option<SourceExcerpt> {
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
    pub implementations: usize,
    pub unowned_files: usize,
    pub total_smell_patterns: usize,
    pub matched_smell_patterns: usize,
    pub blocking_smell_patterns: usize,
    pub review_smell_patterns: usize,
    pub error_smell_patterns: usize,
    pub matched_smell_ids: Vec<String>,
    pub total_findings: usize,
    pub matched_findings: usize,
    pub blocking_findings: usize,
    pub review_signals: usize,
    pub within_pattern_limits: usize,
    pub matched_rule_ids: Vec<String>,
    pub error_count: usize,
}

#[derive(Debug, Serialize)]
pub struct SmellResult {
    pub smell_id: String,
    pub smell: String,
    pub category: String,
    pub reference_url: String,
    pub reference_check: ReferenceCheck,
    pub applicability: String,
    pub state: SmellState,
    pub interpretation: &'static str,
    pub coverage_status: CoverageStatus,
    pub evaluated_findings: usize,
    pub matched_findings: usize,
    pub blocking_findings: usize,
    pub review_signals: usize,
    pub within_pattern_limits: usize,
    pub affected_files: usize,
    pub affected_symbols: usize,
    pub measured_rule_ids: Vec<String>,
    pub matched_rule_ids: Vec<String>,
    #[serde(flatten)]
    pub rule_status: RuleStatusIds,
    pub matched_finding_indices: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub struct RuleStatusIds {
    pub pending_rule_ids: Vec<String>,
    pub disabled_rule_ids: Vec<String>,
    pub excluded_rule_ids: Vec<String>,
    pub incomplete_rule_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SmellState {
    BlockingMatch,
    ReviewMatch,
    CheckedNoMatchInMeasuredScope,
    Pending,
    Excluded,
    Disabled,
    NotApplicable,
    Error,
}

impl SmellState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::BlockingMatch => "blocking_match",
            Self::ReviewMatch => "review_match",
            Self::CheckedNoMatchInMeasuredScope => "checked_no_match_in_measured_scope",
            Self::Pending => "pending",
            Self::Excluded => "excluded",
            Self::Disabled => "disabled",
            Self::NotApplicable => "not_applicable",
            Self::Error => "error",
        }
    }

    pub(super) fn interpretation(self) -> &'static str {
        match self {
            Self::NotApplicable => {
                "This canonical smell does not apply to the selected language model."
            }
            Self::Error => "Measurement is incomplete; do not infer that this smell is absent.",
            Self::BlockingMatch => {
                "One or more required deterministic rules matched this smell pattern."
            }
            Self::ReviewMatch => {
                "One or more report-only deterministic rules matched; semantic review is required before deciding whether to refactor."
            }
            Self::CheckedNoMatchInMeasuredScope => {
                "No enabled implemented rule matched in its defined source scope; this is not proof that the semantic smell is absent."
            }
            Self::Pending => {
                "No detector for this smell ran because its registered rules are not implemented yet."
            }
            Self::Excluded => {
                "All registered detectors for this smell were excluded by the resolved policy groups."
            }
            Self::Disabled => "All registered detectors for this smell are disabled by policy.",
        }
    }
}

impl std::fmt::Display for SmellState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    MeasuredWithPendingRules,
    MeasuredDefinedScope,
    Pending,
    Excluded,
    Disabled,
    NotApplicable,
    Incomplete,
}

impl CoverageStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::MeasuredWithPendingRules => "measured_with_pending_rules",
            Self::MeasuredDefinedScope => "measured_defined_scope",
            Self::Pending => "pending",
            Self::Excluded => "excluded",
            Self::Disabled => "disabled",
            Self::NotApplicable => "not_applicable",
            Self::Incomplete => "incomplete",
        }
    }
}

impl std::fmt::Display for CoverageStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub(super) struct SmellStatus {
    pub(super) state: SmellState,
    pub(super) coverage: CoverageStatus,
}

fn measured_coverage(measured: bool, pending: bool, excluded: bool) -> CoverageStatus {
    if measured && pending {
        CoverageStatus::MeasuredWithPendingRules
    } else if measured {
        CoverageStatus::MeasuredDefinedScope
    } else if pending {
        CoverageStatus::Pending
    } else if excluded {
        CoverageStatus::Excluded
    } else {
        CoverageStatus::Disabled
    }
}

fn measured_state(
    blocking_findings: usize,
    review_signals: usize,
    measured: bool,
    pending: bool,
    excluded: bool,
) -> SmellState {
    if blocking_findings > 0 {
        SmellState::BlockingMatch
    } else if review_signals > 0 {
        SmellState::ReviewMatch
    } else if measured {
        SmellState::CheckedNoMatchInMeasuredScope
    } else if pending {
        SmellState::Pending
    } else if excluded {
        SmellState::Excluded
    } else {
        SmellState::Disabled
    }
}

pub(super) fn smell_state(
    applicable: bool,
    incomplete: bool,
    blocking_findings: usize,
    review_signals: usize,
    measured: bool,
    pending: bool,
    excluded: bool,
) -> SmellStatus {
    if !applicable {
        return SmellStatus {
            state: SmellState::NotApplicable,
            coverage: CoverageStatus::NotApplicable,
        };
    }
    if incomplete {
        return SmellStatus {
            state: SmellState::Error,
            coverage: CoverageStatus::Incomplete,
        };
    }
    SmellStatus {
        state: measured_state(
            blocking_findings,
            review_signals,
            measured,
            pending,
            excluded,
        ),
        coverage: measured_coverage(measured, pending, excluded),
    }
}

#[derive(Debug, Serialize)]
pub struct ImplementationSummary {
    pub verdict: String,
    pub total_smell_patterns: usize,
    pub matched_smell_patterns: usize,
    pub blocking_smell_patterns: usize,
    pub review_smell_patterns: usize,
    pub error_smell_patterns: usize,
    pub matched_smell_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ImplementationResult {
    pub implementation_id: String,
    pub implementation_root: Option<String>,
    pub ownership: &'static str,
    pub implementation_types: Vec<String>,
    pub runtime_types: Vec<String>,
    pub runtime_manifests: Vec<String>,
    pub scanned_files: usize,
    pub summary: ImplementationSummary,
    pub smell_results: Vec<SmellResult>,
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

#[derive(Clone, Debug, Serialize)]
pub struct ReferenceCheck {
    pub required: bool,
    pub non_negotiable: bool,
    pub action: &'static str,
    pub required_before: &'static str,
    pub unavailable_action: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryScope {
    pub available: bool,
    pub commit_count: usize,
    pub maximum_commits: usize,
}
