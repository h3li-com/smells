#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

scanner=${SMELLS_BIN:-./target/debug/smells}
report=${SMELLS_REPORT:-target/quality/smells-report.json}
mkdir -p target/quality

set +e
"$scanner" check --path . --policy quality-policy.json \
    --only-group source --format table --report "$report"
status=$?
set -e

if [ "$status" -ne 0 ]; then
    printf 'self smell scan failed; complete deterministic JSON report: %s\n' "$report" >&2
    exit "$status"
fi

printf 'self smell scan passed; complete deterministic JSON report: %s\n' "$report"
