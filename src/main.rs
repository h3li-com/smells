mod input;
mod patterns;
mod policy;
mod portable;
mod report;
mod scan;

use std::{env, fs, path::PathBuf, process::ExitCode};

fn run() -> Result<u8, String> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.is_empty() || args == ["--help"] {
        println!(
            "smells check (--path DIR | --staged) --policy FILE [--format table|json]\nsmells contracts validate --policy FILE\nsmells rules [--rule-pack rust-v1|python-v1|typescript-v1]"
        );
        return Ok(0);
    }
    if args.first().is_some_and(|argument| argument == "rules") {
        let rule_pack = match args.as_slice() {
            [_] => "rust-v1",
            [_, flag, value] if flag == "--rule-pack" => value,
            _ => return Err("usage: smells rules [--rule-pack PACK]".into()),
        };
        let _registry = policy::registry(rule_pack)?;
        println!("{}", policy::registry_json(rule_pack)?.trim());
        return Ok(0);
    }
    let contracts = args.starts_with(&["contracts".into(), "validate".into()]);
    if !contracts && args[0] != "check" {
        return Err("unknown command; use --help".into());
    }
    let mut index = if contracts { 2 } else { 1 };
    let mut source = None;
    let mut policy_path = None;
    let mut format = None;
    let mut staged = false;
    while index < args.len() {
        let flag = &args[index];
        if flag == "--staged" && !contracts && !staged {
            staged = true;
            index += 1;
            continue;
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--policy" if policy_path.is_none() => policy_path = Some(PathBuf::from(value)),
            "--path" if source.is_none() && !contracts => source = Some(PathBuf::from(value)),
            "--format"
                if format.is_none() && !contracts && matches!(value.as_str(), "table" | "json") =>
            {
                format = Some(value.clone())
            }
            _ => return Err(format!("unknown, repeated or invalid option: {flag}")),
        }
        index += 2;
    }
    let policy_path = policy_path.ok_or("--policy is required")?;
    if contracts {
        let text =
            fs::read_to_string(policy_path).map_err(|e| format!("cannot read policy: {e}"))?;
        let registry = policy::registry_for_policy(&text)?;
        let _policy = policy::parse(&text, &registry)?;
        println!(
            "{{\"status\":\"valid_contracts\",\"smells\":{},\"rules\":{},\"implemented_source_rules\":{}}}",
            registry.smells.len(),
            registry.rules.len(),
            registry
                .rules
                .iter()
                .filter(|r| r.implementation == "implemented")
                .count()
        );
        return Ok(0);
    }
    if staged == source.is_some() {
        return Err("select exactly one of --staged and --path".into());
    }
    let captured = if staged {
        input::staged(&policy_path)?
    } else {
        input::working_tree(&source.unwrap(), &policy_path)?
    };
    let mut report = match captured.registry.language.as_str() {
        "rust" => scan::check(&captured.input, &captured.registry),
        "python" | "typescript" => portable::check(&captured.input, &captured.registry),
        language => return Err(format!("unsupported registry language: {language}")),
    };
    report.finish();
    if format.as_deref() == Some("json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        report.print_table();
    }
    Ok(report.exit())
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
