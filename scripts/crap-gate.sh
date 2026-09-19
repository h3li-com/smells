#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

mkdir -p target/quality
required_target=5
hard_limit=10
coverage_report=target/quality/lcov.info
crap_report=target/quality/crap-report.json

# rustup installations are discovered by cargo-llvm-cov. Homebrew Rust does not
# ship llvm-tools-preview, so use Homebrew's matching LLVM binaries when present.
if [ -z "${LLVM_COV:-}" ] && [ -x /opt/homebrew/opt/llvm/bin/llvm-cov ]; then
    LLVM_COV=/opt/homebrew/opt/llvm/bin/llvm-cov
    export LLVM_COV
fi
if [ -z "${LLVM_PROFDATA:-}" ] && [ -x /opt/homebrew/opt/llvm/bin/llvm-profdata ]; then
    LLVM_PROFDATA=/opt/homebrew/opt/llvm/bin/llvm-profdata
    export LLVM_PROFDATA
fi

cargo llvm-cov --locked --lcov --output-path "$coverage_report"
cargo crap \
    --lcov "$coverage_report" \
    --threshold "$required_target" \
    --format json \
    --sort file \
    --output "$crap_report"

set +e
cargo crap \
    --lcov "$coverage_report" \
    --threshold "$hard_limit" \
    --fail-above \
    --summary
status=$?
set -e

if [ "$status" -ne 0 ]; then
    printf '%s\n' \
        '{' \
        '  "gate": "crap",' \
        '  "result": "blocked",' \
        '  "smell": "Long Method",' \
        '  "pattern_type": "metric",' \
        '  "required_target": 5,' \
        '  "hard_limit": 10,' \
        '  "why": "High decision complexity is risky to change, and insufficient coverage amplifies that risk; CRAP combines both signals.",' \
        '  "remediation": "Reduce decision complexity with behavior-preserving extraction and add focused tests for uncovered branches.",' \
        '  "reference_url": "https://refactoring.guru/smells/long-method",' \
        '  "research_requirement": "NON-NEGOTIABLE: open and read reference_url with a research/tool call before proposing or applying remediation.",' \
        '  "evidence_report": "target/quality/crap-report.json"' \
        '}' >&2
fi

# Emit one agent-readable annotation for every function above the required
# target, regardless of whether the absolute hard limit blocks the commit.
cargo crap \
    --lcov "$coverage_report" \
    --threshold "$required_target" \
    --format github >&2

if [ "$status" -ne 0 ]; then
    exit "$status"
fi

printf 'CRAP gate passed: required target=%s, hard limit=%s, report=%s\n' \
    "$required_target" "$hard_limit" "$crap_report"
