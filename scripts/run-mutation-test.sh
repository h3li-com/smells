#!/bin/sh
set -u

if [ "$#" -lt 1 ]; then
    printf 'usage: run-mutation-test.sh OUTPUT_DIRECTORY [CARGO_MUTANTS_ARGUMENT...]\n' >&2
    exit 2
fi

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
output_directory=$1
shift
cargo_command=${CARGO_COMMAND:-cargo}
python_command=${PYTHON_COMMAND:-python3}

mutation_status=0
case "${MUTATION_IN_PLACE:-true}" in
    true)
        PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 \
            "$cargo_command" mutants --in-place --output "$output_directory" "$@" ||
            mutation_status=$?
        ;;
    false)
        PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 \
            "$cargo_command" mutants --output "$output_directory" "$@" || mutation_status=$?
        ;;
    *)
        printf 'MUTATION_IN_PLACE must be true or false\n' >&2
        exit 2
        ;;
esac

outcomes="$output_directory/mutants.out/outcomes.json"
summary_status=0
if [ -f "$outcomes" ]; then
    "$python_command" "$script_directory/mutation-summary.py" "$outcomes" || summary_status=$?
elif [ "$mutation_status" -eq 0 ]; then
    printf 'mutation runner succeeded without writing %s\n' "$outcomes" >&2
    summary_status=2
fi

if [ "$summary_status" -ne 0 ]; then
    exit "$summary_status"
fi
exit "$mutation_status"
