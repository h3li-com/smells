#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=quality-tool-versions.sh
. "$script_directory/quality-tool-versions.sh"

if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
    printf 'CI quality tool bootstrap supports Linux x86_64 only\n' >&2
    exit 2
fi

install_root=${SMELLS_CI_BIN_DIR:-${CARGO_HOME:-$HOME/.cargo}/bin}
mkdir -p "$install_root"

install_cargo_tool() {
    command_name=$1
    expected=$2
    crate=$3
    version=$4
    shift 4
    if command -v "$command_name" >/dev/null 2>&1 \
        && [ "$("$command_name" "$@" 2>&1)" = "$expected" ]; then
        return
    fi
    cargo install "$crate" --version "$version" --locked
}

install_cargo_tool cargo-crap "cargo-crap $CARGO_CRAP_VERSION" \
    cargo-crap "$CARGO_CRAP_VERSION" --version
install_cargo_tool cargo-llvm-cov "cargo-llvm-cov $CARGO_LLVM_COV_VERSION" \
    cargo-llvm-cov "$CARGO_LLVM_COV_VERSION" --version

temporary=$(mktemp -d "${RUNNER_TEMP:-/tmp}/smells-ci-tools.XXXXXX")
trap 'rm -rf -- "$temporary"' EXIT HUP INT TERM

gitleaks_archive="gitleaks_${GITLEAKS_VERSION}_linux_x64.tar.gz"
curl --fail --location --silent --show-error \
    "https://github.com/gitleaks/gitleaks/releases/download/v$GITLEAKS_VERSION/$gitleaks_archive" \
    --output "$temporary/$gitleaks_archive"
printf '%s  %s\n' \
    "$GITLEAKS_LINUX_X64_SHA256" \
    "$temporary/$gitleaks_archive" | sha256sum --check --status
tar -xzf "$temporary/$gitleaks_archive" -C "$temporary" gitleaks
install -m 0755 "$temporary/gitleaks" "$install_root/gitleaks"

osv_binary=osv-scanner_linux_amd64
curl --fail --location --silent --show-error \
    "https://github.com/google/osv-scanner/releases/download/v$OSV_SCANNER_VERSION/$osv_binary" \
    --output "$temporary/$osv_binary"
printf '%s  %s\n' \
    "$OSV_SCANNER_LINUX_AMD64_SHA256" \
    "$temporary/$osv_binary" | sha256sum --check --status
install -m 0755 "$temporary/$osv_binary" "$install_root/osv-scanner"
