#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    printf 'usage: verify-registry-release.sh vMAJOR.MINOR.PATCH\n' >&2
    exit 2
fi

release_tag=$1

./scripts/verify-release-actor.sh \
    "$GITHUB_REPOSITORY_OWNER" \
    "$GITHUB_ACTOR" \
    "$GITHUB_TRIGGERING_ACTOR" >/dev/null
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
manifest_version=$(git show "$release_commit:Cargo.toml" \
    | sed -n '/^\[package\]/,/^\[/ s/^version = "\([^"]*\)"/\1/p')
test "$manifest_version" = "$version"

printf 'commit=%s\n' "$release_commit"
printf 'version=%s\n' "$version"
