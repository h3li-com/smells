#!/bin/sh
set -eu

if ! command -v jq >/dev/null 2>&1; then
    printf 'benchmark-release-gate.sh requires jq\n' >&2
    exit 2
fi

: "${SMELLS_PYTHON_ROOT:?set SMELLS_PYTHON_ROOT to the 1,397-file reference corpus}"
: "${SMELLS_TYPESCRIPT_ROOT:?set SMELLS_TYPESCRIPT_ROOT to the 875-file reference corpus}"

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
scanner="$repository_root/target/release/smells"
python_policy=${SMELLS_PYTHON_POLICY:-$repository_root/examples/python-quality-policy.json}
typescript_policy=${SMELLS_TYPESCRIPT_POLICY:-$repository_root/examples/typescript-quality-policy.json}
runs=${SMELLS_BENCHMARK_RUNS:-3}

case "$runs" in
    0|*[!0-9]*)
        printf 'SMELLS_BENCHMARK_RUNS must be a positive integer\n' >&2
        exit 2
        ;;
esac

benchmark_directory=$(mktemp -d "${TMPDIR:-/tmp}/smells-release-gate.XXXXXX")
trap 'rm -rf -- "$benchmark_directory"' EXIT HUP INT TERM

if [ "${SMELLS_SKIP_BUILD:-0}" != 1 ]; then
    cargo build --release --locked --manifest-path "$repository_root/Cargo.toml"
fi

run_scanner() {
    threads=$1
    cache=$2
    metrics=$3
    report=$4
    root=$5
    policy=$6
    timing=$7
    set +e
    if [ "$threads" = default ]; then
        SMELLS_CACHE_DIR=$cache SMELLS_METRICS_FILE=$metrics \
            /usr/bin/time -p "$scanner" check --path "$root" --policy "$policy" \
            --format json >"$report" 2>"$timing"
    else
        SMELLS_CACHE_DIR=$cache SMELLS_METRICS_FILE=$metrics \
            RAYON_NUM_THREADS=$threads /usr/bin/time -p "$scanner" check \
            --path "$root" --policy "$policy" --format json >"$report" 2>"$timing"
    fi
    status=$?
    set -e
    if [ "$status" -gt 1 ]; then
        printf 'benchmark scan failed with exit code %s\n' "$status" >&2
        sed -n '1,20p' "$timing" >&2
        exit "$status"
    fi
}

prime_cache() {
    threads=$1
    cache=$2
    root=$3
    policy=$4
    run_scanner "$threads" "$cache" "$benchmark_directory/prime-metrics.json" \
        /dev/null "$root" "$policy" "$benchmark_directory/prime-time"
}

run_case() {
    language=$1
    root=$2
    policy=$3
    expected_files=$4
    maximum_bytes=$5
    maximum_rss=$6
    maximum_seconds=$7
    cache_mode=$8
    threads=$9
    label="$language-$cache_mode-$threads"
    timings="$benchmark_directory/$label-timings"
    rss_values="$benchmark_directory/$label-rss"
    shared_cache="$benchmark_directory/$label-cache"
    report="$benchmark_directory/$label-report.json"

    if [ "$cache_mode" = warm ]; then
        prime_cache "$threads" "$shared_cache" "$root" "$policy"
    fi

    run=1
    while [ "$run" -le "$runs" ]; do
        if [ "$cache_mode" = cold ]; then
            cache="$benchmark_directory/$label-cache-$run"
        else
            cache=$shared_cache
        fi
        metrics="$benchmark_directory/$label-metrics-$run.json"
        timing="$benchmark_directory/$label-time-$run"
        run_scanner "$threads" "$cache" "$metrics" "$report" "$root" "$policy" "$timing"
        awk '$1 == "real" { print $2 }' "$timing" >>"$timings"
        jq -r '.peak_resident_bytes' "$metrics" >>"$rss_values"
        files=$(jq -r '.files' "$metrics")
        if [ "$files" -ne "$expected_files" ]; then
            printf '%s scanned %s files; expected %s\n' "$label" "$files" "$expected_files" >&2
            exit 1
        fi
        run=$((run + 1))
    done

    bytes=$(wc -c <"$report" | tr -d ' ')
    median=$(sort -n "$timings" | awk '
        { value[NR] = $1 }
        END {
            if (NR % 2) print value[(NR + 1) / 2];
            else print (value[NR / 2] + value[NR / 2 + 1]) / 2;
        }
    ')
    peak_rss=$(sort -n "$rss_values" | tail -1)
    printf '%s | median=%ss | peak_rss=%s | report_bytes=%s\n' \
        "$label" "$median" "$peak_rss" "$bytes"

    if [ "$bytes" -gt "$maximum_bytes" ] || [ "$peak_rss" -gt "$maximum_rss" ]; then
        printf '%s exceeded its report-size or memory ceiling\n' "$label" >&2
        exit 1
    fi
    if ! awk -v actual="$median" -v maximum="$maximum_seconds" \
        'BEGIN { exit !(actual <= maximum) }'; then
        printf '%s exceeded its %ss median runtime ceiling\n' "$label" "$maximum_seconds" >&2
        exit 1
    fi
}

for cache_mode in cold warm; do
    for threads in 1 4 default; do
        case "$threads" in
            1) python_seconds=15 ;;
            4) python_seconds=12 ;;
            default) python_seconds=10 ;;
        esac
        run_case python "$SMELLS_PYTHON_ROOT" "$python_policy" \
            1397 138500000 750000000 "$python_seconds" "$cache_mode" "$threads"
        run_case typescript "$SMELLS_TYPESCRIPT_ROOT" "$typescript_policy" \
            875 49375000 400000000 5 "$cache_mode" "$threads"
    done
done
