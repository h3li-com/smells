use std::{
    fmt, fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use super::coverage_prompt::Terminal;
use crate::{
    adapters::outbound::coverage_tool,
    core::{
        model::{ScanRequest, ScanTarget},
        ports::inbound::Scanner,
    },
    coverage, doctor, metrics, policy, report,
};

const USAGE: &str = "smells --version\nsmells check [--path DIR | --staged] [--policy FILE] [--base REF] [--evidence FILE] [--coverage LCOV] [--format table|json] [--report FILE] [--log FILE] [policy group selectors]\nsmells policy show [--policy FILE] [--format table|json] [policy group selectors]\nsmells contracts validate [--policy FILE]\nsmells doctor [--policy FILE] [--path DIR] [--coverage LCOV] [--base REF] [--format table|json]\nsmells rules [--rule-pack rust-v1|python-v1|typescript-v1]\n\ncheck scans --path . unless --path or --staged is given.\n--policy defaults to quality-policy.json in the --path directory (or the current directory).\n--log defaults to smells-findings.log unless --format is given; --format table prints every finding instead.\n--coverage reads LCOV line coverage for exact CRAP scores. Without it, doctor and check (on --path without --base) find lcov.info, coverage/lcov.info or target/llvm-cov/lcov.info, and at a terminal offer to run the project's coverage tool.\n\npolicy group selectors: --group NAME | --only-group NAME | --all-groups | --no-default-groups | --no-group NAME";

enum Command {
    Help,
    Version,
    Rules(String),
    Validate(PathBuf),
    Show(ShowOptions),
    Check(CheckOptions),
    Doctor(DoctorOptions),
}

/// Why the arguments do not make a command. A missing default policy stays
/// separate so the hint about adding one is worked out when the command runs,
/// not while the arguments are read.
#[derive(Debug)]
enum CommandError {
    Invalid(String),
    /// No `--policy` was given and the scan root, held here, has no default policy.
    MissingPolicy(PathBuf),
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::MissingPolicy(root) => write!(
                formatter,
                "no --policy given and {} not found",
                root.join(DEFAULT_POLICY).display()
            ),
        }
    }
}

impl CommandError {
    /// The message for the user, with how to add a policy when one is missing.
    fn into_message(self) -> String {
        match &self {
            Self::Invalid(_) => self.to_string(),
            Self::MissingPolicy(root) => {
                format!("{self}; {}", doctor::missing_policy_hint(root))
            }
        }
    }
}

struct DoctorOptions {
    source: PathBuf,
    policy_path: Option<PathBuf>,
    coverage_path: Option<PathBuf>,
    base_ref: Option<String>,
    format: Option<String>,
}

struct CheckOptions {
    source: Option<PathBuf>,
    policy_path: PathBuf,
    evidence_path: Option<PathBuf>,
    coverage_path: Option<PathBuf>,
    base_ref: Option<String>,
    report_path: Option<PathBuf>,
    log_path: Option<PathBuf>,
    format: Option<String>,
    selection: policy::SelectionOptions,
}

struct ShowOptions {
    policy_path: PathBuf,
    format: Option<String>,
    selection: policy::SelectionOptions,
}

#[derive(Default)]
struct PendingOptions {
    source: Option<PathBuf>,
    policy_path: Option<PathBuf>,
    evidence_path: Option<PathBuf>,
    coverage_path: Option<PathBuf>,
    base_ref: Option<String>,
    report_path: Option<PathBuf>,
    log_path: Option<PathBuf>,
    format: Option<String>,
    staged: bool,
    selection: policy::SelectionOptions,
}

fn rules_command(args: &[String]) -> Result<Command, String> {
    let rule_pack = match args {
        [_] => "rust-v1",
        [_, flag, value] if flag == "--rule-pack" => value,
        _ => return Err("usage: smells rules [--rule-pack PACK]".into()),
    };
    Ok(Command::Rules(rule_pack.to_string()))
}

