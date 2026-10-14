#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)

tag=${1:-}
if ! printf '%s\n' "$tag" \
    | grep -Eq '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
    printf 'release tag must be exactly vMAJOR.MINOR.PATCH\n' >&2
    exit 2
fi

version=${tag#v}
release_note="$repository_root/docs/releases/$tag.md"
if [ ! -f "$release_note" ]; then
    printf 'release note does not exist: docs/releases/%s.md\n' "$tag" >&2
    exit 2
fi

expected_title="# Smells $tag"
if [ "$(sed -n '1p' "$release_note")" != "$expected_title" ]; then
    printf 'release note title must be exactly: %s\n' "$expected_title" >&2
    exit 2
fi

previous_line=0
for heading in \
    '## Why this release' \
    '## What was added' \
    '## What was fixed' \
    '## What was removed' \
    '## Before upgrading' \
    '## How to use it' \
    '## Verification'; do
    matches=$(grep -n -F -x "$heading" "$release_note" || true)
    match_count=$(printf '%s\n' "$matches" | awk 'NF { count += 1 } END { print count + 0 }')
    if [ "$match_count" -ne 1 ]; then
        printf 'release note must contain exactly one section: %s\n' "$heading" >&2
        exit 2
    fi
    line=${matches%%:*}
    if [ "$line" -le "$previous_line" ]; then
        printf 'release note section is missing or out of order: %s\n' "$heading" >&2
        exit 2
    fi
    body=$(awk -v heading="$heading" '
        $0 == heading { inside = 1; next }
        inside && /^## / { exit }
        inside && NF { print; exit }
    ' "$release_note")
    if [ -z "$body" ]; then
        printf 'release note section is empty: %s\n' "$heading" >&2
        exit 2
    fi
    previous_line=$line
done

if grep -Eq 'TODO|TBD|CHANGEME|\{\{' "$release_note"; then
    printf 'release note contains an unfinished placeholder\n' >&2
    exit 2
fi

grep -F "smells==$version" "$release_note" >/dev/null
grep -F "@mindful-time/smells@$version" "$release_note" >/dev/null
grep -F -- "--version $version smells" "$release_note" >/dev/null
grep -F "examples/" "$release_note" >/dev/null
grep -F 'smells check' "$release_note" >/dev/null

printf '%s\n' "$release_note"
