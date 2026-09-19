#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

mkdir -p target/quality

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

cargo llvm-cov --locked --lcov --output-path target/quality/lcov.info
cargo crap \
    --lcov target/quality/lcov.info \
    --threshold 30 \
    --baseline .cargo-crap-baseline.json \
    --fail-regression \
    --summary