fn set_option(
    flag: &str,
    value: &str,
    contracts: bool,
    options: &mut PendingOptions,
) -> Result<(), String> {
    if let Some(slot) = scan_path_slot(flag, options) {
        return set_once(slot, PathBuf::from(value), !contracts, flag);
    }
    match flag {
        "--policy" => set_once(&mut options.policy_path, PathBuf::from(value), true, flag),
        "--base" => set_base_ref(&mut options.base_ref, value, contracts, flag),
        "--format" => set_format(&mut options.format, value, contracts, flag),
        _ => Err(format!("unknown, repeated or invalid option: {flag}")),
    }
}

/// Path options that only a scan accepts.
fn scan_path_slot<'a>(
    flag: &str,
    options: &'a mut PendingOptions,
) -> Option<&'a mut Option<PathBuf>> {
    match flag {
        "--evidence" => Some(&mut options.evidence_path),
        "--coverage" => Some(&mut options.coverage_path),
        "--report" => Some(&mut options.report_path),
        "--log" => Some(&mut options.log_path),
        "--path" => Some(&mut options.source),
        _ => None,
    }
}

fn set_base_ref(
    slot: &mut Option<String>,
    value: &str,
    contracts: bool,
    flag: &str,
) -> Result<(), String> {
    let allowed = !contracts && !value.is_empty() && !value.starts_with('-');
    set_once(slot, value.to_string(), allowed, flag)
}

fn set_format(
    slot: &mut Option<String>,
    value: &str,
    contracts: bool,
    flag: &str,
) -> Result<(), String> {
    let allowed = !contracts && matches!(value, "table" | "json");
    set_once(slot, value.to_string(), allowed, flag)
}

fn set_once<T>(slot: &mut Option<T>, value: T, allowed: bool, flag: &str) -> Result<(), String> {
    if !allowed || slot.is_some() {
        return Err(format!("unknown, repeated or invalid option: {flag}"));
    }
    *slot = Some(value);
    Ok(())
}

fn operation_command(args: &[String], contracts: bool) -> Result<Command, CommandError> {
    let mut index = if contracts { 2 } else { 1 };
    let mut options = PendingOptions::default();
    for _ in 0..args.len() {
        if index >= args.len() {
            break;
        }
        consume_operation_option(args, &mut index, contracts, &mut options)?;
    }
    finish_operation(options, contracts)
}

fn consume_operation_option(
    args: &[String],
    index: &mut usize,
    contracts: bool,
    options: &mut PendingOptions,
) -> Result<(), String> {
    let flag = &args[*index];
    if flag == "--staged" {
        if contracts || options.staged {
            return Err(format!("unknown, repeated or invalid option: {flag}"));
        }
        options.staged = true;
        *index += 1;
        return Ok(());
    }
    if !contracts && selection_flag(args, index, &mut options.selection)? {
        return Ok(());
    }
    let value = args
        .get(*index + 1)
        .ok_or_else(|| format!("missing value for {flag}"))?;
    set_option(flag, value, contracts, options)?;
    *index += 2;
    Ok(())
}

const DEFAULT_POLICY: &str = "quality-policy.json";
/// Where a table-format scan writes its Finding Log when `--log` is not given, so the
/// terminal gets a short summary instead of every finding.
const DEFAULT_LOG: &str = "smells-findings.log";

/// How many errors and blocking findings the short summary names before the log.
const SUMMARY_ISSUES: usize = 3;

/// Uses `--policy` when given, otherwise `quality-policy.json` in `root`; a missing
/// default is an error so an unconfigured repository never scans with no policy.
fn policy_or_default(explicit: Option<PathBuf>, root: &Path) -> Result<PathBuf, CommandError> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let path = root.join(DEFAULT_POLICY);
    if path.is_file() {
        return Ok(path);
    }
    Err(CommandError::MissingPolicy(root.to_path_buf()))
}

/// Validates the source flags and returns the scan root, `.` unless `--path` or `--staged` is given.
fn check_source(options: &PendingOptions) -> Result<Option<PathBuf>, String> {
    if options.staged && options.source.is_some() {
        return Err("--staged and --path cannot be combined".into());
    }
    if options.base_ref.is_some() && options.evidence_path.is_some() {
        return Err("--evidence cannot be used with --base".into());
    }
    Ok(match &options.source {
        None if !options.staged => Some(PathBuf::from(".")),
        source => source.clone(),
    })
}

