# Changelog

All notable changes to Smells are recorded here. Release tags use `vMAJOR.MINOR.PATCH`.

## 0.7.0 - 2026-11-23

- Cut detector false positives found by scanning Smells with itself. The
  observable contract of each changed rule moves to version `2`, so policies must
  pin the new versions:
  - `data_clumps` (all languages) reports one finding per closed group, meaning
    the slots shared by every supporter, instead of one finding per k-subset.
  - `foreign_accesses` (all languages) measures each method that has an owned
    receiver instead of the whole file. Free functions and static methods are no
    longer observed.
  - `navigation_chains` (all languages) counts the longest run of field reads
    and zero-argument calls. Calls with arguments start a new run. Standard
    adapters such as `iter`, `clone`, `unwrap` and `toString` are transparent.
    The first hop from `self`, `this` or `Self` is not counted.
  - `rust.lazy_class` excludes enums.
  - `rust.unused_code` treats crate-level `main`, `#[test]` and `#[bench]`
    functions, and names referenced as path strings in attributes (such as
    `#[serde(default = "name")]`) as reachable.
- Add `--coverage FILE` to `check` and `doctor`. It reads an LCOV artifact and
  replaces the zero-coverage CRAP upper bound with the exact score from measured
  line coverage (`lcov_line_coverage_crap_v1`). Functions with no executable
  line in the artifact keep the bound. A missing, malformed, unmatched or stale
  artifact (one recording lines past the end of a scanned file) exits `2`.
- Make `smells doctor` precise by default. It auto-discovers `lcov.info`,
  `coverage/lcov.info` or `target/llvm-cov/lcov.info`. Without coverage it hides
  non-blocking CRAP estimates that assume 0% coverage, reports how many it hid
  in a `coverage` check (and `estimated_crap_hidden` in JSON), and prints the
  command that produces LCOV for the project's language. Blocking findings are
  never hidden. The burndown compares both sides without non-blocking CRAP so
  that adding coverage does not read as a regression.
- Count cyclomatic complexity on the syntax tree in every language. The old lexical count read keywords inside
  strings and comments, missed Rust `?`, and counted closure `||` and `=>`.
  Rust now uses cargo-crap's decision points (each `?` and match arm, `if`,
  `for`, `while`, `loop`, `&&`, `||`) and agrees with cargo-crap on every
  function of Smells' own source. Python follows radon (`assert`, loop and
  `try` `else`, and every `match` `case` except a wildcard or bare capture count)
  and agrees on 99.6% of 4,120 functions. TypeScript follows ESLint's
  `complexity` rule (optional chaining, default values and logical assignment
  count; `default` does not) and agrees on 16,741 of 16,744 functions. Closures,
  lambdas and nested items are their own scope.
- Score `function_crap` in bands and move it to version `3` in every language,
  with parameters `warning_maximum` (5) and `maximum` (10). A score of 5 or less
  is not a finding. A score above 5 is a finding, and it blocks only when the
  rule is `required`, the score is above 10 and it was measured from supplied
  LCOV coverage. Every other match is a non-blocking warning whose explanation
  says why. A zero-coverage upper bound never blocks. The starter policies make
  `function_crap` `required`, so a measured score above 10 now fails the check.
- Calibrate detectors against seeded Python, TypeScript and Rust fixture
  projects. Rule versions are unchanged, so existing policies keep working:
  - `divergent_change` and `shotgun_surgery` (all languages) take a file's owner
    from the first module below its implementation root, after a leading `src`
    or `lib` and a top-level Python package. Before, every file under `src/` had
    the same owner, so neither rule could match. Both skip the repository's
    initial commit, which is an import rather than a change.
  - `temporary_fields` (Python and TypeScript) does not count `__init__` or
    `constructor` as a use, since constructors initialize every field.
  - `library_capabilities` (Python and TypeScript) counts assignments only, so
    a comparison such as `this.cart.lines.length === 0` is no longer a
    workaround.
  - `rust.port_conformance` does not require trait methods that have a default
    body.
