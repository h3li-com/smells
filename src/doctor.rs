//! `smells doctor`: a deterministic integration preflight.
//!
//! Each check is a pure function of captured input so the verdict never
//! depends on network access or the consuming project's build.

use std::{collections::BTreeMap, fs, path::Path};

use serde_json::{Value, json};

use crate::{
    core::model::History,
    input,
    policy::SelectionOptions,
    suppressions::{self, ParsedSuppressions},
};

/// GitHub resolves the first CODEOWNERS file in this order.
const CODEOWNERS_PATHS: [&str; 3] = [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"];
const HISTORY_WINDOW: usize = 200;

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

fn check(id: &'static str, status: Status, detail: impl Into<String>) -> Check {
    Check {
        id,
        status,
        detail: detail.into(),
    }
}

/// Runs every check against the source root and policy.
pub(crate) fn diagnose(root: &Path, policy_path: &Path) -> Vec<Check> {
    let mut checks = Vec::new();
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
    checks
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

pub(crate) fn to_json(checks: &[Check]) -> Value {
    json!({
        "status": if exit_code(checks) == 0 { "healthy" } else { "unhealthy" },
        "exit_code": exit_code(checks),
        "checks": checks
            .iter()
            .map(|check| json!({"id": check.id, "status": check.status.label(), "detail": check.detail}))
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn to_table(checks: &[Check]) -> String {
    let mut table = String::from("Check | Status | Detail\n");
    for check in checks {
        table.push_str(&format!(
            "{} | {} | {}\n",
            check.id,
            check.status.label(),
            check.detail
        ));
    }
    table
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
        let warned = [
            check("a", Status::Ok, "fine"),
            check("b", Status::Warn, "gap"),
        ];
        assert_eq!(exit_code(&warned), 0);
        assert_eq!(
            to_json(&warned),
            json!({
                "status": "healthy",
                "exit_code": 0,
                "checks": [
                    {"id": "a", "status": "ok", "detail": "fine"},
                    {"id": "b", "status": "warn", "detail": "gap"},
                ],
            })
        );
        assert_eq!(
            to_table(&warned),
            "Check | Status | Detail\na | ok | fine\nb | warn | gap\n"
        );
        let failed = [check("c", Status::Error, "broken")];
        assert_eq!(exit_code(&failed), 2);
        assert_eq!(to_json(&failed)["status"], "unhealthy");
        assert_eq!(to_json(&failed)["checks"][0]["status"], "error");
    }
}
