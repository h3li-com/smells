#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

scanner=${SMELLS_BIN:-./target/debug/smells}
report=${SMELLS_REPORT:-target/quality/smells-report.json}
finding_log=${SMELLS_FINDING_LOG:-target/quality/smells-findings.log}
mkdir -p target/quality

set +e
"$scanner" check --path . --policy quality-policy.json \
    --format table --report "$report" --log "$finding_log"
status=$?
set -e

if [ "$status" -ne 0 ]; then
    printf 'self smell scan failed; read the Finding Log Issue Index first: %s\n' "$finding_log" >&2
    exit "$status"
fi

printf 'self smell scan passed; complete Finding Log: %s\n' "$finding_log"
