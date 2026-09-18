# smells

A standalone Rust code-smell scanner specification, intended for reuse through local Git pre-commit hooks and project quality runners.

**Status: specification only.** This repository contains the extracted 23-smell mapping, proposed detector rules, a generic example policy, and a hook example. There is no Cargo package, scanner executable, or working hook installation yet.

## What this tool will do

Recognize fixed Rust code patterns and report smell, rule ID, qualified symbol, source location, metric, measured value, comparison, threshold, status, and whether a finding blocks a commit. Show function/type values below limits as well as violations, in terminal or JSON reports. No LLM determines a verdict; unavailable required measurements are errors.

The [detector specification](docs/rust-smell-checks.md) inventories all 23 Refactoring.Guru smells, including Large Class measurements across all implementations of a Rust type. It also records proposed structural patterns and the fixtures needed to validate them. Thresholds are project choices, not values prescribed by Refactoring.Guru.

## Reuse across projects

Install the future `smells` executable separately. Each Rust project keeps its own checked-in `quality-policy.json`, architecture rules, domain roles, and exact exceptions. Use [examples/quality-policy.json](examples/quality-policy.json) as a starting point; it contains generic thresholds and no application package names or dependency map.

The planned interface is:

```text
smells check --staged --policy quality-policy.json
smells check --staged --policy quality-policy.json --format json
```

These commands are an implementation contract, not runnable commands today. Source and policy come from the staged snapshot, so unstaged changes cannot supply a passing result. See [project integration](docs/project-integration.md) for configuration and exit behavior.

A consuming project's quality runner will invoke this command alongside OSV, Gitleaks, formatting, tests, coverage, and other checks. The [hook example](hooks/pre-commit.example) is inactive and must not be installed before the CLI and its fixtures work. Projects with existing hooks should add the scanner call to their runner.

## Independence

`smells` is its own Git repository and future Rust executable, using `src/main.rs`. It is not part of a consuming Cargo workspace and does not import application crates. Consuming applications do not import scanner code into their production dependencies. Architecture rules describe project-supplied package identities; no consuming project is hard-coded into the scanner.

Extracted from the `tiny` design session on 2026-09-18. The generic scanner specification is maintained here; application-specific quality policy remains in each consuming repository. No Git remote has been configured.
