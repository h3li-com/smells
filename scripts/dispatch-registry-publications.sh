#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    printf 'usage: dispatch-registry-publications.sh vMAJOR.MINOR.PATCH\n' >&2
    exit 2
fi

release_tag=$1
printf '%s\n' "$release_tag" \
    | grep -Eq '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
: "${GH_TOKEN:?GH_TOKEN is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${GITHUB_RUN_ID:?GITHUB_RUN_ID is required}"
printf '%s\n' "$GITHUB_RUN_ID" | grep -Eq '^[1-9][0-9]*$'

dispatch_workflow() {
    workflow=$1
    output=$(gh workflow run "$workflow" \
        --repo "$GITHUB_REPOSITORY" \
        --ref main \
        -f "tag=$release_tag" \
        -f "source_run_id=$GITHUB_RUN_ID" 2>&1) || {
        printf '%s\n' "$output" >&2
        return 1
    }
    run_url=$(printf '%s\n' "$output" \
        | grep -Eo "https://github.com/$GITHUB_REPOSITORY/actions/runs/[0-9]+" \
        | tail -n 1)
    if [ -z "$run_url" ]; then
        printf 'workflow dispatch did not return a run URL for %s:\n%s\n' \
            "$workflow" "$output" >&2
        return 1
    fi
    printf 'dispatched %s: %s\n' "$workflow" "$run_url" >&2
    printf '%s\n' "${run_url##*/}"
}

wait_for_workflow() {
    workflow=$1
    run_id=$2
    printf 'waiting for %s run %s\n' "$workflow" "$run_id"
    gh run watch "$run_id" \
        --repo "$GITHUB_REPOSITORY" \
        --exit-status \
        --interval 10
}

failed=0
pypi_run=
npm_run=
crates_run=

if ! pypi_run=$(dispatch_workflow publish-pypi.yml); then
    failed=1
fi
if ! npm_run=$(dispatch_workflow publish-npm.yml); then
    failed=1
fi
if ! crates_run=$(dispatch_workflow publish-crates.yml); then
    failed=1
fi

if [ -n "$pypi_run" ] &&
    ! wait_for_workflow publish-pypi.yml "$pypi_run"; then
    failed=1
fi
if [ -n "$npm_run" ] &&
    ! wait_for_workflow publish-npm.yml "$npm_run"; then
    failed=1
fi
if [ -n "$crates_run" ] &&
    ! wait_for_workflow publish-crates.yml "$crates_run"; then
    failed=1
fi

exit "$failed"
