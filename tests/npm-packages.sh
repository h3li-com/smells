#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
packager="$repository_root/scripts/package-npm.sh"
smoke_test="$repository_root/scripts/smoke-test-npm.sh"
version=0.3.0

temporary=$(mktemp -d "${TMPDIR:-/tmp}/smells-npm-packages.XXXXXX")
mkdir -p "$repository_root/target"
absolute_work=$(mktemp -d "$repository_root/target/smells-npm-packages-relative.XXXXXX")
trap 'rm -rf -- "$temporary" "$absolute_work"' EXIT HUP INT TERM
NPM_CONFIG_CACHE="$temporary/npm-cache"
NPM_CONFIG_LOGLEVEL=error
export NPM_CONFIG_CACHE
export NPM_CONFIG_LOGLEVEL

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)
        target=aarch64-apple-darwin
        ;;
    Darwin-x86_64)
        target=x86_64-apple-darwin
        ;;
    Linux-aarch64)
        target=aarch64-unknown-linux-gnu
        ;;
    Linux-x86_64)
        target=x86_64-unknown-linux-gnu
        ;;
    *)
        printf 'unsupported npm package test host: %s-%s\n' "$(uname -s)" "$(uname -m)" >&2
        exit 2
        ;;
esac

fake_binary="$temporary/smells"
printf '%s\n' \
    '#!/bin/sh' \
    'if [ "${1:-}" = --version ]; then' \
    "    printf '%s\\n' 'smells $version'" \
    '    exit 0' \
    'fi' \
    'printf '\''%s\n'\'' "$*"' \
    'exit 7' > "$fake_binary"
chmod +x "$fake_binary"

relative_work=${absolute_work#"$repository_root/"}
(
    cd "$repository_root"
    "$smoke_test" "$target" "$fake_binary" "$relative_work" "$version"
)

forwarded="$temporary/forwarded.txt"
if "$absolute_work/consumer/node_modules/.bin/smells" alpha 'two words' \
    > "$forwarded" 2>&1; then
    printf 'npm launcher did not forward the native exit status\n' >&2
    exit 1
else
    status=$?
fi
test "$status" = 7
test "$(cat "$forwarded")" = 'alpha two words'

if "$packager" unsupported-target "$fake_binary" "$temporary/invalid" "$version" \
    >/dev/null 2>&1; then
    printf 'npm packager accepted an unsupported target\n' >&2
    exit 1
fi

printf 'npm package tests passed\n'
