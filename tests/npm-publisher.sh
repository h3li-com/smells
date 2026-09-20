#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
publisher="$repository_root/npm/publish-packages.mjs"
temporary=$(mktemp -d "${TMPDIR:-/tmp}/smells-npm-publisher.XXXXXX")
trap 'rm -rf -- "$temporary"' EXIT HUP INT TERM

packages="$temporary/packages"
fake_bin="$temporary/bin"
mkdir -p "$packages" "$fake_bin"

for name in \
    mindful-time-smells-darwin-arm64 \
    mindful-time-smells-darwin-x64 \
    mindful-time-smells-linux-arm64-gnu \
    mindful-time-smells-linux-x64-gnu \
    mindful-time-smells-win32-x64-msvc \
    mindful-time-smells
do
    printf 'identical package fixture\n' > "$packages/$name-0.3.0.tgz"
done

printf '%s\n' \
    '#!/bin/sh' \
    'set -eu' \
    'printf '\''%s\n'\'' "$*" >> "$NPM_CALL_LOG"' \
    'case "$1" in' \
    '    view)' \
    '        case " $* " in' \
    '            *" --registry $EXPECTED_REGISTRY "*) ;;' \
    '            *) exit 9 ;;' \
    '        esac' \
    '        printf '\''"%s"\n'\'' "$EXPECTED_INTEGRITY"' \
    '        ;;' \
    '    publish)' \
    '        printf '\''unexpected publish call\n'\'' >&2' \
    '        exit 10' \
    '        ;;' \
    '    *)' \
    '        exit 11' \
    '        ;;' \
    'esac' > "$fake_bin/npm"
chmod +x "$fake_bin/npm"

EXPECTED_INTEGRITY=$(node -e '
const { createHash } = require("node:crypto");
const { readFileSync } = require("node:fs");
process.stdout.write(`sha512-${createHash("sha512").update(readFileSync(process.argv[1])).digest("base64")}`);
' "$packages/mindful-time-smells-0.3.0.tgz")
EXPECTED_REGISTRY=https://npm.pkg.github.com
NPM_CALL_LOG="$temporary/npm-calls.txt"
export EXPECTED_INTEGRITY EXPECTED_REGISTRY NPM_CALL_LOG

PATH="$fake_bin:$PATH" node "$publisher" \
    "$packages" 0.3.0 "$EXPECTED_REGISTRY"

test "$(wc -l < "$NPM_CALL_LOG" | tr -d ' ')" = 6
if grep -v -- "--registry $EXPECTED_REGISTRY" "$NPM_CALL_LOG" >/dev/null; then
    printf 'npm publisher did not pin every registry operation\n' >&2
    exit 1
fi

printf 'npm publisher tests passed\n'
