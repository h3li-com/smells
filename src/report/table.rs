use super::{Finding, ImplementationResult, Location, Report, SourceExcerpt};
use crate::input::Implementation;

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
    println!("Smell pattern | Pattern ID | Result | Matches | Blocking | Review | Coverage");
    for result in &implementation.smell_results {
        println!(
            "{} | {} | {} | {} | {} | {} | {}",
            result.smell,
            result.smell_id,
            result.state,
            result.matched_findings,
            result.blocking_findings,
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
            finding.evaluation.matched
                && Report::finding_in_source_files(finding, &scope.source_files)
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
        print_actionable_finding(*index, finding);
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
    if Report::path_in_source_files(&location.path, Some(&scope.source_files)) {
        symbols.push(symbol);
        locations.push(format!(
            "{}:{}:{}",
            location.path, location.line, location.column
        ));
    }
}

fn print_actionable_finding(index: usize, finding: &Finding) {
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
            "Scan summary | verdict: {} | implementations: {} | matched patterns: {}/{} | blocking patterns: {} | review patterns: {} | matched findings: {} | blocking findings: {} | review signals: {} | errors: {}",
            self.summary.verdict,
            self.summary.implementations,
            self.summary.matched_smell_patterns,
            self.summary.total_smell_patterns,
            self.summary.blocking_smell_patterns,
            self.summary.review_smell_patterns,
            self.summary.matched_findings,
            self.summary.blocking_findings,
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
