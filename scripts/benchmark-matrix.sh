#!/bin/sh
set -eu

runs=${1:-3}
case "$runs" in
    0|*[!0-9]*)
        printf 'runs must be a positive integer\n' >&2
        exit 2
        ;;
esac
if ! command -v jq >/dev/null 2>&1; then
    printf 'benchmark-matrix.sh requires jq\n' >&2
    exit 2
fi

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
matrix_directory=$(mktemp -d "${TMPDIR:-/tmp}/smells-matrix.XXXXXX")
trap 'rm -rf -- "$matrix_directory"' EXIT HUP INT TERM

cargo build --release --locked --manifest-path "$repository_root/Cargo.toml"
export SMELLS_SKIP_BUILD=1
matrix_languages=${SMELLS_MATRIX_LANGUAGES:-"rust python typescript"}
matrix_shapes=${SMELLS_MATRIX_SHAPES:-"no-clone clone-heavy short long"}
matrix_thresholds=${SMELLS_MATRIX_THRESHOLDS:-"5000 6000 7000 8200 9000 9500"}
matrix_cache_modes=${SMELLS_MATRIX_CACHE_MODES:-"cold warm"}
matrix_threads=${SMELLS_MATRIX_THREADS:-"1 4 default"}

write_policy() {
    language=$1
    threshold=$2
    output=$3
    case "$language" in
        rust) template="$repository_root/examples/quality-policy.json" ;;
        python) template="$repository_root/examples/python-quality-policy.json" ;;
        typescript) template="$repository_root/examples/typescript-quality-policy.json" ;;
    esac
    rule="$language.duplicate_functions"
    jq --arg rule "$rule" --argjson threshold "$threshold" '
        .rules |= with_entries(.value.mode = "off")
        | .rules[$rule].mode = "report"
        | .rules[$rule].parameters.minimum_tokens = 4
        | .rules[$rule].parameters.minimum_similarity_basis_points = $threshold
        | .limits.maximum_pairs = 1000000
    ' "$template" >"$output"
}

operator() {
    value=$1
    step=$2
    divisor=1
    position=1
    while [ "$position" -lt "$step" ] && [ "$divisor" -le "$value" ]; do
        divisor=$((divisor * 4))
        position=$((position + 1))
    done
    if [ "$divisor" -gt "$value" ]; then
        selector=0
    else
        selector=$((value / divisor % 4))
    fi
    case "$selector" in
        0) printf '+' ;;
        1) printf '-' ;;
        2) printf '*' ;;
        3) printf '/' ;;
    esac
}

write_corpus() {
    language=$1
    shape=$2
    directory=$3
    mkdir -p "$directory"
    case "$shape" in
        short) functions=64; operations=4; clone=0 ;;
        long) functions=64; operations=40; clone=0 ;;
        no-clone) functions=24; operations=3; clone=0 ;;
        clone-heavy) functions=64; operations=12; clone=1 ;;
    esac
    case "$language" in
        rust) file="$directory/corpus.rs" ;;
        python) file="$directory/corpus.py" ;;
        typescript) file="$directory/corpus.ts" ;;
    esac
    : >"$file"
    function_index=0
    pattern_index=0
    while [ "$function_index" -lt "$functions" ]; do
        if [ "$shape" = no-clone ]; then
            first=$(operator "$pattern_index" 1)
            second=$(operator "$pattern_index" 2)
            third=$(operator "$pattern_index" 3)
            pattern_index=$((pattern_index + 1))
            if [ "$first" = "$second" ] || [ "$first" = "$third" ] \
                || [ "$second" = "$third" ]; then
                continue
            fi
            case "$language" in
                rust) printf 'fn function_%s(value: f64) -> f64 { value %s value %s value %s value }\n' \
                    "$function_index" "$first" "$second" "$third" >>"$file" ;;
                python) printf 'def function_%s(value):\n    return value %s value %s value %s value\n\n' \
                    "$function_index" "$first" "$second" "$third" >>"$file" ;;
                typescript) printf 'function function_%s(value: number): number { return value %s value %s value %s value; }\n' \
                    "$function_index" "$first" "$second" "$third" >>"$file" ;;
            esac
            function_index=$((function_index + 1))
            continue
        fi
        pattern_index=$function_index
        if [ "$clone" -eq 1 ]; then pattern_index=0; fi
        case "$language" in
            rust) printf 'fn function_%s(value: f64) -> f64 {\n    let mut result = value;\n' "$function_index" >>"$file" ;;
            python) printf 'def function_%s(value):\n    result = value\n' "$function_index" >>"$file" ;;
            typescript) printf 'function function_%s(value: number): number {\n  let result = value;\n' "$function_index" >>"$file" ;;
        esac
        step=1
        while [ "$step" -le "$operations" ]; do
            symbol=$(operator "$pattern_index" "$step")
            case "$language" in
                rust) printf '    result = result %s value;\n' "$symbol" >>"$file" ;;
                python) printf '    result = result %s value\n' "$symbol" >>"$file" ;;
                typescript) printf '  result = result %s value;\n' "$symbol" >>"$file" ;;
            esac
            step=$((step + 1))
        done
        case "$language" in
            rust) printf '    result\n}\n' >>"$file" ;;
            python) printf '    return result\n\n' >>"$file" ;;
            typescript) printf '  return result;\n}\n' >>"$file" ;;
        esac
        function_index=$((function_index + 1))
    done
}

