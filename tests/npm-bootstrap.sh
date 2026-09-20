#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
bootstrap="$repository_root/scripts/bootstrap-npm-release.sh"

temporary=$(mktemp -d "${TMPDIR:-/tmp}/smells-npm-bootstrap.XXXXXX")
trap 'rm -rf -- "$temporary"' EXIT HUP INT TERM

fake_bin="$temporary/bin"
fixtures="$temporary/fixtures"
call_log="$temporary/calls.log"
mkdir -p "$fake_bin" "$fixtures"

# Keep this inventory independent of npm/platforms.json so the test detects a
# packaging entry that is accidentally omitted from the bootstrap contract.
for name in \
    mindful-time-smells-darwin-arm64 \
    mindful-time-smells-darwin-x64 \
    mindful-time-smells-linux-arm64-gnu \
    mindful-time-smells-linux-x64-gnu \
    mindful-time-smells-win32-x64-msvc \
    mindful-time-smells
do
    printf '%s\n' "$name" > "$fixtures/$name-0.3.0.tgz"
done
(
    cd "$fixtures"
    shasum -a 256 ./*.tgz > sha256.sum
)

cat > "$fake_bin/gh" <<'EOF'
#!/bin/sh
set -eu
printf 'gh %s\n' "$*" >> "$CALL_LOG"
case "$1 $2" in
    'release verify')
        test "$3" = v0.3.0
        ;;
    'release download')
        test "$3" = v0.3.0
        shift 3
        destination=
        while [ "$#" -gt 0 ]; do
            case "$1" in
                --dir)
                    destination=$2
                    shift 2
                    ;;
                *)
                    shift
                    ;;
            esac
        done
        test -n "$destination"
        cp "$FIXTURES"/* "$destination/"
        ;;
    *)
        exit 9
        ;;
esac
EOF

cat > "$fake_bin/npm" <<'EOF'
#!/bin/sh
set -eu
printf 'npm %s\n' "$*" >> "$CALL_LOG"
test -n "${NPM_CONFIG_USERCONFIG:-}"
case "$1" in
    login)
        printf 'temporary login\n' > "$NPM_CONFIG_USERCONFIG"
        ;;
    whoami)
        printf '%s\n' "${NPM_IDENTITY:-mindfultime}"
        ;;
    logout)
        rm -f "$NPM_CONFIG_USERCONFIG"
        ;;
    *)
        exit 12
        ;;
esac
EOF

cat > "$fake_bin/node" <<'EOF'
#!/bin/sh
set -eu
printf 'node %s\n' "$*" >> "$CALL_LOG"
test "$1" = "$EXPECTED_PUBLISHER"
test "$3" = 0.3.0
test "$4" = https://registry.npmjs.org
test "$#" = 4
test "$(find "$2" -maxdepth 1 -name '*.tgz' -print | wc -l | tr -d ' ')" = 6
EOF

chmod +x "$fake_bin/gh" "$fake_bin/npm" "$fake_bin/node"

CALL_LOG="$call_log"
FIXTURES="$fixtures"
EXPECTED_PUBLISHER="$repository_root/npm/publish-packages.mjs"
export CALL_LOG FIXTURES EXPECTED_PUBLISHER

if NODE_AUTH_TOKEN=forbidden PATH="$fake_bin:$PATH" \
    "$bootstrap" v0.3.0 >/dev/null 2>&1; then
    printf 'bootstrap accepted NODE_AUTH_TOKEN\n' >&2
    exit 1
fi
test ! -e "$call_log"

if NPM_IDENTITY=another-user PATH="$fake_bin:$PATH" \
    "$bootstrap" v0.3.0 >/dev/null 2>&1; then
    printf 'bootstrap accepted a non-owner npm identity\n' >&2
    exit 1
fi
: > "$call_log"

PATH="$fake_bin:$PATH" "$bootstrap" v0.3.0 >/dev/null

grep -F 'npm login --auth-type=web --registry https://registry.npmjs.org' \
    "$call_log" >/dev/null
grep -F 'npm whoami --registry https://registry.npmjs.org' "$call_log" >/dev/null
grep -F 'npm logout --registry https://registry.npmjs.org' "$call_log" >/dev/null
grep -F 'gh release verify v0.3.0 --repo mindful-time/smells' "$call_log" >/dev/null
grep -F 'gh release download v0.3.0 --repo mindful-time/smells' "$call_log" >/dev/null
grep -F 'node '"$EXPECTED_PUBLISHER" "$call_log" >/dev/null

if PATH="$fake_bin:$PATH" "$bootstrap" v0.3.1 >/dev/null 2>&1; then
    printf 'bootstrap accepted a release other than v0.3.0\n' >&2
    exit 1
fi

printf 'npm bootstrap tests passed\n'
