#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
    printf 'usage: prepare-mutation-diff.sh BASE_SHA HEAD_SHA PATCH_PATH\n' >&2
    exit 2
fi

base_sha=$1
head_sha=$2
patch_path=$3

if git diff --quiet "$base_sha...$head_sha" -- ':(glob)src/**/*.rs'; then
    : > "$patch_path"
    printf 'has_mutable_source=false\n'
    exit 0
else
    diff_status=$?
    if [ "$diff_status" -ne 1 ]; then
        exit "$diff_status"
    fi
fi

git diff --binary "$base_sha...$head_sha" > "$patch_path"
printf 'has_mutable_source=true\n'