fn finish_operation(options: PendingOptions, contracts: bool) -> Result<Command, CommandError> {
    if contracts {
        return policy_or_default(options.policy_path, Path::new(".")).map(Command::Validate);
    }
    let source = check_source(&options)?;
    let root = source.as_deref().unwrap_or(Path::new("."));
    // A staged scan reads the policy from the index, so the default stays a bare
    // repository-relative path and its absence is reported by the staged reader.
    let policy_path = match options.policy_path {
        None if options.staged => PathBuf::from(DEFAULT_POLICY),
        explicit => policy_or_default(explicit, root)?,
    };
    let log_path = match (options.log_path, &options.format) {
        (None, None) => Some(PathBuf::from(DEFAULT_LOG)),
        (log_path, _) => log_path,
    };
    Ok(Command::Check(CheckOptions {
        source,
        policy_path,
        evidence_path: options.evidence_path,
        coverage_path: options.coverage_path,
        base_ref: options.base_ref,
        report_path: options.report_path,
        log_path,
        format: options.format,
        selection: options.selection,
    }))
}

fn selection_flag(
    args: &[String],
    index: &mut usize,
    selection: &mut policy::SelectionOptions,
) -> Result<bool, String> {
    if group_selection_flag(args, index, selection)? {
        return Ok(true);
    }
    boolean_selection_flag(args, index, selection)
}

