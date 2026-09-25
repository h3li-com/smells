use super::{
    Finding, ImplementationResult, Location, Report, SourceExcerpt, finding_in_source_files,
    path_in_source_files,
};
use crate::input::Implementation;
use std::io::{self, Write};

struct FindingLogEntry {
    detail: Vec<String>,
}

fn print_implementation_header(implementation: &ImplementationResult, language: &str) {
    let implementation_types = implementation.implementation_types.join(",");
    let runtime_types = implementation.runtime_types.join(",");
    let runtime_manifests = implementation.runtime_manifests.join(",");
    println!(
        "Implementation: {} | root: {} | ownership: {} | types: {} | runtimes: {} | manifests: {} | {} {} files",
        implementation.implementation_id,
        implementation
            .implementation_root
            .as_deref()
            .unwrap_or("unowned"),
        implementation.ownership,
        if implementation_types.is_empty() {
            "-"
        } else {
            &implementation_types
        },
        if runtime_types.is_empty() {
            "-"
        } else {
            &runtime_types
        },
        if runtime_manifests.is_empty() {
            "-"
        } else {
            &runtime_manifests
        },
        implementation.scanned_files,
        language,
    );
}

fn print_smell_results(implementation: &ImplementationResult) {
    println!(
        "Smell pattern | Pattern ID | Result | Matches | Blocking | Ignored | Review | Coverage"
    );
    for result in &implementation.smell_results {
        println!(
            "{} | {} | {} | {} | {} | {} | {} | {}",
            result.smell,
            result.smell_id,
            result.state,
            result.matched_findings,
            result.blocking_findings,
            result.ignored_findings,
            result.review_signals,
            result.coverage_status,
        );
    }
}

fn print_matched_findings(
    report: &Report,
    implementation: &ImplementationResult,
    scope: &Implementation,
) {
    println!("Matched evidence for {}:", implementation.implementation_id);
    println!(
        "Finding | Smell | Repository symbols | Metric | Value | Matches when | Threshold | Status | Repository evidence locations"
    );
    let matched_findings: Vec<_> = report
        .findings
        .iter()
        .enumerate()
        .filter(|(_, finding)| {
            finding.evaluation.matched && finding_in_source_files(finding, &scope.source_files)
        })
        .collect();
    for (index, finding) in &matched_findings {
        print_finding(*index, finding, scope);
    }
    if !matched_findings.is_empty() {
        println!(
            "Actionable findings for {}:",
            implementation.implementation_id
        );
    }
    for (index, finding) in &matched_findings {
        print_actionable_finding(report, *index, finding);
    }
}

fn print_finding(index: usize, finding: &Finding, scope: &Implementation) {
    let mut symbols = vec![];
    let mut locations = vec![];
    push_table_evidence(
        &finding.symbol,
        &finding.location,
        scope,
        &mut symbols,
        &mut locations,
    );
    for (symbol, location) in finding
        .related_symbols
        .iter()
        .zip(&finding.related_locations)
    {
        push_table_evidence(symbol, location, scope, &mut symbols, &mut locations);
    }
    println!(
        "{} | {} | {} | {} | {} | {} | {} | {} | {}",
        index,
        finding.smell,
        symbols.join(","),
        finding.evaluation.metric,
        finding.evaluation.observed,
        finding.evaluation.match_condition,
        finding.evaluation.threshold,
        finding.status,
        locations.join(",")
    );
}

fn push_table_evidence<'a>(
    symbol: &'a str,
    location: &Location,
    scope: &Implementation,
    symbols: &mut Vec<&'a str>,
    locations: &mut Vec<String>,
) {
    if path_in_source_files(&location.path, Some(&scope.source_files)) {
        symbols.push(symbol);
        locations.push(format!(
            "{}:{}:{}",
            location.path, location.line, location.column
        ));
    }
}

fn print_actionable_finding(report: &Report, index: usize, finding: &Finding) {
    println!(
        "Actionable finding {index} | {} | {} | pattern_type={} | certainty={} | status={}",
        finding.smell, finding.rule_id, finding.pattern_type, finding.certainty, finding.status
    );
    println!("  Issue: {}", finding.diagnostic.headline);
    println!(
        "  Primary location: {}:{}:{} | symbol: {}",
        finding.location.path, finding.location.line, finding.location.column, finding.symbol
    );
    println!(
        "  Observed versus threshold: {} = {}; matches when {} {}",
        finding.evaluation.metric,
        finding.evaluation.observed,
        finding.evaluation.match_condition,
        finding.evaluation.threshold
    );
    println!("  Evidence: {}", finding.diagnostic.explanation);
    println!("  Signal: {}", finding.diagnostic.signal);
    print_source_excerpt("Source excerpt", finding.source_excerpt.as_ref());
    for (related_index, excerpt) in finding.related_source_excerpts.iter().enumerate() {
        print_source_excerpt(
            &format!("Related source excerpt {}", related_index + 1),
            Some(excerpt),
        );
    }
    if finding.omitted_related_excerpts > 0 {
        println!(
            "  Related source excerpts omitted: {} (all locations remain in the evidence row and JSON report)",
            finding.omitted_related_excerpts
        );
    }
    println!("  Why it matters: {}", finding.diagnostic.why_it_matters);
    println!("  Remediation: {}", finding.diagnostic.remediation);
    println!("  Contract: {}", finding.diagnostic.contract);
    println!("  Reference URL: {}", finding.diagnostic.reference_url);
    println!("  Review guidance: {}", finding.diagnostic.review);
    let guidance = report
        .guidance_catalog
        .iter()
        .find(|guidance| guidance.guidance_ref == finding.guidance_ref)
        .expect("finding guidance reference must resolve");
    println!("  When to ignore [{}]:", guidance.guidance_ref);
    println!(
        "    provenance={} | source={} | checked={}",
        guidance.provenance, guidance.source_url, guidance.checked_on
    );
    for (index, item) in guidance.items.iter().enumerate() {
        println!("    {}. {item}", index + 1);
    }
    println!(
        "  Suppression form: smells: ignore[{}] -- <non-empty reason>",
        finding.rule_id
    );
}

