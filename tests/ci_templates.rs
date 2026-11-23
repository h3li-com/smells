//! Drift checks for the copyable CI templates in `examples/ci/`.

const GITHUB: &str = include_str!("../examples/ci/github-actions-smells.yml");
const AZURE: &str = include_str!("../examples/ci/azure-pipelines-smells.yml");
const CODEOWNERS: &str = include_str!("../examples/ci/CODEOWNERS");

fn pinned_install() -> String {
    format!("pip install smells=={}", env!("CARGO_PKG_VERSION"))
}

fn assert_fail_closed_gate(template: &str, name: &str) {
    assert!(
        template.contains(&pinned_install()),
        "{name} must pin the crate version"
    );
    assert_eq!(
        template.matches("smells==").count(),
        1,
        "{name} must pin exactly one Smells install"
    );
    for flag in [
        "--policy quality-policy.json",
        "--base \"",
        "--log smells-findings.log",
        "--report smells-report.json",
    ] {
        assert!(template.contains(flag), "{name} must pass {flag}");
    }
    assert!(
        template.contains("exit 2;"),
        "{name} must fail closed without a ratchet base"
    );
}

#[test]
fn github_actions_template_is_an_independent_full_history_ratchet_gate() {
    assert_fail_closed_gate(GITHUB, "GitHub Actions");
    for required in [
        "fetch-depth: 0",
        "persist-credentials: false",
        "permissions:\n  contents: read",
        "if: always()",
        "pull_request:",
        "merge_group:",
    ] {
        assert!(GITHUB.contains(required), "missing {required:?}");
    }
    for line in GITHUB.lines().map(str::trim) {
        assert!(
            !line.starts_with("needs:"),
            "the gate job must not depend on others"
        );
        assert!(!line.starts_with("continue-on-error"), "{line}");
        if let Some(action) = line
            .strip_prefix("- uses: ")
            .or(line.strip_prefix("uses: "))
        {
            let sha = action.split(['@', ' ']).nth(1).unwrap_or_default();
            assert!(
                sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "action must be pinned to a commit SHA: {line}"
            );
        }
    }
}

#[test]
fn azure_template_is_an_independent_full_history_ratchet_gate() {
    assert_fail_closed_gate(AZURE, "Azure Pipelines");
    for required in [
        "fetchDepth: 0",
        "persistCredentials: false",
        "condition: always()",
        "SYSTEM_PULLREQUEST_TARGETBRANCH",
    ] {
        assert!(AZURE.contains(required), "missing {required:?}");
    }
    for line in AZURE.lines().map(str::trim) {
        assert!(
            !line.starts_with("dependsOn"),
            "the gate job must not depend on others"
        );
        assert!(!line.starts_with("continueOnError"), "{line}");
    }
}

#[test]
fn codeowners_example_guards_every_gate_input() {
    for pattern in [
        "*quality-policy.json",
        "*smells-policy.json",
        "*smells-baseline.json",
        "AGENTS.md",
        "CLAUDE.md",
        "/.github/workflows/smells.yml",
        "/azure-pipelines-smells.yml",
    ] {
        assert!(
            CODEOWNERS
                .lines()
                .any(|line| line.split_whitespace().next() == Some(pattern)),
            "CODEOWNERS must own {pattern}"
        );
    }
}