fn group_selection_flag(
    args: &[String],
    index: &mut usize,
    selection: &mut policy::SelectionOptions,
) -> Result<bool, String> {
    let flag = &args[*index];
    let target = match flag.as_str() {
        "--group" => Some(&mut selection.included_groups),
        "--only-group" => Some(&mut selection.only_groups),
        "--no-group" => Some(&mut selection.excluded_groups),
        _ => None,
    };
    if let Some(target) = target {
        let value = args
            .get(*index + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if value.starts_with('-') || value.is_empty() {
            return Err(format!("invalid value for {flag}"));
        }
        target.push(value.clone());
        *index += 2;
        return Ok(true);
    }
    Ok(false)
}

fn boolean_selection_flag(
    args: &[String],
    index: &mut usize,
    selection: &mut policy::SelectionOptions,
) -> Result<bool, String> {
    let flag = &args[*index];
    let boolean = match flag.as_str() {
        "--all-groups" => Some(&mut selection.all_groups),
        "--no-default-groups" => Some(&mut selection.no_default_groups),
        _ => None,
    };
    if let Some(target) = boolean {
        if *target {
            return Err(format!("repeated option: {flag}"));
        }
        *target = true;
        *index += 1;
        return Ok(true);
    }
    Ok(false)
}

fn show_command(args: &[String]) -> Result<Command, CommandError> {
    let mut index = 2;
    let mut policy_path = None;
    let mut format = None;
    let mut selection = policy::SelectionOptions::default();
    for _ in 0..args.len() {
        if index >= args.len() {
            break;
        }
        consume_show_option(
            args,
            &mut index,
            &mut policy_path,
            &mut format,
            &mut selection,
        )?;
    }
    Ok(Command::Show(ShowOptions {
        policy_path: policy_or_default(policy_path, Path::new("."))?,
        format,
        selection,
    }))
}

fn consume_show_option(
    args: &[String],
    index: &mut usize,
    policy_path: &mut Option<PathBuf>,
    format: &mut Option<String>,
    selection: &mut policy::SelectionOptions,
) -> Result<(), String> {
    if selection_flag(args, index, selection)? {
        return Ok(());
    }
    let flag = &args[*index];
    let value = args
        .get(*index + 1)
        .ok_or_else(|| format!("missing value for {flag}"))?;
    match flag.as_str() {
        "--policy" => set_once(policy_path, PathBuf::from(value), true, flag)?,
        "--format" => set_format(format, value, false, flag)?,
        _ => return Err(format!("unknown, repeated or invalid option: {flag}")),
    }
    *index += 2;
    Ok(())
}

/// Options that take the next argument as their value.
const VALUE_OPTIONS: [&str; 12] = [
    "--path",
    "--policy",
    "--base",
    "--evidence",
    "--coverage",
    "--format",
    "--report",
    "--log",
    "--group",
    "--only-group",
    "--no-group",
    "--rule-pack",
];

/// Whether `--help` or `-h` is given as an option, not as an option's value: in
/// `smells check --path -h`, `-h` is the path to scan.
fn wants_help(args: &[String]) -> bool {
    args.iter().enumerate().any(|(index, arg)| {
        matches!(arg.as_str(), "--help" | "-h")
            && !index
                .checked_sub(1)
                .is_some_and(|before| VALUE_OPTIONS.contains(&args[before].as_str()))
    })
}

fn command(args: &[String]) -> Result<Command, CommandError> {
    if args.is_empty() || wants_help(args) {
        return Ok(Command::Help);
    }
    if args == ["--version"] {
        return Ok(Command::Version);
    }
    if args[0] == "rules" {
        return rules_command(args).map_err(CommandError::from);
    }
    if args.starts_with(&["policy".into(), "show".into()]) {
        return show_command(args);
    }
    if args[0] == "doctor" {
        return doctor_command(args).map_err(CommandError::from);
    }
    let contracts = args.starts_with(&["contracts".into(), "validate".into()]);
    if !contracts && args[0] != "check" {
        return Err(CommandError::Invalid("unknown command; use --help".into()));
    }
    operation_command(args, contracts)
}

#[derive(Default)]
struct PendingDoctorOptions {
    source: Option<PathBuf>,
    policy_path: Option<PathBuf>,
    coverage_path: Option<PathBuf>,
    base_ref: Option<String>,
    format: Option<String>,
}

fn set_doctor_option(
    flag: &str,
    value: &str,
    pending: &mut PendingDoctorOptions,
) -> Result<(), String> {
    match flag {
        "--path" => set_once(&mut pending.source, PathBuf::from(value), true, flag),
        "--policy" => set_once(&mut pending.policy_path, PathBuf::from(value), true, flag),
        "--coverage" => set_once(&mut pending.coverage_path, PathBuf::from(value), true, flag),
        "--base" => set_base_ref(&mut pending.base_ref, value, false, flag),
        "--format" => set_format(&mut pending.format, value, false, flag),
        _ => Err(format!("unknown, repeated or invalid option: {flag}")),
    }
}

fn doctor_command(args: &[String]) -> Result<Command, String> {
    let mut pending = PendingDoctorOptions::default();
    for pair in args[1..].chunks(2) {
        let [flag, value] = pair else {
            return Err(format!("missing value for {}", pair[0]));
        };
        set_doctor_option(flag, value, &mut pending)?;
    }
    Ok(Command::Doctor(DoctorOptions {
        source: pending.source.unwrap_or_else(|| PathBuf::from(".")),
        policy_path: pending.policy_path,
        coverage_path: pending.coverage_path,
        base_ref: pending.base_ref,
        format: pending.format,
    }))
}

fn stdout_uses_color() -> bool {
    color_enabled(
        io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
        std::env::var("CLICOLOR_FORCE").ok().as_deref(),
    )
}

fn color_enabled(is_tty: bool, no_color: bool, force: Option<&str>) -> bool {
    if no_color {
        return false;
    }
    if force.is_some_and(|value| !value.is_empty() && value != "0") {
        return true;
    }
    is_tty
}

fn run_doctor(options: &DoctorOptions) -> Result<u8, String> {
    let coverage = Terminal::detect(options.format.as_deref()).coverage(
        options.coverage_path.as_deref(),
        &options.source,
        |root| {
            let language = doctor::policy_language(root, options.policy_path.as_deref());
            coverage_tool::plan(root, language.as_deref())
        },
    );
    let report = doctor::diagnose(
        &options.source,
        options.policy_path.as_deref(),
        coverage.as_deref(),
        options.base_ref.as_deref(),
    );
    if options.format.as_deref() == Some("json") {
        let json = serde_json::to_string_pretty(&doctor::to_json(&report))
            .map_err(|error| error.to_string())?;
        println!("{json}");
    } else {
        print!("{}", doctor::to_table(&report, stdout_uses_color()));
    }
    Ok(doctor::exit_code(&report.checks))
}

fn validate_contracts(policy_path: &PathBuf) -> Result<u8, String> {
    let text = fs::read_to_string(policy_path).map_err(|e| format!("cannot read policy: {e}"))?;
    let registry = policy::registry_for_policy(&text)?;
    let policy = policy::parse(&text, &registry)?;
    let implemented = registry
        .rules
        .iter()
        .filter(|rule| rule.implementation == "implemented")
        .count();
    println!(
        "{{\"status\":\"valid_contracts\",\"smells\":{},\"rules\":{},\"implemented_rules\":{implemented},\"active_rules\":{}}}",
        registry.smells.len(),
        registry.rules.len(),
        policy.resolved.active_rule_ids.len()
    );
    Ok(0)
}

fn resolved_policy_rules(
    registry: &policy::Registry,
    policy: &policy::Policy,
) -> Vec<serde_json::Value> {
    registry
        .rules
        .iter()
        .map(|rule| {
            let when_to_ignore = registry
                .guidance_catalog
                .iter()
                .find(|guidance| guidance.smell_id == rule.smell)
                .expect("validated guidance catalog must cover every rule smell");
            serde_json::json!({
                "rule_id": rule.id,
                "smell_id": rule.smell,
                "guidance_ref": when_to_ignore.guidance_ref,
                "when_to_ignore": when_to_ignore,
                "kind": rule.kind,
                "mode": policy.rules[&rule.id].mode,
                "selected": policy.selected(&rule.id),
                "active": policy.enabled(&rule.id),
                "groups": policy.resolved.rule_groups[&rule.id],
                "required_inputs": rule.inputs,
                "parameters": policy.rules[&rule.id].parameters,
            })
        })
        .collect()
}

fn print_policy_json(
    options: &ShowOptions,
    registry: &policy::Registry,
    policy: &policy::Policy,
    rules: &[serde_json::Value],
) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "status": "resolved_policy",
            "policy_path": options.policy_path,
            "policy_schema_version": policy.schema_version,
            "rule_pack": registry.rule_pack,
            "language": registry.language,
            "scanner_version": policy.scanner_version,
            "selection": policy.resolved,
            "guidance_catalog": registry.guidance_catalog,
            "rules": rules,
        }))
        .map_err(|error| error.to_string())?
    );
    Ok(())
}

