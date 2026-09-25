use crate::metrics::{self, Counter};
use std::collections::{BTreeMap, BTreeSet};

type Shingle = [usize; 4];
type OccurrenceFeature = (Shingle, usize);

struct Fingerprint {
    runs: Vec<(Shingle, usize)>,
    size: usize,
}

pub struct SimilarPair {
    pub left: usize,
    pub right: usize,
    pub intersection: usize,
    pub union: usize,
}

#[cfg(test)]
fn reference_jaccard_pairs(
    token_lists: &[Vec<String>],
    minimum_tokens: u64,
    similarity: u64,
    maximum_pairs: usize,
    overlaps: &impl Fn(usize, usize) -> bool,
) -> Result<Vec<SimilarPair>, String> {
    let fingerprints = token_lists
        .iter()
        .map(|tokens| {
            let mut counts = BTreeMap::new();
            for window in tokens.windows(4) {
                let shingle: [String; 4] = std::array::from_fn(|index| window[index].clone());
                *counts.entry(shingle).or_default() += 1;
            }
            counts
        })
        .collect::<Vec<_>>();
    let mut eligible = token_lists
        .iter()
        .enumerate()
        .filter(|(_, tokens)| tokens.len() as u64 >= minimum_tokens && tokens.len() >= 4)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    eligible.sort_by_key(|index| (fingerprints[*index].values().sum::<usize>(), *index));

    let mut comparisons = 0;
    let mut matches = Vec::new();
    for (position, right) in eligible.iter().copied().enumerate() {
        let mut left_indices = eligible[..position].to_vec();
        left_indices.sort_unstable();
        for left in left_indices {
            if overlaps(left, right) {
                continue;
            }
            comparisons += 1;
            if comparisons > maximum_pairs {
                return Err(pair_budget_error(comparisons, maximum_pairs));
            }
            let left_counts = &fingerprints[left];
            let right_counts = &fingerprints[right];
            let intersection = left_counts
                .iter()
                .map(|(shingle, count)| count.min(right_counts.get(shingle).unwrap_or(&0)))
                .sum::<usize>();
            let union = left_counts.values().sum::<usize>() + right_counts.values().sum::<usize>()
                - intersection;
            if union > 0 && intersection as u128 * 10_000 >= similarity as u128 * union as u128 {
                matches.push(SimilarPair {
                    left,
                    right,
                    intersection,
                    union,
                });
            }
        }
    }
    Ok(matches)
}

fn pair_budget_error(charged: usize, maximum_pairs: usize) -> String {
    format!(
        "maximum_pairs budget exceeded: charged {charged} exact comparisons; policy limit is {maximum_pairs}"
    )
}

#[cfg(test)]
thread_local! {
    static USE_REFERENCE_JOIN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn with_reference_join<T>(operation: impl FnOnce() -> T) -> T {
    USE_REFERENCE_JOIN.with(|enabled| {
        let previous = enabled.replace(true);
        let result = operation();
        enabled.set(previous);
        result
    })
}

struct CandidateAccumulator {
    generations: Vec<u32>,
    generation: u32,
    touched: Vec<usize>,
}

impl CandidateAccumulator {
    fn new(size: usize) -> Self {
        Self {
            generations: vec![0; size],
            generation: 0,
            touched: Vec::new(),
        }
    }

    fn begin(&mut self) {
        self.touched.clear();
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.generations.fill(0);
            self.generation = 1;
        }
    }

    fn insert(&mut self, candidate: usize) -> bool {
        if self.generations[candidate] != self.generation {
            self.generations[candidate] = self.generation;
            self.touched.push(candidate);
            true
        } else {
            false
        }
    }

    fn contains(&self, candidate: usize) -> bool {
        self.generations[candidate] == self.generation
    }
}

fn token_ids(token_lists: &[Vec<String>]) -> BTreeMap<&str, usize> {
    token_lists
        .iter()
        .flat_map(|tokens| tokens.iter().map(String::as_str))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(id, token)| (token, id))
        .collect()
}

