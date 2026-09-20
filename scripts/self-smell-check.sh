#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

report=target/quality/smells-report.json
mkdir -p target/quality

set +e
./target/debug/smells check --path . --policy quality-policy.json \
    --only-group source --format json >"$report"
status=$?
set -e

if [ "$status" -ne 0 ]; then
    printf 'self smell scan failed; complete deterministic evidence follows:\n' >&2
    cat "$report" >&2
    exit "$status"
fi

printf 'self smell scan passed; report: %s\n' "$report"
