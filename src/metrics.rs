use serde::Serialize;
use std::{
    ffi::OsString,
    fs,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Counter {
    Files,
    SyntaxNodes,
    SyntaxTokens,
    Functions,
    NormalizedTokens,
    EligibleFingerprints,
    ExpandedFeatures,
    PrefixLookups,
    PostingEntriesVisited,
    UniqueCandidates,
    ExactComparisons,
    FeatureComparisons,
    RejectedByLength,
    RejectedByPosition,
    RejectedByEarlyVerification,
    MatchingPairs,
    FactCacheHits,
    FactCacheMisses,
}

const COUNTER_COUNT: usize = 18;
static COUNTERS: [AtomicU64; COUNTER_COUNT] = [const { AtomicU64::new(0) }; COUNTER_COUNT];
static METRICS_FILE: OnceLock<Option<OsString>> = OnceLock::new();

fn output_path() -> Option<&'static OsString> {
    METRICS_FILE
        .get_or_init(|| std::env::var_os("SMELLS_METRICS_FILE"))
        .as_ref()
}

pub fn enabled() -> bool {
    output_path().is_some()
}

pub fn reset() {
    if enabled() {
        for counter in &COUNTERS {
            counter.store(0, Ordering::Relaxed);
        }
    }
}

pub fn add(counter: Counter, value: usize) {
    if enabled() {
        COUNTERS[counter as usize].fetch_add(value as u64, Ordering::Relaxed);
    }
}

fn get(counter: Counter) -> u64 {
    COUNTERS[counter as usize].load(Ordering::Relaxed)
}

#[cfg(target_os = "macos")]
fn peak_resident_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage initializes the supplied rusage structure when it succeeds.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
        // SAFETY: the successful call above initialized usage.
        unsafe { usage.assume_init() }.ru_maxrss.max(0) as u64
    } else {
        0
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn peak_resident_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage initializes the supplied rusage structure when it succeeds.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
        // SAFETY: Linux and the supported Unix targets report ru_maxrss in KiB.
        (unsafe { usage.assume_init() }.ru_maxrss.max(0) as u64).saturating_mul(1024)
    } else {
        0
    }
}

#[cfg(not(unix))]
fn peak_resident_bytes() -> u64 {
    0
}

#[derive(Serialize)]
struct Snapshot {
    schema_version: u64,
    files: u64,
    syntax_nodes: u64,
    syntax_tokens: u64,
    functions: u64,
    normalized_tokens: u64,
    eligible_fingerprints: u64,
    expanded_features: u64,
    prefix_lookups: u64,
    posting_entries_visited: u64,
    unique_candidates: u64,
    exact_comparisons_charged: u64,
    feature_comparisons: u64,
    candidates_rejected_by_length: u64,
    candidates_rejected_by_position: u64,
    candidates_rejected_by_early_verification: u64,
    matching_pairs: u64,
    fact_cache_hits: u64,
    fact_cache_misses: u64,
    json_bytes: usize,
    peak_resident_bytes: u64,
}

pub fn write(json_bytes: usize) -> Result<(), String> {
    let Some(path) = output_path() else {
        return Ok(());
    };
    let snapshot = Snapshot {
        schema_version: 2,
        files: get(Counter::Files),
        syntax_nodes: get(Counter::SyntaxNodes),
        syntax_tokens: get(Counter::SyntaxTokens),
        functions: get(Counter::Functions),
        normalized_tokens: get(Counter::NormalizedTokens),
        eligible_fingerprints: get(Counter::EligibleFingerprints),
        expanded_features: get(Counter::ExpandedFeatures),
        prefix_lookups: get(Counter::PrefixLookups),
        posting_entries_visited: get(Counter::PostingEntriesVisited),
        unique_candidates: get(Counter::UniqueCandidates),
        exact_comparisons_charged: get(Counter::ExactComparisons),
        feature_comparisons: get(Counter::FeatureComparisons),
        candidates_rejected_by_length: get(Counter::RejectedByLength),
        candidates_rejected_by_position: get(Counter::RejectedByPosition),
        candidates_rejected_by_early_verification: get(Counter::RejectedByEarlyVerification),
        matching_pairs: get(Counter::MatchingPairs),
        fact_cache_hits: get(Counter::FactCacheHits),
        fact_cache_misses: get(Counter::FactCacheMisses),
        json_bytes,
        peak_resident_bytes: peak_resident_bytes(),
    };
    let bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| format!("cannot serialize scan metrics: {error}"))?;
    fs::write(path, bytes).map_err(|error| format!("cannot write scan metrics: {error}"))
}
