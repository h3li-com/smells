#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
workflow="$repository_root/.github/workflows/release.yml"
ci_workflow="$repository_root/.github/workflows/ci.yml"
github_packages_workflow="$repository_root/.github/workflows/publish-github-packages.yml"
pypi_workflow="$repository_root/.github/workflows/publish-pypi.yml"
npm_workflow="$repository_root/.github/workflows/publish-npm.yml"
crates_recovery_workflow="$repository_root/.github/workflows/publish-crates.yml"
registry_dispatcher="$repository_root/scripts/dispatch-registry-publications.sh"
registry_release_verifier="$repository_root/scripts/verify-registry-release.sh"

if grep -F 'uses: ./.github/workflows/publish-pypi.yml' "$workflow" >/dev/null ||
    grep -F 'uses: ./.github/workflows/publish-npm.yml' "$workflow" >/dev/null ||
    grep -F 'uses: ./.github/workflows/publish-crates.yml' "$workflow" >/dev/null; then
    printf 'release workflow calls an external Trusted Publisher as a reusable workflow\n' >&2
    exit 1
fi
grep -F 'publish-external-registries:' "$workflow" >/dev/null
grep -F 'actions: write' "$workflow" >/dev/null
grep -F './scripts/dispatch-registry-publications.sh "$RELEASE_TAG"' \
    "$workflow" >/dev/null
grep -F 'publish-github-packages:' "$workflow" >/dev/null
grep -F 'uses: ./.github/workflows/publish-github-packages.yml' "$workflow" >/dev/null
grep -F 'packages: write' "$workflow" >/dev/null
grep -F 'EXTERNAL_REGISTRIES_RESULT: ${{ needs.publish-external-registries.result }}' \
    "$workflow" >/dev/null
grep -F 'test "$EXTERNAL_REGISTRIES_RESULT" = success' "$workflow" >/dev/null
grep -F 'GITHUB_PACKAGES_RESULT: ${{ needs.publish-github-packages.result }}' \
    "$workflow" >/dev/null
grep -F 'test "$GITHUB_PACKAGES_RESULT" = success' "$workflow" >/dev/null
if grep -F 'secrets.NPM_TOKEN' "$workflow" >/dev/null; then
    printf 'release workflow still accepts a long-lived npm publishing token\n' >&2
    exit 1
fi
if grep -F 'secrets.CARGO_REGISTRY_TOKEN' "$workflow" >/dev/null; then
    printf 'release workflow still accepts a long-lived crates.io token\n' >&2
    exit 1
fi

grep -F 'workflow_call:' "$github_packages_workflow" >/dev/null
grep -F 'workflow_dispatch:' "$github_packages_workflow" >/dev/null
grep -F 'registry-url: https://npm.pkg.github.com' \
    "$github_packages_workflow" >/dev/null
grep -F 'packages: write' "$github_packages_workflow" >/dev/null
grep -F 'gh release verify "$RELEASE_TAG"' \
    "$github_packages_workflow" >/dev/null
grep -F 'gh release download "$RELEASE_TAG"' \
    "$github_packages_workflow" >/dev/null
grep -F 'node npm/publish-packages.mjs' \
    "$github_packages_workflow" >/dev/null

for registry_workflow in \
    "$pypi_workflow" \
    "$npm_workflow" \
    "$crates_recovery_workflow"; do
    if grep -F 'workflow_call:' "$registry_workflow" >/dev/null; then
        printf 'external Trusted Publisher still exposes workflow_call: %s\n' \
            "$registry_workflow" >&2
        exit 1
    fi
    grep -F 'workflow_dispatch:' "$registry_workflow" >/dev/null
    grep -F 'source_run_id:' "$registry_workflow" >/dev/null
    grep -F '    environment: release' "$registry_workflow" >/dev/null
    grep -F 'actions: read' "$registry_workflow" >/dev/null
    grep -F './scripts/verify-registry-release.sh \' \
        "$registry_workflow" >/dev/null
    grep -F '"$RELEASE_TAG" "$SOURCE_RUN_ID" \' \
        "$registry_workflow" >/dev/null
    grep -F 'git checkout --detach "${{ steps.release.outputs.commit }}"' \
        "$registry_workflow" >/dev/null
    grep -F 'gh release verify "$RELEASE_TAG"' \
        "$registry_workflow" >/dev/null
    grep -F 'gh release verify-asset "$RELEASE_TAG"' \
        "$registry_workflow" >/dev/null
done

grep -F 'workflow_call:' "$github_packages_workflow" >/dev/null
grep -F 'workflow_dispatch:' "$github_packages_workflow" >/dev/null
grep -F '    environment: release' "$github_packages_workflow" >/dev/null
grep -F './scripts/verify-registry-release.sh "$RELEASE_TAG"' \
    "$github_packages_workflow" >/dev/null
grep -F 'git checkout --detach "${{ steps.release.outputs.commit }}"' \
    "$github_packages_workflow" >/dev/null
grep -F 'gh release verify "$RELEASE_TAG"' \
    "$github_packages_workflow" >/dev/null
grep -F 'gh release verify-asset "$RELEASE_TAG"' \
    "$github_packages_workflow" >/dev/null

grep -F 'dispatch_workflow publish-pypi.yml' "$registry_dispatcher" >/dev/null
grep -F 'dispatch_workflow publish-npm.yml' "$registry_dispatcher" >/dev/null
grep -F 'dispatch_workflow publish-crates.yml' "$registry_dispatcher" >/dev/null
grep -F 'gh run watch "$run_id"' "$registry_dispatcher" >/dev/null

grep -F 'test "$(printf '\''%s'\'' "$release" | jq -r .isImmutable)" = true' \
    "$registry_release_verifier" >/dev/null
grep -F 'git merge-base --is-ancestor "$release_commit" HEAD' \
    "$registry_release_verifier" >/dev/null
grep -F 'manifest_version=$(git show "$release_commit:Cargo.toml"' \
    "$registry_release_verifier" >/dev/null

grep -F 'uses: pypa/gh-action-pypi-publish@dc37677b2e1c63e2034f94d8a5b11f265b73ba33 # v1.14.2' \
    "$pypi_workflow" >/dev/null
grep -F 'python scripts/verify-pypi-release.py' \
    "$pypi_workflow" >/dev/null

grep -F 'node npm/publish-packages.mjs' \
    "$npm_workflow" >/dev/null
grep -F 'registry-url: https://registry.npmjs.org' \
    "$npm_workflow" >/dev/null

grep -F 'uses: rust-lang/crates-io-auth-action@c6f97d42243bad5fab37ca0427f495c86d5b1a18 # v1' \
    "$crates_recovery_workflow" >/dev/null
grep -F 'CARGO_REGISTRY_TOKEN: ${{ steps.crates_io_auth.outputs.token }}' \
    "$crates_recovery_workflow" >/dev/null
grep -F './scripts/publish-crate.sh' \
    "$crates_recovery_workflow" >/dev/null

grep -F 'rustup toolchain install "$toolchain" --profile minimal --component cargo' \
    "$ci_workflow" >/dev/null
grep -F 'cargo "+$toolchain" package --locked' "$ci_workflow" >/dev/null

printf 'release workflow tests passed\n'
