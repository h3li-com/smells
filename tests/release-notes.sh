#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
temporary_root=${TMPDIR:-/tmp}
fixture=$(mktemp -d "$temporary_root/smells-release-notes.XXXXXX")
trap 'rm -rf "$fixture"' EXIT HUP INT TERM

mkdir -p "$fixture/scripts" "$fixture/docs/releases"
cp "$repository_root/scripts/verify-release-notes.sh" "$fixture/scripts/"

write_valid_note() {
    cat >"$fixture/docs/releases/v9.8.7.md" <<'EOF'
# Smells v9.8.7

## Why this release

Explain the user problem.

## What was added

- Add one capability.

## What was fixed

- Fix one behavior.

## What was removed

- No public behavior was removed.

## Before upgrading

Update compatible consumers first.

## How to use it

```sh
uv tool install smells==9.8.7
npm install @mindful-time/smells@9.8.7
cargo install --version 9.8.7 smells
curl https://example.invalid/v9.8.7/examples/quality-policy.json
smells check --path . --policy quality-policy.json
```

## Verification

- Record the release evidence.
EOF
}

expect_failure() {
    if "$fixture/scripts/verify-release-notes.sh" "$@" >/dev/null 2>&1; then
        printf 'release-note validation unexpectedly passed: %s\n' "$*" >&2
        exit 1
    fi
}

write_valid_note
"$fixture/scripts/verify-release-notes.sh" v9.8.7 >/dev/null

expect_failure v9.8.6

write_valid_note
sed -i.bak '/## What was fixed/d' "$fixture/docs/releases/v9.8.7.md"
expect_failure v9.8.7

write_valid_note
sed -i.bak '/## What was fixed/a\
## What was fixed' "$fixture/docs/releases/v9.8.7.md"
expect_failure v9.8.7

write_valid_note
sed -i.bak '/Fix one behavior/d' "$fixture/docs/releases/v9.8.7.md"
expect_failure v9.8.7

write_valid_note
sed -i.bak 's/Explain the user problem./TODO/' \
    "$fixture/docs/releases/v9.8.7.md"
expect_failure v9.8.7

write_valid_note
sed -i.bak 's/smells==9.8.7/smells==9.8.6/' \
    "$fixture/docs/releases/v9.8.7.md"
expect_failure v9.8.7

printf 'release note validation tests passed\n'
