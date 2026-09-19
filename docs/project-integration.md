# Project integration

## Policy and scope

Each project owns a checked-in quality-policy.json. Schema v2 selects rust-v1 and pins scanner 0.1.0, with explicit version, mode and every parameter for all 28 rules. Version-1 specification policies are rejected; replace them with the v2 example and review the scope/default changes rather than silently migrating.

Modes are required, report and off. Required matches block; report matches remain reproducible indicators; off is disabled, not passed. The current example enables 7 required size rules and 10 report-only structural rules. All 11 pending compiler/type/contract/history rules are off, with readiness visible in the catalog. Making one required produces an error. The older cargo-crap/Clippy measurement proposals are superseded by the exact source rules in rust-v1; those external checks can remain independent quality-runner steps.

The only implemented scope is authored_all_cfg, including tests and inactive branches. It parses all included source text, not a selected compiled Cargo target. Exclusions are explicit directory names applied at every depth and reported. Generated .rs files included in the corpus are ordinary source input; macro-expanded declarations without source files are outside scope. No active-production, target/features or compiler evidence is guessed. Use the scanner alongside actual compilation/tests.

## Commands

```text
smells rules
smells contracts validate --policy FILE
smells check --path DIRECTORY --policy FILE [--format table|json]
smells check --staged --policy FILE [--format table|json]
```

- Exactly one of --path and --staged is required for a source check. Worktree policy paths are caller-relative or absolute; the policy is a separate captured input. Validation checks policy structure, not source or detector readiness. Its successful status is valid_contracts, not a smell-scan pass.
- --path recursively captures .rs files under the explicit directory, excluding configured directory names. It rejects symlinks in the included corpus and errors on unreadable/empty source input. Worktree capture is not an atomic filesystem transaction; its report refers to the captured bytes, not an unstaged commit guarantee.
- --staged operates on the current Git repository's root. Its policy path is root-relative, without absolute, dot or escaping components. It reads source and policy directly from regular staged blobs, not from the working tree. It checks the index listing before/after capture and rejects concurrent index changes, unmerged entries, symlink source/policy and absent staged policy. Submodule contents are not traversed as part of the superproject's source corpus.
- Both source modes emit the same metric schema, input/implementation digests, 23-item catalog inventory and sorted findings. Locations are 1-based lines/UTF-8-character columns. Unavailable rules have explicit coverage status; no semantic smell is labeled absent from a structural pass.
- Exit 0: configured required checks completed and passed. Exit 1: at least one blocking match. Exit 2: input/configuration/parser/budget/ownership/required-detector error. Errors outrank violations. Input/configuration failures before a report exists are printed to stderr with exit 2; a captured-source scan reports errors in JSON when requested.
- Exact exceptions, compiler evidence manifests and source active-cfg selection are pending. Nonempty exceptions, unknown fields/rules/parameters or unsupported versions/scope are rejected. Source allows do not change independent size-rule verdicts.

## Hook composition

The example hook is not installed. A consuming project must validate the actual scanner release against its fixtures, adopt the explicit all-cfg source scope, and pin the tested revision before integrating it. Add the invocation to the existing quality runner; preserve other hooks. A project requiring unimplemented rules must remain blocked rather than downgrading those requirements implicitly. An LLM hook must enforce each finding's `diagnostic.reference_check`: the model must make an external research/tool call to the exact rule-owned Refactoring.Guru URL before reviewing or remediating the finding. This research is non-negotiable; block review and remediation when the lookup cannot be completed.

The quality runner continues to own security scans, compilation/lints, application tests, fresh coverage and any future evidence production. The scanner never executes a consuming project's build scripts or tests. The source scanner is not a substitute for those checks.

## Live reference E2E

`tests/e2e_live.rs` exercises the agreed public seam end to end: create a temporary Git repository with a blocking staged smell, run the real example hook, verify its failing exit code and actionable JSON, read the finding's non-negotiable reference-check contract, make a live HTTPS research call to the emitted URL, and verify that the expected Refactoring.Guru smell page was returned. Run it explicitly with:

```sh
cargo test --locked --test e2e_live -- --ignored --nocapture
```

The test requires Git, `curl`, and network access. It is ignored by ordinary `cargo test` so external availability cannot change the deterministic scanner verdict or make the offline suite flaky. This test proves the hook-to-actionable-finding-to-reference-research path. The agentic coding environment consumes the hook failure and performs the subsequent investigation; the scanner repository does not embed or separately invoke an LLM provider.
