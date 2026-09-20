#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/smells-crate-package.XXXXXX")
trap 'rm -rf -- "$temporary"' EXIT HUP INT TERM

files="$temporary/files.txt"
cargo package \
    --manifest-path "$repository_root/Cargo.toml" \
    --locked --allow-dirty --list > "$files"

for required in \
    Cargo.lock Cargo.toml LICENSE README.md \
    docs/rust-rule-contracts.md docs/python-rule-contracts.md \
    docs/typescript-rule-contracts.md docs/report-interface.md \
    docs/provider-evidence.md examples/quality-policy.json \
    examples/python-quality-policy.json examples/typescript-quality-policy.json \
    rules/rust-v1.json rules/python-v1.json rules/typescript-v1.json \
    rules/rust-v1-guidance.json rules/portable-v1-guidance.json \
    schemas/quality-policy.schema.json \
    schemas/python-quality-policy.schema.json \
    schemas/typescript-quality-policy.schema.json \
    schemas/provider-evidence.schema.json src/main.rs
do
    grep -Fx "$required" "$files" >/dev/null
done

if grep -Eq '^(tests/|npm/|docs/package-distribution\.md$|CHANGELOG\.md$)' "$files"; then
    printf 'crate package contains files outside the minimal build closure\n' >&2
    exit 1
fi

printf 'signed release crate\n' > "$temporary/release.crate"
printf 'different local crate\n' > "$temporary/local.crate"
if "$repository_root/scripts/publish-crate.sh" \
    0.0.0 "$temporary/release.crate" "$temporary/local.crate" \
    > /dev/null 2>&1; then
    printf 'crate publisher accepted a local package that differs from the release\n' >&2
    exit 1
fi

printf 'crate package tests passed\n'
