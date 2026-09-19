# smells

A Rust-first, deterministic source-pattern scanner. Run it over a Rust codebase to report which configured smell patterns match, their locations, measurements, and thresholds. No LLM decides a finding or commit verdict.

**Status: first executable source pass.** The versioned registry covers the exact 23-item Refactoring.Guru catalog with 28 rules: 17 source rules are implemented, 11 compiler/type/contract/history rules are pending, and 2 original inheritance smells are inapplicable to native Rust. Runtime validation pins the catalog source, audit date, canonical names, categories and mappings rather than merely counting entries. This is not a complete semantic smell detector or a claim to contain every quality concern that could be called a smell. Pending checks never silently pass.

## Run it

```sh
cargo build --locked
./target/debug/smells contracts validate --policy examples/quality-policy.json
./target/debug/smells check --path tests/fixtures/catalog --policy examples/quality-policy.json --format json
```

For another codebase, replace the source directory and policy path. Worktree policy paths are relative to the caller's current directory (or absolute). Reports use corpus-relative source paths, never absolute checkout paths.

For a consuming Git repository with its policy staged:

```sh
smells check --staged --policy quality-policy.json --format json
```

The staged mode captures source and policy from Git index blobs and rejects unstaged policy substitution, source symlinks, unmerged entries, and index changes during capture. It does not execute application code, Cargo builds, or build scripts. The CLI returns 0 when configured required source checks complete and pass, 1 for blocking matches, and 2 for errors. Errors take precedence. Report-only indicators do not block.

## What it detects today

Function size and argument count; type/variant fields, enum variants, aggregated associated functions and lines, and trait functions. Structural indicators cover raw primitive slots, repeated named groups, similar implementations with different interfaces, repeated local enum dispatch, low-use optional fields, comment density, normalized token duplication, data-only/tiny types, and forwarding share.

The implemented scope is **authored_all_cfg**: all parsed Rust source files in the corpus, including tests and inactive conditional branches. It is not the future active-production compiler scope. Local type ownership resolves module paths, imports, re-exports, type aliases, and generic impl targets; it is not a general Rust type checker. Unsupported or ambiguous required measurements error. Expansion-only declarations are outside the source pass.

See the [rule-by-rule contracts](docs/rust-rule-contracts.md) for exact matching definitions, false-positive boundaries, pending rules, and known limitations. The [finding report interface](docs/report-interface.md) defines how a hook or LLM consumes the canonical smell identity, detector type, checked pattern, thresholds, source excerpts, evidence, risk, review guidance, remediation candidates, and the non-negotiable external research call to the per-rule Refactoring.Guru URL. The [registry](rules/rust-v1.json) records readiness and parameters; the [policy example](examples/quality-policy.json) explicitly selects every rule. The [JSON Schema](schemas/quality-policy.schema.json) validates policy shape; Rust validation additionally checks rule-specific keys, versions, duplicate map keys, and bounds.

## Reproducibility and validation

Pin a tested executable revision and use Cargo.lock / --locked for its build. Reports contain input and implementation SHA-256 digests, stable ordered findings, exact ratio numerators/denominators, and all 23 coverage entries. The implementation digest covers executable source, dependencies, registry, policy schema and normative rule contracts. Identical captured inputs with the same executable produce byte-identical JSON, independent of absolute checkout location. Ratios are compared before rounding.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Tests exercise the public CLI against actual Rust source, including threshold boundaries, matching/nonmatching shapes, cross-file ownership, Unicode/comments/raw strings, replay, staged isolation, symlinks, missing implementations, and errors.

The opt-in live E2E test runs the example hook in a temporary Git repository with a blocking staged smell, verifies that the hook returns actionable JSON, follows the finding's emitted Refactoring.Guru URL with `curl`, and verifies that the expected smell page and guidance were returned. It requires network access and is ignored by the deterministic default suite:

```sh
cargo test --locked --test e2e_live -- --ignored --nocapture
```

## Integration and independence

Install the executable separately; consuming applications do not import scanner code. Each project owns its policy. Exact exceptions, active compiler configuration, type-aware evidence, contracts, history, and automatic fixes are not implemented yet. Nonempty exceptions or unsupported policy scope are rejected. If a pending rule is required, the source check exits 2.

Keep OSV, Gitleaks, formatting, Clippy, application tests, coverage, and other checks in the consuming project's quality runner. The [hook example](hooks/pre-commit.example) remains uninstalled: validate and pin this source pass for the project's chosen scope before integration. Do not overwrite existing hooks. See [project integration](docs/project-integration.md).

Extracted from the tiny design session on 2026-09-18. This standalone repository is maintained independently of consuming applications. Private GitHub repository: [mindful-time/smells](https://github.com/mindful-time/smells).
