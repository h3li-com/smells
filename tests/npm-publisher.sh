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
packages=$(CDPATH= cd -- "$packages" && pwd -P)

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

publish_bin="$temporary/publish-bin"
publish_state="$temporary/publish-state"
publish_log="$temporary/publish.log"
mkdir -p "$publish_bin" "$publish_state"

cat > "$publish_bin/npm" <<'EOF'
#!/bin/sh
set -eu
case "$1" in
    view)
        printf 'view %s\n' "$*" >> "$PUBLISH_CALL_LOG"
        package_key=$(printf '%s\n' "$2" \
            | sed 's/^@//; s/@0\.3\.0$//; s#/#-#')
        marker="$PUBLISH_STATE/$package_key-0.3.0.tgz"
        if [ ! -f "$marker" ]; then
            printf 'npm ERR! code E404\n' >&2
            exit 1
        fi
        printf '"%s"\n' "$EXPECTED_INTEGRITY"
        ;;
    publish)
        printf 'publish %s\n' "$*" >> "$PUBLISH_CALL_LOG"
        artifact=${2##*/}
        : > "$PUBLISH_STATE/$artifact"
        ;;
    *)
        exit 12
        ;;
esac
EOF
chmod +x "$publish_bin/npm"

PUBLISH_CALL_LOG="$publish_log"
PUBLISH_STATE="$publish_state"
export PUBLISH_CALL_LOG PUBLISH_STATE

(
    cd "$temporary"
    PATH="$publish_bin:$PATH" node "$publisher" \
        packages 0.3.0 https://registry.npmjs.org --provenance
)

expected_publish_order='mindful-time-smells-darwin-arm64-0.3.0.tgz
mindful-time-smells-darwin-x64-0.3.0.tgz
mindful-time-smells-linux-arm64-gnu-0.3.0.tgz
mindful-time-smells-linux-x64-gnu-0.3.0.tgz
mindful-time-smells-win32-x64-msvc-0.3.0.tgz
mindful-time-smells-0.3.0.tgz'
actual_publish_order=$(sed -n 's#^publish .*\(/mindful-time-.*\.tgz\).*#\1#p' \
    "$publish_log" | sed 's#^.*/##')
test "$actual_publish_order" = "$expected_publish_order"
test "$(grep -c '^view ' "$publish_log")" = 12
test "$(grep -c '^publish ' "$publish_log")" = 6
if [ "$(awk -v prefix="$packages/" \
    '$1 == "publish" && $2 == "publish" && index($3, prefix) == 1 { count += 1 } END { print count + 0 }' \
    "$publish_log")" -ne 6 ]; then
    printf 'npm publisher did not resolve every local tarball to an absolute path\n' >&2
    exit 1
fi
if grep '^publish ' "$publish_log" \
    | grep -v -- '--registry https://registry.npmjs.org --access public --provenance$' \
    >/dev/null; then
    printf 'npm publish omitted required public/provenance options\n' >&2
    exit 1
fi

printf 'npm publisher tests passed\n'
