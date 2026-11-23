//! `smells doctor`: a deterministic integration preflight.
//!
//! Each check is a pure function of captured input so the verdict never
//! depends on network access or the consuming project's build.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use serde_json::{Value, json};

use crate::{
    core::{analyze_captured, model::History},
    input,
    policy::SelectionOptions,
    report::{Finding, Report},
    suppressions::{self, ParsedSuppressions},
};

/// GitHub resolves the first CODEOWNERS file in this order.
const CODEOWNERS_PATHS: [&str; 3] = [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"];
const HISTORY_WINDOW: usize = 200;
const HOTSPOT_LIMIT: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Status {
    Ok,
    Warn,
    Error,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Check {
    pub id: &'static str,
    pub status: Status,
    pub detail: String,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DoctorReport {
    pub checks: Vec<Check>,
    pub diagnostics: Option<Diagnostics>,
    pub burndown: Option<Burndown>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Burndown {
    pub base_ref: String,
    pub base_commit: String,
    pub base_blocking: usize,
    pub current_blocking: usize,
    pub blocking_delta: isize,
    pub base_review: usize,
    pub current_review: usize,
    pub review_delta: isize,
    pub base_ignored: usize,
    pub current_ignored: usize,
    pub ignored_delta: isize,
    pub smell_deltas: Vec<SmellDelta>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SmellDelta {
    pub smell: String,
    pub base_blocking: usize,
    pub current_blocking: usize,
    pub blocking_delta: isize,
    pub base_review: usize,
    pub current_review: usize,
    pub review_delta: isize,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Diagnostics {
    pub scanned_files: usize,
    pub blocking_findings: usize,
    pub ignored_findings: usize,
    pub review_signals: usize,
    pub matched_smells: usize,
    pub total_smells: usize,
    pub smells: Vec<SmellDiagnostic>,
    pub hotspots: Vec<HotspotFile>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SmellDiagnostic {
    pub smell: String,
    pub category: String,
    pub blocking: usize,
    pub ignored: usize,
    pub review: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct HotspotFile {
    pub path: String,
    pub blocking: usize,
    pub review: usize,
}

fn check(id: &'static str, status: Status, detail: impl Into<String>) -> Check {
    Check {
        id,
        status,
        detail: detail.into(),
    }
}

/// Runs every preflight check and, when inputs are valid, builds compact codebase smell
/// diagnostics plus an optional `--base` burndown comparison.
pub(crate) fn diagnose(root: &Path, policy_path: &Path, base_ref: Option<&str>) -> DoctorReport {
    let mut checks = Vec::new();
    let mut diagnostics = None;
    let mut burndown = None;
    match input::working_tree(root, policy_path, &SelectionOptions::default()) {
        Ok(captured) => {
            let policy = &captured.input.policy;
            checks.push(check(
                "policy",
                Status::Ok,
                format!(
                    "{} pinned to scanner {}, {}/{} rules active",
                    captured.registry.rule_pack,
                    policy.scanner_version,
                    policy.resolved.active_rule_ids.len(),
                    captured.registry.rules.len()
                ),
            ));
            checks.push(history_check(&captured.input.history));
            checks.push(suppression_check(&suppressions::parse(
                &captured.input.files,
                &captured.registry,
            )));
            if exit_code(&checks) == 0 {
                diagnostics = diagnose_scan(&captured, &mut checks);
                if let (Some(base), Some(current)) = (base_ref, diagnostics.as_ref()) {
                    burndown = diagnose_burndown(root, policy_path, base, current, &mut checks);
                }
            }
        }
        Err(error) => checks.push(check("policy", Status::Error, error)),
    }
    let read = |name: &str| fs::read_to_string(root.join(name)).ok();
    checks.push(agents_check(read("AGENTS.md").as_deref()));
    checks.push(claude_check(read("CLAUDE.md").as_deref()));
    let codeowners = CODEOWNERS_PATHS
        .iter()
        .find_map(|path| read(path).map(|text| (*path, text)));
    checks.push(codeowners_check(
        codeowners
            .as_ref()
            .map(|(path, text)| (*path, text.as_str())),
        &relative_policy_path(root, policy_path),
    ));
    DoctorReport {
        checks,
        diagnostics,
        burndown,
    }
}

fn diagnose_burndown(
    root: &Path,
    policy_path: &Path,
    base_ref: &str,
    current: &Diagnostics,
    checks: &mut Vec<Check>,
) -> Option<Burndown> {
    let reference = match input::git_ref_for_working_tree(
        root,
        base_ref,
        policy_path,
        &SelectionOptions::default(),
    ) {
        Ok(reference) => reference,
        Err(error) => {
            checks.push(check("base_ref", Status::Error, error));
            return None;
        }
    };
    let base_diagnostics = match reference.captured.as_ref() {
        Some(captured) => match analyze_captured(captured) {
            Ok(report) => {
                if let Some(first) = report.errors.first() {
                    checks.push(check(
                        "base_ref",
                        Status::Error,
                        format!(
                            "base {base_ref} has {} scan error(s); first: {}",
                            report.errors.len(),
                            first.message
                        ),
                    ));
                    return None;
                }
                Some(diagnostics_from_report(&report))
            }
            Err(error) => {
                checks.push(check("base_ref", Status::Error, error));
                return None;
            }
        },
        None => None,
    };
    Some(burndown_between(
        &reference.reference_ref,
        &reference.reference_commit,
        base_diagnostics.as_ref(),
        current,
    ))
}

pub(crate) fn burndown_between(
    base_ref: &str,
    base_commit: &str,
    base: Option<&Diagnostics>,
    current: &Diagnostics,
) -> Burndown {
    let base_blocking = base.map_or(0, |diag| diag.blocking_findings);
    let base_review = base.map_or(0, |diag| diag.review_signals);
    let base_ignored = base.map_or(0, |diag| diag.ignored_findings);
    let smell_deltas = compute_smell_deltas(
        base.map_or(&[][..], |diag| diag.smells.as_slice()),
        &current.smells,
    );
    Burndown {
        base_ref: base_ref.into(),
        base_commit: base_commit.into(),
        base_blocking,
        current_blocking: current.blocking_findings,
        blocking_delta: current.blocking_findings as isize - base_blocking as isize,
        base_review,
        current_review: current.review_signals,
        review_delta: current.review_signals as isize - base_review as isize,
        base_ignored,
        current_ignored: current.ignored_findings,
        ignored_delta: current.ignored_findings as isize - base_ignored as isize,
        smell_deltas,
    }
}

fn compute_smell_deltas(base: &[SmellDiagnostic], current: &[SmellDiagnostic]) -> Vec<SmellDelta> {
    let base_map: BTreeMap<&str, (usize, usize)> = base
        .iter()
        .map(|row| (row.smell.as_str(), (row.blocking, row.review)))
        .collect();
    let current_map: BTreeMap<&str, (usize, usize)> = current
        .iter()
        .map(|row| (row.smell.as_str(), (row.blocking, row.review)))
        .collect();
    let names: BTreeSet<&str> = base_map.keys().chain(current_map.keys()).copied().collect();
    let mut deltas: Vec<SmellDelta> = names
        .into_iter()
        .filter_map(|smell| {
            let (base_blocking, base_review) = base_map.get(smell).copied().unwrap_or((0, 0));
            let (current_blocking, current_review) =
                current_map.get(smell).copied().unwrap_or((0, 0));
            let blocking_delta = current_blocking as isize - base_blocking as isize;
            let review_delta = current_review as isize - base_review as isize;
            (blocking_delta != 0 || review_delta != 0).then(|| SmellDelta {
                smell: smell.into(),
                base_blocking,
                current_blocking,
                blocking_delta,
                base_review,
                current_review,
                review_delta,
            })
        })
        .collect();
    deltas.sort_by(|left, right| {
        left.blocking_delta
            .cmp(&right.blocking_delta)
            .then_with(|| left.review_delta.cmp(&right.review_delta))
            .then_with(|| left.smell.cmp(&right.smell))
    });
    deltas
}

fn diagnose_scan(
    captured: &crate::core::model::CapturedInput,
    checks: &mut Vec<Check>,
) -> Option<Diagnostics> {
    match analyze_captured(captured) {
        Ok(report) => {
            if let Some(first) = report.errors.first() {
                checks.push(check(
                    "scan",
                    Status::Error,
                    format!(
                        "{} scan error(s); first: {}",
                        report.errors.len(),
                        first.message
                    ),
                ));
            }
            Some(diagnostics_from_report(&report))
        }
        Err(error) => {
            checks.push(check("scan", Status::Error, error));
            None
        }
    }
}

pub(crate) fn diagnostics_from_report(report: &Report) -> Diagnostics {
    let signals: Vec<_> = report
        .evidence
        .findings
        .iter()
        .map(FindingSignal::from_finding)
        .collect();
    let smells = summarize_smells(&signals);
    let hotspots = summarize_hotspots(&signals);
    Diagnostics {
        scanned_files: report.scanned_files.len(),
        blocking_findings: report.summary.blocking_findings,
        ignored_findings: report.summary.ignored_findings,
        review_signals: report.summary.review_signals,
        matched_smells: report.summary.matched_smell_patterns,
        total_smells: report.summary.total_smell_patterns,
        smells,
        hotspots,
    }
}

struct FindingSignal<'a> {
    smell: &'a str,
    category: &'a str,
    path: &'a str,
    blocking: bool,
    ignored: bool,
}

impl<'a> FindingSignal<'a> {
    fn from_finding(finding: &'a Finding) -> Self {
        Self {
            smell: &finding.smell,
            category: &finding.category,
            path: &finding.location.path,
            blocking: finding.blocking,
            ignored: finding.status == "ignored_match",
        }
    }
}

fn summarize_smells(signals: &[FindingSignal<'_>]) -> Vec<SmellDiagnostic> {
    let mut grouped: BTreeMap<(&str, &str), Vec<&FindingSignal<'_>>> = BTreeMap::new();
    for signal in signals {
        grouped
            .entry((signal.smell, signal.category))
            .or_default()
            .push(signal);
    }
    let mut rows: Vec<SmellDiagnostic> = grouped
        .into_iter()
        .map(|((smell, category), items)| SmellDiagnostic {
            smell: smell.into(),
            category: category.into(),
            blocking: items.iter().filter(|item| item.blocking).count(),
            ignored: items.iter().filter(|item| item.ignored).count(),
            review: items
                .iter()
                .filter(|item| !item.blocking && !item.ignored)
                .count(),
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .blocking
            .cmp(&left.blocking)
            .then_with(|| right.review.cmp(&left.review))
            .then_with(|| right.ignored.cmp(&left.ignored))
            .then_with(|| left.smell.cmp(&right.smell))
    });
    rows
}

fn summarize_hotspots(signals: &[FindingSignal<'_>]) -> Vec<HotspotFile> {
    let mut grouped: BTreeMap<&str, Vec<&FindingSignal<'_>>> = BTreeMap::new();
    for signal in signals.iter().filter(|signal| !signal.ignored) {
        grouped.entry(signal.path).or_default().push(signal);
    }
    let mut rows: Vec<HotspotFile> = grouped
        .into_iter()
        .map(|(path, items)| HotspotFile {
            path: path.into(),
            blocking: items.iter().filter(|item| item.blocking).count(),
            review: items.iter().filter(|item| !item.blocking).count(),
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .blocking
            .cmp(&left.blocking)
            .then_with(|| right.review.cmp(&left.review))
            .then_with(|| left.path.cmp(&right.path))
    });
    rows.truncate(HOTSPOT_LIMIT);
    rows
}

fn relative_policy_path(root: &Path, policy_path: &Path) -> String {
    let absolute = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let policy = absolute(policy_path);
    policy
        .strip_prefix(absolute(root))
        .unwrap_or(&policy)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(crate) fn history_check(history: &History) -> Check {
    if history.shallow {
        check(
            "git_history",
            Status::Error,
            "shallow clone truncates the 200-commit history window; fetch full history (fetch-depth: 0)",
        )
    } else if !history.available {
        check(
            "git_history",
            Status::Error,
            "Git history is unavailable, so history rules and --base cannot complete",
        )
    } else {
        check(
            "git_history",
            Status::Ok,
            format!(
                "complete; {} commit(s) in the {HISTORY_WINDOW}-commit window change scanned sources",
                history.commits.len()
            ),
        )
    }
}

pub(crate) fn suppression_check(parsed: &ParsedSuppressions) -> Check {
    if let Some(error) = parsed.errors.first() {
        return check(
            "suppressions",
            Status::Error,
            format!(
                "{} invalid directive(s); first at {}:{}: {}",
                parsed.errors.len(),
                error.location.path,
                error.location.line,
                error.message
            ),
        );
    }
    let mut per_rule = BTreeMap::<&str, usize>::new();
    for directive in &parsed.directives {
        *per_rule
            .entry(directive.suppression.rule_id.as_str())
            .or_default() += 1;
    }
    let breakdown = per_rule
        .iter()
        .map(|(rule, count)| format!("{rule}={count}"))
        .collect::<Vec<_>>()
        .join(", ");
    let detail = if breakdown.is_empty() {
        "0 directives".to_string()
    } else {
        format!("{} directive(s): {breakdown}", parsed.directives.len())
    };
    check(
        "suppressions",
        Status::Ok,
        format!("{detail}; `smells check` verifies unused directives"),
    )
}

pub(crate) fn agents_check(agents: Option<&str>) -> Check {
    match agents {
        Some(text) if !text.trim().is_empty() => {
            check("agents_contract", Status::Ok, "AGENTS.md present")
        }
        Some(_) => check("agents_contract", Status::Warn, "AGENTS.md is empty"),
        None => check(
            "agents_contract",
            Status::Warn,
            "AGENTS.md missing; agents have no shared review contract",
        ),
    }
}

pub(crate) fn claude_check(claude: Option<&str>) -> Check {
    match claude {
        None => check("claude_import", Status::Ok, "CLAUDE.md not present"),
        Some(text) if text.lines().any(|line| line.trim() == "@AGENTS.md") => {
            check("claude_import", Status::Ok, "CLAUDE.md imports @AGENTS.md")
        }
        Some(_) => check(
            "claude_import",
            Status::Warn,
            "CLAUDE.md does not import @AGENTS.md",
        ),
    }
}

pub(crate) fn codeowners_check(codeowners: Option<(&str, &str)>, policy_path: &str) -> Check {
    let Some((file, text)) = codeowners else {
        return check(
            "codeowners",
            Status::Warn,
            "no CODEOWNERS file; policy changes need no owner review",
        );
    };
    match code_owners(text, policy_path) {
        Some(owners) => check(
            "codeowners",
            Status::Ok,
            format!("{file} assigns {policy_path} to {owners}"),
        ),
        None => check(
            "codeowners",
            Status::Warn,
            format!("{file} assigns no owner to {policy_path}"),
        ),
    }
}

/// Owners of `path` under CODEOWNERS semantics: the last matching rule wins,
/// and a matching rule without owners leaves the path unowned.
pub(crate) fn code_owners(text: &str, path: &str) -> Option<String> {
    let mut owners = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let pattern = fields.next().unwrap_or_default();
        if owner_pattern_matches(pattern, path) {
            let listed = fields.collect::<Vec<_>>();
            owners = (!listed.is_empty()).then(|| listed.join(" "));
        }
    }
    owners
}

/// Gitignore-style matching for the CODEOWNERS subset: a leading or inner `/`
/// anchors at the root, a trailing `/` matches directories only, `*` stays
/// within one path segment and `**` crosses segments.
pub(crate) fn owner_pattern_matches(pattern: &str, path: &str) -> bool {
    let directory_only = pattern.ends_with('/');
    let trimmed = pattern.trim_end_matches('/');
    let anchored = trimmed.contains('/');
    let trimmed = trimmed.trim_start_matches('/');
    let segments: Vec<&str> = path.split('/').collect();
    let usable = segments.len() - usize::from(directory_only);
    if anchored {
        (1..=usable).any(|end| glob(trimmed.as_bytes(), segments[..end].join("/").as_bytes()))
    } else {
        segments[..usable]
            .iter()
            .any(|segment| glob(trimmed.as_bytes(), segment.as_bytes()))
    }
}

fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern {
        [] => text.is_empty(),
        [b'*', b'*', rest @ ..] => (0..=text.len()).any(|skip| glob(rest, &text[skip..])),
        [b'*', rest @ ..] => {
            let segment = text
                .iter()
                .position(|byte| *byte == b'/')
                .unwrap_or(text.len());
            (0..=segment).any(|skip| glob(rest, &text[skip..]))
        }
        [b'?', rest @ ..] => {
            text.first().is_some_and(|byte| *byte != b'/') && glob(rest, &text[1..])
        }
        [expected, rest @ ..] => text.first() == Some(expected) && glob(rest, &text[1..]),
    }
}

/// Exit 2 when any check would make a scan incomplete; warnings are advisory.
pub(crate) fn exit_code(checks: &[Check]) -> u8 {
    if checks.iter().any(|check| check.status == Status::Error) {
        2
    } else {
        0
    }
}

pub(crate) fn to_json(report: &DoctorReport) -> Value {
    let diagnostics = report.diagnostics.as_ref().map(|diagnostics| {
        json!({
            "scanned_files": diagnostics.scanned_files,
            "blocking_findings": diagnostics.blocking_findings,
            "ignored_findings": diagnostics.ignored_findings,
            "review_signals": diagnostics.review_signals,
            "matched_smells": diagnostics.matched_smells,
            "total_smells": diagnostics.total_smells,
            "smells": diagnostics
                .smells
                .iter()
                .map(|row| json!({
                    "smell": row.smell,
                    "category": row.category,
                    "blocking": row.blocking,
                    "ignored": row.ignored,
                    "review": row.review,
                }))
                .collect::<Vec<_>>(),
            "hotspots": diagnostics
                .hotspots
                .iter()
                .map(|row| json!({
                    "path": row.path,
                    "blocking": row.blocking,
                    "review": row.review,
                }))
                .collect::<Vec<_>>(),
        })
    });
    let burndown = report.burndown.as_ref().map(|burndown| {
        json!({
            "base_ref": burndown.base_ref,
            "base_commit": burndown.base_commit,
            "base_blocking": burndown.base_blocking,
            "current_blocking": burndown.current_blocking,
            "blocking_delta": burndown.blocking_delta,
            "base_review": burndown.base_review,
            "current_review": burndown.current_review,
            "review_delta": burndown.review_delta,
            "base_ignored": burndown.base_ignored,
            "current_ignored": burndown.current_ignored,
            "ignored_delta": burndown.ignored_delta,
            "smell_deltas": burndown
                .smell_deltas
                .iter()
                .map(|row| json!({
                    "smell": row.smell,
                    "base_blocking": row.base_blocking,
                    "current_blocking": row.current_blocking,
                    "blocking_delta": row.blocking_delta,
                    "base_review": row.base_review,
                    "current_review": row.current_review,
                    "review_delta": row.review_delta,
                }))
                .collect::<Vec<_>>(),
        })
    });
    json!({
        "status": if exit_code(&report.checks) == 0 { "healthy" } else { "unhealthy" },
        "exit_code": exit_code(&report.checks),
        "checks": report
            .checks
            .iter()
            .map(|check| json!({"id": check.id, "status": check.status.label(), "detail": check.detail}))
            .collect::<Vec<_>>(),
        "diagnostics": diagnostics,
        "burndown": burndown,
    })
}

pub(crate) fn to_table(report: &DoctorReport) -> String {
    let mut table = String::from("Check | Status | Detail\n");
    for check in &report.checks {
        table.push_str(&format!(
            "{} | {} | {}\n",
            check.id,
            check.status.label(),
            check.detail
        ));
    }
    if let Some(diagnostics) = &report.diagnostics {
        append_diagnostics_table(&mut table, diagnostics);
    }
    if let Some(burndown) = &report.burndown {
        append_burndown_table(&mut table, burndown);
    }
    table
}

fn append_burndown_table(table: &mut String, burndown: &Burndown) {
    let short_commit: String = burndown.base_commit.chars().take(7).collect();
    table.push_str(&format!(
        "\nBurndown since {} ({})\nBlockers: {} -> {} ({:+}) | Review warnings: {} -> {} ({:+}) | Ignored: {} -> {} ({:+})\n",
        burndown.base_ref,
        short_commit,
        burndown.base_blocking,
        burndown.current_blocking,
        burndown.blocking_delta,
        burndown.base_review,
        burndown.current_review,
        burndown.review_delta,
        burndown.base_ignored,
        burndown.current_ignored,
        burndown.ignored_delta,
    ));
    if !burndown.smell_deltas.is_empty() {
        table
            .push_str("Smell | Base blocking | Current blocking | Blocking delta | Review delta\n");
        for row in &burndown.smell_deltas {
            table.push_str(&format!(
                "{} | {} | {} | {:+} | {:+}\n",
                row.smell,
                row.base_blocking,
                row.current_blocking,
                row.blocking_delta,
                row.review_delta
            ));
        }
    }
}

fn append_diagnostics_table(table: &mut String, diagnostics: &Diagnostics) {
    table.push_str(&format!(
        "\nSmell diagnostics | files: {} | blocking: {} | ignored: {} | review: {} | matched smells: {}/{}\n",
        diagnostics.scanned_files,
        diagnostics.blocking_findings,
        diagnostics.ignored_findings,
        diagnostics.review_signals,
        diagnostics.matched_smells,
        diagnostics.total_smells,
    ));
    if !diagnostics.smells.is_empty() {
        table.push_str("Smell | Category | Blocking | Ignored | Review\n");
        for row in &diagnostics.smells {
            table.push_str(&format!(
                "{} | {} | {} | {} | {}\n",
                row.smell, row.category, row.blocking, row.ignored, row.review
            ));
        }
    }
    if !diagnostics.hotspots.is_empty() {
        table.push_str("\nTop hotspot files\nFile | Blocking | Review\n");
        for row in &diagnostics.hotspots {
            table.push_str(&format!(
                "{} | {} | {}\n",
                row.path, row.blocking, row.review
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{core::model::HistoryCommit, policy};

    fn suppressions_in(source: &str) -> ParsedSuppressions {
        let files = BTreeMap::from([("src/lib.rs".to_string(), source.to_string())]);
        suppressions::parse(&files, &policy::registry("rust-v1").unwrap())
    }

    #[test]
    fn history_is_healthy_only_when_complete_and_unshallow() {
        let commit = |id: &str| HistoryCommit {
            id: id.into(),
            files: vec!["src/lib.rs".into()],
        };
        let complete = History {
            available: true,
            shallow: false,
            commits: vec![commit("a"), commit("b")],
        };
        assert_eq!(
            history_check(&complete),
            check(
                "git_history",
                Status::Ok,
                "complete; 2 commit(s) in the 200-commit window change scanned sources"
            )
        );
        let shallow = History {
            shallow: true,
            ..complete.clone()
        };
        let shallow = history_check(&shallow);
        assert_eq!(shallow.status, Status::Error);
        assert!(
            shallow.detail.contains("fetch-depth: 0"),
            "{}",
            shallow.detail
        );
        let missing = history_check(&History::default());
        assert_eq!(missing.status, Status::Error);
        assert!(missing.detail.contains("unavailable"), "{}", missing.detail);
    }

    #[test]
    fn suppressions_are_counted_per_rule_and_invalid_directives_fail_closed() {
        let parsed = suppressions_in(
            "// smells: ignore[rust.function_arguments] -- adapter contract\nfn a(_: u8) {}\n\
             // smells: ignore[rust.function_arguments] -- adapter contract\nfn b(_: u8) {}\n\
             // smells: ignore[rust.type_fields] -- serialized layout\nstruct C;\n",
        );
        assert_eq!(
            suppression_check(&parsed).detail,
            "3 directive(s): rust.function_arguments=2, rust.type_fields=1; \
             `smells check` verifies unused directives"
        );
        assert_eq!(suppression_check(&parsed).status, Status::Ok);
        assert_eq!(
            suppression_check(&suppressions_in("fn a() {}\n")).detail,
            "0 directives; `smells check` verifies unused directives"
        );
        let invalid = suppression_check(&suppressions_in(
            "fn a() {}\n// smells: ignore[rust.function_arguments]\nfn b() {}\n",
        ));
        assert_eq!(invalid.status, Status::Error);
        assert!(
            invalid
                .detail
                .starts_with("1 invalid directive(s); first at src/lib.rs:2: "),
            "{}",
            invalid.detail
        );
    }

    #[test]
    fn agent_contracts_warn_when_missing_empty_or_not_imported() {
        assert_eq!(agents_check(Some("# Rules\n")).status, Status::Ok);
        assert_eq!(agents_check(Some(" \n")).status, Status::Warn);
        assert_eq!(agents_check(None).status, Status::Warn);
        assert_eq!(claude_check(None).status, Status::Ok);
        assert_eq!(
            claude_check(Some("# Claude\n @AGENTS.md \n")).status,
            Status::Ok
        );
        assert_eq!(
            claude_check(Some("see @AGENTS.md first\n")).status,
            Status::Warn
        );
        assert_eq!(claude_check(Some("")).status, Status::Warn);
    }

    #[test]
    fn codeowners_use_the_last_matching_rule_and_allow_explicit_unowning() {
        let text = "# owners\n\n* @all\n*quality-policy.json @quality @lead\nlegacy/ \n";
        assert_eq!(
            code_owners(text, "quality-policy.json").as_deref(),
            Some("@quality @lead")
        );
        assert_eq!(code_owners(text, "src/lib.rs").as_deref(), Some("@all"));
        assert_eq!(code_owners(text, "legacy/quality-policy.json"), None);
        assert_eq!(code_owners("# * @all\n", "a.rs"), None);
        assert_eq!(
            code_owners("*.json @owners\n\n#policy.json\n", "#policy.json").as_deref(),
            Some("@owners")
        );
        assert_eq!(
            codeowners_check(Some(("CODEOWNERS", text)), "quality-policy.json"),
            check(
                "codeowners",
                Status::Ok,
                "CODEOWNERS assigns quality-policy.json to @quality @lead"
            )
        );
        assert_eq!(
            codeowners_check(Some(("CODEOWNERS", text)), "legacy/quality-policy.json"),
            check(
                "codeowners",
                Status::Warn,
                "CODEOWNERS assigns no owner to legacy/quality-policy.json"
            )
        );
        assert_eq!(
            codeowners_check(None, "quality-policy.json").status,
            Status::Warn
        );
    }

    #[test]
    fn owner_patterns_follow_gitignore_anchoring_and_globs() {
        let cases = [
            ("*", "any/path.rs", true),
            ("*quality-policy.json", "quality-policy.json", true),
            (
                "*quality-policy.json",
                "config/python-quality-policy.json",
                true,
            ),
            ("*quality-policy.json", "quality-policy.json.bak", false),
            ("AGENTS.md", "nested/AGENTS.md", true),
            ("src", "src/lib.rs", true),
            ("/quality-policy.json", "quality-policy.json", true),
            ("/quality-policy.json", "config/quality-policy.json", false),
            ("config/", "config/policy.json", true),
            ("config/", "config", false),
            ("/config/*.json", "config/policy.json", true),
            ("/config/*.json", "config/nested/policy.json", false),
            ("docs/**", "docs/a/b.md", true),
            ("/docs/**/b.md", "docs/a/c/b.md", true),
            ("?.md", "a.md", true),
            ("?.md", "ab.md", false),
            ("/x?y", "x/y", false),
            ("/x?y", "xzy", true),
        ];
        for (pattern, path, expected) in cases {
            assert_eq!(
                owner_pattern_matches(pattern, path),
                expected,
                "{pattern} vs {path}"
            );
        }
    }

    #[test]
    fn policy_paths_are_reported_relative_to_the_source_root() {
        assert_eq!(
            relative_policy_path(Path::new("."), Path::new("examples/quality-policy.json")),
            "examples/quality-policy.json"
        );
        assert_eq!(
            relative_policy_path(Path::new("."), Path::new("missing/policy.json")),
            "missing/policy.json"
        );
    }

    #[test]
    fn any_error_makes_the_doctor_unhealthy_while_warnings_stay_advisory() {
        let warned = DoctorReport {
            checks: vec![
                check("a", Status::Ok, "fine"),
                check("b", Status::Warn, "gap"),
            ],
            diagnostics: None,
            burndown: None,
        };
        assert_eq!(exit_code(&warned.checks), 0);
        assert_eq!(
            to_json(&warned),
            json!({
                "status": "healthy",
                "exit_code": 0,
                "checks": [
                    {"id": "a", "status": "ok", "detail": "fine"},
                    {"id": "b", "status": "warn", "detail": "gap"},
                ],
                "diagnostics": null,
                "burndown": null,
            })
        );
        assert_eq!(
            to_table(&warned),
            "Check | Status | Detail\na | ok | fine\nb | warn | gap\n"
        );
        let failed = DoctorReport {
            checks: vec![check("c", Status::Error, "broken")],
            diagnostics: None,
            burndown: None,
        };
        assert_eq!(exit_code(&failed.checks), 2);
        assert_eq!(to_json(&failed)["status"], "unhealthy");
        assert_eq!(to_json(&failed)["checks"][0]["status"], "error");
    }

    #[test]
    fn diagnostics_render_compact_smell_and_hotspot_tables_in_priority_order() {
        let report = DoctorReport {
            checks: vec![check("policy", Status::Ok, "ok")],
            diagnostics: Some(Diagnostics {
                scanned_files: 6,
                blocking_findings: 2,
                ignored_findings: 1,
                review_signals: 4,
                matched_smells: 2,
                total_smells: 23,
                smells: vec![
                    SmellDiagnostic {
                        smell: "Dead Code".into(),
                        category: "dispensables".into(),
                        blocking: 2,
                        ignored: 0,
                        review: 0,
                    },
                    SmellDiagnostic {
                        smell: "Data Class".into(),
                        category: "dispensables".into(),
                        blocking: 0,
                        ignored: 1,
                        review: 4,
                    },
                ],
                hotspots: vec![HotspotFile {
                    path: "src/lib.rs".into(),
                    blocking: 2,
                    review: 4,
                }],
            }),
            burndown: None,
        };
        let table = to_table(&report);
        assert_eq!(
            table,
            "Check | Status | Detail\n\
             policy | ok | ok\n\
             \n\
             Smell diagnostics | files: 6 | blocking: 2 | ignored: 1 | review: 4 | matched smells: 2/23\n\
             Smell | Category | Blocking | Ignored | Review\n\
             Dead Code | dispensables | 2 | 0 | 0\n\
             Data Class | dispensables | 0 | 1 | 4\n\
             \n\
             Top hotspot files\n\
             File | Blocking | Review\n\
             src/lib.rs | 2 | 4\n"
        );
        let json = to_json(&report);
        assert_eq!(json["diagnostics"]["scanned_files"], 6);
        assert_eq!(json["diagnostics"]["blocking_findings"], 2);
        assert_eq!(json["diagnostics"]["ignored_findings"], 1);
        assert_eq!(json["diagnostics"]["review_signals"], 4);
        assert_eq!(json["diagnostics"]["matched_smells"], 2);
        assert_eq!(json["diagnostics"]["total_smells"], 23);
        assert_eq!(json["diagnostics"]["smells"][0]["smell"], "Dead Code");
        assert_eq!(json["diagnostics"]["hotspots"][0]["path"], "src/lib.rs");

        let clean = DoctorReport {
            checks: vec![check("policy", Status::Ok, "ok")],
            diagnostics: Some(Diagnostics {
                scanned_files: 1,
                blocking_findings: 0,
                ignored_findings: 0,
                review_signals: 0,
                matched_smells: 0,
                total_smells: 23,
                smells: Vec::new(),
                hotspots: Vec::new(),
            }),
            burndown: None,
        };
        let clean_table = to_table(&clean);
        assert!(!clean_table.contains("Smell | Category"));
        assert!(!clean_table.contains("Top hotspot files"));
    }

    #[test]
    fn burndown_between_tracks_improvements_regressions_and_table_json_output() {
        let base = Diagnostics {
            scanned_files: 2,
            blocking_findings: 4,
            ignored_findings: 2,
            review_signals: 12,
            matched_smells: 4,
            total_smells: 23,
            smells: vec![
                SmellDiagnostic {
                    smell: "Long Method".into(),
                    category: "bloaters".into(),
                    blocking: 3,
                    ignored: 0,
                    review: 5,
                },
                SmellDiagnostic {
                    smell: "Dead Code".into(),
                    category: "dispensables".into(),
                    blocking: 1,
                    ignored: 0,
                    review: 0,
                },
                SmellDiagnostic {
                    smell: "Data Class".into(),
                    category: "dispensables".into(),
                    blocking: 0,
                    ignored: 0,
                    review: 2,
                },
                SmellDiagnostic {
                    smell: "Comments".into(),
                    category: "dispensables".into(),
                    blocking: 0,
                    ignored: 2,
                    review: 5,
                },
            ],
            hotspots: Vec::new(),
        };
        let current = Diagnostics {
            scanned_files: 2,
            blocking_findings: 1,
            ignored_findings: 1,
            review_signals: 7,
            matched_smells: 2,
            total_smells: 23,
            smells: vec![
                SmellDiagnostic {
                    smell: "Long Method".into(),
                    category: "bloaters".into(),
                    blocking: 1,
                    ignored: 0,
                    review: 2,
                },
                SmellDiagnostic {
                    smell: "Comments".into(),
                    category: "dispensables".into(),
                    blocking: 0,
                    ignored: 1,
                    review: 5,
                },
            ],
            hotspots: Vec::new(),
        };
        let burndown = burndown_between("v0.6.0", "04c02b49999", Some(&base), &current);
        assert_eq!(burndown.blocking_delta, -3);
        assert_eq!(burndown.review_delta, -5);
        assert_eq!(burndown.ignored_delta, -1);
        assert_eq!(burndown.smell_deltas.len(), 3);
        assert_eq!(burndown.smell_deltas[0].smell, "Long Method");
        assert_eq!(burndown.smell_deltas[0].blocking_delta, -2);
        assert_eq!(burndown.smell_deltas[0].review_delta, -3);
        assert_eq!(burndown.smell_deltas[1].smell, "Dead Code");
        assert_eq!(burndown.smell_deltas[1].blocking_delta, -1);
        assert_eq!(burndown.smell_deltas[2].smell, "Data Class");
        assert_eq!(burndown.smell_deltas[2].blocking_delta, 0);
        assert_eq!(burndown.smell_deltas[2].review_delta, -2);

        let report = DoctorReport {
            checks: vec![check("policy", Status::Ok, "ok")],
            diagnostics: Some(current),
            burndown: Some(burndown),
        };
        let table = to_table(&report);
        assert!(table.contains("Burndown since v0.6.0 (04c02b4)\n"));
        assert!(table.contains(
            "Blockers: 4 -> 1 (-3) | Review warnings: 12 -> 7 (-5) | Ignored: 2 -> 1 (-1)\n"
        ));
        assert!(table.contains("Long Method | 3 | 1 | -2 | -3\n"));
        assert!(table.contains("Dead Code | 1 | 0 | -1 | +0\n"));
        assert!(table.contains("Data Class | 0 | 0 | +0 | -2\n"));
        assert!(!table.contains("Comments | 0 | 0"));

        let json = to_json(&report);
        assert_eq!(json["burndown"]["base_ref"], "v0.6.0");
        assert_eq!(json["burndown"]["blocking_delta"], -3);
        assert_eq!(json["burndown"]["ignored_delta"], -1);
        assert_eq!(json["burndown"]["smell_deltas"][0]["smell"], "Long Method");

        // When base has no files yet (None), current findings appear as positive deltas.
        let from_empty = burndown_between(
            "HEAD~1",
            "1111111",
            None,
            report.diagnostics.as_ref().unwrap(),
        );
        assert_eq!(from_empty.base_blocking, 0);
        assert_eq!(from_empty.blocking_delta, 1);
    }

    #[test]
    fn summarize_smells_and_hotspots_order_blockers_before_reviews_and_cap_hotspots() {
        let make = |smell: &'static str, path: &'static str, blocking: bool, ignored: bool| {
            FindingSignal {
                smell,
                category: "dispensables",
                path,
                blocking,
                ignored,
            }
        };
        let findings = vec![
            make("Data Class", "src/a.rs", false, false),
            make("Data Class", "src/a.rs", false, false),
            make("Data Class", "src/b.rs", false, true),
            make("Dead Code", "src/c.rs", true, false),
            make("Comments", "src/d.rs", false, false),
            make("Comments", "src/e.rs", false, false),
            make("Comments", "src/f.rs", false, false),
            make("Comments", "src/g.rs", false, false),
        ];
        let smells = summarize_smells(&findings);
        assert_eq!(smells[0].smell, "Dead Code");
        assert_eq!(smells[0].blocking, 1);
        assert_eq!(smells[1].smell, "Comments");
        assert_eq!(smells[1].review, 4);
        assert_eq!(smells[2].smell, "Data Class");
        assert_eq!(smells[2].ignored, 1);
        assert_eq!(smells[2].review, 2);

        let hotspots = summarize_hotspots(&findings);
        assert_eq!(hotspots.len(), HOTSPOT_LIMIT);
        assert_eq!(hotspots[0].path, "src/c.rs");
        assert_eq!(hotspots[0].blocking, 1);
        assert_eq!(hotspots[1].path, "src/a.rs");
        assert_eq!(hotspots[1].review, 2);
        assert!(!hotspots.iter().any(|row| row.path == "src/b.rs"));
    }
}
