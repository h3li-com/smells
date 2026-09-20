#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
repository=mindful-time/smells
registry=https://registry.npmjs.org

tag=${1:-}
if [ "$tag" != v0.3.0 ]; then
    printf '%s\n' 'manual npm bootstrap is restricted to v0.3.0' >&2
    exit 2
fi
version=$("$script_directory/verify-release-tag.sh" "$tag")

if [ -n "${NODE_AUTH_TOKEN:-}" ] \
    || [ -n "${NPM_TOKEN:-}" ] \
    || [ -n "${NPM_CONFIG__AUTH_TOKEN:-}" ] \
    || [ -n "${npm_config__authToken:-}" ]; then
    printf '%s\n' \
        'manual npm bootstrap refuses token environment variables; use npm login with interactive 2FA' >&2
    exit 2
fi

temporary_root=${TMPDIR:-/tmp}
artifacts=$(mktemp -d "$temporary_root/smells-npm-bootstrap.XXXXXX")
npm_user_config="$artifacts/npmrc"
logged_in=0

cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    if [ "$logged_in" -eq 1 ]; then
        if ! (
            cd "$artifacts"
            npm logout --registry "$registry" >/dev/null
        ); then
            printf '%s\n' \
                'npm logout failed; revoke the temporary CLI session from npm account settings' >&2
            if [ "$status" -eq 0 ]; then
                status=1
            fi
        fi
    fi
    rm -rf -- "$artifacts"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

NPM_CONFIG_USERCONFIG=$npm_user_config
NPM_CONFIG_GLOBALCONFIG=/dev/null
NPM_CONFIG_CACHE=$artifacts/npm-cache
XDG_CACHE_HOME=$artifacts/xdg-cache
export NPM_CONFIG_USERCONFIG NPM_CONFIG_GLOBALCONFIG NPM_CONFIG_CACHE XDG_CACHE_HOME

(
    cd "$artifacts"
    npm login --auth-type=web --registry "$registry"
)
logged_in=1

npm_identity=$(
    cd "$artifacts"
    npm whoami --registry "$registry"
)
if [ "$npm_identity" != mindfultime ]; then
    printf 'npm bootstrap requires identity mindfultime; authenticated as %s\n' \
        "$npm_identity" >&2
    exit 2
fi

gh release verify "$tag" --repo "$repository"
gh release download "$tag" --repo "$repository" --dir "$artifacts"

if [ ! -f "$artifacts/sha256.sum" ]; then
    printf 'release %s does not contain sha256.sum\n' "$tag" >&2
    exit 2
fi

(
    cd "$artifacts"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c sha256.sum
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c sha256.sum
    else
        printf '%s\n' 'sha256sum or shasum is required' >&2
        exit 2
    fi
)

(
    cd "$artifacts"
    node "$repository_root/npm/publish-packages.mjs" \
        "$artifacts" "$version" "$registry"
)

if ! (
    cd "$artifacts"
    npm logout --registry "$registry" >/dev/null
); then
    printf '%s\n' \
        'npm logout failed; revoke the temporary CLI session from npm account settings' >&2
    exit 1
fi
logged_in=0

printf '%s\n' \
    "npm bootstrap published and verified the exact $tag release tarballs." \
    'Configure Trusted Publishing for all six packages before the next release.'
