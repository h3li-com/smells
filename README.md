# smells

A deterministic, Rust-first source-pattern scanner for Rust, Python, and TypeScript. Run it over a codebase to report which configured smell patterns match, their locations, measurements, and thresholds. No LLM decides a finding or commit verdict.

**Status: complete deterministic rule evaluators.** Every language pack maps the exact 23-item Refactoring.Guru catalog to 28 executable rules. `rust-v1` has 17 authored-source detectors and 11 provider-backed evaluators; its 2 native-inheritance smells remain explicitly inapplicable. `python-v1` and `typescript-v1` each have 10 authored-source detectors and 18 provider-backed evaluators. Runtime validation pins the catalog, mappings, thresholds, source input, and complete evidence-bundle bytes, and validates the declared provider identity/configuration shape. Syntax alone does not pretend to prove compiler-, type-, coverage-, contract-, or history-dependent smells: enabling one of those rules without complete evidence fails closed.

| Rule pack | Source files | Source rules | Provider rules | Class model |
| --- | --- | ---: | ---: | --- |
| `rust-v1` | `.rs` | 17 | 11 | `struct`/`enum` state plus all resolved inherent and trait `impl` methods; `trait` is an interface-like contract |
| `python-v1` | `.py`, `.pyi` | 10 | 18 | `class` body state plus direct methods and `self`/`cls` field assignments |
| `typescript-v1` | `.ts`, `.tsx`, `.mts`, `.cts` | 10 | 18 | class/abstract class/expression members, method signatures, and constructor parameter-properties |

## Run it

```sh
cargo build --locked
./target/debug/smells contracts validate --policy examples/quality-policy.json
./target/debug/smells check --path tests/fixtures/catalog --policy examples/quality-policy.json --format json
./target/debug/smells contracts validate --policy examples/python-quality-policy.json
./target/debug/smells check --path path/to/python --policy examples/python-quality-policy.json --format json
./target/debug/smells contracts validate --policy examples/typescript-quality-policy.json
./target/debug/smells check --path path/to/typescript --policy examples/typescript-quality-policy.json --format json
```

Pass `--evidence provider-evidence.json` when any compiler-, type-, coverage-,
contract-, test-, or history-backed rule is enabled. The bundle must pin the
`input_sha256` from the exact source scan; see the [provider evidence
contract](docs/provider-evidence.md). In staged mode the evidence file must also
be staged, so an unstaged result cannot be substituted into a commit verdict.

For another codebase, replace the source directory and policy path. Worktree policy paths are relative to the caller's current directory (or absolute). Reports use corpus-relative source paths, never absolute checkout paths. In a monorepo, the scanner assigns each source file to its nearest ancestor `Cargo.toml`, `pyproject.toml`, or `package.json` and reports smell patterns separately for each repository implementation. Sources without a runtime manifest remain visible under `__unowned__`.

For a consuming Git repository with its policy staged:

```sh
smells check --staged --policy quality-policy.json --format json
```

The policy's `rule_pack` selects the language; there is no separate language flag. Worktree traversal never follows symlinks: it rejects symlinks named with the selected language's source extensions and skips other symlinks. The staged mode captures source and policy from Git index blobs and rejects unstaged policy substitution, source symlinks, unmerged entries, and index changes during capture. It does not execute application code, compilers, builds, or build scripts. The CLI returns 0 when configured required source checks complete and pass, 1 for blocking matches, and 2 for errors. Errors take precedence. Report-only indicators do not block.

## What it detects today

All three packs measure function size and argument count. Python and TypeScript additionally measure class fields, direct methods, and summed method lines, and detect repeated named parameter groups, comment density, normalized token duplication, data-only classes, and tiny classes. Portable duplicate detection uses exact multiset-Jaccard prefix, size, and positional filters before charging the explicit pair budget; these filters remove only pairs that cannot reach the configured threshold. Rust also measures struct/enum/trait size, aggregates methods from every resolved `impl`, and provides its richer Rust-specific structural indicators.