assert_no_clone_corpus() {
    language=$1
    corpus=$2
    policy="$matrix_directory/no-clone-$language-policy.json"
    report="$matrix_directory/no-clone-$language-report.json"
    cache="$matrix_directory/no-clone-$language-cache"
    write_policy "$language" 5000 "$policy"
    set +e
    SMELLS_CACHE_DIR=$cache "$repository_root/target/release/smells" check \
        --path "$corpus" --policy "$policy" --format json >"$report"
    status=$?
    set -e
    if [ "$status" -gt 1 ]; then
        printf 'no-clone corpus validation failed for %s with exit code %s\n' \
            "$language" "$status" >&2
        exit "$status"
    fi
    rule="$language.duplicate_functions"
    matches=$(jq --arg rule "$rule" \
        '[.findings[] | select(.rule_id == $rule)] | length' "$report")
    if [ "$matches" -ne 0 ]; then
        printf 'no-clone corpus produced %s duplicate findings for %s\n' \
            "$matches" "$language" >&2
        exit 2
    fi
}

run_case() {
    label=$1
    language=$2
    root=$3
    base_policy=$4
    threshold=$5
    cache_mode=$6
    threads=$7
    policy="$matrix_directory/policy-$language-$threshold.json"
    if [ "$base_policy" = generated ]; then
        write_policy "$language" "$threshold" "$policy"
    else
        rule="$language.duplicate_functions"
        jq --arg rule "$rule" --argjson threshold "$threshold" '
            .rules |= with_entries(.value.mode = "off")
            | .rules[$rule].mode = "report"
            | .rules[$rule].parameters.minimum_similarity_basis_points = $threshold
        ' "$base_policy" >"$policy"
    fi
    printf '\ncase=%s language=%s threshold=%s cache=%s threads=%s\n' \
        "$label" "$language" "$threshold" "$cache_mode" "$threads"
    "$repository_root/scripts/benchmark-scan.sh" \
        "$root" "$policy" "$runs" "$cache_mode" "$threads"
}

for language in $matrix_languages; do
    for shape in $matrix_shapes; do
        corpus="$matrix_directory/$language-$shape"
        write_corpus "$language" "$shape" "$corpus"
        if [ "$shape" = no-clone ]; then
            assert_no_clone_corpus "$language" "$corpus"
        fi
        for threshold in $matrix_thresholds; do
            for cache_mode in $matrix_cache_modes; do
                for threads in $matrix_threads; do
                    run_case "$shape" "$language" "$corpus" generated \
                        "$threshold" "$cache_mode" "$threads"
                done
            done
        done
    done
done

# Supply all six variables to include the real repositories used for live scans.
if [ -n "${SMELLS_RUST_ROOT:-}" ] && [ -n "${SMELLS_RUST_POLICY:-}" ] \
    && [ -n "${SMELLS_PYTHON_ROOT:-}" ] && [ -n "${SMELLS_PYTHON_POLICY:-}" ] \
    && [ -n "${SMELLS_TYPESCRIPT_ROOT:-}" ] && [ -n "${SMELLS_TYPESCRIPT_POLICY:-}" ]; then
    for threshold in $matrix_thresholds; do
        for cache_mode in $matrix_cache_modes; do
            for threads in $matrix_threads; do
                run_case real-rust rust "$SMELLS_RUST_ROOT" "$SMELLS_RUST_POLICY" \
                    "$threshold" "$cache_mode" "$threads"
                run_case real-python python "$SMELLS_PYTHON_ROOT" "$SMELLS_PYTHON_POLICY" \
                    "$threshold" "$cache_mode" "$threads"
                run_case real-typescript typescript "$SMELLS_TYPESCRIPT_ROOT" \
                    "$SMELLS_TYPESCRIPT_POLICY" "$threshold" "$cache_mode" "$threads"
            done
        done
    done
fi