fn print_policy_table(
    options: &ShowOptions,
    registry: &policy::Registry,
    policy: &policy::Policy,
    rules: &[serde_json::Value],
) {
    println!(
        "Resolved policy: {} ({} / {})",
        options.policy_path.display(),
        registry.rule_pack,
        registry.language
    );
    println!(
        "Active rules: {}/{} | defaults: {}",
        policy.resolved.active_rule_ids.len(),
        registry.rules.len(),
        policy.resolved.default_groups.join(",")
    );
    println!("Rule | Mode | Selected | Groups");
    for rule in rules {
        println!(
            "{} | {} | {} | {}",
            rule["rule_id"]
                .as_str()
                .expect("policy rows are built by resolved_policy_rules"),
            rule["mode"]
                .as_str()
                .expect("policy rows are built by resolved_policy_rules"),
            rule["selected"]
                .as_bool()
                .expect("policy rows are built by resolved_policy_rules"),
            rule["groups"]
                .as_array()
                .expect("policy rows are built by resolved_policy_rules")
                .iter()
                .map(|group| group
                    .as_str()
                    .expect("policy rows are built by resolved_policy_rules"))
                .collect::<Vec<_>>()
                .join(",")
        );
        println!(
            "  Guidance: {}",
            rule["guidance_ref"]
                .as_str()
                .expect("policy rows are built by resolved_policy_rules")
        );
        println!(
            "  Source: {} ({}, checked {})",
            rule["when_to_ignore"]["source_url"]
                .as_str()
                .expect("policy rows are built by resolved_policy_rules"),
            rule["when_to_ignore"]["provenance"]
                .as_str()
                .expect("policy rows are built by resolved_policy_rules"),
            rule["when_to_ignore"]["checked_on"]
                .as_str()
                .expect("policy rows are built by resolved_policy_rules")
        );
        println!("  When to ignore:");
        for (index, item) in rule["when_to_ignore"]["items"]
            .as_array()
            .expect("policy rows are built by resolved_policy_rules")
            .iter()
            .enumerate()
        {
            println!(
                "    {}. {}",
                index + 1,
                item.as_str()
                    .expect("policy rows are built by resolved_policy_rules")
            );
        }
    }
}

