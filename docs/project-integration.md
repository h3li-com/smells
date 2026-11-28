# Project integration

## Policy and scope

Each project owns a checked-in policy. Schema v2 selects exactly one of `rust-v1`, `python-v1`, or `typescript-v1` and pins scanner 0.7.0, with explicit version, mode, and every parameter for all 28 rules. `default_groups` controls the persistent selection; the starter policies use `all`. A mixed-language repository uses one policy and one scanner invocation per language pack; this keeps each input corpus and threshold contract explicit. Version-1 specification policies are rejected rather than silently migrated.

Modes are required, report and off. Required matches block; report matches remain reproducible indicators; off is a legacy/custom-policy opt-out and is disabled, not passed. Every starter rule is required or report and therefore active. The `all` default selects every rule, and built-in collectors run without a separate evidence pipeline. Use `--only-group source` only when the user intentionally opts out of the source/type/history/contract indicators in the `evidence` group.

Rust implements `authored_all_cfg`, including tests and inactive branches. Python and TypeScript implement `authored_source`. Each parses all included source text, not a selected compiled target. Exclusions are explicit directory names applied at every depth and reported. No active-production configuration, type information, macro/decorator expansion, target/features, or compiler evidence is guessed. Use the scanner alongside actual compilation, type checking, and tests.

Repository implementations are discovered from the nearest ancestor `Cargo.toml`, `pyproject.toml`, or `package.json` under the captured scan root. The fixed manifest name supplies the deterministic implementation/runtime type; manifests are never executed. Nested manifests override parent ownership, manifests at one path form one implementation, and sources without a manifest are reported as `__unowned__`. Run `--path` from the monorepo root when repository-relative implementation grouping is required; choosing a nested source directory intentionally limits discovery to manifests inside that captured root.

`maximum_pairs` is a deterministic resource ceiling, not the theoretical number of all callable pairs. Rust, Python, and TypeScript first apply exact multiset-Jaccard minimum-token, size, prefix, and positional filters, then charge one unit for each remaining unique pair whose full similarity is evaluated. Large full-codebase policies can use a larger explicit budget than staged/pre-commit policies; exhaustion identifies the affected rule plus the charged and configured counts, and always returns exit 2 rather than a truncated success.

## Tests and migrations (`source_roles`)

Instead of excluding test suites or migration directories and losing visibility, declare `source_roles` in `quality-policy.json`:

```json
"source_roles": [
  {
    "role": "test_suite",
    "framework": "pytest",
    "paths": ["/tests/**/*.py"]
  },
  {
    "role": "database_migration",
    "framework": "alembic",
    "paths": ["/backend/alembic/versions/**/*.py"],
    "configuration": ["/backend/alembic.ini"]
  }
]
```

- **Test suites** (`pytest`, `unittest`, `vitest`, `jest`, `playwright`, `cargo_test`): long canonical test cases and blocks (`test_*`, `describe`/`it`/`test`, `#[test]`) become visible non-blocking `reported_by_role` review signals, while test helpers and all other rules stay enforced.
- **Database migrations** (Alembic, Django, TypeORM, MikroORM, Kysely, Knex, Sequelize/Umzug, Prisma, Drizzle, SeaORM, SQLx, Diesel, Refinery): canonical migration entrypoints stay measured as non-blocking review signals and SQL bodies are inventoried explicitly. See [migration code analysis](migration-code-analysis.md).

## Measured CRAP scores (`--coverage`)

Pass `--coverage lcov.info` (any LCOV file from `cargo llvm-cov`, `pytest --cov`, or `vitest --coverage`) to score CRAP from real line coverage. Starter policies score CRAP in bands: 5 or less is fine, 6 to 10 is a non-blocking warning, and above 10 blocks. Only a score measured from coverage can block; without coverage, scores assume 0% coverage and stay warnings. `smells doctor` finds `lcov.info`, `coverage/lcov.info`, or `target/llvm-cov/lcov.info` automatically and, without one, hides those 0%-coverage estimates. `smells check` finds the same files for a working-tree scan without `--base`; staged and `--base` scans read coverage only from `--coverage`, because the ratchet reference has no coverage to compare against.