fn fingerprints(token_lists: &[Vec<String>]) -> Vec<Fingerprint> {
    let ids = token_ids(token_lists);
    token_lists
        .iter()
        .map(|tokens| {
            let mut counts = BTreeMap::new();
            for window in tokens.windows(4) {
                let shingle = std::array::from_fn(|index| ids[window[index].as_str()]);
                *counts.entry(shingle).or_default() += 1;
            }
            let size = counts.values().sum();
            Fingerprint {
                runs: counts.into_iter().collect(),
                size,
            }
        })
        .collect()
}

fn feature_ids(
    token_lists: &[Vec<String>],
    fingerprints: &[Fingerprint],
    minimum_tokens: u64,
) -> (BTreeMap<OccurrenceFeature, usize>, Vec<usize>) {
    let mut frequencies = BTreeMap::new();
    for (index, fingerprint) in fingerprints.iter().enumerate() {
        if token_lists[index].len() as u64 >= minimum_tokens {
            for (shingle, count) in &fingerprint.runs {
                for occurrence in 0..*count {
                    *frequencies.entry((*shingle, occurrence)).or_default() += 1;
                }
            }
        }
    }
    let mut ids = BTreeMap::new();
    let mut ordered_frequencies = Vec::with_capacity(frequencies.len());
    for (id, (feature, frequency)) in frequencies.into_iter().enumerate() {
        ids.insert(feature, id);
        ordered_frequencies.push(frequency);
    }
    (ids, ordered_frequencies)
}

fn ordered_features(
    token_lists: &[Vec<String>],
    fingerprints: &[Fingerprint],
    minimum_tokens: u64,
) -> (Vec<Vec<usize>>, usize) {
    let (ids, frequencies) = feature_ids(token_lists, fingerprints, minimum_tokens);
    let records = fingerprints
        .iter()
        .enumerate()
        .map(|(index, fingerprint)| {
            if (token_lists[index].len() as u64) < minimum_tokens {
                return Vec::new();
            }
            let mut features = fingerprint
                .runs
                .iter()
                .flat_map(|(shingle, count)| {
                    (0..*count).map(|occurrence| ids[&(*shingle, occurrence)])
                })
                .collect::<Vec<_>>();
            features.sort_by_key(|id| (frequencies[*id], *id));
            features
        })
        .collect();
    (records, frequencies.len())
}

fn probing_prefix_length(size: usize, similarity: u64) -> usize {
    let required = (similarity as u128 * size as u128).div_ceil(10_000) as usize;
    size - required + 1
}

fn indexed_prefix_length(size: usize, similarity: u64) -> usize {
    (((10_000 - similarity) as u128 * size as u128) / (10_000 + similarity) as u128) as usize + 1
}

enum CandidateDecision {
    Possible,
    RejectedByLength,
    RejectedByPosition,
}

fn candidate_decision(
    left_size: usize,
    right_size: usize,
    left_position: usize,
    right_position: usize,
    similarity: u64,
) -> CandidateDecision {
    if left_size as u128 * 10_000 < similarity as u128 * right_size as u128 {
        return CandidateDecision::RejectedByLength;
    }
    let maximum_overlap = 1 + (left_size - left_position - 1).min(right_size - right_position - 1);
    let required_overlap = (similarity as u128 * (left_size + right_size) as u128)
        .div_ceil(10_000 + similarity as u128);
    if maximum_overlap >= required_overlap as usize {
        CandidateDecision::Possible
    } else {
        CandidateDecision::RejectedByPosition
    }
}

fn exact_similarity(
    left: &Fingerprint,
    right: &Fingerprint,
    similarity: u64,
) -> Option<(usize, usize)> {
    let required = (similarity as u128 * (left.size + right.size) as u128)
        .div_ceil(10_000 + similarity as u128) as usize;
    let (mut left_index, mut right_index, mut intersection) = (0, 0, 0);
    let (mut left_remaining, mut right_remaining) = (left.size, right.size);
    while left_index < left.runs.len() && right_index < right.runs.len() {
        metrics::add(Counter::FeatureComparisons, 1);
        let (left_key, left_count) = left.runs[left_index];
        let (right_key, right_count) = right.runs[right_index];
        match left_key.cmp(&right_key) {
            std::cmp::Ordering::Less => {
                left_remaining -= left_count;
                left_index += 1;
            }
            std::cmp::Ordering::Greater => {
                right_remaining -= right_count;
                right_index += 1;
            }
            std::cmp::Ordering::Equal => {
                intersection += left_count.min(right_count);
                left_remaining -= left_count;
                right_remaining -= right_count;
                left_index += 1;
                right_index += 1;
            }
        }
        if intersection + left_remaining.min(right_remaining) < required {
            metrics::add(Counter::RejectedByEarlyVerification, 1);
            return None;
        }
    }
    if intersection < required {
        metrics::add(Counter::RejectedByEarlyVerification, 1);
        return None;
    }
    Some((intersection, left.size + right.size - intersection))
}