fn print_source_excerpt(label: &str, excerpt: Option<&SourceExcerpt>) {
    let Some(excerpt) = excerpt else {
        println!("  {label}: unavailable");
        return;
    };
    println!(
        "  {label}: {}:{}:{}",
        excerpt.path, excerpt.focus_line, excerpt.focus_column
    );
    for line in &excerpt.lines {
        let marker = if line.line == excerpt.focus_line {
            ">"
        } else {
            " "
        };
        println!("    {marker} {:>6} | {}", line.line, line.text);
    }
}

fn error_log_entry(index: usize, error: &str) -> FindingLogEntry {
    let id = format!("E{:06}", index + 1);
    FindingLogEntry {
        detail: vec![
            format!("Error {id}"),
            "Status: scanner_error (unsuppressible)".into(),
            "Rule: scanner".into(),
            "Primary location: -".into(),
            format!("Error: {error}"),
            "Action: inspect this complete error and its source context; a scanner error cannot be suppressed.".into(),
            String::new(),
        ],
    }
}

fn finding_log_entry(report: &Report, finding: &Finding, status: &'static str) -> FindingLogEntry {
    let guidance = report
        .guidance_catalog
        .iter()
        .find(|guidance| guidance.guidance_ref == finding.guidance_ref)
        .expect("finding guidance reference must resolve");
    let mut detail = vec![
        format!("Finding {}", finding.finding_id),
        format!("Status: {status}"),
        format!("Rule: {} v{}", finding.rule_id, finding.rule_version),
        format!("Policy mode: {:?}", finding.policy_mode).to_lowercase(),
        format!(
            "Primary location: {}:{}:{} | symbol: {}",
            finding.location.path, finding.location.line, finding.location.column, finding.symbol
        ),
    ];
    for (index, (symbol, location)) in finding
        .related_symbols
        .iter()
        .zip(&finding.related_locations)
        .enumerate()
    {
        detail.push(format!(
            "Related location {}: {}:{}:{} | symbol: {}",
            index + 1,
            location.path,
            location.line,
            location.column,
            symbol
        ));
    }
    detail.extend([
        format!(
            "Policy evaluation: {} = {}; matches when {} {}",
            finding.evaluation.metric,
            finding.evaluation.observed,
            finding.evaluation.match_condition,
            finding.evaluation.threshold
        ),
        format!("Why it matched: {}", finding.diagnostic.explanation),
        format!("Evidence: {}", finding.evidence),
        format!("Signal: {}", finding.diagnostic.signal),
        format!("Why it matters: {}", finding.diagnostic.why_it_matters),
        format!("Remediation: {}", finding.diagnostic.remediation),
        format!("Contract: {}", finding.diagnostic.contract),
        format!("Reference URL: {}", finding.diagnostic.reference_url),
        format!("Review guidance: {}", finding.diagnostic.review),
        format!("When to ignore [{}]", guidance.guidance_ref),
        format!("Provenance: {}", guidance.provenance),
        format!("Guidance source: {}", guidance.source_url),
        format!("Guidance checked: {}", guidance.checked_on),
    ]);
    for (index, item) in guidance.items.iter().enumerate() {
        detail.push(format!("  {}. {item}", index + 1));
    }
    detail.push(format!(
        "Suppression form: smells: ignore[{}] -- <non-empty reason>",
        finding.rule_id
    ));
    if let Some(suppression) = &finding.suppression {
        detail.push(format!(
            "Accepted suppression: {}:{}:{} | reason: {}",
            suppression.directive_location.path,
            suppression.directive_location.line,
            suppression.directive_location.column,
            suppression.reason
        ));
    }
    detail.push("Agent requirement: inspect the complete finding, reference, source, callers, and tests before proposing remediation or adding a suppression; never suppress merely to pass the hook.".into());
    if let Some(excerpt) = &finding.source_excerpt {
        detail.push(format!(
            "Source excerpt: {}:{}:{}",
            excerpt.path, excerpt.focus_line, excerpt.focus_column
        ));
        for line in &excerpt.lines {
            let marker = if line.line == excerpt.focus_line {
                ">"
            } else {
                " "
            };
            detail.push(format!("  {marker} {:>6} | {}", line.line, line.text));
        }
    } else {
        detail.push("Source excerpt: unavailable".into());
    }
    detail.push(String::new());
    FindingLogEntry { detail }
}

