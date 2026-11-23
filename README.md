# smells

[![CI](https://github.com/mindful-time/smells/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/mindful-time/smells/actions/workflows/ci.yml)
[![GitHub Release](https://img.shields.io/github/v/release/mindful-time/smells?display_name=tag&sort=semver)](https://github.com/mindful-time/smells/releases/latest)
[![PyPI](https://img.shields.io/pypi/v/smells)](https://pypi.org/project/smells/)
[![PyPI downloads/month](https://api.pepy.tech/personalized-badge/smells?period=month&units=none&left_color=grey&right_color=green&left_text=PyPI%20downloads%2Fmonth)](https://pepy.tech/projects/smells)
[![npm](https://img.shields.io/npm/v/%40mindful-time%2Fsmells)](https://www.npmjs.com/package/@mindful-time/smells)
[![npm downloads](https://img.shields.io/npm/dw/%40mindful-time%2Fsmells)](https://www.npmjs.com/package/@mindful-time/smells)
[![crates.io](https://img.shields.io/crates/v/smells)](https://crates.io/crates/smells)
[![crates.io downloads](https://img.shields.io/crates/d/smells)](https://crates.io/crates/smells)
[![License](https://img.shields.io/github/license/mindful-time/smells)](LICENSE)

**Stop code smells and AI slop before they reach `main`.**

`smells` is a fast, deterministic code-smell scanner for **Rust, Python, TypeScript, and TSX**. It runs **28 built-in rules across all 23 Refactoring.Guru smell categories**, ratchets pull requests against Git history with `--base`, and gives humans and coding agents auditable evidence instead of guesswork.

- **No baseline file to maintain or game:** `--base origin/main` compares your branch against `main` in memory. Legacy debt stays visible, new or worse smells block, and paying down debt locks in automatically.
- **Built for humans and coding agents:** One scan writes a compact numbered Finding Log and a normalized JSON report with exact lines, source excerpts, Refactoring.Guru links, and embedded `when_to_ignore` guidance. No MCP server, plugin, or LLM required.
- **Fail-closed by default:** Shallow Git clones, parser errors, budget exhaustion, unapproved suppression growth, and weakened policy rules exit `2` instead of passing silently.
- **Safe everywhere:** Single native binary. Never runs your app code, tests, compilers, or package scripts.

[Quick start](#quick-start) · [How ratcheting works](#ratchet-existing-repos-with---base) · [CI & hooks](#add-it-to-ci-and-pre-commit) · [Coding agents](#use-with-coding-agents) · [Commands](#commands)

---

## Quick start

### 1. Install `smells`

Pick any channel—every one installs the same native Rust CLI:

```sh
# Python tooling (uv)
uv tool install smells==0.7.0

# Node projects
npm install --save-dev --save-exact @mindful-time/smells@0.7.0

# Rust tooling
cargo install --locked --version 0.7.0 smells
```

Standalone binaries for macOS, Linux, and Windows are also on the [v0.7.0 GitHub Release](https://github.com/mindful-time/smells/releases/tag/v0.7.0).

### 2. Add a starter policy and run `smells doctor`

Download the starter policy for your language ([Rust](examples/quality-policy.json), [Python](examples/python-quality-policy.json), or [TypeScript/TSX](examples/typescript-quality-policy.json)):

```sh
curl -fsSLo quality-policy.json \
  https://raw.githubusercontent.com/mindful-time/smells/v0.7.0/examples/python-quality-policy.json

smells doctor --policy quality-policy.json
```

`smells doctor` checks that your policy is valid, your Git clone is not shallow, suppressions are well-formed, and governance files (`AGENTS.md`, `CODEOWNERS`) are wired up. Commit `quality-policy.json` so developers, CI, and coding agents all share one contract.

### 3. Run your first scan

**Ratchet a pull request or branch against `main` (best for existing repos):**

```sh
smells check \
  --path . \
  --policy quality-policy.json \
  --base origin/main \
  --format table \
  --log smells-findings.log \
  --report smells-report.json
```

**Or scan the full repo (best for greenfield repos or health audits):**

```sh
smells check \
  --path . \
  --policy quality-policy.json \
  --format table \
  --log smells-findings.log \
  --report smells-report.json
```

Terminal output stays short and points straight to `smells-findings.log` (for humans and agents) and `smells-report.json` (for automation). Add `--format json` to print JSON directly to stdout.

---

## How it works

```text
source + checked-in policy (+ optional --base REF)
                      ↓
   28 deterministic rules across 23 smell categories
                      ↓
   compact Finding Log + normalized JSON report + attestation
                      ↓
   exit 0 (pass)  |  exit 1 (blocking smell)  |  exit 2 (incomplete/invalid)
```

### Exit codes

| Exit | Meaning | What to do |
| ---: | --- | --- |
| `0` | Passed (no unsuppressed blocking smells or regressions) | Review any non-blocking or `ignored_match` signals |
| `1` | One or more `required` rules matched or regressed | Open `smells-findings.log` and fix the blocking findings |
| `2` | Scan was invalid or incomplete | Fix the policy, parser, shallow Git history, suppression, or budget error |

Exit `2` **never** means clean. Errors always outrank violations.

### Hard blockers vs. review warnings (`smells check` vs. `smells doctor`)

Every scan checks **all 28 rules across all 23 smells**, separated into two levels so CI stays strict without drowning humans or coding agents in false positives:

- **Hard blockers (`"mode": "required"`):** Fail `smells check` with exit `1` and always appear at the top of the Issue Index. Starter policies set **Bloaters** (Long Method, Long Parameter List, Large Class), **Duplicate Code** (`duplicate_functions`), and **Dead Code** (`unused_code`) to `required` by default—and you can flip any additional rule to `"required"` in `quality-policy.json`.
- **Review warnings (`"mode": "report"`):** Structural and history heuristics (such as `data_class`, `lazy_class`, `function_crap`, `divergent_change`, `shotgun_surgery`, and `navigation_chains`) that still run on every scan and appear as non-blocking `review` findings in `smells-findings.log` and JSON, without bloating the 4-line stdout summary or failing CI on normal DTOs/wrappers.

Use the two commands for their distinct jobs:

| Command | Role | Output |
| --- | --- | --- |
| **`smells check --base origin/main`** | **PR & pre-commit gate** | 4-line stdout summary with burndown `delta` + full Finding Log & JSON; exits `1` on new/worse `required` smells, `2` on broken setup |
| **`smells doctor [--base REF]`** | **Setup, diagnostics & burndown** | Audits policy, Git history, suppressions, `AGENTS.md`, `CODEOWNERS`, prints a compact 23-smell breakdown (`blocking` vs. `review`) + top 5 hotspot files, and with `--base REF` shows total & per-smell burndown since that commit/tag; exits `2` only if setup/scan is broken |

---

## Ratchet existing repos with `--base`

You do not have to clean up years of legacy debt on day one. Pass `--base <REF>` (for example `--base origin/main` in CI, or `--base HEAD` in a pre-commit hook) and Smells computes an in-memory **Debt Envelope** from that Git commit:

- **Legacy debt does not block you:** Existing violations stay visible in the report without failing unrelated PRs.
- **New or worse smells block (exit `1`):** Adding a new violation or making an existing one worse fails the check.
- **Burndown locks in automatically and shows on stdout:** Every `--base` scan prints `base`, `legacy`, `delta` (e.g. `-3` when you pay down debt), `new_ignores`, and any `weakened_rules` right on terminal/CI stdout, in `smells-findings.log`, and in `smells-report.json`—no baseline JSON file to regenerate or merge-conflict.
- **Anti-gaming guardrails are on by default:** Adding new `smells: ignore` suppressions (`blocked_by_suppression_growth`) or weakening `required` rules (`blocked_by_policy_weakening`) fails closed unless reviewed intentionally.
- **Blocks bloaters, Duplicate Code, and Dead Code out of the box:** Starter policies set bloaters plus `duplicate_functions` (Duplicate Code) and `unused_code` (Dead Code) to `required`, while all remaining smells run and appear as `review` signals (and any rule can be switched to `required` in `quality-policy.json`).

Every report also includes a default-on `attestation` block recording exact rule counts, policy digest, history completeness, and exit code.

---

## Add it to CI and pre-commit

### Copy-ready CI gates (GitHub Actions & Azure Pipelines)

Ready-to-use, fail-closed templates live in [`examples/ci/`](examples/ci/):

- [`examples/ci/github-actions-smells.yml`](examples/ci/github-actions-smells.yml) — independent `smells` job on `pull_request` and `merge_group`, `fetch-depth: 0`, pinned `0.7.0`, `--base`, and always-uploaded Finding Log + JSON report.
- [`examples/ci/azure-pipelines-smells.yml`](examples/ci/azure-pipelines-smells.yml) — independent job with `fetchDepth: 0`, `--base`, and always-published artifacts.
- [`examples/ci/CODEOWNERS`](examples/ci/CODEOWNERS) — requires owner review for changes to `*quality-policy.json`, `AGENTS.md`, `CLAUDE.md`, and CI gate files.

> **Important:** Always check out full Git history (`fetch-depth: 0` / `fetchDepth: 0`). History rules (`Divergent Change`, `Shotgun Surgery`) use up to 200 commits and exit `2` on a shallow clone instead of reporting a false clean result.

See [CI gate templates & branch-protection checklist](docs/project-integration.md#ci-gate-templates) for setup details.

### Pre-commit hook (`--staged`)

`--staged` scans the exact Git index snapshot rather than unstaged working-tree edits. With [`pre-commit`](https://pre-commit.com/):

```yaml
repos:
  - repo: local
    hooks:
      - id: smells
        name: deterministic smell scan
        language: system
        entry: uvx --from smells==0.7.0 smells check --staged --base HEAD --policy quality-policy.json --format table --log smells-findings.log --report smells-report.json
        pass_filenames: false
```

Node projects can use `npx --no-install smells check ...`; Rust projects can call `smells check ...` directly. An inactive shell hook example is also in [`hooks/pre-commit.example`](hooks/pre-commit.example).

---

## Use with coding agents

`smells` needs no MCP server, plugin, or model-specific API. The scanner supplies deterministic evidence; the agent researches the exact Refactoring.Guru reference, verifies context, and makes a small behavior-preserving fix.

Add this contract to your repository's root `AGENTS.md` (and put `@AGENTS.md` in `CLAUDE.md` if you use Claude Code):

```md
## Smell review

- Run `smells check --path . --policy quality-policy.json --base origin/main --format table --log smells-findings.log --report smells-report.json` before reviewing or refactoring code smells.
- Treat exit code `2` as an incomplete scan, never as clean.
- Open `smells-findings.log`, start at the Issue Index, and resolve every `scanner_error` and `blocking` finding first (always listed at the top).
- Check `review` warnings for files touched in the current change; use `smells doctor --policy quality-policy.json` for a compact repo-wide diagnostic overview instead of reading every legacy `review` entry.
- Before fixing or suppressing a finding, open and read its exact Refactoring.Guru URL and embedded `when_to_ignore` guidance. If the URL cannot be consulted, report research as incomplete and stop.
- Verify findings against declarations, callers, and tests. Add `smells: ignore[exact-rule-id] -- reason` only when the verified exception applies—never merely to pass a hook.
- Prefer small, behavior-preserving edits, then rerun project tests and `smells check`.
```

Works out of the box with **Codex**, **Claude Code**, **Pi**, and any other CLI-capable agent.

---

## Real-world code: suppressions, tests, migrations, and monorepos

### Audited source suppressions

When a finding is genuinely justified after checking its Refactoring.Guru `when_to_ignore` guidance, suppress only the next declaration with an explicit rule ID and reason:

```python
# smells: ignore[python.function_arguments] -- stable external API
def publish(a, b, c, d):
    ...
```

```rust
// smells: ignore[rust.function_arguments] -- compatibility boundary
fn publish(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) {}
```

```typescript
// smells: ignore[typescript.function_arguments] -- stable external API
publish(a: number, b: number, c: number, d: number): void {}
```

Suppressed findings stay visible in reports as `ignored_match` (never hidden). Malformed, unknown, duplicate, misplaced, or unused directives fail closed with exit `2`.

### Tests and database migrations (`source_roles`)

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

- **Test suites** (`pytest`, `unittest`, `vitest`, `jest`, `playwright`, `cargo_test`): long canonical test cases/blocks (`test_*`, `describe`/`it`/`test`, `#[test]`) become visible non-blocking `reported_by_role` review signals, while test helpers and all other rules stay strictly enforced.
- **Database migrations** (Alembic, Django, TypeORM, MikroORM, Kysely, Knex, Sequelize/Umzug, Prisma, Drizzle, SeaORM, SQLx, Diesel, Refinery): canonical migration entrypoints stay measured as non-blocking review signals and SQL bodies are inventoried explicitly. See [migration code analysis](docs/migration-code-analysis.md).

### Monorepos, rule groups, and optional evidence

- **Monorepos:** Run from the repo root. Files are grouped automatically under their nearest `Cargo.toml`, `pyproject.toml`, or `package.json` (or `__unowned__`). Mixed-language repos use one policy file and one scan per language pack.
- **Rule groups:** Starter policies run `all` 28 rules. Narrow intentionally with `--only-group`, `--group`, or `--no-group` (`source`, `evidence`, or any of the five Refactoring.Guru categories). Preview with `smells policy show`.
- **Optional higher-fidelity evidence:** Built-in collectors work standalone with zero setup. If you already produce compiler, coverage, type, or architecture facts, pass `--evidence provider-evidence.json` (cannot be combined with `--base`). See [provider evidence](docs/provider-evidence.md).

---

## Supported languages & smells

| Language pack | Files scanned | Active rules | Class-like model |
| --- | --- | ---: | --- |
| `rust-v1` | `.rs` | 28 | `struct`/`enum` state + inherent & trait `impl` methods |
| `python-v1` | `.py`, `.pyi` | 28 | Classes, direct methods, and `self`/`cls` fields |
| `typescript-v1` | `.ts`, `.tsx`, `.mts`, `.cts` | 28 | Classes, abstract classes, interfaces, signatures, and authored fields |

All three packs cover the same **23 canonical Refactoring.Guru smells** (28 executable rules):

| Category | Smells covered |
| --- | --- |
| **Bloaters** | Long Method, Large Class, Primitive Obsession, Long Parameter List, Data Clumps |
| **OO Abusers** | Alternative Classes with Different Interfaces, Refused Bequest, Switch Statements, Temporary Field |
| **Change Preventers** | Divergent Change, Parallel Inheritance Hierarchies, Shotgun Surgery |
| **Dispensables** | Comments, Duplicate Code, Data Class, Dead Code, Lazy Class, Speculative Generality |
| **Couplers** | Feature Envy, Inappropriate Intimacy, Incomplete Library Class, Message Chains, Middle Man |

*Not yet supported: Java, Kotlin, JavaScript, and JSX.* Use Smells alongside your compiler, type checker, tests, coverage, and security tools—it complements them rather than replacing them.

---

## Commands

| Command | What it does |
| --- | --- |
| `smells check --path DIR --policy FILE [--base REF]` | Scan a directory (optionally ratcheted against a Git ref) |
| `smells check --staged --policy FILE [--base REF]` | Scan the exact Git staged index snapshot |
| `smells doctor --policy FILE [--path DIR] [--base REF]` | Audit setup/governance, print compact 23-smell & hotspot diagnostics, and optionally show burndown since `REF` |
| `smells contracts validate --policy FILE` | Validate a policy file against its schema and rule pack |
| `smells policy show --policy FILE` | Print the resolved rule set and parameters |
| `smells rules [--rule-pack PACK]` | List all rules, categories, and versions in a language pack |
| `smells --version` | Print the installed scanner version |

---

## Release provenance & documentation

| Channel | Package | Install command |
| --- | --- | --- |
| PyPI | [`smells`](https://pypi.org/project/smells/0.7.0/) | `uv tool install smells==0.7.0` |
| npm | [`@mindful-time/smells`](https://www.npmjs.com/package/@mindful-time/smells/v/0.7.0) | `npm install --save-dev --save-exact @mindful-time/smells@0.7.0` |
| crates.io | [`smells`](https://crates.io/crates/smells/0.7.0) | `cargo install --locked --version 0.7.0 smells` |
| GitHub | [v0.7.0 release](https://github.com/mindful-time/smells/releases/tag/v0.7.0) | Prebuilt archive for macOS, Linux, or Windows |

All packages originate from one signed, immutable GitHub Release with checksums and CycloneDX/SPDX SBOMs.

### Further reading

- [Project integration & CI setup](docs/project-integration.md)
- [Report interface & `--base` ratchet semantics](docs/report-interface.md)
- [Migration code analysis](docs/migration-code-analysis.md)
- [Provider evidence](docs/provider-evidence.md)
- Rule contracts: [Rust](docs/rust-rule-contracts.md) · [Python](docs/python-rule-contracts.md) · [TypeScript](docs/typescript-rule-contracts.md)
- Internals & governance: [Architecture](docs/architecture.md) · [Package distribution](docs/package-distribution.md) · [Release process](docs/release-process.md) · [Repository governance](docs/repository-governance.md)

### Developing `smells`

Run the full local gate before committing or pushing (PRs must originate from a fork):

```sh
make pre-commit-push
```

Licensed under the [MIT License](LICENSE). Repository: [mindful-time/smells](https://github.com/mindful-time/smells).
