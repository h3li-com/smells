#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
dispatcher="$repository_root/scripts/dispatch-registry-publications.sh"

temporary=$(mktemp -d "${TMPDIR:-/tmp}/smells-registry-publication.XXXXXX")
trap 'rm -rf -- "$temporary"' EXIT HUP INT TERM

fake_bin="$temporary/bin"
fixture_repository="$temporary/repository"
call_log="$temporary/calls.log"
mkdir -p "$fake_bin" "$fixture_repository/scripts"

cat > "$fake_bin/gh" <<'EOF'
#!/bin/sh
set -eu

printf 'gh %s\n' "$*" >> "$CALL_LOG"

case "$1 $2" in
    'workflow run')
        workflow=$3
        if [ "${FAIL_DISPATCH_WORKFLOW:-}" = "$workflow" ]; then
            printf 'dispatch failed for %s\n' "$workflow" >&2
            exit 7
        fi
        case "$workflow" in
            publish-pypi.yml) run_id=101 ;;
            publish-npm.yml) run_id=202 ;;
            publish-crates.yml) run_id=303 ;;
            *) exit 8 ;;
        esac
        printf 'https://github.com/%s/actions/runs/%s\n' \
            "$GITHUB_REPOSITORY" "$run_id"
        ;;
    'run watch')
        run_id=$3
        if [ "${FAIL_WATCH_RUN_ID:-}" = "$run_id" ]; then
            exit 9
        fi
        ;;
    'release view')
        jq -n \
            --arg commit "$RELEASE_COMMIT" \
            --arg tag "$RELEASE_TAG" \
            '{
                isDraft: false,
                isImmutable: true,
                isPrerelease: false,
                tagName: $tag,
                targetCommitish: $commit
            }'
        ;;
    'api repos/'*)
        jq -n \
            --argjson id "$SOURCE_RUN_ID" \
            --arg commit "$RELEASE_COMMIT" \
            --arg owner "$GITHUB_REPOSITORY_OWNER" \
            --arg path "${SOURCE_WORKFLOW_PATH:-.github/workflows/release.yml}" \
            --arg repository "$GITHUB_REPOSITORY" \
            --arg status "${SOURCE_RUN_STATUS:-in_progress}" \
            '{
                id: $id,
                name: "Release",
                path: $path,
                event: "workflow_dispatch",
                status: $status,
                head_branch: "main",
                head_sha: $commit,
                head_repository: {full_name: $repository},
                actor: {login: $owner},
                triggering_actor: {login: $owner}
            }'
        ;;
    *)
        exit 10
        ;;
esac
EOF
chmod +x "$fake_bin/gh"

CALL_LOG="$call_log"
export CALL_LOG

# The dispatcher must start every external Trusted Publisher and wait for each
# top-level run before reporting success.
GH_TOKEN=test-token \
GITHUB_REPOSITORY=mindful-time/smells \
GITHUB_RUN_ID=42 \
PATH="$fake_bin:$PATH" \
    "$dispatcher" v0.5.0 >/dev/null 2>&1

for workflow in publish-pypi.yml publish-npm.yml publish-crates.yml; do
    grep -F "gh workflow run $workflow --repo mindful-time/smells --ref main -f tag=v0.5.0 -f source_run_id=42" \
        "$call_log" >/dev/null
done
for run_id in 101 202 303; do
    grep -F "gh run watch $run_id --repo mindful-time/smells --exit-status --interval 10" \
        "$call_log" >/dev/null
done

# One failed child must fail the dispatcher without preventing it from waiting
# for the other publications, so their outcomes are never abandoned.
: > "$call_log"
if GH_TOKEN=test-token \
    GITHUB_REPOSITORY=mindful-time/smells \
    GITHUB_RUN_ID=42 \
    FAIL_WATCH_RUN_ID=202 \
    PATH="$fake_bin:$PATH" \
    "$dispatcher" v0.5.0 >/dev/null 2>&1; then
    printf 'registry dispatcher accepted a failed child publication\n' >&2
    exit 1
fi
for run_id in 101 202 303; do
    grep -F "gh run watch $run_id " "$call_log" >/dev/null
done

# Exercise release authorization through the verifier's command-line boundary
# against a real Git repository and a mocked GitHub API.
cp "$repository_root/scripts/verify-registry-release.sh" \
    "$fixture_repository/scripts/verify-registry-release.sh"
cp "$repository_root/scripts/verify-release-actor.sh" \
    "$fixture_repository/scripts/verify-release-actor.sh"
(
    cd "$fixture_repository"
    git init -q
    git config user.email tests@example.invalid
    git config user.name 'Smells tests'
    cat > Cargo.toml <<'EOF'
[package]
name = "smells-fixture"
version = "0.5.0"
EOF
    git add Cargo.toml scripts
    git commit -qm fixture
    git tag v0.5.0
)

release_commit=$(git -C "$fixture_repository" rev-parse HEAD)
common_environment() {
    export GITHUB_REF=refs/heads/main
    export GITHUB_REPOSITORY=mindful-time/smells
    export GITHUB_REPOSITORY_OWNER=mindful-time
    export GITHUB_EVENT_NAME=workflow_dispatch
    export RELEASE_COMMIT="$release_commit"
    export RELEASE_TAG=v0.5.0
}

common_environment
(
    cd "$fixture_repository"
    GITHUB_ACTOR=mindful-time \
    GITHUB_TRIGGERING_ACTOR=mindful-time \
    PATH="$fake_bin:$PATH" \
        ./scripts/verify-registry-release.sh v0.5.0 >/dev/null
)

common_environment
(
    cd "$fixture_repository"
    GITHUB_ACTOR='github-actions[bot]' \
    GITHUB_TRIGGERING_ACTOR='github-actions[bot]' \
    SOURCE_RUN_ID=9001 \
    PATH="$fake_bin:$PATH" \
        ./scripts/verify-registry-release.sh v0.5.0 9001 >/dev/null
)

common_environment
if (
    cd "$fixture_repository"
    GITHUB_ACTOR='github-actions[bot]' \
    GITHUB_TRIGGERING_ACTOR='github-actions[bot]' \
    SOURCE_RUN_ID=9001 \
    SOURCE_RUN_STATUS=completed \
    PATH="$fake_bin:$PATH" \
        ./scripts/verify-registry-release.sh v0.5.0 9001 >/dev/null 2>&1
); then
    printf 'registry verifier accepted a completed source release run\n' >&2
    exit 1
fi

common_environment
if (
    cd "$fixture_repository"
    GITHUB_ACTOR='github-actions[bot]' \
    GITHUB_TRIGGERING_ACTOR='github-actions[bot]' \
    SOURCE_RUN_ID=9001 \
    SOURCE_WORKFLOW_PATH=.github/workflows/ci.yml \
    PATH="$fake_bin:$PATH" \
        ./scripts/verify-registry-release.sh v0.5.0 9001 >/dev/null 2>&1
); then
    printf 'registry verifier accepted the wrong source workflow\n' >&2
    exit 1
fi

printf 'registry publication tests passed\n'