#[derive(Clone, Copy)]
enum FindingLogTarget<'a> {
    Error(usize, &'a str),
    Finding(&'a Finding, &'static str),
}

fn finding_log_targets(report: &Report) -> Vec<FindingLogTarget<'_>> {
    let mut targets: Vec<_> = report
        .errors
        .iter()
        .enumerate()
        .map(|(index, error)| FindingLogTarget::Error(index, error.as_str()))
        .collect();
    for status in ["blocking", "ignored", "review"] {
        targets.extend(
            report
                .findings
                .iter()
                .filter(|finding| match status {
                    "blocking" => finding.blocking,
                    "ignored" => finding.status == "ignored_match",
                    "review" => finding.status == "indicator",
                    _ => false,
                })
                .map(|finding| FindingLogTarget::Finding(finding, status)),
        );
    }
    targets
}

fn log_entry(report: &Report, target: FindingLogTarget<'_>) -> FindingLogEntry {
    match target {
        FindingLogTarget::Error(index, error) => error_log_entry(index, error),
        FindingLogTarget::Finding(finding, status) => finding_log_entry(report, finding, status),
    }
}

fn write_finding_log_header(report: &Report, output: &mut impl Write) -> io::Result<()> {
    writeln!(output, "Smells Finding Log")?;
    writeln!(
        output,
        "Scan summary | verdict: {} | findings: {} | blocking: {} | ignored: {} | review: {} | errors: {}",
        report.summary.verdict,
        report.summary.matched_findings,
        report.summary.blocking_findings,
        report.summary.ignored_findings,
        report.summary.review_signals,
        report.summary.error_count
    )?;
    writeln!(
        output,
        "Read the Issue Index first, then jump to each exact detail line."
    )?;
    writeln!(output)?;
    writeln!(output, "Issue Index")?;
    Ok(())
}

fn write_finding_log_index(
    report: &Report,
    targets: &[FindingLogTarget<'_>],
    output: &mut impl Write,
) -> io::Result<()> {
    let mut detail_line = targets.len() + 7;
    for target in targets {
        let detail_line_count = log_entry(report, *target).detail.len();
        match target {
            FindingLogTarget::Error(index, _) => {
                writeln!(
                    output,
                    "E{:06} | scanner_error | scanner | - | detail line {}",
                    index + 1,
                    detail_line
                )?;
            }
            FindingLogTarget::Finding(finding, status) => {
                writeln!(
                    output,
                    "{} | {} | {} | {}:{}:{} | detail line {}",
                    finding.finding_id,
                    status,
                    finding.rule_id,
                    finding.location.path,
                    finding.location.line,
                    finding.location.column,
                    detail_line
                )?;
            }
        }
        detail_line += detail_line_count;
    }
    writeln!(output)?;
    Ok(())
}

fn write_finding_log_details(
    report: &Report,
    targets: Vec<FindingLogTarget<'_>>,
    output: &mut impl Write,
) -> io::Result<()> {
    for target in targets {
        let entry = log_entry(report, target);
        for line in entry.detail {
            writeln!(output, "{line}")?;
        }
    }
    Ok(())
}

pub(super) fn write_finding_log(report: &Report, mut output: impl Write) -> io::Result<()> {
    let targets = finding_log_targets(report);
    write_finding_log_header(report, &mut output)?;
    write_finding_log_index(report, &targets, &mut output)?;
    write_finding_log_details(report, targets, &mut output)
}

impl Report {
    pub fn print_table(&self) {
        println!(
            "Scope: {} | source: {} | {} {} files",
            self.metadata.scope,
            self.metadata.source_mode,
            self.scanned_files.len(),
            self.metadata.language,
        );
        println!(
            "Scan summary | verdict: {} | implementations: {} | matched patterns: {}/{} | blocking patterns: {} | review patterns: {} | matched findings: {} | blocking findings: {} | ignored findings: {} | review signals: {} | errors: {}",
            self.summary.verdict,
            self.summary.implementations,
            self.summary.matched_smell_patterns,
            self.summary.total_smell_patterns,
            self.summary.blocking_smell_patterns,
            self.summary.review_smell_patterns,
            self.summary.matched_findings,
            self.summary.blocking_findings,
            self.summary.ignored_findings,
            self.summary.review_signals,
            self.summary.error_count,
        );
        for (implementation, scope) in self
            .implementation_results
            .iter()
            .zip(&self.implementation_scopes)
        {
            print_implementation_header(implementation, &self.metadata.language);
            print_smell_results(implementation);
            print_matched_findings(self, implementation, scope);
        }
        for error in &self.errors {
            eprintln!("ERROR: {error}");
        }
    }
}
