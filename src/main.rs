mod input;
mod patterns;
mod policy;
mod portable;
mod report;
mod scan;

use std::{env, fs, path::PathBuf, process::ExitCode};

const USAGE: &str = "smells check (--path DIR | --staged) --policy FILE [--format table|json]\nsmells contracts validate --policy FILE\nsmells rules [--rule-pack rust-v1|python-v1|typescript-v1]";

enum Command {
    Help,
    Rules(String),
    Validate(PathBuf),
    Check(CheckOptions),
}

struct CheckOptions {
    source: Option<PathBuf>,
    policy_path: PathBuf,
    format: Option<String>,
    staged: bool,
}

#[derive(Default)]
struct PendingOptions {
    source: Option<PathBuf>,
    policy_path: Option<PathBuf>,
    format: Option<String>,
    staged: bool,
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
    source: &mut Option<PathBuf>,
    policy_path: &mut Option<PathBuf>,
    format: &mut Option<String>,
) -> Result<(), String> {
    match flag {
        "--policy" if policy_path.is_none() => *policy_path = Some(PathBuf::from(value)),
        "--path" if source.is_none() && !contracts => *source = Some(PathBuf::from(value)),
        "--format" if format.is_none() && !contracts && matches!(value, "table" | "json") => {
            *format = Some(value.to_string());
        }
        _ => return Err(format!("unknown, repeated or invalid option: {flag}")),
    }
    Ok(())
}

fn operation_command(args: &[String], contracts: bool) -> Result<Command, String> {
    let mut index = if contracts { 2 } else { 1 };
    let mut options = PendingOptions::default();
    while index < args.len() {
        let flag = &args[index];
        if flag == "--staged" && !contracts && !options.staged {
            options.staged = true;
            index += 1;
            continue;
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        set_option(
            flag,
            value,
            contracts,
            &mut options.source,
            &mut options.policy_path,
            &mut options.format,
        )?;
        index += 2;
    }
    finish_operation(options, contracts)
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
        format: options.format,
        staged: options.staged,
    }))
}

fn command(args: &[String]) -> Result<Command, String> {
    if args.is_empty() || args == ["--help"] {
        return Ok(Command::Help);
    }
    if args[0] == "rules" {
        return rules_command(args);
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
    let _policy = policy::parse(&text, &registry)?;
    let implemented = registry
        .rules
        .iter()
        .filter(|rule| rule.implementation == "implemented")
        .count();
    println!(
        "{{\"status\":\"valid_contracts\",\"smells\":{},\"rules\":{},\"implemented_source_rules\":{implemented}}}",
        registry.smells.len(),
        registry.rules.len()
    );
    Ok(0)
}

fn capture(options: &CheckOptions) -> Result<input::CapturedInput, String> {
    let captured = match &options.source {
        Some(source) => input::working_tree(source, &options.policy_path)?,
        None if options.staged => input::staged(&options.policy_path)?,
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

fn check(options: CheckOptions) -> Result<u8, String> {
    let captured = capture(&options)?;
    let mut report = scan(&captured)?;
    report.finish();
    print_report(&report, options.format.as_deref())?;
    Ok(report.exit())
}

fn print_report(report: &report::Report, format: Option<&str>) -> Result<(), String> {
    if format == Some("json") {
        println!(
            "{}",
            serde_json::to_string_pretty(report).map_err(|error| error.to_string())?
        );
    } else {
        report.print_table();
    }
    Ok(())
}

fn run() -> Result<u8, String> {
    let args: Vec<_> = env::args().skip(1).collect();
    match command(&args)? {
        Command::Help => {
            println!("{USAGE}");
            Ok(0)
        }
        Command::Rules(rule_pack) => {
            let _registry = policy::registry(&rule_pack)?;
            println!("{}", policy::registry_json(&rule_pack)?.trim());
            Ok(0)
        }
        Command::Validate(policy_path) => validate_contracts(&policy_path),
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
