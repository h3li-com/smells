#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
collector="$repository_root/scripts/collect-release-artifacts.sh"

temporary=$(mktemp -d "${TMPDIR:-/tmp}/smells-release-artifacts.XXXXXX")
trap 'rm -rf -- "$temporary"' EXIT HUP INT TERM

input="$temporary/input"
output="$temporary/output"
mkdir -p \
    "$input/release-linux/dist" \
    "$input/release-linux/release" \
    "$input/release-macos/dist" \
    "$input/release-macos/release" \
    "$input/release-windows/release"

printf 'linux wheel\n' > "$input/release-linux/dist/smells-linux.whl"
printf 'linux archive\n' > "$input/release-linux/release/smells-linux.tar.gz"
printf 'linux checksum\n' > "$input/release-linux/release/smells-linux.tar.gz.sha256"
printf 'linux npm package\n' > "$input/release-linux/release/smells-linux.tgz"
printf 'Rust crate\n' > "$input/release-linux/release/smells.crate"
printf 'macOS wheel\n' > "$input/release-macos/dist/smells-macos.whl"
printf 'macOS archive\n' > "$input/release-macos/release/smells-macos.tar.gz"
printf 'macOS checksum\n' > "$input/release-macos/release/smells-macos.tar.gz.sha256"
printf 'Windows archive\n' > "$input/release-windows/release/smells-windows.zip"
printf 'Windows checksum\n' > "$input/release-windows/release/smells-windows.zip.sha256"
printf 'ignored\n' > "$input/release-linux/metadata.txt"

"$collector" "$input" "$output"

expected='smells-linux.tar.gz
smells-linux.tar.gz.sha256
smells-linux.tgz
smells-linux.whl
smells-macos.tar.gz
smells-macos.tar.gz.sha256
smells-macos.whl
smells-windows.zip
smells-windows.zip.sha256
smells.crate'
actual=$(find "$output" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; | sort)
test "$actual" = "$expected"
test "$(cat "$output/smells-linux.whl")" = 'linux wheel'
test ! -e "$output/metadata.txt"

duplicate_input="$temporary/duplicate-input"
duplicate_output="$temporary/duplicate-output"
mkdir -p "$duplicate_input/one" "$duplicate_input/two"
printf 'first\n' > "$duplicate_input/one/smells.whl"
printf 'second\n' > "$duplicate_input/two/smells.whl"

if "$collector" "$duplicate_input" "$duplicate_output" >/dev/null 2>&1; then
    printf 'collector accepted duplicate release artifact basenames\n' >&2
    exit 1
fi

printf 'release artifact collection tests passed\n'