fn show_policy(options: &ShowOptions) -> Result<u8, String> {
    let text = fs::read_to_string(&options.policy_path)
        .map_err(|error| format!("cannot read policy: {error}"))?;
    let registry = policy::registry_for_policy(&text)?;
    let policy = policy::parse_with_selection(&text, &registry, &options.selection)?;
    let rules = resolved_policy_rules(&registry, &policy);
    if options.format.as_deref() == Some("json") {
        print_policy_json(options, &registry, &policy, &rules)?;
    } else {
        print_policy_table(options, &registry, &policy, &rules);
    }
    Ok(0)
}

fn emit_report(report: &report::Report, options: &CheckOptions) -> Result<usize, String> {
    let saved_json_bytes = options
        .report_path
        .as_deref()
        .map(|path| write_json_report(report, path))
        .transpose()?;
    if let Some(path) = options.log_path.as_deref() {
        write_finding_log(report, path)?;
    }
    let stdout_json_bytes = match options.log_path.as_deref() {
        Some(log_path) if options.format.as_deref() != Some("json") => {
            print_hook_summary(report, log_path, options.report_path.as_deref());
            0
        }
        _ => print_report(report, options.format.as_deref())?,
    };
    Ok(saved_json_bytes.unwrap_or(stdout_json_bytes))
}

fn check(options: &CheckOptions, scanner: &dyn Scanner) -> Result<u8, String> {
    metrics::reset();
    let target = match options.source.as_deref() {
        Some(source) => ScanTarget::WorkingTree(source),
        None => ScanTarget::Staged,
    };
    let coverage_path = check_coverage(options);
    let request = ScanRequest {
        target,
        policy_path: &options.policy_path,
        evidence_path: options.evidence_path.as_deref(),
        coverage_path: coverage_path.as_deref(),
        base_ref: options.base_ref.as_deref(),
        selection: &options.selection,
    };
    let report = scanner.check(&request)?;
    metrics::write(emit_report(&report, options)?)?;
    Ok(report.exit())
}

/// Uses `--coverage`, else finds or offers coverage for a plain working-tree scan. Staged
/// and `--base` scans skip it because the ratchet reference has no coverage to match.
fn check_coverage(options: &CheckOptions) -> Option<PathBuf> {
    if options.coverage_path.is_some() {
        return options.coverage_path.clone();
    }
    let source = options.source.as_deref()?;
    if options.base_ref.is_some() {
        return None;
    }
    if let Some(found) = coverage::discover(source) {
        eprintln!(
            "smells: reading coverage from {} (found automatically; pass --coverage FILE to choose another)",
            found.display()
        );
        return Some(found);
    }
    Terminal::detect(options.format.as_deref()).offer(source, |root| {
        let language = doctor::policy_language(root, Some(&options.policy_path));
        coverage_tool::plan(root, language.as_deref())
    })
}

