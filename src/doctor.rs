//! `smells doctor`: a deterministic integration preflight.
//!
//! Each check is a pure function of captured input so the verdict never
//! depends on network access or the consuming project's build.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};

use crate::{
    core::{
        analyze_captured,
        model::{CapturedInput, History},
    },
    coverage, input,
    policy::SelectionOptions,
    report::{Finding, Report},
    suppressions::{self, ParsedSuppressions},
};

/// GitHub resolves the first CODEOWNERS file in this order.
const CODEOWNERS_PATHS: [&str; 3] = [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"];
const STANDARD_POLICY_FILES: [&str; 5] = [
    "quality-policy.json",
    "rust-quality-policy.json",
    "python-quality-policy.json",
    "typescript-quality-policy.json",
    "smells-policy.json",
];
const DEFAULT_BASE_REFS: [&str; 2] = ["origin/main", "origin/master"];
/// Conventional LCOV outputs of `cargo llvm-cov`, `pytest-cov` and `vitest`/`jest`.
const COVERAGE_FILES: [&str; 3] = [
    "lcov.info",
    "coverage/lcov.info",
    "target/llvm-cov/lcov.info",
];
const HISTORY_WINDOW: usize = 200;
const HOTSPOT_LIMIT: usize = 5;

const RUST_STARTER_POLICY: &str = include_str!("../examples/quality-policy.json");
const PYTHON_STARTER_POLICY: &str = include_str!("../examples/python-quality-policy.json");
const TYPESCRIPT_STARTER_POLICY: &str = include_str!("../examples/typescript-quality-policy.json");

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
    pub base_score: u8,
    pub current_score: u8,
    pub score_delta: isize,
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
    pub score: u8,
    pub grade: &'static str,
    pub scanned_files: usize,
    pub blocking_findings: usize,
    pub ignored_findings: usize,
    pub review_signals: usize,
    pub matched_smells: usize,
    pub total_smells: usize,
    /// Non-blocking CRAP findings left out because they assume 0% coverage.
    pub estimated_crap_hidden: usize,
    pub smells: Vec<SmellDiagnostic>,
    pub hotspots: Vec<HotspotFile>,
}

/// Which non-blocking CRAP findings a diagnostics view leaves out. Blocking findings are
/// always kept so hiding never weakens the gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CrapView {
    /// Hide only findings scored by the zero-coverage upper bound.
    Measured,
    /// Hide every non-blocking CRAP finding so a covered tree compares fairly with a base
    /// that has no coverage artifact.
    Comparable,
}

impl CrapView {
    fn hides(self, finding: &Finding) -> bool {
        !finding.blocking
            && finding.rule_id.ends_with(".function_crap")
            && match self {
                Self::Measured => {
                    finding
                        .evidence
                        .pointer("/provider_evidence/coverage_assumption_percent")
                        == Some(&json!(0))
                }
                Self::Comparable => true,
            }
    }
}

pub(crate) fn health_score(
    scanned_files: usize,
    blocking_findings: usize,
    review_signals: usize,
) -> u8 {
    let blocking_penalty = blocking_findings.saturating_mul(10).min(75);
    let review_penalty = (review_signals / scanned_files.max(1)).min(25);
    100u8.saturating_sub(u8::try_from(blocking_penalty + review_penalty).unwrap_or(u8::MAX))
}

/// The grade of a scan that reported errors, whatever its score.
pub(crate) const INCOMPLETE_GRADE: &str = "Incomplete";

pub(crate) fn health_grade(score: u8) -> &'static str {
    if score >= 75 {
        "Great"
    } else if score >= 50 {
        "Needs work"
    } else {
        "Critical"
    }
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

enum ResolvedPolicy {
    File(PathBuf),
    BuiltIn {
        rule_pack: &'static str,
        text: &'static str,
    },
}

fn resolve_policy(root: &Path, explicit: Option<&Path>) -> Result<ResolvedPolicy, String> {
    if let Some(path) = explicit {
        return Ok(ResolvedPolicy::File(path.to_path_buf()));
    }
    let found: Vec<PathBuf> = STANDARD_POLICY_FILES
        .iter()
        .map(|name| root.join(name))
        .filter(|path| path.is_file())
        .collect();
    match <[PathBuf; 1]>::try_from(found) {
        Ok([single]) => Ok(ResolvedPolicy::File(single)),
        Err(found) if found.is_empty() => infer_starter_policy(root),
        Err(found) => {
            let names = found
                .iter()
                .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "multiple policy files found ({names}); pass --policy FILE"
            ))
        }
    }
}