Rust uses **authored_all_cfg**: all parsed Rust source files in the corpus, including tests and inactive conditional branches. Python and TypeScript use **authored_source** through pinned Tree-sitter grammars. These are not compiler-selected production scopes. A narrow TypeScript compatibility reparse handles valid generic-call type arguments shaped as `typeof import("literal")`; other parse errors still fail closed. Local Rust ownership resolves module paths, imports, re-exports, type aliases, and generic impl targets; the portable packs do not yet resolve imports or types. Unsupported or ambiguous required measurements error.

See the exact contracts for [Rust](docs/rust-rule-contracts.md), [Python](docs/python-rule-contracts.md), and [TypeScript](docs/typescript-rule-contracts.md). The [finding report interface](docs/report-interface.md) leads with repository-relative `implementation_results`, each containing one result per canonical smell pattern, then links matched patterns to detailed rule evidence by stable finding indexes. It defines how a hook or LLM consumes implementation ownership, smell identity, detector type, thresholds, source excerpts, evidence, review/remediation guidance, and the non-negotiable external research call to the exact Refactoring.Guru URL. Each pack has a registry, explicit policy example, and JSON Schema; runtime validation additionally checks rule-specific keys, versions, duplicate map keys, and bounds.

## Reproducibility and validation

Pin a tested executable revision and use Cargo.lock / --locked for its build. Reports contain input and implementation SHA-256 digests, repository implementation results, all 23 canonically ordered repository smell results, stable ordered findings, exact ratio numerators/denominators, and all 23 coverage entries. The input digest includes the captured runtime manifests used for repository ownership. The human-readable table prints one implementation section at a time and shows only that implementation's matched repository-relative evidence beneath it. The implementation digest covers executable source, dependencies, registry, policy schema and normative rule contracts. Identical captured inputs with the same executable produce byte-identical JSON, independent of absolute checkout location. Ratios are compared before rounding.

Duplicate detection uses exact prefix, size, and positional filters before an
exact multiset-Jaccard comparison. The filters cannot remove a pair capable of
meeting the configured threshold, and `maximum_pairs` counts only exact
comparisons that remain. Portable-language fingerprints intern normalized
tokens into fixed-size numeric keys; this changes neither evidence nor report
ordering.

Python and TypeScript parse/fact extraction runs in bounded Rayon workers,
reuses one Tree-sitter parser per grammar per worker, walks each syntax tree
once with a primary cursor and active function/class scopes, and merges results
in canonical source-path order. Small declaration-local helper walks extract
parameters and descendants without repeating function-body metric traversal. A
versioned content-addressed fact cache is stored
under the operating-system temporary directory by default; set
`SMELLS_CACHE_DIR` to select a persistent location. Keys include exact source,
path, language, enabled fact needs, locked dependencies, and extraction code.
Writes use a temporary file plus atomic rename, and corrupt cache data is a miss.
On Unix, the cache root is pinned to an open directory handle and descendants
are opened relative to it with `O_NOFOLLOW`; symlinked roots are rejected,
symlinked entries or descendants are misses, and redirected writes are skipped.
Cache hits and misses produce byte-identical reports. JSON is streamed directly
to stdout to avoid a second report-sized allocation.

Use the release benchmark runner to measure a codebase without retaining the
potentially large JSON report. It accepts a scan path, policy path, run count,
cache mode, and worker count. Warm mode primes both timing and counter caches
before the first measured run; cold mode gives every run a fresh cache. Every
run includes deterministic counters for
parsing, exact-join filters/comparisons, cache use, JSON bytes, and peak memory:

```sh
./scripts/benchmark-scan.sh . quality-policy.json 5
./scripts/benchmark-scan.sh path/to/python examples/python-quality-policy.json 5 cold 1
```