fn print_hook_summary(
    report: &report::Report,
    log_path: &std::path::Path,
    report_path: Option<&std::path::Path>,
) {
    println!(
        "Scan summary | verdict: {} | findings: {} | blocking: {} | ignored: {} | review: {} | errors: {}{}",
        report.summary.verdict,
        report.summary.matched_findings,
        report.summary.blocking_findings,
        report.summary.ignored_findings,
        report.summary.review_signals,
        report.summary.error_count,
        report::ratchet_summary_note(report),
    );
    let issues = report::top_issues(report, SUMMARY_ISSUES);
    for issue in &issues {
        println!("  {issue}");
    }
    let hidden = (report.summary.blocking_findings + report.summary.error_count)
        .saturating_sub(issues.len());
    if hidden > 0 {
        println!("  +{hidden} more in the Finding Log");
    }
    println!("Finding Log: {}", log_path.display());
    println!("Start at the Issue Index, then read every referenced detail.");
    if let Some(path) = report_path {
        println!("JSON Evidence Report: {}", path.display());
    }
}

struct CountingWriter<W> {
    inner: W,
    bytes: usize,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.bytes += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn print_report(report: &report::Report, format: Option<&str>) -> Result<usize, String> {
    if format == Some("json") {
        let stdout = io::stdout();
        let mut output = CountingWriter {
            inner: stdout.lock(),
            bytes: 0,
        };
        serde_json::to_writer(&mut output, report).map_err(|error| error.to_string())?;
        writeln!(output).map_err(|error| error.to_string())?;
        Ok(output.bytes)
    } else {
        report.print_table();
        Ok(0)
    }
}

fn write_json_report(report: &report::Report, path: &std::path::Path) -> Result<usize, String> {
    let file = fs::File::create(path)
        .map_err(|error| format!("cannot create report {}: {error}", path.display()))?;
    let mut output = CountingWriter {
        inner: io::BufWriter::new(file),
        bytes: 0,
    };
    serde_json::to_writer(&mut output, report)
        .map_err(|error| format!("cannot write report {}: {error}", path.display()))?;
    writeln!(output)
        .map_err(|error| format!("cannot finish report {}: {error}", path.display()))?;
    output
        .flush()
        .map_err(|error| format!("cannot flush report {}: {error}", path.display()))?;
    Ok(output.bytes)
}

fn write_finding_log(report: &report::Report, path: &std::path::Path) -> Result<(), String> {
    let file = fs::File::create(path)
        .map_err(|error| format!("cannot create finding log {}: {error}", path.display()))?;
    let mut output = io::BufWriter::new(file);
    report::write_finding_log(report, &mut output)
        .and_then(|()| output.flush())
        .map_err(|error| format!("cannot write finding log {}: {error}", path.display()))
}

pub(crate) fn run(args: &[String], scanner: &dyn Scanner) -> Result<u8, String> {
    match command(args).map_err(CommandError::into_message)? {
        Command::Help => {
            println!("{USAGE}");
            Ok(0)
        }
        Command::Version => {
            println!("smells {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Command::Rules(rule_pack) => {
            let _registry = policy::registry(&rule_pack)?;
            println!("{}", policy::registry_json(&rule_pack)?.trim());
            Ok(0)
        }
        Command::Validate(policy_path) => validate_contracts(&policy_path),
        Command::Show(options) => show_policy(&options),
        Command::Check(options) => check(&options, scanner),
        Command::Doctor(options) => run_doctor(&options),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    struct FlushFails;

    impl Write for FlushFails {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("flush failed"))
        }
    }

    #[test]
    fn color_policy_respects_no_color_force_and_tty() {
        assert!(!color_enabled(true, true, Some("1")));
        assert!(color_enabled(false, false, Some("1")));
        assert!(!color_enabled(false, false, Some("0")));
        assert!(!color_enabled(false, false, Some("")));
        assert!(color_enabled(true, false, None));
        assert!(!color_enabled(false, false, None));
    }

    #[test]
    fn counting_writer_counts_delegated_writes_and_propagates_flush_failures() {
        let mut writer = CountingWriter {
            inner: FlushFails,
            bytes: 0,
        };

        writer.write_all(b"report").unwrap();

        assert_eq!(writer.bytes, 6);
        assert_eq!(writer.flush().unwrap_err().to_string(), "flush failed");
    }

    #[test]
    fn staged_is_rejected_in_contracts_mode_and_when_repeated() {
        let Err(contracts) = command(&arguments(&[
            "contracts",
            "validate",
            "--staged",
            "--policy",
            "quality-policy.json",
        ])) else {
            panic!("contracts mode accepted --staged");
        };
        assert_eq!(
            contracts.to_string(),
            "unknown, repeated or invalid option: --staged"
        );

        let Err(repeated) = command(&arguments(&[
            "check",
            "--staged",
            "--staged",
            "--policy",
            "quality-policy.json",
        ])) else {
            panic!("check mode accepted repeated --staged");
        };
        assert_eq!(
            repeated.to_string(),
            "unknown, repeated or invalid option: --staged"
        );
    }

    #[test]
    fn base_ref_is_validated_and_mutually_exclusive_with_evidence() {
        let Ok(Command::Check(parsed)) = command(&arguments(&[
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
        ])) else {
            panic!("check mode rejected valid --base");
        };
        assert_eq!(parsed.base_ref.as_deref(), Some("HEAD"));

        let Err(conflict) = command(&arguments(&[
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--evidence",
            "provider.json",
            "--policy",
            "quality-policy.json",
        ])) else {
            panic!("check mode accepted --base with --evidence");
        };
        assert_eq!(
            conflict.to_string(),
            "--evidence cannot be used with --base"
        );

        for invalid in [
            vec![
                "contracts",
                "validate",
                "--base",
                "HEAD",
                "--policy",
                "quality-policy.json",
            ],
            vec![
                "check",
                "--staged",
                "--base",
                "-bad",
                "--policy",
                "quality-policy.json",
            ],
        ] {
            assert!(command(&arguments(&invalid)).is_err());
        }
    }

    #[test]
    fn check_defaults_to_the_current_directory_and_a_finding_log() {
        let Ok(Command::Check(parsed)) =
            command(&arguments(&["check", "--policy", "quality-policy.json"]))
        else {
            panic!("bare check was rejected");
        };
        assert_eq!(parsed.source, Some(PathBuf::from(".")));
        assert_eq!(parsed.log_path, Some(PathBuf::from(DEFAULT_LOG)));

        for (format, log) in [("table", None), ("json", None)] {
            let Ok(Command::Check(parsed)) = command(&arguments(&[
                "check",
                "--policy",
                "quality-policy.json",
                "--format",
                format,
            ])) else {
                panic!("check rejected --format {format}");
            };
            assert_eq!(parsed.log_path, log);
        }

        let Ok(Command::Check(staged)) = command(&arguments(&[
            "check",
            "--staged",
            "--policy",
            "quality-policy.json",
            "--log",
            "custom.log",
        ])) else {
            panic!("staged check was rejected");
        };
        assert_eq!(staged.source, None);
        assert_eq!(staged.log_path, Some(PathBuf::from("custom.log")));

        let Err(both) = command(&arguments(&[
            "check",
            "--staged",
            "--path",
            ".",
            "--policy",
            "quality-policy.json",
        ])) else {
            panic!("check accepted --staged with --path");
        };
        assert_eq!(both.to_string(), "--staged and --path cannot be combined");
    }

    #[test]
    fn help_is_printed_for_any_command() {
        for args in [
            vec!["--help"],
            vec!["-h"],
            vec!["check", "--help"],
            vec!["doctor", "-h"],
            vec!["policy", "show", "--help"],
            vec!["check", "--path", "src", "-h"],
        ] {
            assert!(
                matches!(command(&arguments(&args)), Ok(Command::Help)),
                "{args:?}"
            );
        }
    }

    #[test]
    fn help_given_as_an_option_value_is_not_help() {
        for args in [
            vec!["check", "--path", "-h", "--policy", "quality-policy.json"],
            vec![
                "check",
                "--base",
                "--help",
                "--policy",
                "quality-policy.json",
            ],
            vec!["doctor", "--policy", "-h"],
            vec!["rules", "--rule-pack", "--help"],
        ] {
            assert!(
                !matches!(command(&arguments(&args)), Ok(Command::Help)),
                "{args:?}"
            );
        }
    }
}