- Measure CRAP coverage over the function body, not the declaration line.
  Python and JavaScript coverage mark a `def` or `function` line as run when the
  module loads, which gave never-called functions coverage credit.
- Lower the refused-bequest `minimum_unused_percent` default from 80 to 67 in
  the Python and TypeScript registries and starter policies, so a subclass that
  refuses 3 of 4 inherited methods matches.
- Move `rust.dependency_contract` to version `2`. A private field is visible in
  its own module and every descendant module, so only matching accesses from
  descendant module files of the owner's module count; accesses elsewhere cannot
  compile and are name collisions.
- Raise the `duplicate_functions` `minimum_tokens` from 20 to 40 in the Rust,
  Python and TypeScript starter policies. Shorter pairs were mostly visitor and
  helper boilerplate. Exclude `fixtures` from the repository's own scan.
- Make `--policy` optional for `check`, `policy show`, and `contracts validate`:
  it defaults to `quality-policy.json` in the `--path` directory (or the current
  directory); a missing default still exits `2` and names the expected file.
- Update `rustix` from 1.1.5 to 1.1.6 to resolve RUSTSEC-2026-0486.
- Fail closed when a shallow Git clone truncates the 200-commit history window:
  history rules now exit `2` with an actionable `fetch-depth: 0` error, and
  `history_scope.shallow` plus the
  `git_history_truncated_by_shallow_clone_fetch_full_history` limitation disclose
  the cause instead of reporting truncated history as clean.
- Add native in-memory Git-reference Debt Envelope ratcheting (`--base <REF>`)
  for both `--staged` and `--path` scans, enforcing implementation-scoped
  non-regression without committed baseline files, and print the ratchet
  burndown summary (`base`, `legacy`, `delta`, `new_ignores`, and
  `weakened_rules`) in table stdout, compact `--log` stdout, the Finding Log,
  and JSON reports.
- Add default-on anti-gaming guardrails and run-completeness attestation
  (`attestation` on every report; burndown lock, suppression-growth blocking,
  and required-rule policy-weakening detection under `--base <REF>`).
- Add `test_suite` support to `source_roles` (`pytest`, `unittest`, `vitest`,
  `jest`, `playwright`, and `cargo_test`): canonical test cases and test blocks
  exceeding `function_lines` are surfaced as auditable `reported_by_role` review
  signals. Test helpers and other rules' findings in test code are non-blocking
  indicators (see the calibration entry below).
- Fix `--base` so diverging one copy of a Duplicate Code pair no longer counts
  as a regression. A duplicate pair's burden is now its shared token count
  (`intersection`) instead of `intersection + union`. Enable
  `duplicate_functions` and `unused_code` as `required` by default in the Rust,
  Python, and TypeScript starter policies, and document their burden semantics
  in `docs/report-interface.md`.
- Add fail-closed CI gate templates in `examples/ci/` (GitHub Actions and Azure
  Pipelines with full history, a pinned release, `--base`, and always-uploaded
  artifacts), a governance CODEOWNERS snippet, and a branch-protection
  checklist. `tests/ci_templates.rs` keeps the template pin equal to the crate
  version.
- Add `smells doctor [--policy FILE] [--path DIR] [--base REF] [--format table|json]`, a
  zero-flag integration preflight, colored 0–100 codebase health scorecard, and
  burndown audit. When `--policy` or `--base` is omitted, it auto-discovers a
  single policy file (or falls back with a warning to the built-in Rust/Python/
  TypeScript starter) and compares against `origin/main` or `origin/master`
  when present; checks policy validity and scanner pin, Git history
  completeness, suppression health, `AGENTS.md`/`CLAUDE.md` wiring, and
  CODEOWNERS coverage; summarizes all 23 smells (`blocking` vs. `ignored` vs.
  `review`) plus the top 5 hotspot files. Broken prerequisites or scan errors
  exit `2`; governance gaps and diagnostic smell summaries exit `0`.
