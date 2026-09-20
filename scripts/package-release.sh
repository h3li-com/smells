#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
invocation_directory=$PWD

target=${1:?usage: package-release.sh TARGET BINARY OUTPUT_DIRECTORY}
binary=${2:?usage: package-release.sh TARGET BINARY OUTPUT_DIRECTORY}
output=${3:?usage: package-release.sh TARGET BINARY OUTPUT_DIRECTORY}

case "$binary" in
    /*) ;;
    *) binary="$invocation_directory/$binary" ;;
esac
case "$output" in
    /*) ;;
    *) output="$invocation_directory/$output" ;;
esac

if [ ! -f "$binary" ]; then
    printf 'release binary does not exist: %s\n' "$binary" >&2
    exit 2
fi

mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd)
archive_name="smells-$target"
temporary_root=${RUNNER_TEMP:-/tmp}
if command -v cygpath >/dev/null 2>&1; then
    temporary_root=$(cygpath -u "$temporary_root")
fi
temporary=$(mktemp -d "$temporary_root/smells-release.XXXXXX")
trap 'rm -rf -- "$temporary"' EXIT HUP INT TERM
staging="$temporary/$archive_name"
mkdir -p "$staging/examples" "$staging/schemas"
cp "$binary" "$staging/"
cp "$repository_root/CHANGELOG.md" "$repository_root/README.md" "$staging/"
cp "$repository_root"/examples/*.json "$staging/examples/"
cp "$repository_root"/schemas/*.json "$staging/schemas/"

case "$target" in
    *-pc-windows-*)
        archive="$archive_name.zip"
        (
            cd "$temporary"
            7z a -bd -tzip "$output/$archive" "$archive_name" >/dev/null
            cd "$output"
            sha256sum "$archive" >"$archive.sha256"
        )
        ;;
    *)
        archive="$archive_name.tar.gz"
        (
            tar -C "$temporary" -czf "$output/$archive" "$archive_name"
            cd "$output"
            if command -v sha256sum >/dev/null 2>&1; then
                sha256sum "$archive" >"$archive.sha256"
            else
                shasum -a 256 "$archive" >"$archive.sha256"
            fi
        )
        ;;
esac

printf '%s/%s\n' "$output" "$archive"
