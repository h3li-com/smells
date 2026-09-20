#!/bin/sh
set -eu

release_owner=${1:-}
shift || true

if [ -z "$release_owner" ] || [ "$#" -eq 0 ]; then
    printf 'usage: verify-release-actor.sh RELEASE_OWNER ACTOR...\n' >&2
    exit 2
fi

for actor in "$@"; do
    if [ "$actor" != "$release_owner" ]; then
        printf 'release is restricted to repository owner %s; received %s\n' \
            "$release_owner" "${actor:-<empty>}" >&2
        exit 2
    fi
done

printf '%s\n' "$release_owner"
