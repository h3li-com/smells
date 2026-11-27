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

`smells` is a fast, deterministic code-smell scanner for **Rust, Python, TypeScript, and TSX**. It checks all 23 [Refactoring.Guru](https://refactoring.guru/refactoring/smells) smells with 28 rules, and it can compare your branch against `main` so old debt doesn't block you but new smells do.

It is a single native binary. It never runs your code, tests, or package scripts.

---

## Quick start

### 1. Install

Use whichever package manager you already have:

```sh
uv tool install smells==0.7.0                                    # Python
npm install --save-dev --save-exact @mindful-time/smells@0.7.0   # Node
cargo install --locked --version 0.7.0 smells                    # Rust
```

Prebuilt binaries for macOS, Linux, and Windows are on the [v0.7.0 release page](https://github.com/mindful-time/smells/releases/tag/v0.7.0).

### 2. See how your code is doing

From your repository root:

```sh
smells doctor
```

No config needed. You get a 0–100 health score, a table of all 23 smells, your worst files, and a list of setup tips.

### 3. Add a policy and run the check

`smells check` is the strict gate you put in pre-commit and CI. It reads `quality-policy.json` from the folder you scan. `smells doctor` ends with **Next steps** that name the exact download command for your language, for example:

```sh
curl -fsSLo quality-policy.json \
  https://raw.githubusercontent.com/mindful-time/smells/v0.7.0/examples/python-quality-policy.json
```

(Use `typescript-quality-policy.json` for TypeScript or `quality-policy.json` for Rust. If you forget, `smells check` prints this command and exits `2`.)

Then run:

```sh
smells check --base origin/main
```

`smells check` scans the current folder. The terminal shows a short summary and every finding goes to `smells-findings.log`. Open it and start at the **Issue Index** at the top: blocking findings come first, each with a file, line, and a pointer to its details. Add `smells-findings.log` to `.gitignore`.

`--base origin/main` only fails on smells your branch adds or makes worse. Leave it off to check the whole repository. Pass `--format table` to print every finding in the terminal instead.

---

## Reading the result

| Exit | Meaning | What to do |
| ---: | --- | --- |
| `0` | No blocking smells | Optionally look at the `review` findings in the log |
| `1` | A blocking smell was found (or got worse) | Fix the `blocking` findings at the top of the log |
| `2` | The scan could not finish | Fix the error it prints (bad policy, parser error, shallow Git clone, …). Exit `2` never means clean |

Findings come in two levels:

- **Blocking** (`"mode": "required"`): fails the check. The starter policies block Long Method, Long Parameter List, Large Class, Duplicate Code, and Dead Code.
- **Review** (`"mode": "report"`): every other smell. Listed in the log, never fails the check.

You can make any rule blocking by setting its `mode` to `"required"` in `quality-policy.json`.

### When a finding is intentional

Read the finding's `when_to_ignore` guidance in the log first. If it applies, put a comment above the declaration with the exact rule ID and a reason:

```python
# smells: ignore[python.function_arguments] -- stable external API
def publish(a, b, c, d):
    ...
```

Rust and TypeScript use `//` comments with the same format. Ignored findings stay visible in the report as `ignored_match`. A typo'd, unknown, or unused ignore makes the scan exit `2`.

---

## Use it in pre-commit and CI

**Pre-commit** with [`pre-commit`](https://pre-commit.com/). `--staged` scans exactly what you are about to commit:

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

Node projects can use `npx --no-install smells check ...`. Rust projects can call `smells check ...` directly.

**CI:** copy a ready-made template from [`examples/ci/`](examples/ci/) ([GitHub Actions](examples/ci/github-actions-smells.yml), [Azure Pipelines](examples/ci/azure-pipelines-smells.yml)). Check out the full Git history (`fetch-depth: 0`): some rules read history, and a shallow clone exits `2` rather than passing. See [CI setup](docs/project-integration.md#ci-gate-templates).

---

## Use it with coding agents

No MCP server or plugin is needed. Add this to your repository's `AGENTS.md` (and put `@AGENTS.md` in `CLAUDE.md` for Claude Code):

```md
## Smell review

- Run `smells check --base origin/main --report smells-report.json` before reviewing or refactoring code smells.
- Treat exit code `2` as an incomplete scan, never as clean.
- Open `smells-findings.log`, start at the Issue Index, and fix every `scanner_error` and `blocking` finding first.
- Before fixing or ignoring a finding, read its Refactoring.Guru link and `when_to_ignore` guidance.
- Add `smells: ignore[exact-rule-id] -- reason` only when that guidance really applies, never just to pass a hook.
- Make small, behavior-preserving edits, then rerun the tests and `smells check`.
```

Works with Codex, Claude Code, Pi, and any agent that can run a command.

---

## Supported languages

| Language | Files | Policy file to start from |
| --- | --- | --- |
| Rust | `.rs` | [`examples/quality-policy.json`](examples/quality-policy.json) |
| Python | `.py`, `.pyi` | [`examples/python-quality-policy.json`](examples/python-quality-policy.json) |
| TypeScript | `.ts`, `.tsx`, `.mts`, `.cts` | [`examples/typescript-quality-policy.json`](examples/typescript-quality-policy.json) |

Each language checks the same 23 smells:

| Category | Smells |
| --- | --- |
| **Bloaters** | Long Method, Large Class, Primitive Obsession, Long Parameter List, Data Clumps |
| **OO Abusers** | Alternative Classes with Different Interfaces, Refused Bequest, Switch Statements, Temporary Field |
| **Change Preventers** | Divergent Change, Parallel Inheritance Hierarchies, Shotgun Surgery |
| **Dispensables** | Comments, Duplicate Code, Data Class, Dead Code, Lazy Class, Speculative Generality |
| **Couplers** | Feature Envy, Inappropriate Intimacy, Incomplete Library Class, Message Chains, Middle Man |

Not supported yet: Java, Kotlin, JavaScript, and JSX. `smells` complements your compiler, type checker, tests, and security tools; it doesn't replace them.

---

## Commands

| Command | What it does |
| --- | --- |
| `smells doctor` | Health score, smell table, worst files, setup tips, and next steps. No config needed |
| `smells check` | Strict scan of the current folder, findings in `smells-findings.log`. Add `--base origin/main` to check only what changed, or `--path DIR` for another folder |
| `smells check --staged --base HEAD` | Strict scan of the files staged for commit |
| `smells policy show` | Show which rules your policy runs and their settings |
| `smells contracts validate` | Check that your policy file is valid |
| `smells rules` | List every rule in a language pack |
| `smells --version` | Print the installed version |

Run `smells --help` (or `smells check --help`) for every option.

---

## Learn more

- [Project integration](docs/project-integration.md): policies, tests and migrations (`source_roles`), monorepos, coverage for CRAP scores, rule groups, and CI
- [Report format and `--base` ratchet](docs/report-interface.md)
- [Migration code analysis](docs/migration-code-analysis.md)
- [Provider evidence](docs/provider-evidence.md)
- Rule details: [Rust](docs/rust-rule-contracts.md) · [Python](docs/python-rule-contracts.md) · [TypeScript](docs/typescript-rule-contracts.md)
- Internals: [Architecture](docs/architecture.md) · [Package distribution](docs/package-distribution.md) · [Release process](docs/release-process.md) · [Repository governance](docs/repository-governance.md)

All packages are built from one signed, immutable GitHub Release with checksums and CycloneDX/SPDX SBOMs.

## Developing `smells`

Run the full local gate before committing or pushing (PRs must come from a fork):

```sh
make pre-commit-push
```

Licensed under the [MIT License](LICENSE). Repository: [mindful-time/smells](https://github.com/mindful-time/smells).
