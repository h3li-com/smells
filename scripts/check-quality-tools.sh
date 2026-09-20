#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=quality-tool-versions.sh
. "$script_directory/quality-tool-versions.sh"

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'quality gate: required command is unavailable: %s\n' "$1" >&2
        exit 2
    fi
}

require_version() {
    label=$1
    expected=$2
    shift 2
    if ! actual=$("$@" 2>&1); then
        printf 'quality gate: could not execute %s\n' "$label" >&2
        exit 2
    fi
    if [ "$actual" != "$expected" ]; then
        printf 'quality gate: %s must be %s; found %s\n' "$label" "$expected" "$actual" >&2
        exit 2
    fi
}

require_version_first_line() {
    label=$1
    expected=$2
    shift 2
    if ! output=$("$@" 2>&1); then
        printf 'quality gate: could not execute %s\n' "$label" >&2
        exit 2
    fi
    actual=$(printf '%s\n' "$output" | sed -n '1p')
    if [ "$actual" != "$expected" ]; then
        printf 'quality gate: %s must start with %s; found %s\n' "$label" "$expected" "$actual" >&2
        exit 2
    fi
}

require_command cargo
require_command gitleaks
require_command node
require_command npm
require_command osv-scanner
require_command python3

require_version 'cargo crap --version' "cargo-crap $CARGO_CRAP_VERSION" cargo crap --version
require_version 'cargo llvm-cov --version' \
    "cargo-llvm-cov $CARGO_LLVM_COV_VERSION" cargo llvm-cov --version
require_version 'gitleaks version' "$GITLEAKS_VERSION" gitleaks version
require_version_first_line 'osv-scanner --version' \
    "osv-scanner version: $OSV_SCANNER_VERSION" osv-scanner --version
