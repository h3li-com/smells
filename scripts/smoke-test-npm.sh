#!/bin/sh
set -eu

target=${1:?usage: smoke-test-npm.sh TARGET BINARY WORK_DIRECTORY VERSION}
binary=${2:?usage: smoke-test-npm.sh TARGET BINARY WORK_DIRECTORY VERSION}
work=${3:?usage: smoke-test-npm.sh TARGET BINARY WORK_DIRECTORY VERSION}
version=${4:?usage: smoke-test-npm.sh TARGET BINARY WORK_DIRECTORY VERSION}

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
packager="$script_directory/package-npm.sh"

platform_output="$work/platform"
root_output="$work/root"
consumer="$work/consumer"
mkdir -p "$platform_output" "$root_output" "$consumer"

"$packager" "$target" "$binary" "$platform_output" "$version"
"$packager" root - "$root_output" "$version"

platform_package=$(find "$platform_output" -maxdepth 1 -name '*.tgz' -print)
root_package=$(find "$root_output" -maxdepth 1 -name '*.tgz' -print)
test -n "$platform_package"
test -n "$root_package"
test "$(find "$platform_output" -maxdepth 1 -name '*.tgz' | wc -l | tr -d ' ')" = 1
test "$(find "$root_output" -maxdepth 1 -name '*.tgz' | wc -l | tr -d ' ')" = 1

printf '%s\n' '{"name":"smells-package-smoke","private":true}' > "$consumer/package.json"
npm install \
    --prefix "$consumer" \
    --offline \
    --ignore-scripts \
    --no-audit \
    --no-fund \
    --omit=optional \
    "$platform_package" \
    "$root_package" >/dev/null

test "$("$consumer/node_modules/.bin/smells" --version)" = "smells $version"
cmp "$repository_root/LICENSE" \
    "$consumer/node_modules/@mindful-time/smells/LICENSE"
platform_license=$(find "$consumer/node_modules/@mindful-time" \
    -mindepth 2 -maxdepth 2 -name LICENSE \
    ! -path '*/@mindful-time/smells/LICENSE' -print)
test -n "$platform_license"
cmp "$repository_root/LICENSE" "$platform_license"
grep -F "@mindful-time/smells@$version" \
    "$consumer/node_modules/@mindful-time/smells/README.md" >/dev/null
if grep -F '__SMELLS_VERSION__' \
    "$consumer/node_modules/@mindful-time/smells/README.md" >/dev/null; then
    printf 'npm README contains an unresolved version placeholder\n' >&2
    exit 1
fi

node -e '
const root = require(process.argv[1]);
const expected = process.argv[2];
if (root.name !== "@mindful-time/smells" || root.version !== expected) process.exit(1);
if (root.scripts) process.exit(1);
const versions = Object.values(root.optionalDependencies || {});
if (versions.length !== 5 || versions.some((version) => version !== expected)) process.exit(1);
' "$consumer/node_modules/@mindful-time/smells/package.json" "$version"

printf 'npm package smoke test passed for %s\n' "$target"