fn infer_starter_policy(root: &Path) -> Result<ResolvedPolicy, String> {
    let candidates: [(&[&str], &str, &str); 3] = [
        (&["Cargo.toml"], "rust-v1", RUST_STARTER_POLICY),
        (
            &["pyproject.toml", "setup.py", "requirements.txt"],
            "python-v1",
            PYTHON_STARTER_POLICY,
        ),
        (
            &["tsconfig.json", "package.json"],
            "typescript-v1",
            TYPESCRIPT_STARTER_POLICY,
        ),
    ];
    let matched: Vec<_> = candidates
        .into_iter()
        .filter(|(manifests, _, _)| manifests.iter().any(|file| root.join(file).is_file()))
        .collect();
    match matched.as_slice() {
        [(_, rule_pack, text)] => Ok(ResolvedPolicy::BuiltIn { rule_pack, text }),
        [] => Err(
            "no quality-policy.json found and language could not be inferred; pass --policy FILE"
                .into(),
        ),
        _ => Err(
            "no quality-policy.json found and multiple language manifests exist; pass --policy FILE"
                .into(),
        ),
    }
}

fn detect_default_base(root: &Path) -> Option<&'static str> {
    DEFAULT_BASE_REFS.into_iter().find(|candidate| {
        Command::new("git")
            .args([
                "-C",
                &root.to_string_lossy(),
                "rev-parse",
                "--verify",
                "--quiet",
                candidate,
            ])
            .output()
            .is_ok_and(|output| output.status.success())
    })
}

/// Runs every preflight check and, when inputs are valid, builds compact codebase smell
/// diagnostics plus an optional (or auto-detected) `--base` burndown comparison.
pub(crate) fn diagnose(
    root: &Path,
    policy_path: Option<&Path>,
    coverage_path: Option<&Path>,
    base_ref: Option<&str>,
) -> DoctorReport {
    let mut checks = Vec::new();
    let resolved = resolve_policy(root, policy_path);
    let owned_policy_target = match &resolved {
        Ok(ResolvedPolicy::File(path)) => relative_policy_path(root, path),
        _ => "quality-policy.json".into(),
    };
    let (diagnostics, burndown) = match &resolved {
        Ok(policy_source) => {
            let options = InputOptions {
                coverage_path,
                base_ref,
            };
            diagnose_inputs(root, policy_source, options, &mut checks)
        }
        Err(error) => {
            checks.push(check("policy", Status::Error, error));
            (None, None)
        }
    };
    append_governance_checks(root, &owned_policy_target, &mut checks);
    DoctorReport {
        checks,
        diagnostics,
        burndown,
    }
}

fn append_governance_checks(root: &Path, owned_policy_target: &str, checks: &mut Vec<Check>) {
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
        owned_policy_target,
    ));
}

#[derive(Clone, Copy)]
struct InputOptions<'a> {
    coverage_path: Option<&'a Path>,
    base_ref: Option<&'a str>,
}

fn diagnose_inputs(
    root: &Path,
    policy_source: &ResolvedPolicy,
    options: InputOptions<'_>,
    checks: &mut Vec<Check>,
) -> (Option<Diagnostics>, Option<Burndown>) {
    let captured = match policy_source {
        ResolvedPolicy::File(path) => input::working_tree(root, path, &SelectionOptions::default()),
        ResolvedPolicy::BuiltIn { text, .. } => {
            input::working_tree_with_policy_text(root, text, &SelectionOptions::default())
        }
    };
    let mut captured = match captured {
        Ok(captured) => captured,
        Err(error) => {
            checks.push(check("policy", Status::Error, error));
            return (None, None);
        }
    };
    checks.push(policy_check(policy_source, &captured));
    let coverage_file = attach_coverage(root, options.coverage_path, &mut captured, checks);
    checks.push(history_check(&captured.input.history));
    checks.push(suppression_check(&suppressions::parse(
        &captured.input.files,
        &captured.registry,
    )));
    if exit_code(checks) != 0 {
        return (None, None);
    }
    let Some(report) = diagnose_scan(&captured, checks) else {
        return (None, None);
    };
    let diagnostics = diagnostics_from_report(&report, CrapView::Measured);
    checks.push(coverage_check(
        coverage_file.as_deref(),
        &captured,
        diagnostics.estimated_crap_hidden,
    ));
    let comparable = diagnostics_from_report(&report, CrapView::Comparable);
    let burndown = run_burndown(root, policy_source, options.base_ref, &comparable, checks);
    (Some(diagnostics), burndown)
}

