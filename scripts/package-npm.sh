#!/bin/sh
set -eu

target=${1:?usage: package-npm.sh TARGET|root BINARY|- OUTPUT_DIRECTORY VERSION}
binary=${2:?usage: package-npm.sh TARGET|root BINARY|- OUTPUT_DIRECTORY VERSION}
output=${3:?usage: package-npm.sh TARGET|root BINARY|- OUTPUT_DIRECTORY VERSION}
version=${4:?usage: package-npm.sh TARGET|root BINARY|- OUTPUT_DIRECTORY VERSION}

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)

exec node "$repository_root/npm/build-package.mjs" \
    "$target" "$binary" "$output" "$version"
