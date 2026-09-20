# Project integration

## Policy and scope

Each project owns a checked-in policy. Schema v2 selects exactly one of `rust-v1`, `python-v1`, or `typescript-v1` and pins scanner 0.1.0, with explicit version, mode, and every parameter for all 28 rules. A mixed-language repository uses one policy and one scanner invocation per language pack; this keeps each input corpus and threshold contract explicit. Version-1 specification policies are rejected rather than silently migrated.

Modes are required, report and off. Required matches block; report matches remain reproducible indicators; off is disabled, not passed. The Rust example enables 7 required size rules and 10 report-only structural rules. The Python and TypeScript examples enable 5 required size rules and 5 report-only structural rules. Provider-backed rules are off in the examples. Enabling one requires a complete bundle tied to the exact scan input; otherwise the scan errors.

Rust implements `authored_all_cfg`, including tests and inactive branches. Python and TypeScript implement `authored_source`. Each parses all included source text, not a selected compiled target. Exclusions are explicit directory names applied at every depth and reported. No active-production configuration, type information, macro/decorator expansion, target/features, or compiler evidence is guessed. Use the scanner alongside actual compilation, type checking, and tests.

Repository implementations are discovered from the nearest ancestor `Cargo.toml`, `pyproject.toml`, or `package.json` under the captured scan root. The fixed manifest name supplies the deterministic implementation/runtime type; manifests are never executed. Nested manifests override parent ownership, manifests at one path form one implementation, and sources without a manifest are reported as `__unowned__`. Run `--path` from the monorepo root when repository-relative implementation grouping is required; choosing a nested source directory intentionally limits discovery to manifests inside that captured root.

`maximum_pairs` is a deterministic resource ceiling, not the theoretical number of all callable pairs. Rust, Python, and TypeScript first apply exact multiset-Jaccard minimum-token, size, prefix, and positional filters, then charge one unit for each remaining unique pair whose full similarity is evaluated. Large full-codebase policies can use a larger explicit budget than staged/pre-commit policies; exhaustion always returns exit 2 rather than a truncated success.

## Commands

```text
smells rules [--rule-pack rust-v1|python-v1|typescript-v1]
smells contracts validate --policy FILE
smells check --path DIRECTORY --policy FILE [--evidence FILE] [--format table|json]
smells check --staged --policy FILE [--evidence FILE] [--format table|json]
```

- Exactly one of --path and --staged is required for a source check. Worktree policy paths are caller-relative or absolute; the policy is a separate captured input. Validation checks policy structure, not source or detector readiness. Its successful status is valid_contracts, not a smell-scan pass.
- --path recursively captures only the selected pack's extensions: `.rs`; `.py`/`.pyi`; or `.ts`/`.tsx`/`.mts`/`.cts`, plus the fixed runtime manifests used for implementation ownership. It excludes configured directory names and never follows symlinks. A symlink whose path has a selected source extension or runtime-manifest name is rejected; other symlinks are skipped because they cannot enter that language corpus. It errors on unreadable/empty source input. Worktree capture is not an atomic filesystem transaction; its report refers to the captured bytes, not an unstaged commit guarantee.
- --staged operates on the current Git repository's root. Its policy path is root-relative, without absolute, dot or escaping components. It reads source, runtime manifests, and policy directly from regular staged blobs, not from the working tree. It checks the index listing before/after capture and rejects concurrent index changes, unmerged entries, symlink source/policy/manifest and absent staged policy. Submodule contents are not traversed as part of the superproject's source corpus.
- Both source modes emit report schema v4 with the same metric schema, input/provider/implementation digests, repository-relative `implementation_results`, 23 canonically ordered repository `smell_results`, 23-item coverage inventory and sorted findings. Hooks should route by implementation and then smell-level state before following `matched_finding_indices` to exact rule evidence and 1-based line/UTF-8-character-column locations. Unavailable rules have explicit coverage status; no semantic smell is labeled absent from a structural pass.
- Exit 0: configured required checks completed and passed. Exit 1: at least one blocking match. Exit 2: input/configuration/parser/budget/ownership/required-detector error. Errors outrank violations. Input/configuration failures before a report exists are printed to stderr with exit 2; a captured-source scan reports errors in JSON when requested.
- Exact exceptions, automatic provider execution, and source active-cfg selection remain outside the scanner. Provider evidence manifests are validated through `--evidence`; nonempty exceptions, unknown fields/rules/parameters or unsupported versions/scope are rejected. Source allows do not change independent size-rule verdicts.

## Hook composition

The example hook is not installed. A consuming project must validate the actual scanner release against its fixtures, adopt the selected explicit source scope, and pin the tested revision before integrating it. Add one invocation per language policy to the existing quality runner and preserve other hooks. Stop on the first exit 1/2, or collect the separate JSON reports without merging their language/rule-pack identities. A project requiring unimplemented rules must remain blocked rather than downgrading those requirements implicitly. An LLM hook must read `implementation_results` first, inspect `__unowned__`, follow each implementation's matched pattern indexes, and enforce the smell/finding `reference_check`: the model must make an external research/tool call to the exact Refactoring.Guru URL before reviewing or remediating the finding. This research is non-negotiable; block review and remediation when the lookup cannot be completed.

The quality runner continues to own security scans, compilation/lints, application tests, fresh coverage and any future evidence production. The scanner never executes a consuming project's build scripts or tests. The source scanner is not a substitute for those checks.

## Live reference E2E

`tests/e2e_live.rs` exercises the agreed public seam end to end: create a temporary Git repository with a blocking staged smell, run the real example hook, verify its failing exit code and actionable JSON, read the finding's non-negotiable reference-check contract, make a live HTTPS research call to the emitted URL, and verify that the expected Refactoring.Guru smell page was returned. Run it explicitly with:

```sh
cargo test --locked --test e2e_live -- --ignored --nocapture
```

The test requires Git, `curl`, and network access. It is ignored by ordinary `cargo test` so external availability cannot change the deterministic scanner verdict or make the offline suite flaky. This test proves the hook-to-actionable-finding-to-reference-research path. The agentic coding environment consumes the hook failure and performs the subsequent investigation; the scanner repository does not embed or separately invoke an LLM provider.
