# Project integration

## Policy and scope

Each project owns a checked-in policy. Schema v2 selects exactly one of `rust-v1`, `python-v1`, or `typescript-v1` and pins scanner 0.1.0, with explicit version, mode, and every parameter for all 28 rules. A mixed-language repository uses one policy and one scanner invocation per language pack; this keeps each input corpus and threshold contract explicit. Version-1 specification policies are rejected rather than silently migrated.

Modes are required, report and off. Required matches block; report matches remain reproducible indicators; off is disabled, not passed. The Rust example enables 7 required size rules and 10 report-only structural rules. The Python and TypeScript examples enable 5 required size rules and 5 report-only structural rules. Every pending rule is off, with readiness visible in the catalog. Making one required produces an error.

Rust implements `authored_all_cfg`, including tests and inactive branches. Python and TypeScript implement `authored_source`. Each parses all included source text, not a selected compiled target. Exclusions are explicit directory names applied at every depth and reported. No active-production configuration, type information, macro/decorator expansion, target/features, or compiler evidence is guessed. Use the scanner alongside actual compilation, type checking, and tests.

## Commands

```text
smells rules [--rule-pack rust-v1|python-v1|typescript-v1]
smells contracts validate --policy FILE
smells check --path DIRECTORY --policy FILE [--format table|json]
smells check --staged --policy FILE [--format table|json]
```

- Exactly one of --path and --staged is required for a source check. Worktree policy paths are caller-relative or absolute; the policy is a separate captured input. Validation checks policy structure, not source or detector readiness. Its successful status is valid_contracts, not a smell-scan pass.
- --path recursively captures only the selected pack's extensions: `.rs`; `.py`/`.pyi`; or `.ts`/`.tsx`/`.mts`/`.cts`. It excludes configured directory names and never follows symlinks. A symlink whose path has a selected source extension is rejected; other symlinks are skipped because they cannot enter that language corpus. It errors on unreadable/empty source input. Worktree capture is not an atomic filesystem transaction; its report refers to the captured bytes, not an unstaged commit guarantee.
- --staged operates on the current Git repository's root. Its policy path is root-relative, without absolute, dot or escaping components. It reads source and policy directly from regular staged blobs, not from the working tree. It checks the index listing before/after capture and rejects concurrent index changes, unmerged entries, symlink source/policy and absent staged policy. Submodule contents are not traversed as part of the superproject's source corpus.
- Both source modes emit the same metric schema, input/implementation digests, 23-item catalog inventory and sorted findings. Locations are 1-based lines/UTF-8-character columns. Unavailable rules have explicit coverage status; no semantic smell is labeled absent from a structural pass.
- Exit 0: configured required checks completed and passed. Exit 1: at least one blocking match. Exit 2: input/configuration/parser/budget/ownership/required-detector error. Errors outrank violations. Input/configuration failures before a report exists are printed to stderr with exit 2; a captured-source scan reports errors in JSON when requested.
- Exact exceptions, compiler evidence manifests and source active-cfg selection are pending. Nonempty exceptions, unknown fields/rules/parameters or unsupported versions/scope are rejected. Source allows do not change independent size-rule verdicts.

## Hook composition

The example hook is not installed. A consuming project must validate the actual scanner release against its fixtures, adopt the selected explicit source scope, and pin the tested revision before integrating it. Add one invocation per language policy to the existing quality runner and preserve other hooks. Stop on the first exit 1/2, or collect the separate JSON reports without merging their language/rule-pack identities. A project requiring unimplemented rules must remain blocked rather than downgrading those requirements implicitly. An LLM hook must enforce each finding's `diagnostic.reference_check`: the model must make an external research/tool call to the exact rule-owned Refactoring.Guru URL before reviewing or remediating the finding. This research is non-negotiable; block review and remediation when the lookup cannot be completed.

The quality runner continues to own security scans, compilation/lints, application tests, fresh coverage and any future evidence production. The scanner never executes a consuming project's build scripts or tests. The source scanner is not a substitute for those checks.

## Live reference E2E

`tests/e2e_live.rs` exercises the agreed public seam end to end: create a temporary Git repository with a blocking staged smell, run the real example hook, verify its failing exit code and actionable JSON, read the finding's non-negotiable reference-check contract, make a live HTTPS research call to the emitted URL, and verify that the expected Refactoring.Guru smell page was returned. Run it explicitly with:

```sh
cargo test --locked --test e2e_live -- --ignored --nocapture
```

The test requires Git, `curl`, and network access. It is ignored by ordinary `cargo test` so external availability cannot change the deterministic scanner verdict or make the offline suite flaky. This test proves the hook-to-actionable-finding-to-reference-research path. The agentic coding environment consumes the hook failure and performs the subsequent investigation; the scanner repository does not embed or separately invoke an LLM provider.
