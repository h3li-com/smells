#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
prepare_diff="$repository_root/scripts/prepare-mutation-diff.sh"
workflow="$repository_root/.github/workflows/mutation.yml"
makefile="$repository_root/Makefile"

grep -F -- '- "scripts/prepare-mutation-diff.sh"' "$workflow" >/dev/null
grep -F -- '- "scripts/run-mutation-test.sh"' "$workflow" >/dev/null
grep -F 'shard: [0, 1, 2, 3, 4, 5, 6, 7]' "$workflow" >/dev/null
grep -F 'shard: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]' \
    "$workflow" >/dev/null
grep -F 'name: mutation-diff-results-${{ matrix.shard }}-of-8' "$workflow" >/dev/null
grep -F 'name: mutation-full-results-${{ matrix.shard }}-of-16' "$workflow" >/dev/null

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/smells-mutation-workflow.XXXXXX")
trap 'rm -rf "$temporary_root"' EXIT HUP INT TERM

fixture="$temporary_root/repository"
mkdir -p "$fixture/src" "$fixture/tests"
git -C "$fixture" init --quiet
git -C "$fixture" config user.email tests@smells.invalid
git -C "$fixture" config user.name "smells tests"
printf 'pub fn answer() -> u8 { 42 }\n' > "$fixture/src/lib.rs"
printf '# initial test\n' > "$fixture/tests/check.rs"
git -C "$fixture" add src/lib.rs tests/check.rs
git -C "$fixture" commit --quiet -m initial
base=$(git -C "$fixture" rev-parse HEAD)

printf '# test-only change\n' >> "$fixture/tests/check.rs"
git -C "$fixture" add tests/check.rs
git -C "$fixture" commit --quiet -m test-only
test_only_head=$(git -C "$fixture" rev-parse HEAD)
test_only_patch="$temporary_root/test-only.patch"
test_only_output=$(cd "$fixture" && \
    "$prepare_diff" "$base" "$test_only_head" "$test_only_patch")

test "$test_only_output" = 'has_mutable_source=false'
test ! -s "$test_only_patch"

printf 'pub fn answer() -> u8 { 43 }\n' > "$fixture/src/lib.rs"
git -C "$fixture" add src/lib.rs
git -C "$fixture" commit --quiet -m source-change
source_head=$(git -C "$fixture" rev-parse HEAD)
source_patch="$temporary_root/source.patch"
source_output=$(cd "$fixture" && \
    "$prepare_diff" "$test_only_head" "$source_head" "$source_patch")

test "$source_output" = 'has_mutable_source=true'
test -s "$source_patch"
grep -F 'src/lib.rs' "$source_patch" >/dev/null

mutation_command=$(make -n -f "$makefile" mutation-diff \
    MUTATION_DIFF="$source_patch" MUTATION_SHARD=3/8)
printf '%s\n' "$mutation_command" | grep -F 'MUTATION_IN_PLACE=false' >/dev/null
printf '%s\n' "$mutation_command" | grep -F 'CARGO_MUTANTS_JOBS="${CARGO_MUTANTS_JOBS:-4}"' >/dev/null
printf '%s\n' "$mutation_command" | grep -F -- '--copy-vcs true' >/dev/null
printf '%s\n' "$mutation_command" | grep -F -- '--shard 3/8' >/dev/null
printf '%s\n' "$mutation_command" | grep -F -- '--sharding round-robin' >/dev/null

full_mutation_command=$(make -n -f "$makefile" mutation-full MUTATION_SHARD=7/16)
printf '%s\n' "$full_mutation_command" | grep -F 'MUTATION_IN_PLACE=false' >/dev/null
printf '%s\n' "$full_mutation_command" | grep -F 'CARGO_MUTANTS_JOBS="${CARGO_MUTANTS_JOBS:-4}"' >/dev/null
printf '%s\n' "$full_mutation_command" | grep -F -- '--copy-vcs true' >/dev/null
printf '%s\n' "$full_mutation_command" | grep -F -- '--shard 7/16' >/dev/null
printf '%s\n' "$full_mutation_command" | grep -F -- '--sharding round-robin' >/dev/null

printf 'mutation workflow tests passed\n'