struct JoinState {
    postings: Vec<Vec<(usize, usize)>>,
    candidates: CandidateAccumulator,
    seen: CandidateAccumulator,
    rejected_by_length: CandidateAccumulator,
    comparisons: usize,
    matches: Vec<SimilarPair>,
}

impl JoinState {
    fn new(record_count: usize, feature_count: usize) -> Self {
        Self {
            postings: vec![Vec::new(); feature_count],
            candidates: CandidateAccumulator::new(record_count),
            seen: CandidateAccumulator::new(record_count),
            rejected_by_length: CandidateAccumulator::new(record_count),
            comparisons: 0,
            matches: Vec::new(),
        }
    }

    fn begin_record(&mut self) {
        self.candidates.begin();
        self.seen.begin();
        self.rejected_by_length.begin();
    }

    fn collect_candidates(
        &mut self,
        right: usize,
        ordered: &[Vec<usize>],
        similarity: u64,
        overlaps: &impl Fn(usize, usize) -> bool,
    ) {
        let right_size = ordered[right].len();
        for (right_position, feature) in ordered[right]
            .iter()
            .take(probing_prefix_length(right_size, similarity))
            .enumerate()
        {
            metrics::add(Counter::PrefixLookups, 1);
            for (left, left_position) in &self.postings[*feature] {
                metrics::add(Counter::PostingEntriesVisited, 1);
                if overlaps(*left, right) {
                    continue;
                }
                self.seen.insert(*left);
                match candidate_decision(
                    ordered[*left].len(),
                    right_size,
                    *left_position,
                    right_position,
                    similarity,
                ) {
                    CandidateDecision::Possible => {
                        self.candidates.insert(*left);
                    }
                    CandidateDecision::RejectedByLength => {
                        self.rejected_by_length.insert(*left);
                    }
                    CandidateDecision::RejectedByPosition => {}
                }
            }
        }
    }

    fn record_filter_metrics(&self) {
        metrics::add(Counter::UniqueCandidates, self.candidates.touched.len());
        metrics::add(
            Counter::RejectedByLength,
            self.rejected_by_length.touched.len(),
        );
        metrics::add(
            Counter::RejectedByPosition,
            self.seen
                .touched
                .iter()
                .filter(|candidate| {
                    !self.candidates.contains(**candidate)
                        && !self.rejected_by_length.contains(**candidate)
                })
                .count(),
        );
    }

    fn verify_candidates(
        &mut self,
        right: usize,
        fingerprints: &[Fingerprint],
        similarity: u64,
        maximum_pairs: usize,
    ) -> Result<(), String> {
        self.candidates.touched.sort_unstable();
        for index in 0..self.candidates.touched.len() {
            let left = self.candidates.touched[index];
            self.comparisons += 1;
            metrics::add(Counter::ExactComparisons, 1);
            if self.comparisons > maximum_pairs {
                return Err(pair_budget_error(self.comparisons, maximum_pairs));
            }
            if let Some((intersection, union)) =
                exact_similarity(&fingerprints[left], &fingerprints[right], similarity)
            {
                metrics::add(Counter::MatchingPairs, 1);
                self.matches.push(SimilarPair {
                    left,
                    right,
                    intersection,
                    union,
                });
            }
        }
        Ok(())
    }

    fn index_record(&mut self, right: usize, ordered: &[Vec<usize>], similarity: u64) {
        let right_size = ordered[right].len();
        for (position, feature) in ordered[right]
            .iter()
            .take(indexed_prefix_length(right_size, similarity))
            .enumerate()
        {
            self.postings[*feature].push((right, position));
        }
    }
}

