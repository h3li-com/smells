#!/bin/sh
set -eu

input=${1:?usage: collect-release-artifacts.sh INPUT_DIRECTORY OUTPUT_DIRECTORY}
output=${2:?usage: collect-release-artifacts.sh INPUT_DIRECTORY OUTPUT_DIRECTORY}

if [ ! -d "$input" ]; then
    printf 'release artifact input directory does not exist: %s\n' "$input" >&2
    exit 2
fi

input=$(CDPATH= cd -- "$input" && pwd)
mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd)

case "$output/" in
    "$input/" | "$input/"*)
        printf 'release artifact output must be outside the input directory\n' >&2
        exit 2
        ;;
esac

if find "$output" -mindepth 1 -maxdepth 1 -print -quit | grep -q .; then
    printf 'release artifact output directory is not empty: %s\n' "$output" >&2
    exit 2
fi

temporary_root=${RUNNER_TEMP:-${TMPDIR:-/tmp}}
manifest=$(mktemp "$temporary_root/smells-release-artifacts.XXXXXX")
trap 'rm -f -- "$manifest"' EXIT HUP INT TERM

find "$input" -type f \( \
    -name '*.whl' -o \
    -name '*.tar.gz' -o \
    -name '*.zip' -o \
    -name '*.sha256' \
\) -print | LC_ALL=C sort > "$manifest"

if [ ! -s "$manifest" ]; then
    printf 'no release artifacts found under: %s\n' "$input" >&2
    exit 2
fi

count=0
while IFS= read -r artifact; do
    name=${artifact##*/}
    destination="$output/$name"
    if [ -e "$destination" ]; then
        printf 'duplicate release artifact basename: %s\n' "$name" >&2
        exit 2
    fi
    cp "$artifact" "$destination"
    count=$((count + 1))
done < "$manifest"

printf 'collected %s release artifacts in %s\n' "$count" "$output"
