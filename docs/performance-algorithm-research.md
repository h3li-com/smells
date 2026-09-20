# Exact scan performance: algorithm research and recommendations

Research date: 2026-09-20

## Conclusion

The duplicate detector is already in the right algorithm family. Both implementations
now perform an exact, global-frequency-ordered prefix join with length and position
filters, followed by an exact multiset-Jaccard comparison:

- Rust: `src/patterns.rs`
- Python and TypeScript: `src/portable.rs`

This is an AllPairs/PPJoin-style filter-and-verify join, not an exhaustive all-pairs
comparison. AllPairs introduced the scalable prefix-index approach, and PPJoin added
position-aware filtering ([Bayardo, Ma, and Srikant, 2007](https://research.google/pubs/scaling-up-all-pairs-similarity-search/);
[Xiao et al., 2008](https://www.cse.unsw.edu.au/~lxue/WWW08.pdf)). The current design
should be improved incrementally rather than replaced.

The next work should be, in order:

1. add early-terminating exact verification and retain prefix-overlap state;
2. shorten the indexed prefix using the asymmetric self-join bound;
3. replace tree maps and string features in the hot path with dense, canonical integer
   IDs and flat vectors;
4. make portable-language fact extraction policy-aware and one-pass;
5. parallelize independent file parsing with a canonical merge;
6. add a content-addressed per-file fact cache;
7. stream JSON to the output writer.

PPJoin+ suffix filtering and AdaptJoin should not be the default next step. A broad
main-memory evaluation found that efficient merge verification often inspects only a
small constant number of features, while the more elaborate PPJoin+ and AdaptJoin
filters were often the slowest choices
([Mann, Augsten, and Bouros, 2016](https://www.vldb.org/pvldb/vol9/p636-mann.pdf)).
They are valid exact algorithms, but must earn their complexity on this scanner's own
corpora.

MinHash and locality-sensitive hashing are approximate estimators/candidate selectors.
They cannot replace the exact join in a deterministic commit verdict. Broder's original
construction explicitly estimates resemblance from a fixed-size random sample
([Broder, 1997](https://www.cs.princeton.edu/courses/archive/spring13/cos598C/broder97resemblance.pdf)).
An approximate prefilter may omit a qualifying pair, so it must not be allowed to discard
candidates. Using it only to prioritize candidates while eventually checking all of them
does not reduce the exact work.

## What uv's implementation suggests

uv does not solve source-pattern similarity, so its resolver is not a replacement for the
scanner's exact join. Its execution and cache architecture are directly relevant:

- **Bounded parallel CPU work.** uv derives build/install concurrency from
  `available_parallelism`, uses semaphores for bounded work classes, and runs independent
  wheel installs through Rayon
  ([uv concurrency](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-configuration/src/concurrency.rs#L45-L81);
  [parallel installs](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-installer/src/installer.rs#L157-L185)).
  The scanner should apply the same pattern to independent per-file parse/fact work, then
  merge results by canonical input index. It should not parallelize pair-budget charging.
- **Versioned, atomic cache storage.** uv gives cache buckets explicit format versions,
  uses content-addressed file/archive areas, and persists completed temporary artifacts by
  rename
  ([bucket versions and content-addressed files](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-cache/src/lib.rs#L1268-L1325);
  [atomic persistence](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-cache/src/lib.rs#L410-L455)).
  A scanner fact cache should likewise be schema-versioned, atomically written, and safe
  for concurrent readers.
- **Validate once, access cheaply.** uv stores some metadata with `rkyv`; its owned archive
  validates bytes at construction and then provides allocation-free archived access
  ([uv `OwnedArchive`](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-client/src/rkyvutil.rs#L44-L145)).
  This is a later optimization for a fact cache, after a simpler serialized cache proves
  that deserialization is material.
- **Coalesce filesystem work and restore order.** uv clusters related glob keys before
  walking each common base once, and explicitly sorts filesystem entries whose operating-
  system order is unspecified
  ([clustered glob traversal](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-cache-info/src/cache_info.rs#L226-L275);
  [sorted paths](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-installer/src/site_packages.rs#L678-L694)).
  The scanner should perform one source traversal and always sort after concurrent or
  filesystem-originated collection.

One uv behavior should **not** be copied: uv's default local-project rebuild heuristic is
based mainly on metadata timestamps for `pyproject.toml`, `setup.py`, and `setup.cfg`
([uv cache semantics](https://docs.astral.sh/uv/concepts/cache/#dependency-caching)). That
tradeoff is acceptable for dependency builds but too weak for a fail-closed smell verdict.
Scanner cache identity must use the exact source content SHA-256 plus language, grammar,
normalization, fact-schema, source path/ownership inputs, and rule-needs version.

## Why the multiset transformation is exact

The scanner compares bags of normalized four-token windows. For a gram `g` with count
`c`, the implementation expands it conceptually into occurrence features
`(g, 0) ... (g, c - 1)`. Therefore, for two fingerprints `A` and `B`:

```text
|expanded(A) intersect expanded(B)| = sum_g min(count_A(g), count_B(g))
|expanded(A) union expanded(B)|     = sum_g max(count_A(g), count_B(g))
```

This is the multiset generalization of Jaccard, also called Ruzicka similarity in the
multiset-join literature
([Metwally and Faloutsos, 2012](https://www.vldb.org/pvldb/vol5/p704_ahmedmetwally_vldb2012.pdf)).
It allows exact set-join filters to be applied without changing the scanner's evidence.
The occurrence number is part of the feature identity; dropping it would silently change
the metric from multiset Jaccard to set Jaccard.

## Ranked recommendations

### 1. Early-terminating verification with a candidate accumulator

Current candidate generation deduplicates matching postings into a `BTreeSet`, then the
verification merge computes the complete intersection and union for every candidate.
This throws away useful work already performed during prefix probing.

Replace the candidate set with reusable dense state indexed by record ID:

- an epoch/generation array, so candidate state does not need to be cleared globally;
- accumulated prefix overlap;
- the last matching positions in both records;
- a `touched` vector of candidate IDs, sorted before verification to preserve canonical
  order.

For feature-vector lengths `m` and `n`, similarity threshold `t / 10_000`, the exact
required overlap is:

```text
required = ceil(t * (m + n) / (10_000 + t))
```

During the merge, reject as soon as:

```text
overlap_so_far + min(m - left_position, n - right_position) < required
```

This is the early-stopping verification used in the empirical study's Algorithm 1. The
study reports that false candidates commonly needed only 0.3 to 1.5 feature comparisons
on many datasets
([Mann et al., Algorithm 1 and Section 5.4](https://www.vldb.org/pvldb/vol9/p636-mann.pdf)).
For a matching pair, continue the merge to obtain the exact intersection and union that
the report contract exposes. For a nonmatching pair, stop immediately when the upper
bound falls below `required`.

This should be implemented in the shared conceptual path for Rust, Python, and
TypeScript. It is exact and should have low implementation risk.

### 2. Use the shorter indexed prefix for an ordered self-join

Records are already processed by nondecreasing feature count, so every indexed record is
no longer than the record currently being probed. That ordering permits an asymmetric
prefix pair:

```text
indexed shorter record: floor((1 - similarity) / (1 + similarity) * size) + 1
probing longer record:  floor((1 - similarity) * size) + 1
```

The current code uses the longer, symmetric prefix for both sides. The asymmetric bound
is exact for Jaccard when the indexed record is the shorter side. It reduces posting-list
memory and false candidates without changing recall
([Wang et al., Lemma 3.4](https://www.vldb.org/pvldb/vol10/p925-wang.pdf)). The same
self-join optimization is described as indexing with the equivalent overlap of two
equal-length records in the 2016 evaluation
([Mann et al., Section 2.2](https://www.vldb.org/pvldb/vol9/p636-mann.pdf)).

Keep all arithmetic integral and use ceiling/floor transformations proven equivalent to
the formulas above. Add threshold-boundary tests before changing the prefix lengths.

### 3. Dense canonical feature IDs and flat indexes

Portable fingerprints already intern normalized tokens into `[usize; 4]`, but still use
tree maps for fingerprint counts, feature frequencies, and posting lookup. Rust still
uses allocated `Vec<String>` four-gram keys in its duplicate path.

Use a two-level, collision-free canonical encoding:

1. assign normalized-token IDs by sorted token text;
2. assign occurrence-feature IDs by sorted `([token_id; 4], occurrence)`;
3. store each record as a flat `Vec<FeatureId>` ordered by `(global_frequency,
   canonical_feature_id)`;
4. store postings as `Vec<Vec<Posting>>`, indexed by `FeatureId`;
5. store the original multiset as a sorted run-length-encoded vector when exact evidence
   needs raw counts.

This removes repeated string comparison, four-gram allocation, and logarithmic tree
lookup from the hottest loops. A vector provides constant-time indexing and contiguous
storage ([Rust `Vec` documentation](https://doc.rust-lang.org/std/vec/)). Do not derive
feature identity from a truncated hash. Either use canonical interning or retain the full
key to resolve collisions.

A randomly seeded `HashMap` must not define feature or report order. Rust documents that
the default `HashMap` seed is random
([Rust `HashMap` documentation](https://doc.rust-lang.org/std/collections/struct.HashMap.html)).
A hash table may be used as a private lookup accelerator only if IDs and externally
observable iteration order are established separately and deterministically.

### 4. One-pass, policy-aware portable fact extraction

The portable scanner currently revisits function and class subtrees to compute code rows,
comment rows, tokens, fields, accessor shape, and method totals. Nested functions can
make this repeated traversal especially costly.

Build a `RuleNeeds` execution plan from the validated registry and policy, then make one
primary traversal of each Tree-sitter tree. Maintain active function/class scopes and update only facts required
by the report contract and enabled detectors. Leaf nodes can contribute normalized tokens,
code rows, and comment rows during the same traversal. To preserve current semantics,
leaves inside a nested function must contribute to every active enclosing function if the
old recursive collector did so.

Use one reusable `TreeCursor` for the primary traversal. Bounded declaration-local helper
cursors may extract parameters and specific descendants, but must not repeat the function-body
metric traversal. Tree-sitter describes `TreeCursor` as the efficient
stateful tree-walking API, and its Rust node documentation recommends cursor reuse to
avoid unnecessary allocation
([TreeCursor 0.25.10](https://docs.rs/tree-sitter/0.25.10/tree_sitter/struct.TreeCursor.html);
[Node::children 0.25.10](https://docs.rs/tree-sitter/0.25.10/tree_sitter/struct.Node.html#method.children)).

Also reuse a configured `Parser` per grammar instead of creating and configuring one per
file. TypeScript and TSX need separate parser instances because they use different
grammars.

Tree-sitter queries are an alternative for discovering functions, classes, parameters,
and comments, but the current scanner does not use them. If queries are introduced,
compile one query with multiple patterns per language, dispatch on `pattern_index`, reuse
the immutable query, and use a separate reusable `QueryCursor` per worker. The official
API says a query is immutable/shareable and a query cursor is reusable but not shared
([Tree-sitter Query API](https://github.com/tree-sitter/tree-sitter/blob/master/docs/src/using-parsers/queries/4-api.md)).
Always check `did_exceed_match_limit`; a required scan must fail closed rather than accept
truncated query results.

### 5. Deterministic parallel parsing and fact extraction

Parsing and local fact extraction are independent by file. Enumerate the already sorted
input files, process them in parallel, collect one result per input index, and merge in
that original index order. Rayon demonstrates that collecting an indexed parallel
iterator preserves indexed order
([Rayon `ParallelIterator`](https://docs.rs/rayon/latest/rayon/iter/trait.ParallelIterator.html)).

Safe parallel phases:

- `syn::parse_file` for Rust;
- Tree-sitter parse and local facts for Python/TypeScript, with a parser per worker;
- fingerprint construction after the global token dictionary is fixed;
- exact verification of a fully materialized, canonically ordered candidate list.

Keep global frequency computation, prefix-index candidate enumeration, pair-budget
charging, and report merge in deterministic order. If exact verification is parallelized,
first enumerate candidates sequentially, apply `maximum_pairs`, then collect verification
results by candidate index and sort findings canonically. Thread scheduling must not decide
which pair is charged or reported first.

### 6. Content-addressed fact caching; incremental trees only in a resident mode

For the one-shot CLI, cache per-file normalized facts rather than Tree-sitter trees. Key a
cache entry by at least:

- source content SHA-256;
- scanner/fact-schema and normalization version;
- language and exact grammar version;
- rule-needs mask;
- any source-scope option that changes facts.

Load cached local facts, merge them in canonical path order, then rerun corpus-global
rules such as duplicate detection and data clumps. Cache corruption or a version mismatch
must cause recomputation, never a skipped check. Cache writes should be atomic. Cache
normalized token identities, not corpus-local integer IDs; re-intern them after cache
loading.

Tree-sitter's incremental API is more useful for a daemon/watch mode that retains the old
source and tree. The official API requires applying `Tree::edit`, passing the old tree to
`Parser::parse`, and then exposes `changed_ranges`
([Tree-sitter incremental parsing](https://docs.rs/tree-sitter/0.25.10/tree_sitter/struct.Tree.html#method.changed_ranges)).
Without the prior edit description, a cold CLI cache cannot safely obtain that benefit.

### 7. Stream serialization to the output writer

`main.rs` currently calls `serde_json::to_string_pretty(report)` and then prints the
result. For large reports this creates a second report-sized `String`. Serialize directly
to a locked stdout writer with `serde_json::to_writer_pretty`, then write one newline.
Serde JSON officially supports serializing directly to any `io::Write`
([serde_json 1.0.151](https://docs.rs/serde_json/1.0.151/serde_json/)).

This reduces peak memory and one large copy; it will not materially reduce parsing or
duplicate-join CPU. Preserve the exact current bytes with a golden differential test.
True finding-by-finding streaming would require redesign because `report.finish()` sorts
and cross-links findings. Do not stream unsorted findings merely to lower memory.

### 8. Benchmark-gated exact filters, not default complexity

If counters still show too many candidates after recommendations 1-3, evaluate these
independently:

- **PEL / MPJoin removal:** apply a position-dependent upper length bound and remove
  postings that no future, longer probe can satisfy. These are exact and can reduce
  posting visits.
- **GroupJoin:** batch records with identical prefixes. Code-clone corpora may contain
  unusually many equal prefixes, so this could be more useful here than on generic text.
- **PPJoin+ suffix filter:** recursively bound suffix overlap using pivots and binary
  search.
- **AdaptJoin:** choose an extended prefix and required prefix overlap using a cost model
  ([Wang, Li, and Feng, 2012](https://dbgroup.cs.tsinghua.edu.cn/jnwang/papers/sigmod2012-adaptjoin.pdf)).

All are exact when implemented from their proofs. However, the comparative evaluation
found AllPairs, PPJoin, and GroupJoin generally competitive, while PPJoin+ suffix work
and AdaptJoin's cost machinery often cost more than fast verification
([Mann et al., 2016](https://www.vldb.org/pvldb/vol9/p636-mann.pdf)). Add them only when
measurements show that candidate verification, rather than parsing, indexing, or report
construction, remains dominant.

Distributed V-SMART-Join explicitly supports exact multiset similarities and is useful
evidence that the metric can scale, but its multi-stage MapReduce architecture is not a
good fit for a local pre-commit hook
([Metwally and Faloutsos, 2012](https://research.google/pubs/v-smart-join-a-scalable-mapreduce-framework-for-all-pair-similarity-joins-of-multisets-and-vectors/)).

## Required counters and benchmark matrix

Wall-clock measurements alone do not explain regressions. Emit opt-in benchmark counters
without adding them to deterministic production reports:

- files, Tree-sitter syntax-node visits, Rust lexer tokens, functions, and total normalized
  tokens; syntax-node visits and lexer tokens are distinct counters rather than a
  cross-language comparison;
- eligible fingerprints and expanded feature count;
- prefix lookups and posting entries visited;
- unique candidates and exact comparisons charged;
- feature comparisons during verification;
- candidates rejected by length, position, and early verification;
- matching pairs, fact-cache hits/misses, JSON bytes, and peak resident memory.

Benchmark each language on:

1. the real Rust, Python, and TypeScript projects already used for live tests;
2. a no-clone corpus with many functions;
3. a clone-heavy corpus with identical and near-identical bodies;
4. short and long functions;
5. similarity thresholds from 5,000 through 9,500 basis points;
6. cold and warm file caches;
7. one thread and the default bounded worker count.

Use medians from multiple release-build runs. Keep deterministic algorithmic tests for
candidate counts and `maximum_pairs`; do not make normal CI depend on wall-clock limits.

## Measured post-optimization profile

Release scans on the current real repositories, with output discarded, separate the
remaining costs as follows. Individual values are diagnostic runs rather than CI timing
limits:

| Corpus | Full JSON | Full table | Duplicate rule(s) off | All rules off |
| --- | ---: | ---: | ---: | ---: |
| Rust scanner repository | 0.17 s warm | — | 0.04 s | previously 0.03 s |
| Python `ai-unify-be` | 5.23 s | 4.93 s | 2.89 s | 2.14 s |
| TypeScript `ai-unify-fe` | 2.19 s | 2.10 s | 1.35 s | 0.89 s |

The small JSON-versus-table difference rejects serialization CPU as the main remaining
runtime bottleneck, although direct writer serialization is still valuable for peak
memory. Duplicate analysis accounts for about 2.34 seconds in Python and 0.84 seconds in
TypeScript. The all-off floor confirms that parsing and unconditional fact extraction are
the other major target. The current pretty JSON reports are also large: about 221 MB for
Python and 83.7 MB for TypeScript.

After the shared dense exact join, policy-aware extraction, deterministic portable
parallelism, parser reuse, content-addressed fact cache, indexed report metadata, and
streamed JSON were implemented, release scans measured:

| Corpus | Previous warm | New cold cache | New warm cache |
| --- | ---: | ---: | ---: |
| Rust scanner repository | 0.17 s | n/a | 0.20 s |
| Python `ai-unify-be` | 5.23 s | 4.48 s | 3.61 s |
| TypeScript `ai-unify-fe` | 2.19 s | 1.46 s | 1.24 s |

These are diagnostic three-run observations rather than CI limits. Rust stays serial
because `syn`/`proc_macro2` AST spans are not `Send`; unsafe wrappers are not justified
for a scan already near 0.1 seconds. Portable per-file facts are plain serializable data,
so their parallel merge remains safe and canonical.

## Rule-pack completeness audit

All three registries contain the exact 23-item catalog and 28 executable rule contracts,
and all three example policies pass runtime contract validation. The optimization work
keeps the authored-source detector counts at 17 for Rust and 10 for both Python and
TypeScript. Complete deterministic provider evaluators now cover the remaining 11 Rust
rules and 18 portable rules:

| Pack | Source rules | Provider-backed rules | Executable rules | Inapplicable smells |
| --- | ---: | ---: | ---: | ---: |
| `rust-v1` | 17 | 11 | 28/28 | 2 |
| `python-v1` | 10 | 18 | 28/28 | 0 |
| `typescript-v1` | 10 | 18 | 28/28 | 0 |

Provider-backed does not mean source syntax can prove semantic or historical facts. An
enabled compiler-, type-, coverage-, contract-, test-, or history-backed rule requires a
complete provider bundle tied to the exact captured input SHA-256. The scanner validates
the declared provider identity/configuration shape, locations, and exact measurement keys,
pins the complete bundle digest, then applies the configured thresholds itself. The hook
must authenticate and invoke the expected producer; `complete` remains that producer's
explicit trust-boundary assertion. Missing or stale evidence errors. TypeScript
now also has matched parity fixtures for comment share, duplicate functions, data clumps,
data class, and lazy class.

The one-pass portable extractor, opt-in counters, and complete benchmark matrix are now
implemented. `SMELLS_METRICS_FILE` keeps diagnostic data outside production reports;
`scripts/benchmark-scan.sh` captures metrics schema v2 per run, while
`scripts/benchmark-matrix.sh` covers generated no-clone, clone-heavy, short, and long
corpora at 5,000, 6,000, 7,000,
8,200, 9,000, and 9,500 basis points with cold/warm caches and one/default worker counts.
Warm cases prime the timing and metrics caches before all measured samples; cold cases
use a distinct empty cache for every sample.
The no-clone corpus uses all 24 permutations of three distinct operators, and the runner proves the
label before timing by requiring zero duplicate findings at 5,000 basis points.
Optional real-project environment variables add the three live corpora without baking
machine-specific paths into the repository.

## Correctness and determinism gates

Every optimization must satisfy all of the following:

- **No false negatives.** Only a proven necessary condition may discard a pair.
- **Exact multiset metric.** Preserve occurrence features and `min`/`max` multiplicities.
- **Integer threshold arithmetic.** Continue comparing integer numerators and
  denominators; do not introduce floating-point boundary decisions.
- **Canonical feature order.** Global frequency first, then a stable canonical feature
  identity; never randomized hash iteration.
- **Canonical pair order.** Parallel execution and caches must not affect budget charging,
  errors, findings, related symbols, or serialized bytes.
- **Stable budget meaning.** `maximum_pairs` counts exact candidate verifications after
  safe filters, in canonical order, whether facts came from cache or live parsing.
- **Fail closed.** Parser cancellation, query match-limit exhaustion, worker failure, and
  budget exhaustion remain errors. Cache corruption is never evidence: it becomes a miss
  and must be replaced by a successful live parse or the scan errors. Unix cache reads and
  writes use a process-pinned cache-root directory handle and traverse descendants with
  `O_NOFOLLOW`, so a symlinked root/leaf/descendant or later ancestor path-swap cannot
  redirect evidence.
- **Differential proof.** For generated and real corpora, compare the old and new semantic
  reports at thresholds immediately below, at, and immediately above every computed
  similarity boundary. Ignore only the expected executable implementation digest.
- **Cross-language parity.** Run the same duplicate fixtures through Rust, Python, and
  TypeScript with language-appropriate syntax and require equal token counts,
  intersection, union, threshold decisions, and pair-budget behavior.

The deterministic test suite enforces these gates with an independent brute-force
multiset oracle below, at, and above every generated fixture boundary, a representative
comparison over this repository's real source corpus, canonical budget-order tests, and
matched Rust/Python/TypeScript fixtures whose token counts, intersection, union, 6,000
basis-point boundary, and fail-closed pair budget are identical.