pub fn exact_jaccard_pairs(
    token_lists: &[Vec<String>],
    minimum_tokens: u64,
    similarity: u64,
    maximum_pairs: usize,
    overlaps: impl Fn(usize, usize) -> bool,
) -> Result<Vec<SimilarPair>, String> {
    #[cfg(test)]
    if USE_REFERENCE_JOIN.with(std::cell::Cell::get) {
        return reference_jaccard_pairs(
            token_lists,
            minimum_tokens,
            similarity,
            maximum_pairs,
            &overlaps,
        );
    }
    let fingerprints = fingerprints(token_lists);
    let (ordered, feature_count) = ordered_features(token_lists, &fingerprints, minimum_tokens);
    let mut eligible = (0..ordered.len())
        .filter(|index| !ordered[*index].is_empty())
        .collect::<Vec<_>>();
    eligible.sort_by_key(|index| (ordered[*index].len(), *index));
    metrics::add(Counter::EligibleFingerprints, eligible.len());
    metrics::add(
        Counter::ExpandedFeatures,
        ordered.iter().map(Vec::len).sum(),
    );

    let mut join = JoinState::new(ordered.len(), feature_count);
    for right in eligible {
        join.begin_record();
        join.collect_candidates(right, &ordered, similarity, &overlaps);
        join.record_filter_metrics();
        join.verify_candidates(right, &fingerprints, similarity, maximum_pairs)?;
        join.index_record(right, &ordered, similarity);
    }
    Ok(join.matches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

    struct ReportWorkspace {
        root: PathBuf,
    }

    impl ReportWorkspace {
        fn new(source: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "smells-differential-report-{}-{}",
                std::process::id(),
                NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            fs::write(root.join("lib.rs"), source).unwrap();
            Self { root }
        }

        fn write_policy(&self, threshold: u64) {
            let mut policy: Value =
                serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap();
            for selection in policy["rules"].as_object_mut().unwrap().values_mut() {
                selection["mode"] = json!("off");
            }
            policy["rules"]["rust.duplicate_functions"]["mode"] = json!("required");
            policy["rules"]["rust.duplicate_functions"]["parameters"]["minimum_tokens"] = json!(4);
            policy["rules"]["rust.duplicate_functions"]["parameters"]["minimum_similarity_basis_points"] =
                json!(threshold);
            policy["limits"]["maximum_pairs"] = json!(1_000_000);
            fs::write(
                self.root.join("quality-policy.json"),
                serde_json::to_vec(&policy).unwrap(),
            )
            .unwrap();
        }
    }

    impl Drop for ReportWorkspace {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    fn semantic_report(workspace: &ReportWorkspace, threshold: u64, reference: bool) -> Value {
        workspace.write_policy(threshold);
        let captured = crate::input::working_tree(
            &workspace.root,
            &workspace.root.join("quality-policy.json"),
            &crate::policy::SelectionOptions::default(),
        )
        .unwrap();
        let mut report = if reference {
            with_reference_join(|| crate::scan::check(&captured.input, &captured.registry))
        } else {
            crate::scan::check(&captured.input, &captured.registry)
        };
        report.attach_sources(&captured.input.files);
        report.finish();
        serde_json::to_value(report).unwrap()
    }

    fn report_boundaries(report: &Value) -> BTreeSet<u64> {
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["rule_id"] == "rust.duplicate_functions")
            .map(|finding| {
                let observed = &finding["evaluation"]["observed"];
                observed["intersection"].as_u64().unwrap() * 10_000
                    / observed["union"].as_u64().unwrap()
            })
            .collect()
    }

    fn assert_semantic_reports_match(workspace: &ReportWorkspace) {
        let boundaries = report_boundaries(&semantic_report(workspace, 1, true));
        assert!(!boundaries.is_empty());
        let thresholds = boundaries
            .into_iter()
            .flat_map(|boundary| {
                [
                    boundary.saturating_sub(1).max(1),
                    boundary.max(1),
                    boundary.saturating_add(1).min(10_000),
                ]
            })
            .collect::<BTreeSet<_>>();
        for threshold in thresholds {
            assert_eq!(
                semantic_report(workspace, threshold, false),
                semantic_report(workspace, threshold, true),
                "optimized report diverged from the independent reference at {threshold} basis points"
            );
        }
    }

    fn reference_fingerprints(token_lists: &[Vec<String>]) -> Vec<BTreeMap<[String; 4], usize>> {
        token_lists
            .iter()
            .map(|tokens| {
                let mut counts = BTreeMap::new();
                for window in tokens.windows(4) {
                    *counts
                        .entry(std::array::from_fn(|index| window[index].clone()))
                        .or_default() += 1;
                }
                counts
            })
            .collect()
    }

    fn reference_intersection_union(
        left: &BTreeMap<[String; 4], usize>,
        right: &BTreeMap<[String; 4], usize>,
    ) -> (usize, usize) {
        let intersection = left
            .iter()
            .map(|(shingle, count)| count.min(right.get(shingle).unwrap_or(&0)))
            .sum::<usize>();
        let union = left.values().sum::<usize>() + right.values().sum::<usize>() - intersection;
        (intersection, union)
    }

    fn boundary_thresholds(token_lists: &[Vec<String>]) -> BTreeSet<u64> {
        let fingerprints = reference_fingerprints(token_lists);
        let mut thresholds = BTreeSet::new();
        for left in 0..fingerprints.len() {
            for right in left + 1..fingerprints.len() {
                let (intersection, union) =
                    reference_intersection_union(&fingerprints[left], &fingerprints[right]);
                if union == 0 {
                    continue;
                }
                let boundary = intersection * 10_000 / union;
                thresholds.insert(boundary.saturating_sub(1).max(1) as u64);
                thresholds.insert(boundary.max(1) as u64);
                thresholds.insert((boundary + 1).min(10_000) as u64);
            }
        }
        thresholds
    }

    fn brute_pairs(
        token_lists: &[Vec<String>],
        minimum_tokens: u64,
        similarity: u64,
    ) -> BTreeSet<(usize, usize, usize, usize)> {
        let fingerprints = reference_fingerprints(token_lists);
        let mut pairs = BTreeSet::new();
        for left in 0..token_lists.len() {
            for right in left + 1..token_lists.len() {
                if (token_lists[left].len() as u64) < minimum_tokens
                    || (token_lists[right].len() as u64) < minimum_tokens
                {
                    continue;
                }
                let (intersection, union) =
                    reference_intersection_union(&fingerprints[left], &fingerprints[right]);
                if union > 0 && intersection as u128 * 10_000 >= similarity as u128 * union as u128
                {
                    pairs.insert((left, right, intersection, union));
                }
            }
        }
        pairs
    }

    #[test]
    fn asymmetric_prefix_keeps_exact_threshold_matches() {
        let shared = (0..19).map(|index| format!("shared-{index}"));
        let first = shared.clone().chain(["left".into()]).collect();
        let second = shared.chain(["right".into()]).collect();
        let pairs = exact_jaccard_pairs(&[first, second], 4, 8_000, 1, |_, _| false).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!((pairs[0].intersection, pairs[0].union), (16, 18));
    }

    #[test]
    fn exact_join_matches_brute_force_across_boundaries() {
        let base = (0..28)
            .map(|index| format!("token-{index}"))
            .collect::<Vec<_>>();
        let mut close = base.clone();
        close[12] = "changed-once".into();
        let mut farther = base.clone();
        for index in [4, 9, 14, 19] {
            farther[index] = format!("changed-{index}");
        }
        let repeated = (0..28)
            .map(|index| format!("repeat-{}", index % 3))
            .collect::<Vec<_>>();
        let reversed = base.iter().cloned().rev().collect::<Vec<_>>();
        let token_lists = vec![
            base.clone(),
            close,
            farther,
            repeated,
            reversed,
            vec!["short".into(), "body".into(), "only".into()],
            base,
        ];
        for minimum in [4, 8, 20] {
            for similarity in [4_000, 8_200, 10_000] {
                let actual =
                    exact_jaccard_pairs(&token_lists, minimum, similarity, usize::MAX, |_, _| {
                        false
                    })
                    .unwrap()
                    .into_iter()
                    .map(|pair| {
                        let (left, right) = if pair.left < pair.right {
                            (pair.left, pair.right)
                        } else {
                            (pair.right, pair.left)
                        };
                        (left, right, pair.intersection, pair.union)
                    })
                    .collect::<BTreeSet<_>>();
                assert_eq!(
                    actual,
                    brute_pairs(&token_lists, minimum, similarity),
                    "minimum={minimum}, similarity={similarity}"
                );
            }
        }
    }

    #[test]
    fn exact_join_matches_brute_force_below_at_and_above_every_fixture_boundary() {
        let base = (0..36)
            .map(|index| format!("token-{index}"))
            .collect::<Vec<_>>();
        let mut one_change = base.clone();
        one_change[35] = "one-change".into();
        let mut two_changes = base.clone();
        two_changes[17] = "middle-change".into();
        two_changes[35] = "last-change".into();
        let repeated = (0..36)
            .map(|index| format!("repeat-{}", index % 4))
            .collect::<Vec<_>>();
        let token_lists = vec![base, one_change, two_changes, repeated];
        for similarity in boundary_thresholds(&token_lists) {
            let actual = exact_jaccard_pairs(&token_lists, 4, similarity, usize::MAX, |_, _| false)
                .unwrap()
                .into_iter()
                .map(|pair| {
                    let (left, right) = if pair.left < pair.right {
                        (pair.left, pair.right)
                    } else {
                        (pair.right, pair.left)
                    };
                    (left, right, pair.intersection, pair.union)
                })
                .collect::<BTreeSet<_>>();
            assert_eq!(
                actual,
                brute_pairs(&token_lists, 4, similarity),
                "similarity={similarity}"
            );
        }
    }

    #[test]
    fn exact_join_charges_candidates_in_canonical_pair_order() {
        let shared = (0..24)
            .map(|index| format!("token-{index}"))
            .collect::<Vec<_>>();
        let lists = vec![shared.clone(), shared.clone(), shared];
        match exact_jaccard_pairs(&lists, 4, 10_000, 1, |_, _| false) {
            Err(error) => assert_eq!(
                error,
                "maximum_pairs budget exceeded: charged 2 exact comparisons; policy limit is 1"
            ),
            Ok(_) => panic!("pair budget should fail closed"),
        }
        let pairs = exact_jaccard_pairs(&lists[..2], 4, 10_000, 1, |_, _| false).unwrap();
        assert_eq!(
            pairs
                .iter()
                .map(|pair| (pair.left, pair.right))
                .collect::<Vec<_>>(),
            [(0, 1)]
        );
    }

    #[test]
    fn exact_join_matches_brute_force_on_the_scanner_source_corpus() {
        let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut paths = std::fs::read_dir(source_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
            .collect::<Vec<_>>();
        paths.sort();
        let token_lists = paths
            .iter()
            .map(|path| {
                let source = std::fs::read_to_string(path).unwrap();
                rustc_lexer::tokenize(&source, rustc_lexer::FrontmatterAllowed::No)
                    .filter(|token| {
                        !matches!(
                            token.kind,
                            rustc_lexer::TokenKind::Whitespace
                                | rustc_lexer::TokenKind::LineComment { .. }
                                | rustc_lexer::TokenKind::BlockComment { .. }
                        )
                    })
                    .map(|token| format!("{:?}", token.kind))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for similarity in [5_000, 8_200, 9_500] {
            let actual =
                exact_jaccard_pairs(&token_lists, 20, similarity, usize::MAX, |_, _| false)
                    .unwrap()
                    .into_iter()
                    .map(|pair| {
                        let (left, right) = if pair.left < pair.right {
                            (pair.left, pair.right)
                        } else {
                            (pair.right, pair.left)
                        };
                        (left, right, pair.intersection, pair.union)
                    })
                    .collect::<BTreeSet<_>>();
            assert_eq!(
                actual,
                brute_pairs(&token_lists, 20, similarity),
                "similarity={similarity}"
            );
        }
    }

    #[test]
    fn optimized_join_matches_independent_semantic_reports_at_all_boundaries() {
        let generated = ReportWorkspace::new(
            r#"
fn generated_a(value:i32)->i32 { value + value + value + value + value }
fn generated_b(value:i32)->i32 { value + value + value + value - value }
fn generated_c(value:i32)->i32 { value + value + value - value - value }
fn generated_d(value:i32)->i32 { value + value - value - value * value }
"#,
        );
        assert_semantic_reports_match(&generated);

        let real_source =
            fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/metrics.rs"))
                .unwrap();
        assert_semantic_reports_match(&ReportWorkspace::new(&real_source));
    }
}