- Parse TypeScript tagged templates with type arguments, such as
  ``sql<Row>`SELECT 1` ``. Tree-sitter-typescript 0.23.2, the latest release,
  recovers them with a zero-width `!`, and the scan exited `2` with a parse
  error. That inserted `!` is no longer a syntax error; every other error or
  missing node still exits `2`.
- Parse TypeScript import types that tree-sitter-typescript 0.23.2 rejects:
  `import("m").T<U>`, `import("m").T[K]`, and `import("m")` or
  `typeof import("m").f` inside call type arguments. When a file fails its first
  parse, Smells reparses a same-length copy with these import types blanked.
  Dynamic `import("m")` calls are left as they are, and any error that remains
  still exits `2`.
- Parse invalid `\u` and `\x` escapes in tagged templates, such as
  ``String.raw`C:\Users\alice\user.ts` ``. ECMAScript allows them because the
  tag receives the raw text, but tree-sitter rejects them. The reparse turns
  each into the same-length `\q`. The same escapes in an untagged template or a
  string are real syntax errors and still exit `2`. This also covers a template
  that opens on such an escape, as in ``String.raw`\u-1` ``, where tree-sitter
  loses the template node. A template whose `${}` substitution holds a string,
  comment, regex or nested template is not rewritten, so it still exits `2`.
- Parse TypeScript `in` and `out` variance annotations on type parameters, such
  as `class Declaration<out Handler>`, which tree-sitter-typescript 0.23.2
  rejects. The reparse blanks the modifiers in type parameter lists that failed
  to parse; any error that remains still exits `2`.
- Key the portable fact cache on the TypeScript compatibility parser as well.
  Before this, a change to which files parse could still serve a cached parse
  error from an earlier build for files whose content had not changed.
- Calibrate against twelve production codebases (axum, ripgrep, tokio, serde,
  httpx, rich, flask, fastapi, zod, hono, vite and typescript-eslint). Rule
  versions are unchanged:
  - Test code never blocks. A finding whose every location is test code (a
    conventional test path, a Cargo `tests`, `examples` or `benches` target root,
    a Rust `#[cfg(test)]` module, or a `#[test]` or `#[rstest]` function) is an
    `indicator` whose explanation says so. `unused_code` reports test
    declarations as a separate `<path>::unused-test-declarations` finding, so a
    production declaration in the same file still blocks.
  - `duplicate_functions` joins passing pairs into connected families and
    reports one finding per family, with the other members as related symbols.
    Rust bodies with the same qualified symbol, which are mutually exclusive cfg
    alternatives such as `#[cfg(unix)]` and `#[cfg(windows)]` versions of one
    function, are not compared.
  - `unused_code` treats Rust `_` names, items under `#[allow]` or `#[expect]`
    naming `dead_code` or `unused`, exported TypeScript `_` declarations and
    single-underscore Python protocol hooks such as `_missing_` as reachable.
  - `python.function_arguments` no longer counts keyword-only parameters that
    have a default. Keyword-only parameters without a default still count.
  - `data_clumps` counts each distinct slot population once, so repeated
    identical signatures no longer exhaust `maximum_group_combinations`.
- Resolve `mod` declarations in Cargo target roots other than `lib.rs` and
  `main.rs` (files directly in `tests/`, `examples/` and `benches/`,
  `src/bin/*.rs` and `build.rs`) next to the root file. Before, the scan exited
  `2` with a missing module source.
- Accept two imports that bind one name, such as axum's `FromRef` trait and
  derive macro, unless a path resolves through that name.
- Name the line and column in every parse error:
  `parse error in <path>:<line>:<column>: <reason>`.
- Parse TypeScript overloaded call signatures, `export type * from`, and
  members named `abstract`, which tree-sitter-typescript 0.23.2 rejects.
- Grade a `smells doctor` scan with errors as `Incomplete` instead of a score,
  because the skipped code cannot be vouched for.

## 0.6.0 - 2026-10-14

