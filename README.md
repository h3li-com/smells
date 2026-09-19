# smells

A deterministic, Rust-first source-pattern scanner for Rust, Python, and TypeScript. Run it over a codebase to report which configured smell patterns match, their locations, measurements, and thresholds. No LLM decides a finding or commit verdict.

**Status: executable authored-source passes.** Every language pack maps the exact 23-item Refactoring.Guru catalog to 28 deterministic rules. `rust-v1` implements 17 source rules, with 11 evidence-dependent rules pending and 2 inheritance smells inapplicable to native Rust. `python-v1` and `typescript-v1` each implement 10 portable source rules and explicitly mark 18 semantic/provider rules pending. Runtime validation pins the catalog source, audit date, canonical names, categories, mappings, and readiness. This is not a claim that syntax alone can prove every semantic smell. Pending checks never silently pass.

| Rule pack | Source files | Implemented source rules | Class model |
| --- | --- | ---: | --- |
| `rust-v1` | `.rs` | 17 | `struct`/`enum` state plus all resolved inherent and trait `impl` methods; `trait` is an interface-like contract |
| `python-v1` | `.py`, `.pyi` | 10 | `class` body state plus direct methods and `self`/`cls` field assignments |
| `typescript-v1` | `.ts`, `.tsx`, `.mts`, `.cts` | 10 | class/abstract class/expression members, method signatures, and constructor parameter-properties |

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

For another codebase, replace the source directory and policy path. Worktree policy paths are relative to the caller's current directory (or absolute). Reports use corpus-relative source paths, never absolute checkout paths.

For a consuming Git repository with its policy staged:

```sh
smells check --staged --policy quality-policy.json --format json
```

The policy's `rule_pack` selects the language; there is no separate language flag. Worktree traversal never follows symlinks: it rejects symlinks named with the selected language's source extensions and skips other symlinks. The staged mode captures source and policy from Git index blobs and rejects unstaged policy substitution, source symlinks, unmerged entries, and index changes during capture. It does not execute application code, compilers, builds, or build scripts. The CLI returns 0 when configured required source checks complete and pass, 1 for blocking matches, and 2 for errors. Errors take precedence. Report-only indicators do not block.

## What it detects today

All three packs measure function size and argument count. Python and TypeScript additionally measure class fields, direct methods, and summed method lines, and detect repeated named parameter groups, comment density, normalized token duplication, data-only classes, and tiny classes. Rust also measures struct/enum/trait size, aggregates methods from every resolved `impl`, and provides its richer Rust-specific structural indicators.

Rust uses **authored_all_cfg**: all parsed Rust source files in the corpus, including tests and inactive conditional branches. Python and TypeScript use **authored_source** through pinned Tree-sitter grammars. These are not compiler-selected production scopes. Local Rust ownership resolves module paths, imports, re-exports, type aliases, and generic impl targets; the portable packs do not yet resolve imports or types. Unsupported or ambiguous required measurements error.

See the exact contracts for [Rust](docs/rust-rule-contracts.md), [Python](docs/python-rule-contracts.md), and [TypeScript](docs/typescript-rule-contracts.md). The [finding report interface](docs/report-interface.md) defines how a hook or LLM consumes smell identity, detector type, thresholds, source excerpts, evidence, review/remediation guidance, and the non-negotiable external research call to the exact per-rule Refactoring.Guru URL. Each pack has a registry, explicit policy example, and JSON Schema; runtime validation additionally checks rule-specific keys, versions, duplicate map keys, and bounds.

## Reproducibility and validation

Pin a tested executable revision and use Cargo.lock / --locked for its build. Reports contain input and implementation SHA-256 digests, stable ordered findings, exact ratio numerators/denominators, and all 23 coverage entries. The implementation digest covers executable source, dependencies, registry, policy schema and normative rule contracts. Identical captured inputs with the same executable produce byte-identical JSON, independent of absolute checkout location. Ratios are compared before rounding.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Tests exercise the public CLI against actual Rust, Python, TypeScript, and TSX source, including class ownership, threshold boundaries, matching/nonmatching shapes, cross-file Rust ownership, replay, staged policy/source isolation, selected-source symlink rejection, non-source symlink skipping, monorepo cache exclusions, pending implementations, and errors.

The opt-in live E2E test runs the example hook in a temporary Git repository with a blocking staged smell, verifies that the hook returns actionable JSON, follows the finding's emitted Refactoring.Guru URL with `curl`, and verifies that the expected smell page and guidance were returned. It requires network access and is ignored by the deterministic default suite:

```sh
cargo test --locked --test e2e_live -- --ignored --nocapture
```

## Integration and independence

Install the executable separately; consuming applications do not import scanner code. Each policy selects exactly one language pack. A mixed-language repository invokes the scanner once per checked-in language policy, from the same pre-commit runner if desired. Exact exceptions, active compiler configuration, type-aware evidence, contracts, history, and automatic fixes are not implemented yet. Nonempty exceptions or unsupported policy scope are rejected. If a pending rule is required, the source check exits 2.

Keep OSV, Gitleaks, formatting, Clippy, application tests, coverage, and other checks in the consuming project's quality runner. The [hook example](hooks/pre-commit.example) remains uninstalled: validate and pin this source pass for the project's chosen scope before integration. Do not overwrite existing hooks. See [project integration](docs/project-integration.md).

Extracted from the tiny design session on 2026-09-18. This standalone repository is maintained independently of consuming applications. Private GitHub repository: [mindful-time/smells](https://github.com/mindful-time/smells).
