# smells

Install once. Scan every configured smell. Give humans and coding agents evidence they can audit.

`smells` is a standalone, deterministic code-smell scanner for Rust, Python, TypeScript, and TSX.

A default `smells check` runs all 28 active rules across all 23 Refactoring.Guru smell categories. The scanner collects its own bounded evidence, so every repository gets a complete review without building a custom evidence pipeline.

The scanner—not an LLM—decides whether a rule matches. Humans and agents use the resulting locations, measurements, thresholds, source excerpts, and research links to decide what should change.

[Quick start](#five-minute-quick-start) · [Use cases](#when-to-use-smells) · [Coding agents](#use-smells-with-coding-agents) · [Integrate](#integrate-smells-into-a-repository) · [Reports](#understand-the-result)

## What Smells gives you

```text
source + checked-in policy
          ↓
28 deterministic rules across 23 smell categories
          ↓
auditable table and JSON evidence
          ↓
human or coding-agent review grounded in Refactoring.Guru
```

- Exact repository-relative files, lines, columns, symbols, and source excerpts.
- The observed measurement, comparison, configured threshold, and match status.
- Separate blocking violations and non-blocking review signals.
- An explicit result for every canonical smell, including excluded, inapplicable, and incomplete states.
- A mandatory Refactoring.Guru URL and research contract in every matched finding.
- Stable JSON for Codex, Claude Code, Pi, CI jobs, hooks, and other automation.
- Fail-closed exit codes when parsing, history, ownership, inputs, or analysis budgets are incomplete.

`smells` does not execute the repository's application code, builds, tests, package scripts, or compilers.

## When to use Smells

| Use case | What Smells contributes |
| --- | --- |
| AI-assisted code review | Gives the agent deterministic findings instead of asking it to discover smells from scratch |
| Refactoring work | Grounds each proposal in a location, threshold, source excerpt, and required design reference |
| Pull-request CI | Blocks configured violations and invalid or incomplete scans with distinct exit codes |
| Pre-commit review | Scans the complete staged Git snapshot before a commit is created |
| Codebase health audit | Produces a reproducible baseline across all 23 smell categories |
| Monorepo triage | Assigns findings to the nearest Cargo, Python, or Node implementation manifest |
| Team policy | Keeps thresholds, modes, groups, exclusions, and budgets in a reviewed JSON file |
| Higher-fidelity analysis | Accepts optional compiler, coverage, type, test, or architecture evidence per rule |

Use Smells alongside formatting, compilation, type checking, tests, coverage, security scans, and human design review. It complements those checks; it does not replace them.

## Supported languages

| Language pack | Files scanned | Active built-in rules | Class-like model |
| --- | --- | ---: | --- |
| `rust-v1` | `.rs` | 28 | `struct`/`enum` state plus inherent and trait `impl` methods |
| `python-v1` | `.py`, `.pyi` | 28 | Classes, direct methods, and `self`/`cls` field assignments |
| `typescript-v1` | `.ts`, `.tsx`, `.mts`, `.cts` | 28 | Classes, abstract classes, interfaces, signatures, and authored fields |

All three packs cover the same 23 canonical categories. Some categories use more than one rule, which is why each pack contains 28 rules.

| Refactoring.Guru category | Smells covered |
| --- | --- |
| Bloaters | Long Method, Large Class, Primitive Obsession, Long Parameter List, Data Clumps |
| Object-Orientation Abusers | Alternative Classes with Different Interfaces, Refused Bequest, Switch Statements, Temporary Field |
| Change Preventers | Divergent Change, Parallel Inheritance Hierarchies, Shotgun Surgery |
| Dispensables | Comments, Duplicate Code, Data Class, Dead Code, Lazy Class, Speculative Generality |
| Couplers | Feature Envy, Inappropriate Intimacy, Incomplete Library Class, Message Chains, Middle Man |

Java, Kotlin, JavaScript, and JSX are not currently supported. React and React Native repositories can scan TypeScript and TSX, but not JavaScript, JSX, or native Android source.

## Five-minute quick start

### 1. Install the CLI

Choose one channel. Every channel provides the same Rust executable and rule behavior.

General-purpose installation with [uv](https://docs.astral.sh/uv/concepts/tools/):

```sh
uv tool install smells==0.4.0
smells --version
```

Inside a Node project:

```sh
npm install --save-dev --save-exact @mindful-time/smells@0.4.0
npx --no-install smells --version
```

With a Rust toolchain:

```sh
cargo install --locked --version 0.4.0 smells
smells --version
```

Prebuilt archives for macOS, Linux, and Windows are available from the [v0.4.0 GitHub Release](https://github.com/mindful-time/smells/releases/tag/v0.4.0).

### 2. Add a policy to the repository

Choose the policy that matches the language being scanned:

| Language | Starter policy |
| --- | --- |
| Rust | [`examples/quality-policy.json`](examples/quality-policy.json) |
| Python | [`examples/python-quality-policy.json`](examples/python-quality-policy.json) |
| TypeScript/TSX | [`examples/typescript-quality-policy.json`](examples/typescript-quality-policy.json) |

For example, add the immutable v0.4.0 Python starter to another repository:

```sh
curl -fsSLo quality-policy.json \
  https://raw.githubusercontent.com/mindful-time/smells/v0.4.0/examples/python-quality-policy.json
smells contracts validate --policy quality-policy.json
```

Commit the policy. Local developers, CI, hooks, and coding agents should all use that same reviewed contract.

### 3. Scan the repository

Use the table for people and keep the complete JSON report for tools:

```sh
smells check \
  --path . \
  --policy quality-policy.json \
  --format table \
  --report smells-report.json
```

This is one scan. The terminal shows actionable matches while `smells-report.json` retains matched and nonmatching measurements, coverage states, digests, and implementation ownership.

Use JSON on standard output when another process consumes the report directly:

```sh
smells check --path . --policy quality-policy.json --format json
```

## Understand the result

### Exit codes

| Exit | Meaning | Required action |
| ---: | --- | --- |
| `0` | Every selected active required rule completed and passed | Continue; review-only signals may still exist |
| `1` | One or more required rules matched | Review the blocking findings |
| `2` | The scan was invalid or incomplete | Fix the input, policy, parser, ownership, history, budget, or evidence error |

An exit code of `2` never means “clean.” Errors take precedence over violations, and violations take precedence over a successful result.

### Blocking and review findings

| Rule mode | Effect |
| --- | --- |
| `required` | A match is a blocking violation and produces exit `1` |
| `report` | A match is a review signal and does not change an otherwise successful exit |
| `off` | An explicit custom-policy opt-out; shipped starter policies do not use it |

Each matched finding contains:

- The canonical smell, rule ID, pattern type, and certainty.
- A primary symbol and location plus any related symbols and locations.
- The observed value, match condition, and configured threshold.
- A source excerpt when the rule maps to authored source.
- Why the signal matters and what the reviewer should inspect.
- A behavior-preserving remediation direction.
- The exact Refactoring.Guru URL.
- A non-negotiable instruction to research that page before review or remediation.

`checked_no_match_in_measured_scope` means the detector ran and stayed within its threshold. It does not claim the code is free of every possible design problem.

`incomplete` means a prerequisite was unavailable. For example, selected history rules are incomplete outside a Git repository and force exit `2` instead of returning a false clean result.

See the [report interface](docs/report-interface.md) before building a custom parser or integration.

## Use Smells with coding agents

Smells needs no MCP server, plugin, model provider, or agent-specific API. The agent runs the CLI, reads the report, researches each finding's exact reference URL, inspects the code, and proposes a behavior-preserving change.

### Add a shared agent contract

Put this section in the consuming repository's root `AGENTS.md`:

```md
## Smell review

- Run `smells check --path . --policy quality-policy.json --format table --report smells-report.json` before reviewing or refactoring code smells.
- Treat exit code 2 as an incomplete scan, never as a clean result.
- Review blocking findings before review-only signals.
- Treat each finding as evidence to investigate, not proof that a defect exists.
- Before judging or changing a matched finding, open and read its exact `diagnostic.reference_url`.
- NON-NEGOTIABLE RESEARCH: If the URL cannot be consulted, report the research as incomplete and stop review or remediation for that finding.
- Verify the finding against the surrounding design, tests, and repository contracts.
- Prefer small behavior-preserving changes. Run the project's tests and rerun Smells after editing.
```

Keep the policy and these instructions under version control. The scanner supplies evidence; the agent supplies contextual judgement.

### Codex

[Codex reads `AGENTS.md`](https://developers.openai.com/codex/guides/agents-md/) from the repository root toward its working directory. Put the shared contract at the root. Add narrower overrides only where needed.

Ask Codex:

```text
Run the repository's Smells scan. Explain the exit code and prioritize blocking findings.
For every finding you review, follow its reference_check and read the exact
Refactoring.Guru URL first. Do not edit code until the evidence is verified against
the repository. Then propose the smallest behavior-preserving change and test it.
```

### Claude Code

Current Claude Code versions can read `AGENTS.md`. If the project already has a `CLAUDE.md`, add this import so one contract remains authoritative:

```md
@AGENTS.md
```

Claude Code documents this pattern in [Share one file with other coding tools](https://code.claude.com/docs/en/memory#share-one-file-with-other-coding-tools).

Ask Claude Code the same review prompt. The contract and JSON schema do not change by model or provider.

### Pi

[Pi loads `AGENTS.md` or `CLAUDE.md`](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/quickstart.md#give-pi-project-instructions) from the project path. Restart Pi or run `/reload` after changing the instructions.

Start Pi in the repository, then ask it to run the shared Smells workflow:

```text
Use the repository Smell review instructions. Run the scan, research each matched
finding's exact reference URL, and return a prioritized review before changing code.
```

### Other coding agents

Place the shared contract in the tool's project-instruction file or include it in the task prompt. The only required capabilities are running a local command, reading JSON and source, and opening the emitted HTTPS reference URL.

Do not ask an agent to “fix every smell.” Ask it to verify findings, explain tradeoffs, preserve behavior, test each change, and rerun the deterministic scan.

## Integrate Smells into a repository

### Keep one source of truth

A typical single-language integration contains:

```text
quality-policy.json   reviewed rules, modes, thresholds, exclusions, and budgets
AGENTS.md             shared human/agent review contract
CLAUDE.md             optional `@AGENTS.md` import
smells-report.json    generated report; usually ignored by Git
```

Pin the Smells version in the installation command. Upgrade the executable and policy contract deliberately, then review the resulting report changes.

### Run it in CI

Add the same command to the repository's existing quality job:

```sh
uvx --from smells==0.4.0 smells check \
  --path . \
  --policy quality-policy.json \
  --format table \
  --report smells-report.json
```

For GitHub Actions, check out full history with `fetch-depth: 0`. Divergent Change and Shotgun Surgery use up to 200 commits of bounded Git co-change history.

Preserve `smells-report.json` as a CI artifact when agents or reviewers need the complete evidence. Let exit `1` block configured violations and exit `2` block incomplete analysis.

### Run it before commits

`--staged` scans the complete tracked source snapshot in the Git index. It reads source, runtime manifests, policy, and optional evidence from staged blobs rather than unstaged replacements.

With the Python `pre-commit` framework:

```yaml
repos:
  - repo: local
    hooks:
      - id: smells
        name: deterministic smell scan
        language: system
        entry: uvx --from smells==0.4.0 smells check --staged --policy quality-policy.json --format json
        pass_filenames: false
```

Node projects can replace the entry with `npx --no-install smells check ...`. Rust development images can install Smells once and use `smells check ...` directly.

The repository also includes an inactive [shell hook example](hooks/pre-commit.example). Merge it into an existing hook instead of overwriting other quality checks.

### Scan a monorepo

Run from the monorepo root. Each source file is assigned to its nearest `Cargo.toml`, `pyproject.toml`, or `package.json`, and each implementation receives a separate report section.

Nested manifests override parent ownership. Source without a matching manifest stays visible under `__unowned__`.

A mixed-language repository needs one checked-in policy and one invocation per language pack:

```sh
smells check --path . --policy rust-quality-policy.json --format json
smells check --path . --policy python-quality-policy.json --format json
smells check --path . --policy typescript-quality-policy.json --format json
```

Keep the reports separate so their language-pack identities and thresholds remain explicit.

## How the default scan works

Every starter policy selects `all`. A normal scan runs all 28 rules and returns one explicit result for each of the 23 canonical smells.

Built-in collectors cover:

- Function size, arguments, class/type size, fields, and method lines.
- Data clumps, duplicate callable bodies, comment density, data classes, and tiny classes.
- Primitive-heavy state, repeated dispatch, temporary fields, forwarding types, and alternative interfaces.
- Inheritance rejection, interface conformance, private-member access, message chains, and extension workarounds.
- Conservative zero-coverage CRAP bounds, unused private declarations, and unused generic parameters.
- Up to 200 Git commits of co-change history for Divergent Change and Shotgun Surgery.

These are bounded structural indicators, not proof of a design defect. The report names each collector, certainty, scope, and limitation so a reviewer can judge the evidence honestly.

## Configure what runs

The checked-in policy owns the language pack, rule modes, thresholds, exclusions, default groups, and analysis budgets.

Inspect the resolved selection before scanning:

```sh
smells policy show --policy quality-policy.json --format table
smells rules --rule-pack python-v1
```

Available groups are `all`, `source`, `evidence`, and the five canonical smell categories. Starter policies select `all`; users must make any narrower selection intentionally.

| Selector | Effect |
| --- | --- |
| `--group NAME` | Add a group to configured defaults |
| `--only-group NAME` | Replace defaults with only the named groups |
| `--all-groups` | Select every rule |
| `--no-default-groups` | Start without configured defaults |
| `--no-group NAME` | Remove a group after inclusions; exclusions win |

Unknown, duplicate, or conflicting selectors return exit `2`. The resolved selection and selected/excluded rules appear in every report.

### Optional higher-fidelity evidence

`--evidence` is not required for a complete default scan. It is an optional replacement for individual built-in observations when a team already has stronger compiler, type, coverage, test, or architecture facts.

```sh
smells check \
  --path . \
  --policy quality-policy.json \
  --evidence provider-evidence.json \
  --format json
```

Rules omitted from the evidence bundle continue using built-in collectors. Supplied evidence must match the exact input digest and fails closed when stale, malformed, duplicated, or incomplete.

Start with the [provider evidence guide](docs/provider-evidence.md) and [JSON Schema](schemas/provider-evidence.schema.json).

## Safety and limitations

- Smells captures source and fixed runtime manifests without following symlinks.
- It never executes repository code, dependencies, build scripts, tests, compilers, or package managers.
- Deterministic analysis budgets prevent unbounded work; exhaustion returns exit `2`.
- Missing Git history, unresolved ownership, parser failure, and invalid evidence are incomplete—not clean.
- Rust scans authored branches, including inactive `cfg` source; Python and TypeScript scan authored source text.
- Built-in type, coverage, and architecture indicators do not claim compiler-level or runtime certainty.
- The scanner reports evidence but never edits or automatically refactors source.

Run the project's compiler, type checker, tests, fresh coverage, and security tooling alongside Smells.

## Installation and release provenance

| Channel | Package | Installation |
| --- | --- | --- |
| PyPI | [`smells`](https://pypi.org/project/smells/0.4.0/) | `uv tool install smells==0.4.0` |
| npm | [`@mindful-time/smells`](https://www.npmjs.com/package/@mindful-time/smells/v/0.4.0) | `npm install --save-dev --save-exact @mindful-time/smells@0.4.0` |
| crates.io | [`smells`](https://crates.io/crates/smells/0.4.0) | `cargo install --locked --version 0.4.0 smells` |
| GitHub | [v0.4.0 release](https://github.com/mindful-time/smells/releases/tag/v0.4.0) | Download the archive for the host platform |

All registry payloads originate from one immutable GitHub Release. Protected workflows verify its commit, attestations, checksums, and exact assets before publishing through short-lived identities.

See [package distribution](docs/package-distribution.md) and the [release process](docs/release-process.md) for platform packages, provenance verification, SBOMs, and recovery guarantees.

## Reference documentation

- [Project integration](docs/project-integration.md)
- [Report interface](docs/report-interface.md)
- [Provider evidence](docs/provider-evidence.md)
- [Rust rule contracts](docs/rust-rule-contracts.md)
- [Python rule contracts](docs/python-rule-contracts.md)
- [TypeScript rule contracts](docs/typescript-rule-contracts.md)
- [Performance and algorithm research](docs/performance-algorithm-research.md)
- [Repository governance](docs/repository-governance.md)

## Developing Smells

Run the deterministic local gate:

```sh
make pre-commit-push
```

It runs formatting, build, Clippy, 131 offline tests, the scanner against itself, CRAP analysis, Gitleaks, and OSV. Pull requests run the same fail-closed gate and must originate from a fork.

The optional live test exercises the complete finding-to-Refactoring.Guru research path:

```sh
cargo test --locked --test e2e_live -- --ignored --nocapture
```

## Project status

Version `0.4.0` is available from GitHub Releases, PyPI, npm, GitHub Packages, and crates.io. Smells is open-source software licensed under the [MIT License](LICENSE).

Repository: [mindful-time/smells](https://github.com/mindful-time/smells)
