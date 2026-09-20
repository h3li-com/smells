#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
workflow="$repository_root/.github/workflows/release.yml"
github_packages_workflow="$repository_root/.github/workflows/publish-github-packages.yml"

grep -F 'publish-github-packages:' "$workflow" >/dev/null
grep -F 'uses: ./.github/workflows/publish-github-packages.yml' "$workflow" >/dev/null
grep -F 'packages: write' "$workflow" >/dev/null
grep -F 'GITHUB_PACKAGES_RESULT: ${{ needs.publish-github-packages.result }}' \
    "$workflow" >/dev/null
grep -F 'test "$GITHUB_PACKAGES_RESULT" = success' "$workflow" >/dev/null
grep -F 'uses: rust-lang/crates-io-auth-action@c6f97d42243bad5fab37ca0427f495c86d5b1a18 # v1' \
    "$workflow" >/dev/null
grep -F 'CARGO_REGISTRY_TOKEN: ${{ steps.crates_io_auth.outputs.token }}' \
    "$workflow" >/dev/null

if grep -F 'secrets.NPM_TOKEN' "$workflow" >/dev/null; then
    printf 'release workflow still accepts a long-lived npm publishing token\n' >&2
    exit 1
fi
if grep -F 'secrets.CARGO_REGISTRY_TOKEN' "$workflow" >/dev/null; then
    printf 'release workflow still accepts a long-lived crates.io token\n' >&2
    exit 1
fi
grep -F 'id-token: write' "$workflow" >/dev/null

grep -F 'workflow_call:' "$github_packages_workflow" >/dev/null
grep -F 'workflow_dispatch:' "$github_packages_workflow" >/dev/null
grep -F 'registry-url: https://npm.pkg.github.com' \
    "$github_packages_workflow" >/dev/null
grep -F 'packages: write' "$github_packages_workflow" >/dev/null
grep -F 'test "$(jq -r .isImmutable <<<"$release")" = true' \
    "$github_packages_workflow" >/dev/null
grep -F 'test "$(git rev-list -n 1 "$RELEASE_TAG")" = "$release_commit"' \
    "$github_packages_workflow" >/dev/null
grep -F 'manifest_version=$(git show "$release_commit:Cargo.toml"' \
    "$github_packages_workflow" >/dev/null
grep -F 'gh release verify "$RELEASE_TAG"' \
    "$github_packages_workflow" >/dev/null
grep -F 'gh release download "$RELEASE_TAG"' \
    "$github_packages_workflow" >/dev/null
grep -F 'node npm/publish-packages.mjs' \
    "$github_packages_workflow" >/dev/null

printf 'release workflow tests passed\n'