- Classify high-confidence Python, TypeScript, and Rust database migration source
  roles without removing migration code from the measured corpus, and report exact
  canonical entrypoint findings as visible nonblocking review signals.
- Inventory recognized Drizzle, Prisma, SQLx, Diesel, and Refinery SQL migration
  artifacts as `migration_body_not_analyzed`, with staged/working parity and
  source-first budget enforcement.
- Advance the report contract from schema 7 to schema 11, adding versioned
  migration detectors, exact declaration provenance, unresolved artifact
  candidates, and SQLx integrity metadata; introduce rule version 2 for the line
  metrics whose applicability recognizes exact migration declaration roles.
- Identify the embedded concept-catalog and source-manifest versions directly in
  every JSON report while retaining `implementation_sha256` for exact artifact
  identity.
- Preserve each external registry's Trusted Publisher identity by dispatching PyPI,
  npmjs.com, and crates.io as top-level workflows, while validating the originating
  owner-approved Release run and waiting for every publication result.

## 0.5.0 - 2026-09-25

- Embed one versioned `when_to_ignore` record for each of the 23 canonical smells,
  inherited by all 28 rules and available offline through policy, table, log, and
  JSON interfaces.
- Add strict source-local `smells: ignore[exact-rule-id] -- reason` directives for
  the next declaration, with fail-closed validation and fully visible ignored
  findings.
- Introduce report schema 7 with compact measurements, matched findings, stable
  finding IDs, normalized guidance, and suppression audit records.
- Add the complete numbered Finding Log with an error/blocking/ignored/review
  Issue Index and exact detail-line pointers, while keeping hook stdout compact.
- Precompute alternative-interface features and charge pair budgets only for
  candidates that survive deterministic exact filters.
- Add offline cross-language suppression tests and an opt-in live freshness check
  for attributed Refactoring.Guru guidance.
- Report pair-budget failures against the exact incomplete rule with charged and
  configured comparison counts, and retain its suppressions as
  `unverified_due_to_incomplete_rule` instead of falsely declaring them unused.

## 0.4.0 - 2026-09-21

- Run all 28 rules and report all 23 canonical smells by default without requiring
  consuming repositories to generate provider-evidence bundles.
- Add built-in Rust, Python, and TypeScript collectors for type-shape, CRAP risk,
  dead code, architecture, inheritance, delegation, and coupling indicators.
- Capture up to 200 Git commits automatically for divergent-change and
  shotgun-surgery indicators, binding the captured history into `input_sha256`;
  selected history rules are explicitly incomplete when Git is unavailable.
- Keep `--evidence` as an optional per-rule higher-fidelity override.
- Report only scanner-owned inputs as required; compiler, coverage, type, test,
  and project-contract bundles are optional overrides.
- Preserve non-negotiable Refactoring.Guru research: an agent must open and read
  the exact emitted URL before review or remediation, or stop as incomplete.

## 0.3.0 - 2026-09-20

- Prepare the prebuilt scanner wheels as the `smells` PyPI tool package.
- Add `@mindful-time/smells` plus five exact-version native npm packages without
  install scripts or runtime downloads, published identically to npmjs.com and
  GitHub Packages.
- Prepare the verified Rust source package as `smells` on crates.io.
- Bootstrap the first npm release interactively from the immutable GitHub Release;
  subsequent npm releases use tokenless Trusted Publishing.
- Verify every registry by installing the released version after publication.

## 0.2.1 - 2026-09-20

- License the project under the OSI-approved MIT License.
- Embed `LICENSE` in standalone archives and binary Python wheels.

## 0.2.0 - 2026-09-20

- Activate all 28 policy rules by default for Rust, Python, and TypeScript.
- Add deterministic uv-style policy group selection and resolved-policy reporting.
- Add prebuilt standalone archives and binary Python wheels for five host targets.
- Add fork-only pull-request CI, release checksums, CycloneDX/SPDX SBOMs, and
  signed immutable GitHub Release attestations.

## 0.1.0 - 2026-09-19

- Initial deterministic multi-language scanner and self-hosted quality gates.