fn discover_coverage(root: &Path) -> Option<PathBuf> {
    COVERAGE_FILES
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
}

/// Loads explicit or conventional LCOV into the captured input. A coverage file that is
/// present but unusable is an error rather than a silent fallback to the upper bound.
fn attach_coverage(
    root: &Path,
    explicit: Option<&Path>,
    captured: &mut CapturedInput,
    checks: &mut Vec<Check>,
) -> Option<String> {
    let path = explicit
        .map(Path::to_path_buf)
        .or_else(|| discover_coverage(root))?;
    let label = relative_policy_path(root, &path);
    match coverage::load(&path, root, &captured.input.files) {
        Ok(loaded) => {
            captured.input.coverage = Some(loaded);
            Some(label)
        }
        Err(error) => {
            checks.push(check(
                "coverage",
                Status::Error,
                format!("{label}: {error}"),
            ));
            None
        }
    }
}

fn coverage_hint(language: &str) -> &'static str {
    match language {
        "python" => "pytest --cov --cov-report=lcov:lcov.info",
        "typescript" => "vitest run --coverage --coverage.reporter=lcov",
        _ => "cargo llvm-cov --lcov --output-path lcov.info",
    }
}

fn coverage_check(file: Option<&str>, captured: &CapturedInput, hidden: usize) -> Check {
    match (file, captured.input.coverage.as_ref()) {
        (Some(file), Some(coverage)) => check(
            "coverage",
            Status::Ok,
            format!(
                "{file} covers {} scanned file(s); CRAP scores use measured line coverage",
                coverage.files.len()
            ),
        ),
        _ => check(
            "coverage",
            Status::Warn,
            format!(
                "no LCOV coverage found; hid {hidden} non-blocking CRAP estimate(s) that assume 0% coverage. Run `{}` or pass --coverage FILE",
                coverage_hint(&captured.registry.language)
            ),
        ),
    }
}

fn policy_check(source: &ResolvedPolicy, captured: &crate::core::model::CapturedInput) -> Check {
    let policy = &captured.input.policy;
    match source {
        ResolvedPolicy::File(_) => check(
            "policy",
            Status::Ok,
            format!(
                "{} pinned to scanner {}, {}/{} rules active",
                captured.registry.rule_pack,
                policy.scanner_version,
                policy.resolved.active_rule_ids.len(),
                captured.registry.rules.len()
            ),
        ),
        ResolvedPolicy::BuiltIn { rule_pack, .. } => check(
            "policy",
            Status::Warn,
            format!(
                "no quality-policy.json found; using built-in {rule_pack} starter policy ({}/{} rules active)",
                policy.resolved.active_rule_ids.len(),
                captured.registry.rules.len()
            ),
        ),
    }
}

fn run_burndown(
    root: &Path,
    policy_source: &ResolvedPolicy,
    explicit_base: Option<&str>,
    current: &Diagnostics,
    checks: &mut Vec<Check>,
) -> Option<Burndown> {
    let (target_ref, strict) = match explicit_base {
        Some(base) => (base, true),
        None => (detect_default_base(root)?, false),
    };
    diagnose_burndown(root, policy_source, target_ref, strict, current, checks)
}

