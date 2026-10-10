#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
runner="$repository_root/scripts/run-mutation-test.sh"

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/smells-mutation-runner.XXXXXX")
trap 'rm -rf "$temporary_root"' EXIT HUP INT TERM

fake_cargo="$temporary_root/cargo"
cat > "$fake_cargo" <<'EOF'
#!/bin/sh
set -eu

output=
while [ "$#" -gt 0 ]; do
    if [ "$1" = --output ]; then
        output=$2
        shift 2
    else
        shift
    fi
done

if [ "${FAKE_MUTANTS_OUTCOME:-valid}" != missing ]; then
    mkdir -p "$output/mutants.out"
    if [ "${FAKE_MUTANTS_OUTCOME:-valid}" = invalid ]; then
        printf 'not JSON\n' > "$output/mutants.out/outcomes.json"
    else
        printf '%s\n' \
            '{"outcomes":[{"scenario":{"Mutant":{}},"summary":"CaughtMutant"}]}' \
            > "$output/mutants.out/outcomes.json"
    fi
fi
exit "${FAKE_MUTANTS_STATUS:-0}"
EOF
chmod +x "$fake_cargo"

run_case() {
    expected_status=$1
    outcome=$2
    mutation_status=$3
    output_directory="$temporary_root/$outcome-$mutation_status"
    actual_status=0
    FAKE_MUTANTS_OUTCOME=$outcome \
        FAKE_MUTANTS_STATUS=$mutation_status \
        CARGO_COMMAND=$fake_cargo \
        "$runner" "$output_directory" --file src/core/migration.rs \
        >/dev/null 2>&1 || actual_status=$?
    test "$actual_status" -eq "$expected_status"
}

run_case 0 valid 0
run_case 3 valid 3
run_case 3 missing 3
run_case 2 missing 0
run_case 2 invalid 3

printf 'mutation runner tests passed\n'
