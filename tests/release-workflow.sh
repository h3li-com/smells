#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
workflow="$repository_root/.github/workflows/release.yml"

grep -F 'publish-github-packages:' "$workflow" >/dev/null
grep -F 'registry-url: https://npm.pkg.github.com' "$workflow" >/dev/null
grep -F 'packages: write' "$workflow" >/dev/null
grep -F 'GITHUB_PACKAGES_RESULT: ${{ needs.publish-github-packages.result }}' \
    "$workflow" >/dev/null
grep -F 'test "$GITHUB_PACKAGES_RESULT" = success' "$workflow" >/dev/null

if grep -F 'secrets.NPM_TOKEN' "$workflow" >/dev/null; then
    printf 'release workflow still accepts a long-lived npm publishing token\n' >&2
    exit 1
fi
grep -F 'id-token: write' "$workflow" >/dev/null

printf 'release workflow tests passed\n'
