#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)

tag=${1:-}
if ! printf '%s\n' "$tag" \
    | grep -Eq '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
    printf 'release tag must be exactly vMAJOR.MINOR.PATCH\n' >&2
    exit 2
fi

version=$(sed -n '/^\[package\]/,/^\[/ s/^version = "\([^"]*\)"/\1/p' \
    "$repository_root/Cargo.toml")
if [ -z "$version" ]; then
    printf 'could not read the Cargo package version\n' >&2
    exit 2
fi
if [ "$tag" != "v$version" ]; then
    printf 'release tag %s does not match Cargo package version %s\n' "$tag" "$version" >&2
    exit 2
fi

printf '%s\n' "$version"
