# smells

[![CI](https://github.com/mindful-time/smells/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/mindful-time/smells/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/smells)](https://pypi.org/project/smells/)
[![npm](https://img.shields.io/npm/v/%40mindful-time%2Fsmells)](https://www.npmjs.com/package/@mindful-time/smells)
[![crates.io](https://img.shields.io/crates/v/smells)](https://crates.io/crates/smells)
[![PyPI downloads/month](https://api.pepy.tech/personalized-badge/smells?period=month&units=none&left_color=grey&right_color=green&left_text=downloads%2Fmonth)](https://pepy.tech/projects/smells)
[![License](https://img.shields.io/github/license/mindful-time/smells)](LICENSE)

**Stop code smells and AI slop before they reach `main`.**

`smells` checks Rust, Python and TypeScript for all 23 [Refactoring.Guru](https://refactoring.guru/refactoring/smells) code smells. It is one native binary, gives the same answer every run, and never executes your code. Point it at `main` and it blocks only the smells your branch adds, so old debt never stops a commit.

## Install

```sh
uv tool install smells==0.7.0                                    # Python
npm install --save-dev --save-exact @mindful-time/smells@0.7.0   # Node
cargo install --locked --version 0.7.0 smells                    # Rust
```

Prebuilt binaries for macOS, Linux and Windows are on the [release page](https://github.com/mindful-time/smells/releases/tag/v0.7.0).

## Get started

**1. See where you stand.** No config needed:

```sh
smells doctor
```

```text
Codebase Health
  ██████████████████░░  90/100  Great
  1 files · 1 blocking · 0 ignored · 0 review warnings · 1/23 smells matched

Smells (hard blockers first)
  Smell                Category  Blocking  Ignored  Review
  Long Parameter List  bloaters         1        0       0

Next steps
  1. smells check
```

Doctor also checks your setup (policy, Git history, coverage, `AGENTS.md`, CODEOWNERS). If it can't find coverage, it offers to measure it, so CRAP scores use real numbers.

**2. Add a policy.** Doctor's next steps print the exact command for your language. For Python:

```sh
curl -fsSLo quality-policy.json \
  https://raw.githubusercontent.com/mindful-time/smells/v0.7.0/examples/python-quality-policy.json
```

Use `typescript-quality-policy.json` for TypeScript or `quality-policy.json` for Rust.

**3. Run the check.**

```sh
smells check --base origin/main
```

```text
Scan summary | verdict: blocked_by_required_patterns | findings: 2 | blocking: 1 | ignored: 0 | review: 1 | errors: 0
  F000001 | blocking | python.function_arguments | orders.py:1:1
Finding Log: smells-findings.log
```

Every finding goes to `smells-findings.log`, with blocking findings at the top. Each one says why it matched, how to fix it, and when it's fine to ignore. Add `smells-findings.log` and `lcov.info` to `.gitignore`.

Leave off `--base` to check the whole repository.

## Exit codes

| Exit | Meaning |
| ---: | --- |
| `0` | Nothing blocking. Review findings may still be in the log |
| `1` | A blocking smell was added or got worse. Fix it |
| `2` | The scan couldn't finish (bad policy, parse error, shallow clone…). Never treat it as clean |

**Blocking or review.** In the starter policies, Long Method, Long Parameter List, Large Class, Duplicate Code and Dead Code block (`"mode": "required"`). Every other smell is a review finding (`"mode": "report"`) and never fails the check. Set any rule's `mode` to `"required"` to make it block.

**Intentional code.** If the finding's `when_to_ignore` guidance applies, add a comment above the declaration with the rule ID and a reason:

```python
# smells: ignore[python.function_arguments] -- stable external API
def publish(a, b, c, d, e, f, g, h):
    ...
```

Rust and TypeScript use `//`. Ignored findings stay in the report, and an unknown or unused ignore exits `2`.

## Pre-commit

With [`pre-commit`](https://pre-commit.com/), `--staged` scans exactly what you're committing:

```yaml
repos:
  - repo: local
    hooks:
      - id: smells
        name: smells
        language: system
        entry: uvx --from smells==0.7.0 smells check --staged --base HEAD
        pass_filenames: false
```

Node projects can use `npx --no-install smells check …`, and Rust projects can call `smells check …` directly.

A commit that adds a new `smells: ignore` is always blocked, so a person reviews every ignore. Once reviewed, commit with `git commit --no-verify`.

## CI

Copy a template from [`examples/ci/`](examples/ci/): [GitHub Actions](examples/ci/github-actions-smells.yml) or [Azure Pipelines](examples/ci/azure-pipelines-smells.yml). Fetch full history (`fetch-depth: 0`); a shallow clone exits `2` rather than passing. More in [CI setup](docs/project-integration.md#ci-gate-templates).

## Coding agents

No MCP server or plugin needed. Add this to `AGENTS.md` (and `@AGENTS.md` to `CLAUDE.md` for Claude Code):

```md
## Smell review

- Run `smells check --base origin/main --report smells-report.json` before reviewing or refactoring code smells.
- Treat exit code `2` as an incomplete scan, never as clean.
- Open `smells-findings.log`, start at the Issue Index, and fix every `scanner_error` and `blocking` finding first.
- Before fixing or ignoring a finding, read its Refactoring.Guru link and `when_to_ignore` guidance.
- Add `smells: ignore[exact-rule-id] -- reason` only when that guidance really applies, never just to pass a hook.
- Make small, behavior-preserving edits, then rerun the tests and `smells check`.
```

Works with Claude Code, Codex, Pi and any agent that can run a command.

## What it checks

| Language | Files | Starter policy |
| --- | --- | --- |
| Rust | `.rs` | [`quality-policy.json`](examples/quality-policy.json) |
| Python | `.py`, `.pyi` | [`python-quality-policy.json`](examples/python-quality-policy.json) |
| TypeScript | `.ts`, `.tsx`, `.mts`, `.cts` | [`typescript-quality-policy.json`](examples/typescript-quality-policy.json) |

<details>
<summary>All 23 smells, checked in every language</summary>

| Category | Smells |
| --- | --- |
| **Bloaters** | Long Method, Large Class, Primitive Obsession, Long Parameter List, Data Clumps |
| **OO Abusers** | Alternative Classes with Different Interfaces, Refused Bequest, Switch Statements, Temporary Field |
| **Change Preventers** | Divergent Change, Parallel Inheritance Hierarchies, Shotgun Surgery |
| **Dispensables** | Comments, Duplicate Code, Data Class, Dead Code, Lazy Class, Speculative Generality |
| **Couplers** | Feature Envy, Inappropriate Intimacy, Incomplete Library Class, Message Chains, Middle Man |

</details>

Not supported yet: Java, Kotlin, JavaScript and JSX. `smells` sits next to your compiler, type checker, tests and security tools; it doesn't replace them.

## Commands

| Command | What it does |
| --- | --- |
| `smells doctor` | Health score, smell table, setup checks and next steps |
| `smells check` | Strict scan of the current folder. `--base REF` checks only what changed, `--path DIR` scans another folder |
| `smells check --staged --base HEAD` | Scan the files staged for commit |
| `smells policy show` | Show the rules your policy runs |
| `smells contracts validate` | Validate your policy file |
| `smells rules` | List every rule in a language pack |

Run `smells <command> --help` for every option.

## Docs

- [Project integration](docs/project-integration.md): policies, monorepos, tests and migrations, coverage and CRAP, CI
- [Report format and the `--base` ratchet](docs/report-interface.md)
- Rule details: [Rust](docs/rust-rule-contracts.md) · [Python](docs/python-rule-contracts.md) · [TypeScript](docs/typescript-rule-contracts.md)
- Internals: [Architecture](docs/architecture.md) · [Migration analysis](docs/migration-code-analysis.md) · [Provider evidence](docs/provider-evidence.md) · [Distribution](docs/package-distribution.md) · [Releases](docs/release-process.md) · [Governance](docs/repository-governance.md)

Every package is built from one signed, immutable GitHub Release with checksums and CycloneDX/SPDX SBOMs.

## Contributing

Run the full local gate before committing or pushing. Pull requests come from a fork.

```sh
make pre-commit-push
```

[MIT License](LICENSE)