fn diagnose_burndown(
    root: &Path,
    policy_source: &ResolvedPolicy,
    base_ref: &str,
    strict: bool,
    current: &Diagnostics,
    checks: &mut Vec<Check>,
) -> Option<Burndown> {
    let reference = capture_base_reference(root, policy_source, base_ref);
    let reference = match reference {
        Ok(reference) => reference,
        Err(error) => {
            record_base_error(strict, error, checks);
            return None;
        }
    };
    let base_diagnostics = match reference.captured.as_ref() {
        Some(captured) => match analyze_base_snapshot(captured, base_ref) {
            Ok(diagnostics) => Some(diagnostics),
            Err(error) => {
                record_base_error(strict, error, checks);
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

fn capture_base_reference(
    root: &Path,
    policy_source: &ResolvedPolicy,
    base_ref: &str,
) -> Result<crate::core::model::ReferenceSnapshot, String> {
    match policy_source {
        ResolvedPolicy::File(path) => {
            input::git_ref_for_working_tree(root, base_ref, path, &SelectionOptions::default())
        }
        ResolvedPolicy::BuiltIn { text, .. } => input::git_ref_for_working_tree_with_policy_text(
            root,
            base_ref,
            text,
            &SelectionOptions::default(),
        ),
    }
}

fn analyze_base_snapshot(
    captured: &crate::core::model::CapturedInput,
    base_ref: &str,
) -> Result<Diagnostics, String> {
    let report = analyze_captured(captured)?;
    if let Some(first) = report.errors.first() {
        return Err(format!(
            "base {base_ref} has {} scan error(s); first: {}",
            report.errors.len(),
            first.message
        ));
    }
    Ok(diagnostics_from_report(&report, CrapView::Comparable))
}

fn record_base_error(strict: bool, error: String, checks: &mut Vec<Check>) {
    if strict {
        checks.push(check("base_ref", Status::Error, error));
    }
}

pub(crate) fn burndown_between(
    base_ref: &str,
    base_commit: &str,
    base: Option<&Diagnostics>,
    current: &Diagnostics,
) -> Burndown {
    let base_score = base.map_or(100, |diag| diag.score);
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
        base_score,
        current_score: current.score,
        score_delta: isize::from(current.score) - isize::from(base_score),
        base_blocking,
        current_blocking: current.blocking_findings,
        blocking_delta: current.blocking_findings.cast_signed() - base_blocking.cast_signed(),
        base_review,
        current_review: current.review_signals,
        review_delta: current.review_signals.cast_signed() - base_review.cast_signed(),
        base_ignored,
        current_ignored: current.ignored_findings,
        ignored_delta: current.ignored_findings.cast_signed() - base_ignored.cast_signed(),
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
            let blocking_delta = current_blocking.cast_signed() - base_blocking.cast_signed();
            let review_delta = current_review.cast_signed() - base_review.cast_signed();
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

fn diagnose_scan(captured: &CapturedInput, checks: &mut Vec<Check>) -> Option<Report> {
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
            Some(report)
        }
        Err(error) => {
            checks.push(check("scan", Status::Error, error));
            None
        }
    }
}

pub(crate) fn diagnostics_from_report(report: &Report, view: CrapView) -> Diagnostics {
    let (hidden, shown): (Vec<&Finding>, Vec<&Finding>) = report
        .evidence
        .findings
        .iter()
        .partition(|finding| view.hides(finding));
    let estimated_crap_hidden = hidden
        .iter()
        .filter(|finding| CrapView::Measured.hides(finding))
        .count();
    let count = |keep: fn(&Finding) -> bool| shown.iter().filter(|finding| keep(finding)).count();
    let blocking_findings = count(|finding| finding.blocking);
    let ignored_findings = count(|finding| finding.status == "ignored_match");
    let review_signals =
        count(|finding| matches!(finding.status.as_str(), "indicator" | "reported_by_role"));
    let signals: Vec<_> = shown.into_iter().map(FindingSignal::from_finding).collect();
    let smells = summarize_smells(&signals);
    let hotspots = summarize_hotspots(&signals);
    let scanned_files = report.scanned_files.len();
    let score = health_score(scanned_files, blocking_findings, review_signals);
    Diagnostics {
        score,
        // A scan with errors skipped code, so no score can vouch for the whole tree.
        grade: if report.errors.is_empty() {
            health_grade(score)
        } else {
            INCOMPLETE_GRADE
        },
        scanned_files,
        blocking_findings,
        ignored_findings,
        review_signals,
        matched_smells: report.summary.matched_smell_patterns,
        total_smells: report.summary.total_smell_patterns,
        estimated_crap_hidden,
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
            "score": diagnostics.score,
            "grade": diagnostics.grade,
            "scanned_files": diagnostics.scanned_files,
            "blocking_findings": diagnostics.blocking_findings,
            "ignored_findings": diagnostics.ignored_findings,
            "review_signals": diagnostics.review_signals,
            "matched_smells": diagnostics.matched_smells,
            "total_smells": diagnostics.total_smells,
            "estimated_crap_hidden": diagnostics.estimated_crap_hidden,
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
            "base_score": burndown.base_score,
            "current_score": burndown.current_score,
            "score_delta": burndown.score_delta,
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

#[derive(Clone, Copy)]
struct Palette {
    color: bool,
}

impl Palette {
    fn paint(self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    fn count(self, value: usize, width: usize, active_code: &str) -> String {
        let formatted = format!("{value:>width$}");
        let code = if value == 0 { "2" } else { active_code };
        self.paint(code, &formatted)
    }

    fn delta(self, value: isize, width: usize, higher_is_better: bool) -> String {
        let formatted = format!("{value:>+width$}");
        let code = delta_color(value, higher_is_better);
        self.paint(code, &formatted)
    }
}

fn delta_color(value: isize, higher_is_better: bool) -> &'static str {
    match (value.signum(), higher_is_better) {
        (0, _) => "2",
        (1, true) | (-1, false) => "32",
        _ => "31",
    }
}

fn score_color(score: u8) -> &'static str {
    if score >= 75 {
        "32"
    } else if score >= 50 {
        "33"
    } else {
        "31"
    }
}

fn status_badge(status: Status, palette: Palette) -> String {
    match status {
        Status::Ok => palette.paint("32", "✓"),
        Status::Warn => palette.paint("33", "!"),
        Status::Error => palette.paint("31", "✗"),
    }
}

fn health_bar(score: u8, palette: Palette) -> String {
    let filled = usize::from(score) / 5;
    let empty = 20 - filled;
    format!(
        "{}{}",
        palette.paint(score_color(score), &"█".repeat(filled)),
        palette.paint("2", &"░".repeat(empty))
    )
}

pub(crate) fn to_table(report: &DoctorReport, color: bool) -> String {
    let palette = Palette { color };
    let mut output = format!(
        "{}\n\n{}\n",
        palette.paint(
            "1;36",
            &format!("smells doctor v{}", env!("CARGO_PKG_VERSION"))
        ),
        palette.paint("1", "Setup & Governance"),
    );
    let id_width = report
        .checks
        .iter()
        .map(|check| check.id.len())
        .max()
        .unwrap_or(0);
    for check in &report.checks {
        let _ = writeln!(
            output,
            "  {} {:<id_width$}  {}",
            status_badge(check.status, palette),
            check.id,
            check.detail,
        );
    }
    if let Some(diagnostics) = &report.diagnostics {
        append_diagnostics_table(&mut output, diagnostics, palette);
    }
    if let Some(burndown) = &report.burndown {
        append_burndown_table(&mut output, burndown, palette);
    }
    output
}

fn append_diagnostics_table(output: &mut String, diagnostics: &Diagnostics, palette: Palette) {
    let color = score_color(diagnostics.score);
    let score_label = format!("{}/100", diagnostics.score);
    let _ = write!(
        output,
        "\n{}\n  {}  {}  {}\n  {} files · {} blocking · {} ignored · {} review warnings · {}/{} smells matched\n",
        palette.paint("1", "Codebase Health"),
        health_bar(diagnostics.score, palette),
        palette.paint(&format!("1;{color}"), &score_label),
        palette.paint(color, diagnostics.grade),
        diagnostics.scanned_files,
        palette.paint(
            if diagnostics.blocking_findings > 0 {
                "1;31"
            } else {
                "32"
            },
            &diagnostics.blocking_findings.to_string(),
        ),
        diagnostics.ignored_findings,
        palette.paint(
            if diagnostics.review_signals > 0 {
                "33"
            } else {
                "2"
            },
            &diagnostics.review_signals.to_string(),
        ),
        diagnostics.matched_smells,
        diagnostics.total_smells,
    );
    append_smells_section(output, &diagnostics.smells, palette);
    append_hotspots_section(output, &diagnostics.hotspots, palette);
}

fn append_smells_section(output: &mut String, smells: &[SmellDiagnostic], palette: Palette) {
    if smells.is_empty() {
        return;
    }
    let smell_width = smells
        .iter()
        .map(|row| row.smell.len())
        .max()
        .unwrap_or(0)
        .max("Smell".len());
    let category_width = smells
        .iter()
        .map(|row| row.category.len())
        .max()
        .unwrap_or(0)
        .max("Category".len());
    let _ = write!(
        output,
        "\n{}\n  {}\n",
        palette.paint("1", "Smells (hard blockers first)"),
        palette.paint(
            "2",
            &format!(
                "{:<smell_width$}  {:<category_width$}  {:>8}  {:>7}  {:>6}",
                "Smell", "Category", "Blocking", "Ignored", "Review"
            ),
        ),
    );
    for row in smells {
        let _ = writeln!(
            output,
            "  {:<smell_width$}  {:<category_width$}  {}  {}  {}",
            row.smell,
            palette.paint("2", &format!("{:<category_width$}", row.category)),
            palette.count(row.blocking, 8, "1;31"),
            palette.count(row.ignored, 7, "36"),
            palette.count(row.review, 6, "33"),
        );
    }
}

fn append_hotspots_section(output: &mut String, hotspots: &[HotspotFile], palette: Palette) {
    if hotspots.is_empty() {
        return;
    }
    let file_width = hotspots
        .iter()
        .map(|row| row.path.len())
        .max()
        .unwrap_or(0)
        .max("File".len());
    let _ = write!(
        output,
        "\n{}\n  {}\n",
        palette.paint("1", "Top Hotspot Files"),
        palette.paint(
            "2",
            &format!(
                "{:<file_width$}  {:>8}  {:>6}",
                "File", "Blocking", "Review"
            ),
        ),
    );
    for row in hotspots {
        let _ = writeln!(
            output,
            "  {:<file_width$}  {}  {}",
            row.path,
            palette.count(row.blocking, 8, "1;31"),
            palette.count(row.review, 6, "33"),
        );
    }
}

fn append_burndown_table(output: &mut String, burndown: &Burndown, palette: Palette) {
    let short_commit: String = burndown.base_commit.chars().take(7).collect();
    let _ = write!(
        output,
        "\n{}\n  Score: {} -> {} ({}) · Blockers: {} -> {} ({}) · Review: {} -> {} ({}) · Ignored: {} -> {} ({})\n",
        palette.paint(
            "1",
            &format!("Burndown since {} ({short_commit})", burndown.base_ref),
        ),
        burndown.base_score,
        burndown.current_score,
        palette.delta(burndown.score_delta, 0, true),
        burndown.base_blocking,
        burndown.current_blocking,
        palette.delta(burndown.blocking_delta, 0, false),
        burndown.base_review,
        burndown.current_review,
        palette.delta(burndown.review_delta, 0, false),
        burndown.base_ignored,
        burndown.current_ignored,
        palette.delta(burndown.ignored_delta, 0, false),
    );
    if burndown.smell_deltas.is_empty() {
        return;
    }
    let smell_width = burndown
        .smell_deltas
        .iter()
        .map(|row| row.smell.len())
        .max()
        .unwrap_or(0)
        .max("Smell".len());
    let _ = writeln!(
        output,
        "  {}",
        palette.paint(
            "2",
            &format!(
                "{:<smell_width$}  {:>4}  {:>7}  {:>5}  {:>6}",
                "Smell", "Base", "Current", "Delta", "Review"
            ),
        ),
    );
    for row in &burndown.smell_deltas {
        let _ = writeln!(
            output,
            "  {:<smell_width$}  {:>4}  {:>7}  {}  {}",
            row.smell,
            row.base_blocking,
            row.current_blocking,
            palette.delta(row.blocking_delta, 5, false),
            palette.delta(row.review_delta, 6, false),
        );
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
    fn coverage_hint_names_each_language_lcov_command() {
        assert_eq!(
            coverage_hint("python"),
            "pytest --cov --cov-report=lcov:lcov.info"
        );
        assert_eq!(
            coverage_hint("typescript"),
            "vitest run --coverage --coverage.reporter=lcov"
        );
        assert_eq!(
            coverage_hint("rust"),
            "cargo llvm-cov --lcov --output-path lcov.info"
        );
    }

    #[test]
    fn history_is_healthy_only_when_complete_and_unshallow() {
        let commit = |id: &str| HistoryCommit {
            id: id.into(),
            root: false,
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
            ..complete
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
            to_table(&warned, false),
            format!(
                "smells doctor v{}\n\nSetup & Governance\n  ✓ a  fine\n  ! b  gap\n",
                env!("CARGO_PKG_VERSION")
            )
        );
        let colored = to_table(&warned, true);
        assert!(colored.contains("\x1b[32m✓\x1b[0m"), "{colored}");
        assert!(colored.contains("\x1b[33m!\x1b[0m"), "{colored}");
        let failed = DoctorReport {
            checks: vec![check("c", Status::Error, "broken")],
            diagnostics: None,
            burndown: None,
        };
        assert_eq!(exit_code(&failed.checks), 2);
        assert_eq!(to_json(&failed)["status"], "unhealthy");
        assert_eq!(to_json(&failed)["checks"][0]["status"], "error");
        assert!(to_table(&failed, true).contains("\x1b[31m✗\x1b[0m"));
    }

    #[test]
    fn health_score_and_grade_cover_every_band_and_cap() {
        assert_eq!(health_score(0, 0, 0), 100);
        assert_eq!(health_grade(100), "Great");
        assert_eq!(health_score(10, 0, 20), 98);
        assert_eq!(health_score(1, 0, 30), 75);
        assert_eq!(health_grade(75), "Great");
        assert_eq!(health_score(1, 2, 6), 74);
        assert_eq!(health_grade(74), "Needs work");
        assert_eq!(health_score(1, 5, 0), 50);
        assert_eq!(health_grade(50), "Needs work");
        assert_eq!(health_score(1, 5, 1), 49);
        assert_eq!(health_grade(49), "Critical");
        assert_eq!(health_score(1, 10, 40), 0);
        assert_eq!(health_grade(0), "Critical");
        assert_eq!(score_color(75), "32");
        assert_eq!(score_color(50), "33");
        assert_eq!(score_color(49), "31");
        assert_eq!(delta_color(0, true), "2");
        assert_eq!(delta_color(0, false), "2");
        assert_eq!(delta_color(2, true), "32");
        assert_eq!(delta_color(-2, true), "31");
        assert_eq!(delta_color(-2, false), "32");
        assert_eq!(delta_color(2, false), "31");
        let palette = Palette { color: true };
        assert_eq!(palette.count(0, 3, "31"), "\x1b[2m  0\x1b[0m");
        assert_eq!(palette.count(2, 3, "31"), "\x1b[31m  2\x1b[0m");
    }

    #[test]
    fn diagnostics_render_compact_smell_and_hotspot_tables_in_priority_order() {
        let report = DoctorReport {
            checks: vec![check("policy", Status::Ok, "ok")],
            diagnostics: Some(Diagnostics {
                score: 80,
                grade: "Great",
                scanned_files: 6,
                blocking_findings: 2,
                ignored_findings: 1,
                review_signals: 4,
                matched_smells: 2,
                total_smells: 23,
                estimated_crap_hidden: 0,
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
        let table = to_table(&report, false);
        assert!(
            table.contains("Codebase Health\n  ████████████████░░░░  80/100  Great\n"),
            "{table}"
        );
        assert!(
            table.contains(
                "6 files · 2 blocking · 1 ignored · 4 review warnings · 2/23 smells matched"
            ),
            "{table}"
        );
        assert!(
            table.contains("Dead Code   dispensables         2        0       0\n"),
            "{table}"
        );
        assert!(
            table.contains("Data Class  dispensables         0        1       4\n"),
            "{table}"
        );
        assert!(table.contains("Top Hotspot Files\n  File        Blocking  Review\n  src/lib.rs         2       4\n"), "{table}");
        assert!(to_table(&report, true).contains("\x1b[1;32m80/100\x1b[0m"));

        let json = to_json(&report);
        assert_eq!(json["diagnostics"]["score"], 80);
        assert_eq!(json["diagnostics"]["grade"], "Great");
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
                score: 100,
                grade: "Great",
                scanned_files: 1,
                blocking_findings: 0,
                ignored_findings: 0,
                review_signals: 0,
                matched_smells: 0,
                total_smells: 23,
                estimated_crap_hidden: 0,
                smells: Vec::new(),
                hotspots: Vec::new(),
            }),
            burndown: None,
        };
        let clean_table = to_table(&clean, false);
        assert!(
            clean_table.contains("████████████████████  100/100  Great"),
            "{clean_table}"
        );
        assert!(!clean_table.contains("Smells (hard blockers first)"));
        assert!(!clean_table.contains("Top Hotspot Files"));
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one fixture walks the full burndown table and its JSON"
    )]
    fn burndown_between_tracks_improvements_regressions_and_table_json_output() {
        let base = Diagnostics {
            score: 54,
            grade: "Needs work",
            scanned_files: 2,
            blocking_findings: 4,
            ignored_findings: 2,
            review_signals: 12,
            matched_smells: 4,
            total_smells: 23,
            estimated_crap_hidden: 0,
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
            score: 87,
            grade: "Great",
            scanned_files: 2,
            blocking_findings: 1,
            ignored_findings: 1,
            review_signals: 7,
            matched_smells: 2,
            total_smells: 23,
            estimated_crap_hidden: 0,
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
        assert_eq!(burndown.base_score, 54);
        assert_eq!(burndown.current_score, 87);
        assert_eq!(burndown.score_delta, 33);
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
        let table = to_table(&report, false);
        assert!(
            table.contains("Burndown since v0.6.0 (04c02b4)\n"),
            "{table}"
        );
        assert!(
            table.contains(
                "Score: 54 -> 87 (+33) · Blockers: 4 -> 1 (-3) · Review: 12 -> 7 (-5) · Ignored: 2 -> 1 (-1)\n"
            ),
            "{table}"
        );
        assert!(
            table.contains("Long Method     3        1     -2      -3\n"),
            "{table}"
        );
        assert!(
            table.contains("Dead Code       1        0     -1      +0\n"),
            "{table}"
        );
        assert!(
            table.contains("Data Class      0        0     +0      -2\n"),
            "{table}"
        );
        assert!(!table.contains("Comments        0        0"));
        assert!(to_table(&report, true).contains("\x1b[32m+33\x1b[0m"));

        let json = to_json(&report);
        assert_eq!(json["burndown"]["base_ref"], "v0.6.0");
        assert_eq!(json["burndown"]["score_delta"], 33);
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
        assert_eq!(from_empty.base_score, 100);
        assert_eq!(from_empty.score_delta, -13);
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

    #[test]
    fn infer_starter_policy_and_record_base_error_handle_all_manifest_combinations() {
        let base_dir =
            std::env::temp_dir().join(format!("smells-doctor-infer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base_dir);
        fs::create_dir_all(&base_dir).unwrap();

        assert!(
            infer_starter_policy(&base_dir)
                .err()
                .unwrap()
                .contains("language could not be inferred")
        );

        for (manifest, expected_pack) in [
            ("Cargo.toml", "rust-v1"),
            ("pyproject.toml", "python-v1"),
            ("package.json", "typescript-v1"),
        ] {
            let dir = base_dir.join(expected_pack);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(manifest), "").unwrap();
            match infer_starter_policy(&dir) {
                Ok(ResolvedPolicy::BuiltIn { rule_pack, .. }) => {
                    assert_eq!(rule_pack, expected_pack);
                }
                _ => panic!("expected built-in {expected_pack}"),
            }
        }

        let multi = base_dir.join("multi");
        fs::create_dir_all(&multi).unwrap();
        fs::write(multi.join("Cargo.toml"), "").unwrap();
        fs::write(multi.join("package.json"), "").unwrap();
        assert!(
            infer_starter_policy(&multi)
                .err()
                .unwrap()
                .contains("multiple language manifests")
        );

        let custom = base_dir.join("custom");
        fs::create_dir_all(custom.join(".github")).unwrap();
        fs::write(custom.join("custom-policy.json"), "{}").unwrap();
        fs::write(
            custom.join(".github/CODEOWNERS"),
            "/custom-policy.json @team\n",
        )
        .unwrap();
        let report = diagnose(
            &custom,
            Some(&custom.join("custom-policy.json")),
            None,
            None,
        );
        let codeowners = report
            .checks
            .iter()
            .find(|entry| entry.id == "codeowners")
            .unwrap();
        assert_eq!(codeowners.status, Status::Ok);
        assert!(codeowners.detail.contains("custom-policy.json"));
        let _ = fs::remove_dir_all(&base_dir);

        let mut checks = Vec::new();
        record_base_error(false, "ignored".into(), &mut checks);
        assert!(checks.is_empty());
        record_base_error(true, "bad ref".into(), &mut checks);
        assert_eq!(checks, vec![check("base_ref", Status::Error, "bad ref")]);
    }
}