When no file is found and you run `doctor` or `check` at a terminal, smells offers to measure coverage. It shows the exact commands first: an install step if the tool is missing (`cargo-llvm-cov`, `pytest-cov`, or `@vitest/coverage-v8` at your installed vitest version), then the test run (`cargo llvm-cov`, `pytest --cov`, `vitest run --coverage` or `jest --coverage`, through `uv` or your lockfile's package manager). Nothing runs unless you answer `y`. A spinner shows progress, test output goes to a log file, and a failed run leaves the scan on its 0%-coverage estimates. The policy's language picks the tool, so a Rust crate packaged for Python with maturin gets `cargo llvm-cov`; manifests decide only when no policy can be read. The offer never appears when `CI` is set, with `--format json`, or when stdin or stderr is not a terminal.

With coverage, CRAP scores in files the LCOV omits stay 0%-coverage estimates. `check` reports them; doctor hides them and counts them in its `coverage` check. `cargo llvm-cov` does not report Rust integration tests under `tests/` as covered sources, so their functions are the usual case.

## Commands

```text
smells --version
smells rules [--rule-pack rust-v1|python-v1|typescript-v1]
smells contracts validate [--policy FILE]
smells doctor [--policy FILE] [--path DIRECTORY] [--base REF] [--format table|json]
smells policy show [--policy FILE] [--format table|json] [group selectors]
smells check [--path DIRECTORY] [--policy FILE] [--base REF] [--evidence FILE] [--format table|json] [--report JSON] [--log LOG] [group selectors]
smells check --staged [--policy FILE] [--base REF] [--evidence FILE] [--format table|json] [--report JSON] [--log LOG] [group selectors]
```

Group selectors are repeatable `--group NAME`, repeatable `--only-group NAME`,
`--all-groups`, `--no-default-groups`, and repeatable `--no-group NAME`.
Configured defaults are applied first, additions next, and exclusions last.
`--only-group` replaces defaults and conflicts with positive/default controls;
`--no-group` always wins. Available groups are `all`, `source`, `evidence`, and
the five canonical Refactoring.Guru categories. Unknown, duplicate, or
conflicting selectors fail closed. `policy show` exposes the exact resolved rule
set before a scan.

- `check` scans `--path .` unless `--path` or `--staged` is given; combining `--path` and `--staged` exits 2. Without `--policy`, `check` reads `quality-policy.json` from the scanned root; when it is missing, the scan exits 2 and prints the exact `curl` command for the matching starter policy. Without `--log` and `--format`, `check` prints a short summary and writes the complete Finding Log to `smells-findings.log`; `--format table` prints every finding instead. Every command accepts `--help`. Worktree policy paths are caller-relative or absolute; the policy is a separate captured input. Validation checks policy structure, not source or detector readiness. Its successful status is valid_contracts, not a smell-scan pass.
- --path recursively captures the selected pack's extensions (`.rs`; `.py`/`.pyi`; or `.ts`/`.tsx`/`.mts`/`.cts`), the fixed runtime manifests used for implementation ownership, and recognized SQL migration artifacts. Recognized artifact bytes count toward the file budget and input digest even though their bodies are inventoried rather than analyzed. It excludes configured directory names and never follows symlinks. A symlink whose path has a selected source extension, recognized SQL artifact, or runtime-manifest name is rejected; other symlinks are skipped because they cannot enter that language corpus. It errors on unreadable/empty source input. Worktree capture is not an atomic filesystem transaction; its report refers to the captured bytes, not an unstaged commit guarantee.
- --staged operates on the current Git repository's root. Its policy path is root-relative, without absolute, dot or escaping components. It reads source, runtime manifests, recognized SQL migration artifacts, and policy directly from regular staged blobs, not from the working tree. Artifact bytes participate in the staged file budget and input digest. It checks the index listing before/after capture and rejects concurrent index changes, unmerged entries, symlink source/policy/manifest/artifact and absent staged policy. Submodule contents are not traversed as part of the superproject's source corpus.
- Both source modes emit report schema v11 with resolved `policy_selection`, source-role, migration-artifact, and unresolved-candidate inventories, exact migration declaration provenance, migration integrity metadata, input/provider/implementation digests, bounded `history_scope`, repository-relative `implementation_results`, 23 smell results, 23 immutable `when_to_ignore` records, compact measurements, matched findings, and suppression audit records. Findings reference guidance once by `guidance_ref`. Hooks route from the Finding Log Issue Index to exact 1-based source locations, and use normalized JSON for programmatic evidence access.
- Exit 0: configured required checks completed without unsuppressed required matches, including `passed_with_ignored_findings`. Exit 1: at least one blocking match. Exit 2: input/configuration/parser/budget/ownership/suppression/collector-prerequisite error. Errors outrank violations. Missing Git makes selected history rules incomplete regardless of mode.
- The only exception mechanism is a syntax-aware source comment: `smells: ignore[exact-rule-id] -- non-empty reason`. It applies to that rule on the next declaration, allowing blank lines and decorators/attributes. Separate rules require separate directives. Malformed, unknown, misplaced, duplicate, and unused directives exit 2. Multi-location findings can be suppressed only at the reported primary declaration; scanner failures are unsuppressible. Ignored findings remain visible with `ignored_match` state and their reason. When the targeted rule is incomplete, its directive remains visible as `unverified_due_to_incomplete_rule` because the scanner cannot prove it unused; the scan still exits 2.
- Automatic provider execution and source active-cfg selection remain outside the scanner. Provider evidence manifests are validated through `--evidence`; unknown fields/groups/rules/parameters or unsupported versions/scope are rejected. A suppression does not alter the policy or hide the underlying match.

## Integration preflight

`smells doctor` audits an integration and codebase health with zero required flags. When `--policy` is omitted, it auto-discovers a single standard policy file under `--path` (default `.`), or falls back with a warning to the built-in Rust, Python, or TypeScript starter policy when a single language manifest (`Cargo.toml`, `pyproject.toml`, or `package.json`) is present; ambiguous layouts fail closed. When `--base` is omitted, it automatically compares against `origin/main` or `origin/master` when present. It reports one row per setup check, each `ok`, `warn`, or `error`:

- `policy`: the policy parses, matches its rule pack, and pins the running scanner version.
- `git_history`: Git history is available and no shallow-clone boundary truncates the 200-commit window.
- `suppressions`: total and per-rule `smells: ignore` directives. Malformed or unknown directives are errors. Unused directives need a full scan, so `smells check` reports them.
- `agents_contract`: `AGENTS.md` exists and is not empty.
- `claude_import`: if a `CLAUDE.md` exists, it contains an `@AGENTS.md` import line.
- `codeowners`: the first of `.github/CODEOWNERS`, `CODEOWNERS`, or `docs/CODEOWNERS` assigns an owner to the policy file. The last matching rule wins, as on GitHub.

When the policy, Git history, and suppression syntax checks pass, `smells doctor` also runs the scanner in diagnostic mode and appends a colored, human-readable **Codebase Health** scorecard (0–100 score and grade, respecting `NO_COLOR` and `CLICOLOR_FORCE`), a compact breakdown across all 23 smells (`blocking` vs. `ignored` vs. `review`), and the top 5 hotspot files (also returned under `"diagnostics"` in `--format json`). Passing `--base REF` (for example `--base v0.6.0` or `--base origin/main`) adds a **Burndown** section and JSON `"burndown"` summary comparing score, total blockers, review warnings, ignored findings, and per-smell deltas against that Git reference. Any `error` (including a scan parser, budget, or invalid `--base` error) exits `2`, because the matching scan would be incomplete or rejected. Governance warnings and matched smells exit `0` (`smells check` remains the hard CI gate). When the policy cannot be read, the input-dependent checks and diagnostics are skipped and the governance checks still run. The Debt Envelope is computed in memory from `--base`, so there is no baseline file to verify.

## CI gate templates

`examples/ci/` holds copyable, fail-closed templates. Copy them from the release tag that matches the pinned version, because each template pins the scanner release it ships with and `tests/ci_templates.rs` fails if that pin drifts from the crate version.

- `github-actions-smells.yml` runs on `pull_request` and `merge_group`. It checks out full history (`fetch-depth: 0`) without persisted credentials, installs the pinned release, and scans with `--base` set to the pull request or merge-group base commit. It always uploads `smells-findings.log` and `smells-report.json`.
- `azure-pipelines-smells.yml` does the same with `fetchDepth: 0` and `--base origin/<target branch>`. Azure Repos ignores YAML `pr:` triggers, so attach the pipeline as a Build Validation branch policy.
- `CODEOWNERS` assigns the policy files, agent instruction files, and gate pipeline files to one owner team, so weakening the gate needs the same review as changing it.

Both templates run the gate as an independent job with no `needs:` or `dependsOn`, never use `continue-on-error`, and exit `2` when no ratchet base is available. Exit `1` and exit `2` both fail the job.

Branch-protection checklist:

1. Mark the `smells` job (GitHub) or the Build Validation policy (Azure) as required on the protected branch.
2. Require Code Owner review and dismiss stale approvals when new commits are pushed.
3. Require branches to be up to date before merging, or use a merge queue so `merge_group` runs the gate on the merged result.
4. Block force pushes and branch deletion, and do not let administrators bypass the rules.
5. Keep `fetch-depth: 0` (or `fetchDepth: 0`). A shallow clone makes history rules and `--base` exit `2`.
6. Review the uploaded Finding Log before any suppression is approved.

## Hook composition

The example hook is not installed. A consuming project must validate the actual scanner release against its fixtures, adopt the selected explicit source scope, and pin the tested revision before integrating it. Add one full-policy invocation per language to the existing quality runner and preserve other hooks. Save one complete numbered Finding Log with `--log` and one normalized JSON Evidence Report with `--report`; do not pipe a huge table through terminal capture.

Hook stdout names both artifacts and tells the agent to start at the Finding Log Issue Index. The agent must then read every referenced error, blocking, ignored, and review detail. Each finding supplies all locations, exact policy evaluation, why it matched, rule/version, URL, resolved `when_to_ignore`, and exact suppression form. Before review, remediation, or suppression, the agent must open and read the exact Refactoring.Guru URL, then inspect the declaration, callers, and tests. If research is unavailable, it must report that fact and stop those actions. It may never add a suppression merely to make the hook pass.

The quality runner continues to own security scans, compilation/lints, application tests, fresh coverage and any future evidence production. The scanner never executes a consuming project's build scripts or tests, except a coverage run a person accepts at the interactive prompt. The source scanner is not a substitute for those checks.

## Live reference E2E

`tests/e2e_live.rs` exercises the public seam end to end: create a temporary Git repository with a blocking staged smell, run the real example hook, verify the Finding Log and JSON report, read the finding's non-negotiable research contract, make a live HTTPS call to the emitted URL, and verify that the expected Refactoring.Guru page was returned. A second live test checks each embedded verbatim `when_to_ignore` item against its source page. Run it explicitly with:

```sh
cargo test --locked --test e2e_live -- --ignored --nocapture
```

The test requires Git, `curl`, and network access. It is ignored by ordinary `cargo test` so external availability cannot change the deterministic scanner verdict or make the offline suite flaky. This test proves the hook-to-actionable-finding-to-reference-research path. The agentic coding environment consumes the hook failure and performs the subsequent investigation; the scanner repository does not embed or separately invoke an LLM provider.
