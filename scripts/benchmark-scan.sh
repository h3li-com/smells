#!/bin/sh
set -eu

scan_path=${1:-.}
policy_path=${2:-quality-policy.json}
runs=${3:-5}
cache_mode=${4:-warm}
threads=${5:-default}

case "$runs" in
    0|*[!0-9]*)
        printf 'runs must be a positive integer\n' >&2
        exit 2
        ;;
esac
case "$cache_mode" in
    warm|cold) ;;
    *)
        printf 'cache mode must be warm or cold\n' >&2
        exit 2
        ;;
esac
case "$threads" in
    default) ;;
    0|*[!0-9]*)
        printf 'threads must be default or a positive integer\n' >&2
        exit 2
        ;;
esac

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
scanner="$repository_root/target/release/smells"
benchmark_directory=$(mktemp -d "${TMPDIR:-/tmp}/smells-benchmark.XXXXXX")
results_file="$benchmark_directory/timings"
timing_file="$benchmark_directory/time"
metrics_file="$benchmark_directory/metrics.json"
shared_cache="$benchmark_directory/cache"
trap 'rm -rf -- "$benchmark_directory"' EXIT HUP INT TERM

if [ "${SMELLS_SKIP_BUILD:-0}" != 1 ]; then
    cargo build --release --locked --manifest-path "$repository_root/Cargo.toml"
fi

prime_cache() {
    directory=$1
    set +e
    if [ "$threads" = default ]; then
        SMELLS_CACHE_DIR=$directory "$scanner" check \
            --path "$scan_path" --policy "$policy_path" --format json >/dev/null
    else
        SMELLS_CACHE_DIR=$directory RAYON_NUM_THREADS=$threads "$scanner" check \
            --path "$scan_path" --policy "$policy_path" --format json >/dev/null
    fi
    status=$?
    set -e
    if [ "$status" -gt 1 ]; then
        printf 'cache priming scan failed with exit code %s\n' "$status" >&2
        exit "$status"
    fi
}

if [ "$cache_mode" = warm ]; then
    prime_cache "$shared_cache"
    prime_cache "$benchmark_directory/metrics-cache"
fi

run=1
while [ "$run" -le "$runs" ]; do
    if [ "$cache_mode" = cold ]; then
        cache_directory="$benchmark_directory/cache-$run"
        metrics_cache_directory="$benchmark_directory/metrics-cache-$run"
    else
        cache_directory=$shared_cache
        metrics_cache_directory="$benchmark_directory/metrics-cache"
    fi
    set +e
    if [ "$threads" = default ]; then
        SMELLS_CACHE_DIR=$cache_directory /usr/bin/time -p "$scanner" check \
            --path "$scan_path" \
            --policy "$policy_path" \
            --format json \
            >/dev/null 2>"$timing_file"
    else
        SMELLS_CACHE_DIR=$cache_directory RAYON_NUM_THREADS=$threads \
            /usr/bin/time -p "$scanner" check \
            --path "$scan_path" \
            --policy "$policy_path" \
            --format json \
            >/dev/null 2>"$timing_file"
    fi
    status=$?
    set -e
    if [ "$status" -gt 1 ]; then
        printf 'scan failed with exit code %s\n' "$status" >&2
        exit "$status"
    fi
    set +e
    if [ "$threads" = default ]; then
        SMELLS_CACHE_DIR=$metrics_cache_directory SMELLS_METRICS_FILE=$metrics_file \
            "$scanner" check --path "$scan_path" --policy "$policy_path" --format json \
            >/dev/null
    else
        SMELLS_CACHE_DIR=$metrics_cache_directory SMELLS_METRICS_FILE=$metrics_file \
            RAYON_NUM_THREADS=$threads "$scanner" check \
            --path "$scan_path" --policy "$policy_path" --format json >/dev/null
    fi
    metrics_status=$?
    set -e
    if [ "$metrics_status" -gt 1 ]; then
        printf 'counter scan failed with exit code %s\n' "$metrics_status" >&2
        exit "$metrics_status"
    fi
    elapsed=$(awk '$1 == "real" { print $2 }' "$timing_file")
    if [ -z "$elapsed" ]; then
        printf 'could not read elapsed time\n' >&2
        exit 2
    fi
    printf '%s\n' "$elapsed" >>"$results_file"
    metrics=$(tr -d '\n' <"$metrics_file")
    printf 'run %s: %ss | counters: %s\n' "$run" "$elapsed" "$metrics"
    run=$((run + 1))
done

sort -n "$results_file" | awk '
    { values[NR] = $1; total += $1 }
    END {
        if (NR % 2 == 1) {
            median = values[(NR + 1) / 2]
        } else {
            median = (values[NR / 2] + values[NR / 2 + 1]) / 2
        }
        printf "median: %.3fs | min: %.3fs | max: %.3fs | mean: %.3fs\n", \
            median, values[1], values[NR], total / NR
    }
'
