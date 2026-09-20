#!/bin/sh
set -eu

version=${1:?usage: publish-crate.sh VERSION CRATE_FILE}
crate_file=${2:?usage: publish-crate.sh VERSION CRATE_FILE}
user_agent="mindful-time-smells-release/$version (https://github.com/mindful-time/smells)"
api="https://crates.io/api/v1/crates/smells/$version"
temporary_root=${RUNNER_TEMP:-${TMPDIR:-/tmp}}
response=$(mktemp "$temporary_root/smells-crates-response.XXXXXX")
trap 'rm -f -- "$response"' EXIT HUP INT TERM

expected=$(sha256sum "$crate_file" | cut -d ' ' -f 1)

registry_checksum() {
    status=$(curl -sS -H "User-Agent: $user_agent" -o "$response" -w '%{http_code}' "$api")
    case "$status" in
        200)
            jq -er '.version.checksum' "$response"
            ;;
        404)
            return 1
            ;;
        *)
            printf 'crates.io metadata request failed with HTTP %s\n' "$status" >&2
            return 2
            ;;
    esac
}

if existing=$(registry_checksum); then
    if [ "$existing" != "$expected" ]; then
        printf 'crates.io digest mismatch for smells %s\n' "$version" >&2
        exit 2
    fi
    printf 'crates.io package already matches: smells %s\n' "$version"
    exit 0
else
    status=$?
    test "$status" = 1 || exit "$status"
fi

publish_status=0
cargo publish --locked || publish_status=$?

attempt=1
while [ "$attempt" -le 10 ]; do
    if actual=$(registry_checksum); then
        if [ "$actual" != "$expected" ]; then
            printf 'crates.io digest mismatch after publishing smells %s\n' "$version" >&2
            exit 2
        fi
        printf 'verified crates.io package: smells %s\n' "$version"
        exit 0
    else
        status=$?
        test "$status" = 1 || exit "$status"
    fi
    sleep 3
    attempt=$((attempt + 1))
done

printf 'crates.io package did not become visible after cargo publish (status %s)\n' \
    "$publish_status" >&2
exit 1
