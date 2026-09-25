# Project integration

## Policy and scope

Each project owns a checked-in policy. Schema v2 selects exactly one of `rust-v1`, `python-v1`, or `typescript-v1` and pins scanner 0.5.0, with explicit version, mode, and every parameter for all 28 rules. `default_groups` controls the persistent selection; the starter policies use `all`. A mixed-language repository uses one policy and one scanner invocation per language pack; this keeps each input corpus and threshold contract explicit. Version-1 specification policies are rejected rather than silently migrated.

Modes are required, report and off. Required matches block; report matches remain reproducible indicators; off is a legacy/custom-policy opt-out and is disabled, not passed. Every starter rule is required or report and therefore active. The `all` default selects every rule, and built-in collectors run without a separate evidence pipeline. Use `--only-group source` only when the user intentionally opts out of the source/type/history/contract indicators in the `evidence` group.

Rust implements `authored_all_cfg`, including tests and inactive branches. Python and TypeScript implement `authored_source`. Each parses all included source text, not a selected compiled target. Exclusions are explicit directory names applied at every depth and reported. No active-production configuration, type information, macro/decorator expansion, target/features, or compiler evidence is guessed. Use the scanner alongside actual compilation, type checking, and tests.

Repository implementations are discovered from the nearest ancestor `Cargo.toml`, `pyproject.toml`, or `package.json` under the captured scan root. The fixed manifest name supplies the deterministic implementation/runtime type; manifests are never executed. Nested manifests override parent ownership, manifests at one path form one implementation, and sources without a manifest are reported as `__unowned__`. Run `--path` from the monorepo root when repository-relative implementation grouping is required; choosing a nested source directory intentionally limits discovery to manifests inside that captured root.

`maximum_pairs` is a deterministic resource ceiling, not the theoretical number of all callable pairs. Rust, Python, and TypeScript first apply exact multiset-Jaccard minimum-token, size, prefix, and positional filters, then charge one unit for each remaining unique pair whose full similarity is evaluated. Large full-codebase policies can use a larger explicit budget than staged/pre-commit policies; exhaustion always returns exit 2 rather than a truncated success.

## Commands

```text
smells --version
smells rules [--rule-pack rust-v1|python-v1|typescript-v1]
smells contracts validate --policy FILE
smells policy show --policy FILE [--format table|json] [group selectors]
smells check --path DIRECTORY --policy FILE [--evidence FILE] [--format table|json] [--report JSON] [--log LOG] [group selectors]
smells check --staged --policy FILE [--evidence FILE] [--format table|json] [--report JSON] [--log LOG] [group selectors]
```

Group selectors are repeatable `--group NAME`, repeatable `--only-group NAME`,
`--all-groups`, `--no-default-groups`, and repeatable `--no-group NAME`.
Configured defaults are applied first, additions next, and exclusions last.
`--only-group` replaces defaults and conflicts with positive/default controls;
`--no-group` always wins. Available groups are `all`, `source`, `evidence`, and
the five canonical Refactoring.Guru categories. Unknown, duplicate, or
conflicting selectors fail closed. `policy show` exposes the exact resolved rule
set before a scan.

- Exactly one of --path and --staged is required for a source check. Worktree policy paths are caller-relative or absolute; the policy is a separate captured input. Validation checks policy structure, not source or detector readiness. Its successful status is valid_contracts, not a smell-scan pass.
- --path recursively captures only the selected pack's extensions: `.rs`; `.py`/`.pyi`; or `.ts`/`.tsx`/`.mts`/`.cts`, plus the fixed runtime manifests used for implementation ownership. It excludes configured directory names and never follows symlinks. A symlink whose path has a selected source extension or runtime-manifest name is rejected; other symlinks are skipped because they cannot enter that language corpus. It errors on unreadable/empty source input. Worktree capture is not an atomic filesystem transaction; its report refers to the captured bytes, not an unstaged commit guarantee.
- --staged operates on the current Git repository's root. Its policy path is root-relative, without absolute, dot or escaping components. It reads source, runtime manifests, and policy directly from regular staged blobs, not from the working tree. It checks the index listing before/after capture and rejects concurrent index changes, unmerged entries, symlink source/policy/manifest and absent staged policy. Submodule contents are not traversed as part of the superproject's source corpus.
- Both source modes emit report schema v7 with resolved `policy_selection`, input/provider/implementation digests, bounded `history_scope`, repository-relative `implementation_results`, 23 smell results, 23 immutable `when_to_ignore` records, compact measurements, matched findings, and suppression audit records. Findings reference guidance once by `guidance_ref`. Hooks route from the Finding Log Issue Index to exact 1-based source locations, and use normalized JSON for programmatic evidence access.
- Exit 0: configured required checks completed without unsuppressed required matches, including `passed_with_ignored_findings`. Exit 1: at least one blocking match. Exit 2: input/configuration/parser/budget/ownership/suppression/collector-prerequisite error. Errors outrank violations. Missing Git makes selected history rules incomplete regardless of mode.
- The only exception mechanism is a syntax-aware source comment: `smells: ignore[exact-rule-id] -- non-empty reason`. It applies to that rule on the next declaration, allowing blank lines and decorators/attributes. Separate rules require separate directives. Malformed, unknown, misplaced, duplicate, and unused directives exit 2. Multi-location findings can be suppressed only at the reported primary declaration; scanner failures are unsuppressible. Ignored findings remain visible with `ignored_match` state and their reason.
- Automatic provider execution and source active-cfg selection remain outside the scanner. Provider evidence manifests are validated through `--evidence`; unknown fields/groups/rules/parameters or unsupported versions/scope are rejected. A suppression does not alter the policy or hide the underlying match.

## Hook composition

The example hook is not installed. A consuming project must validate the actual scanner release against its fixtures, adopt the selected explicit source scope, and pin the tested revision before integrating it. Add one full-policy invocation per language to the existing quality runner and preserve other hooks. Save one complete numbered Finding Log with `--log` and one normalized JSON Evidence Report with `--report`; do not pipe a huge table through terminal capture.

Hook stdout names both artifacts and tells the agent to start at the Finding Log Issue Index. The agent must then read every referenced error, blocking, ignored, and review detail. Each finding supplies all locations, exact policy evaluation, why it matched, rule/version, URL, resolved `when_to_ignore`, and exact suppression form. Before review, remediation, or suppression, the agent must open and read the exact Refactoring.Guru URL, then inspect the declaration, callers, and tests. If research is unavailable, it must report that fact and stop those actions. It may never add a suppression merely to make the hook pass.

The quality runner continues to own security scans, compilation/lints, application tests, fresh coverage and any future evidence production. The scanner never executes a consuming project's build scripts or tests. The source scanner is not a substitute for those checks.

## Live reference E2E

`tests/e2e_live.rs` exercises the public seam end to end: create a temporary Git repository with a blocking staged smell, run the real example hook, verify the Finding Log and JSON report, read the finding's non-negotiable research contract, make a live HTTPS call to the emitted URL, and verify that the expected Refactoring.Guru page was returned. A second live test checks each embedded verbatim `when_to_ignore` item against its source page. Run it explicitly with:

```sh
cargo test --locked --test e2e_live -- --ignored --nocapture
```

The test requires Git, `curl`, and network access. It is ignored by ordinary `cargo test` so external availability cannot change the deterministic scanner verdict or make the offline suite flaky. This test proves the hook-to-actionable-finding-to-reference-research path. The agentic coding environment consumes the hook failure and performs the subsequent investigation; the scanner repository does not embed or separately invoke an LLM provider.
