#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    printf 'usage: verify-registry-release.sh vMAJOR.MINOR.PATCH [SOURCE_RUN_ID]\n' >&2
    exit 2
fi

release_tag=$1
source_run_id=${2:-}

test "$GITHUB_REF" = refs/heads/main
printf '%s\n' "$release_tag" \
    | grep -Eq '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'

version=${release_tag#v}
release=$(gh release view "$release_tag" \
    --json isDraft,isImmutable,isPrerelease,tagName,targetCommitish)
test "$(printf '%s' "$release" | jq -r .tagName)" = "$release_tag"
test "$(printf '%s' "$release" | jq -r .isDraft)" = false
test "$(printf '%s' "$release" | jq -r .isImmutable)" = true
test "$(printf '%s' "$release" | jq -r .isPrerelease)" = false

release_commit=$(printf '%s' "$release" | jq -r .targetCommitish)
git cat-file -e "$release_commit^{commit}"
test "$(git rev-list -n 1 "$release_tag")" = "$release_commit"
git merge-base --is-ancestor "$release_commit" HEAD

if [ -z "$source_run_id" ]; then
    ./scripts/verify-release-actor.sh \
        "$GITHUB_REPOSITORY_OWNER" \
        "$GITHUB_ACTOR" \
        "$GITHUB_TRIGGERING_ACTOR" >/dev/null
else
    printf '%s\n' "$source_run_id" | grep -Eq '^[1-9][0-9]*$'
    test "$GITHUB_EVENT_NAME" = workflow_dispatch
    source_run=$(gh api \
        "repos/$GITHUB_REPOSITORY/actions/runs/$source_run_id")
    printf '%s' "$source_run" | jq -e \
        --arg commit "$release_commit" \
        --arg owner "$GITHUB_REPOSITORY_OWNER" \
        --arg repository "$GITHUB_REPOSITORY" \
        --argjson run_id "$source_run_id" \
        '.id == $run_id and
         .name == "Release" and
         .path == ".github/workflows/release.yml" and
         .event == "workflow_dispatch" and
         .status == "in_progress" and
         .head_branch == "main" and
         .head_sha == $commit and
         .head_repository.full_name == $repository and
         .actor.login == $owner and
         .triggering_actor.login == $owner' >/dev/null
fi

manifest_version=$(git show "$release_commit:Cargo.toml" \
    | sed -n '/^\[package\]/,/^\[/ s/^version = "\([^"]*\)"/\1/p')
test "$manifest_version" = "$version"

printf 'commit=%s\n' "$release_commit"
printf 'version=%s\n' "$version"