Set `SMELLS_METRICS_FILE` on a normal JSON scan to write the same versioned
counter object separately from the deterministic report. Metrics schema v2
reports Tree-sitter CST visits as `syntax_nodes` for Python/TypeScript and Rust
lexer work separately as `syntax_tokens`; the two quantities are not treated as
cross-language equivalents. Run the full generated
matrix (all languages, corpus shapes, thresholds, cold/warm caches, and one/default
worker counts) with `./scripts/benchmark-matrix.sh 3`. Before timing, the matrix
scans each generated no-clone corpus at its lowest threshold and fails unless it
contains zero duplicate findings. The matrix also accepts the
six `SMELLS_{RUST,PYTHON,TYPESCRIPT}_{ROOT,POLICY}` variables to include real
projects in the same run.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

## Repository quality hooks

This repository dogfoods the scanner through one fail-closed gate used by both
the checked-in pre-commit and pre-push hooks:

```sh
make install-hooks
make pre-commit-push
```

The shared gate checks formatting, performs a locked all-target Cargo build,
runs Clippy and tests, scans this repository with `quality-policy.json`, enforces
the CRAP limits, runs Gitleaks, and checks `Cargo.lock` with OSV-Scanner. A
failing self-scan prints the complete JSON evidence for an agent; a passing scan
is saved at `target/quality/smells-report.json`.
Gitleaks checks both the staged patch and repository history, so the shared gate
has the correct coverage in both hook contexts.

The toolchain is intentionally pinned by `scripts/check-quality-tools.sh`:
`cargo-crap 0.5.0`, `cargo-llvm-cov 0.8.7`, Gitleaks 8.30.1, and OSV-Scanner
2.3.8. CRAP has a required target of 5 and an absolute hard limit of 10. Every
function above 5 is emitted as an agent-readable annotation with file, line,
score, complexity, and coverage; any function above 10 blocks the hook. The
complete report is saved at `target/quality/crap-report.json`, and a blocking
result includes remediation guidance plus the mandatory Refactoring.Guru Long
Method research URL. On a rustup toolchain, `cargo-llvm-cov` discovers
`llvm-tools-preview`; the gate also supports the matching Homebrew LLVM
installation used by Homebrew Rust.

Tests exercise the public CLI against actual Rust, Python, TypeScript, and TSX source, including class ownership, threshold boundaries, matching/nonmatching shapes, cross-file Rust ownership, replay, staged policy/source isolation, selected-source symlink rejection, non-source symlink skipping, monorepo cache exclusions, pinned provider evidence, and fail-closed errors.

The opt-in live E2E test runs the example hook in a temporary Git repository with a blocking staged smell, verifies that the hook returns actionable JSON, follows the finding's emitted Refactoring.Guru URL with `curl`, and verifies that the expected smell page and guidance were returned. It requires network access and is ignored by the deterministic default suite:

```sh
cargo test --locked --test e2e_live -- --ignored --nocapture
```

## Integration and independence

Install the executable separately; consuming applications do not import scanner code. Each policy selects exactly one language pack. A mixed-language repository invokes the scanner once per checked-in language policy, from the same pre-commit runner if desired. External tools produce provider facts; the scanner validates the bundle structure, input binding, measurement contract, locations, and declared completeness, then owns every threshold verdict. The hook pipeline remains responsible for invoking and authenticating the declared provider/configuration; the scanner records those declarations and pins the full bundle digest but cannot prove that a producer's `complete: true` assertion is truthful. Exact exceptions, provider execution, active compiler configuration selection, and automatic fixes remain outside the scanner. Nonempty exceptions or unsupported policy scope are rejected.

Keep OSV, Gitleaks, formatting, Clippy, application tests, coverage, and other checks in each consuming project's quality runner. This scanner repository has its own checked-in hooks and deliberately runs those checks against itself. The [consumer hook example](hooks/pre-commit.example) remains uninstalled: validate and pin this source pass for the consuming project's chosen scope before integration. Do not overwrite existing hooks. See [project integration](docs/project-integration.md).

Extracted from the tiny design session on 2026-09-18. This standalone repository is maintained independently of consuming applications. Private GitHub repository: [mindful-time/smells](https://github.com/mindful-time/smells).
