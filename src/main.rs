mod collectors;
mod evidence;
mod evidence_evaluators;
mod input;
mod metrics;
mod patterns;
mod policy;
mod portable;
mod report;
mod rule_runtime;
mod scan;
mod similarity;
mod suppressions;
mod typescript_compat;

use std::{
    env, fs,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

const USAGE: &str = "smells --version\nsmells check (--path DIR | --staged) --policy FILE [--evidence FILE] [--format table|json] [--report FILE] [--log FILE] [policy group selectors]\nsmells policy show --policy FILE [--format table|json] [policy group selectors]\nsmells contracts validate --policy FILE\nsmells rules [--rule-pack rust-v1|python-v1|typescript-v1]\n\npolicy group selectors: --group NAME | --only-group NAME | --all-groups | --no-default-groups | --no-group NAME";

enum Command {
    Help,
    Version,
    Rules(String),
    Validate(PathBuf),
    Show(ShowOptions),
    Check(CheckOptions),
}

struct CheckOptions {
    source: Option<PathBuf>,
    policy_path: PathBuf,
    evidence_path: Option<PathBuf>,
    report_path: Option<PathBuf>,
    log_path: Option<PathBuf>,
    format: Option<String>,
    staged: bool,
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
    match flag {
        "--policy" => set_once(&mut options.policy_path, PathBuf::from(value), true, flag),
        "--evidence" => set_once(
            &mut options.evidence_path,
            PathBuf::from(value),
            !contracts,
            flag,
        ),
        "--report" => set_once(
            &mut options.report_path,
            PathBuf::from(value),
            !contracts,
            flag,
        ),
        "--log" => set_once(
            &mut options.log_path,
            PathBuf::from(value),
            !contracts,
            flag,
        ),
        "--path" => set_once(&mut options.source, PathBuf::from(value), !contracts, flag),
        "--format" => set_format(&mut options.format, value, contracts, flag),
        _ => Err(format!("unknown, repeated or invalid option: {flag}")),
    }
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

fn operation_command(args: &[String], contracts: bool) -> Result<Command, String> {
    let mut index = if contracts { 2 } else { 1 };
    let mut options = PendingOptions::default();
    while index < args.len() {
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

fn finish_operation(options: PendingOptions, contracts: bool) -> Result<Command, String> {
    let policy_path = options.policy_path.ok_or("--policy is required")?;
    if contracts {
        return Ok(Command::Validate(policy_path));
    }
    if options.staged == options.source.is_some() {
        return Err("select exactly one of --staged and --path".into());
    }
    Ok(Command::Check(CheckOptions {
        source: options.source,
        policy_path,
        evidence_path: options.evidence_path,
        report_path: options.report_path,
        log_path: options.log_path,
        format: options.format,
        staged: options.staged,
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

fn show_command(args: &[String]) -> Result<Command, String> {
    let mut index = 2;
    let mut policy_path = None;
    let mut format = None;
    let mut selection = policy::SelectionOptions::default();
    while index < args.len() {
        consume_show_option(
            args,
            &mut index,
            &mut policy_path,
            &mut format,
            &mut selection,
        )?;
    }
    Ok(Command::Show(ShowOptions {
        policy_path: policy_path.ok_or("--policy is required")?,
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

fn command(args: &[String]) -> Result<Command, String> {
    if args.is_empty() || args == ["--help"] {
        return Ok(Command::Help);
    }
    if args == ["--version"] {
        return Ok(Command::Version);
    }
    if args[0] == "rules" {
        return rules_command(args);
    }
    if args.starts_with(&["policy".into(), "show".into()]) {
        return show_command(args);
    }
    let contracts = args.starts_with(&["contracts".into(), "validate".into()]);
    if !contracts && args[0] != "check" {
        return Err("unknown command; use --help".into());
    }
    operation_command(args, contracts)
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
            rule["rule_id"].as_str().unwrap(),
            rule["mode"].as_str().unwrap(),
            rule["selected"].as_bool().unwrap(),
            rule["groups"]
                .as_array()
                .unwrap()
                .iter()
                .map(|group| group.as_str().unwrap())
                .collect::<Vec<_>>()
                .join(",")
        );
        println!("  Guidance: {}", rule["guidance_ref"].as_str().unwrap());
        println!(
            "  Source: {} ({}, checked {})",
            rule["when_to_ignore"]["source_url"].as_str().unwrap(),
            rule["when_to_ignore"]["provenance"].as_str().unwrap(),
            rule["when_to_ignore"]["checked_on"].as_str().unwrap()
        );
        println!("  When to ignore:");
        for (index, item) in rule["when_to_ignore"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            println!("    {}. {}", index + 1, item.as_str().unwrap());
        }
    }
}

fn show_policy(options: ShowOptions) -> Result<u8, String> {
    let text = fs::read_to_string(&options.policy_path)
        .map_err(|error| format!("cannot read policy: {error}"))?;
    let registry = policy::registry_for_policy(&text)?;
    let policy = policy::parse_with_selection(&text, &registry, &options.selection)?;
    let rules = resolved_policy_rules(&registry, &policy);
    if options.format.as_deref() == Some("json") {
        print_policy_json(&options, &registry, &policy, &rules)?;
    } else {
        print_policy_table(&options, &registry, &policy, &rules);
    }
    Ok(0)
}

fn capture(options: &CheckOptions) -> Result<(input::CapturedInput, Option<Vec<u8>>), String> {
    let captured = match &options.source {
        Some(source) => {
            let captured = input::working_tree(source, &options.policy_path, &options.selection)?;
            let evidence = options
                .evidence_path
                .as_ref()
                .map(|path| {
                    fs::read(path)
                        .map_err(|error| format!("cannot read provider evidence: {error}"))
                })
                .transpose()?;
            (captured, evidence)
        }
        None if options.staged => input::staged(
            &options.policy_path,
            options.evidence_path.as_deref(),
            &options.selection,
        )?,
        None => return Err("select exactly one of --staged and --path".into()),
    };
    Ok(captured)
}

fn scan(captured: &input::CapturedInput) -> Result<report::Report, String> {
    Ok(match captured.registry.language.as_str() {
        "rust" => scan::check(&captured.input, &captured.registry),
        "python" | "typescript" => portable::check(&captured.input, &captured.registry),
        language => return Err(format!("unsupported registry language: {language}")),
    })
}

fn load_evidence(
    bytes: Option<&[u8]>,
    captured: &input::CapturedInput,
) -> Result<(Option<evidence::EvidenceBundle>, String), String> {
    let Some(bytes) = bytes else {
        return Ok((None, String::new()));
    };
    let bundle = evidence::parse(bytes)?;
    evidence::validate(&bundle, &captured.input, &captured.registry)?;
    Ok((Some(bundle), evidence::digest(bytes)))
}

fn build_report(
    captured: &input::CapturedInput,
    evidence_bytes: Option<&[u8]>,
) -> Result<report::Report, String> {
    let (evidence, evidence_sha256) = load_evidence(evidence_bytes, captured)?;
    let mut report = scan(captured)?;
    report.set_provider_evidence_digest(evidence_sha256);
    evidence::apply(
        evidence.as_ref(),
        &captured.input,
        &captured.registry,
        &mut report,
    );
    report.apply_suppressions(suppressions::parse(
        &captured.input.files,
        &captured.registry,
    ));
    report.attach_sources(&captured.input.files);
    report.finish();
    Ok(report)
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
    let stdout_json_bytes =
        if options.format.as_deref() != Some("json") && options.log_path.is_some() {
            print_hook_summary(
                report,
                options.log_path.as_deref().unwrap(),
                options.report_path.as_deref(),
            );
            0
        } else {
            print_report(report, options.format.as_deref())?
        };
    let json_bytes = saved_json_bytes.unwrap_or(stdout_json_bytes);
    metrics::write(json_bytes)?;
    Ok(saved_json_bytes.unwrap_or(stdout_json_bytes))
}

fn check(options: CheckOptions) -> Result<u8, String> {
    metrics::reset();
    let (captured, evidence_bytes) = capture(&options)?;
    metrics::add(metrics::Counter::Files, captured.input.files.len());
    let report = build_report(&captured, evidence_bytes.as_deref())?;
    metrics::write(emit_report(&report, &options)?)?;
    Ok(report.exit())
}

fn print_hook_summary(
    report: &report::Report,
    log_path: &std::path::Path,
    report_path: Option<&std::path::Path>,
) {
    println!(
        "Scan summary | verdict: {} | findings: {} | blocking: {} | ignored: {} | review: {} | errors: {}",
        report.summary.verdict,
        report.summary.matched_findings,
        report.summary.blocking_findings,
        report.summary.ignored_findings,
        report.summary.review_signals,
        report.summary.error_count,
    );
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

fn run() -> Result<u8, String> {
    let args: Vec<_> = env::args().skip(1).collect();
    match command(&args)? {
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
        Command::Show(options) => show_policy(options),
        Command::Check(options) => check(options),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("smells: {error}");
            ExitCode::from(2)
        }
    }
}
