# smells

Deterministic code-smell evidence for Rust, Python, and TypeScript.

`smells` scans a codebase and answers five practical questions:

1. What smell pattern was detected?
2. Where is it in the repository?
3. What measurement crossed which threshold?
4. Why might it matter, and what should be reviewed?
5. Which Refactoring.Guru page must the review agent research before judging the finding?

The scanner—not an LLM—decides whether a configured rule matches. The same
source, policy, evidence, and executable produce the same ordered JSON report.
This makes `smells` suitable for local review, CI, pre-commit hooks, and agentic
coding environments.

[Quick start](#quick-start) · [Understand results](#understand-the-result) ·
[Pre-commit hook](#add-it-to-a-pre-commit-hook) ·
[Monorepos](#monorepo-behavior) ·
[Choose rules](#choose-which-policy-groups-run) ·
[Full-pattern evidence](#source-rules-and-full-pattern-evidence) ·
[Rule reference](#policies-and-rule-reference) ·
[Install](#install-smells)

## At a glance

- Scans Rust, Python, TypeScript, and TSX source without executing application code.
- Reports repository-relative files, lines, symbols, measurements, and thresholds.
- Separates blocking rules from review-only signals.
- Understands monorepos and groups results by the nearest runtime manifest.
- Emits human-readable tables or complete JSON for hooks and coding agents.
- The release pipeline produces versioned GitHub binaries, PyPI wheels,
  native npm packages on npmjs.com and GitHub Packages, and a crates.io package
  from one release build.
- Fails closed when parsing or analysis budgets are incomplete.
- Covers all 23 Refactoring.Guru smell categories through 28 rules per language pack.

> **Important:** every rule in every starter policy is active. The default group is
> `all`, and `smells check` runs all 28 rules without requiring a separate evidence
> pipeline. Built-in collectors provide conservative source, type-shape, contract,
> coverage-risk, and Git-history indicators. `--evidence` remains available as an
> optional higher-fidelity replacement for individual built-in rule observations.

## Supported languages

| Language pack | Files scanned | Built-in rules | Optional evidence overrides | What counts as a class-like type |
| --- | --- | ---: | ---: | --- |
| `rust-v1` | `.rs` | 28 | 11 | `struct`/`enum` state plus inherent and trait `impl` methods; traits are interface-like contracts |
| `python-v1` | `.py`, `.pyi` | 28 | 18 | `class` state, direct methods, and `self`/`cls` field assignments |
| `typescript-v1` | `.ts`, `.tsx`, `.mts`, `.cts` | 28 | 18 | Classes, abstract classes, interfaces, method signatures, and authored fields |

Java and Kotlin are not currently supported. A React Native repository can scan
its TypeScript/TSX source, but not its native Android code.

## Quick start

### 1. Install Smells

The fastest cross-language option is the prebuilt
[PyPI tool package](https://pypi.org/project/smells/0.4.0/):

```sh
uv tool install smells==0.4.0
smells --version
```

Node projects can pin the
[native npm package](https://www.npmjs.com/package/@mindful-time/smells). It does
not download anything during installation:

```sh
npm install --save-dev --save-exact @mindful-time/smells@0.4.0
npx --no-install smells --version
```

After the one-time crates.io ownership bootstrap is complete, Rust users can build
the same executable from its verified source package:

```sh
cargo install --locked --version 0.4.0 smells
```

Until then, use a standalone release, PyPI, or npm. Unlike those prebuilt channels,
`cargo install` requires Rust 1.88 and compiles locally. See
[package distribution](docs/package-distribution.md) for package names, platform
selection, registry provenance, and owner bootstrap requirements.

### Install a standalone release

Download the archive for your host from the matching
[GitHub Release](https://github.com/mindful-time/smells/releases). Verify both its
checksum and GitHub provenance before installing it. For macOS Apple Silicon:

```sh
gh release download v0.4.0 \
  --repo mindful-time/smells \
  --pattern 'smells-aarch64-apple-darwin.tar.gz*'
shasum -a 256 -c smells-aarch64-apple-darwin.tar.gz.sha256
gh release verify v0.4.0 --repo mindful-time/smells
gh release verify-asset v0.4.0 \
  smells-aarch64-apple-darwin.tar.gz \
  --repo mindful-time/smells
tar -xzf smells-aarch64-apple-darwin.tar.gz
install -m 0755 smells-aarch64-apple-darwin/smells ~/.local/bin/smells
smells --version
```

The same release contains macOS Intel, Linux AMD64/ARM64 GNU, and Windows AMD64
archives. It also contains the exact wheel, npm, and crates.io package payloads
published by the release workflow. After downloading a wheel directly:

```sh
uv tool install ./smells-0.4.0-py3-none-PLATFORM.whl
```

The wheel is a delivery mechanism; the scanner does not become a Python program.
There is deliberately no source distribution, so an unsupported platform fails
instead of compiling Rust unexpectedly. See the [release process](docs/release-process.md)
for the artifact contract and supported targets.

Contributors can still build from source with Rust 1.88:

```sh
git clone https://github.com/mindful-time/smells.git
cd smells
cargo install --path . --locked
```

### 2. Choose a starter policy

| Your codebase | Starter policy |
| --- | --- |
| Rust | `examples/quality-policy.json` |
| Python | `examples/python-quality-policy.json` |
| TypeScript/TSX | `examples/typescript-quality-policy.json` |

A policy selects the language pack, rule modes, thresholds, excluded directory
names, default rule groups, and analysis budgets. There is no separate language
flag. Every starter has `"default_groups": ["all"]` and 28 active rules.

Copy the matching starter policy into the repository you want to scan. For a
Python project:

```sh
cp examples/python-quality-policy.json /path/to/python-project/quality-policy.json
cd /path/to/python-project
smells contracts validate --policy quality-policy.json
```

Use the Rust or TypeScript starter from the table in the same way. Review its
thresholds, then commit `quality-policy.json` with the project so local, CI, and
hook scans all use the same contract.

### 3. Scan a codebase

Use the table format for an interactive first run:

```sh
smells check \
  --path . \
  --policy quality-policy.json \
  --format table
```

Replace the path and policy for Rust or TypeScript. Scan from a monorepo root if
you want findings grouped by each nested implementation.

Use JSON for CI, hooks, or coding agents:

```sh
smells check \
  --path . \
  --policy quality-policy.json \
  --format json > smells-report.json
```

JSON reports can be large because they retain both matched and nonmatching
measurements so the verdict can be audited.

To keep that complete JSON report while also showing actionable output in the
terminal or CI log, use `--report` with the table format:

```sh
smells check \
  --path . \
  --policy quality-policy.json \
  --format table \
  --report target/quality/smells-report.json
```

This performs one scan. The log includes the verdict, implementation, smell
pattern, evidence locations, observed value, threshold, source lines, reason,
remediation, Refactoring.Guru URL, and review guidance for
every match. `--report` writes the complete machine-readable JSON from that
same scan.

## Understand the result

### Exit codes

| Exit code | Meaning | What to do |
| ---: | --- | --- |
| `0` | Every selected active required rule completed and passed | Continue |
| `1` | One or more required rules matched | Review the blocking findings |
| `2` | The scan was incomplete or invalid | Fix the input, policy, parser, ownership, budget, or evidence error |

Review-only findings do not change a successful exit code. Errors always take
precedence over matches.

### Read the table from top to bottom

An abbreviated table report looks like this:

```text
Scan summary | verdict: blocked_by_required_patterns | implementations: 1 | matched patterns: 2/23 | blocking patterns: 1 | review patterns: 1 | matched findings: 11 | blocking findings: 3 | review signals: 8 | errors: 0
Implementation: backend | root: backend | ... | 120 python files
Smell pattern  | Pattern ID     | Result         | Matches | Blocking | Review | Coverage
Long Method    | long-method    | blocking_match | 3       | 3        | 0      | measured_defined_scope
Duplicate Code | duplicate-code | review_match   | 8       | 0        | 8      | measured_defined_scope
Dead Code      | dead-code      | excluded       | 0       | 0        | 0      | excluded

Matched evidence for backend:
Finding | Smell       | Repository symbols          | Metric                   | Value | Matches when | Threshold | Status    | Repository evidence locations
42      | Long Method | app/jobs.py::process_batch | authored body code lines | 146   | >            | 100       | violation | app/jobs.py:42:1

Actionable finding 42 | Long Method | python.function_lines | pattern_type=metric | certainty=exact_source_metric | status=violation
  Issue: Long Method: python.function_lines violation
  Primary location: app/jobs.py:42:1 | symbol: app/jobs.py::process_batch
  Observed versus threshold: authored body code lines = 146; matches when > 100
  Evidence: Observed authored body code lines = 146; this satisfies the configured match condition > 100.
  Signal: A callable's authored body contains more code lines than the configured maximum.
  Source excerpt: app/jobs.py:42:1
    >     42 | def process_batch(...):
  Why it matters: A large callable can conceal multiple responsibilities and make focused testing harder.
  Remediation: Extract coherent steps into named callables or a focused class/module while preserving behavior with tests.
  Reference URL: https://refactoring.guru/smells/long-method
  Review guidance: NON-NEGOTIABLE RESEARCH: Perform an external research/tool call to the reference URL and read the page before reviewing or remediating ...
```

Read it in this order:

1. **Implementation** tells you which application, package, or service owns the result.
2. **Blocking** rows caused exit code `1` and should be handled first.
3. **Review** rows are deterministic signals that still require design judgement.
4. **Matched evidence** shows the exact symbol, measurement, threshold, and location.
5. **Actionable findings** are self-contained for every match: they add pattern
   type, source lines, why the signal matters, remediation guidance, its exact
   reference URL, and focused review guidance.
6. **Excluded** means the resolved group selection did not run that detector; it
   does not mean no smell exists. **Disabled** is reserved for an explicit legacy
   `off` mode.

`checked_no_match_in_measured_scope` means the selected active detector ran and stayed
within its configured threshold. `not_applicable` means the smell does not map
to that language's model. `error` with `coverage_status: incomplete` means a
collector prerequisite such as Git history was unavailable; it is never a clean
result.

### Rule modes

| Mode | Behavior |
| --- | --- |
| `required` | A match blocks with exit code `1` |
| `report` | A match is recorded for human or agent review but does not block |
| `off` | Legacy/custom-policy opt-out; the shipped policies do not use it |

### What a finding contains

Each matched JSON finding is self-contained. It includes:

- Smell name, category, pattern type, and rule identity.
- File, one-based line and column, symbol, and related locations.
- The observed measurement, comparison, and configured threshold.
- A source excerpt when applicable.
- Why the signal matters and what the reviewer should inspect.
- A behavior-preserving remediation direction.
- The exact Refactoring.Guru URL.
- A mandatory Refactoring.Guru research contract for agent grounding.

A shortened finding looks like this:

```json
{
  "smell_id": "long-method",
  "smell": "Long Method",
  "pattern_type": "metric",
  "symbol": "process_batch",
  "location": {
    "path": "src/jobs.py",
    "line": 42,
    "column": 1
  },
  "evaluation": {
    "metric": "authored body code lines",
    "observed": 146,
    "match_condition": ">",
    "threshold": 100,
    "matched": true
  },
  "diagnostic": {
    "why_it_matters": "A large callable can conceal multiple responsibilities and make focused testing harder.",
    "remediation": "Extract coherent steps into named callables or a focused class/module while preserving behavior with tests.",
    "reference_url": "https://refactoring.guru/smells/long-method",
    "reference_check": {
      "required": true,
      "non_negotiable": true,
      "action": "perform_external_research_call_to_reference_url",
      "required_before": "review_or_remediation",
      "unavailable_action": "report_reference_research_incomplete_and_do_not_review_or_remediate"
    }
  },
  "status": "violation",
  "blocking": true
}
```

See the complete [report interface](docs/report-interface.md) before building a
custom integration.

## Scan a working tree or the Git index snapshot

### Whole repository

```sh
smells check --path . --policy quality-policy.json --format table
```

`--path` recursively captures only the selected language's supported extensions.
Configured cache/build directories are excluded by name. Symlinks are never
followed, so dependency caches cannot silently redirect a scan outside its root.

### Git index snapshot for pre-commit

Run this from the consuming Git repository:

```sh
smells check --staged --policy quality-policy.json --format json
```

Despite the name, `--staged` does not scan only paths changed by the next commit.
It scans the complete tracked source snapshot in the Git index for the selected
language. It reads source, runtime manifests, policy, and optional evidence from
index blobs—not from unstaged working-tree replacements. The policy itself must
be staged.

For a mixed Rust/Python/TypeScript repository, keep one policy per language and
invoke the scanner once for each policy.

## Add it to a pre-commit hook

The repository includes an inactive [consumer hook example](hooks/pre-commit.example).
Pin and validate the scanner version before adding it to another project, and
merge the command into any existing hook rather than overwriting that hook.

For the Python `pre-commit` framework, `uvx` provides a pinned, cached executable
without checking scanner source into the consuming repository:

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

A Node project that already pins `@mindful-time/smells` can use the same hook with
`entry: npx --no-install smells check ...`. A Rust-oriented development image can
install `smells` once with Cargo and keep the plain `smells check ...` entry. In all
three cases the policy stays checked into the consuming repository and the hook scans
the same Git-index snapshot.

A minimal single-language hook is:

```sh
#!/bin/sh
set -eu

command -v smells >/dev/null 2>&1 || {
  printf '%s\n' 'smells: scanner executable is unavailable' >&2
  exit 2
}

exec smells check --staged --policy quality-policy.json --format json
```

Keep formatting, compilation, tests, Gitleaks, OSV, and other project checks in
the same quality pipeline. `smells` complements them; it does not replace them.
See [project integration](docs/project-integration.md) for mixed-language hooks,
scope guarantees, and fail-closed behavior.

## Monorepo behavior

When scanning from a monorepo root, each source file is assigned to its nearest
ancestor manifest:

- `Cargo.toml` for Rust
- `pyproject.toml` for Python
- `package.json` for TypeScript

The report gets one `implementation_results` section per discovered runtime.
Nested manifests take ownership from parent manifests. Source without a matching
manifest remains visible under `__unowned__` instead of disappearing.

This makes it possible to answer “which application or service owns this smell?”
without scanning every directory separately.

## Source rules and full-pattern evidence

The starter policies activate all 28 rules and select `all` by default. A normal
scan therefore evaluates every rule and returns one explicit result for each of
the 23 canonical smells:

```sh
smells check --path . --policy quality-policy.json --format table
```

Built-in collectors cover deterministic measurements such as:

- Function size and argument count.
- Class/type fields, methods, and total method lines.
- Repeated parameter groups and duplicate callable bodies.
- Comment density, data-only classes, and tiny classes.
- Primitive-heavy slots, repeated dispatch, temporary fields, forwarding classes,
  alternative interfaces, inheritance rejection, and interface conformance.
- Conservative zero-coverage CRAP upper bounds, unused private declarations, and
  unused generic parameters.
- Private-member access, navigation chains, library-extension workarounds, and
  domain-significant primitive field names.
- Up to 200 Git commits of co-change history for divergent change and shotgun
  surgery when the scanned path is inside a repository.

Reports expose `history_scope.available`, `history_scope.commit_count`, and the
collector's maximum commit bound. Outside a Git repository, every selected
history rule is `incomplete` and the scan exits `2`; unavailable history is never
reported as a clean measurement, regardless of rule mode.

These are deliberately bounded structural indicators, not proof of a design
defect. For example, the built-in CRAP collector assumes zero coverage and reports
a conservative upper bound; unused-code detection is restricted to unreferenced
private declarations; architectural checks use authored interfaces, visibility,
and naming conventions. The finding's `evidence.provider_evidence` object names
the exact built-in algorithm and scope.

Teams that already have compiler diagnostics, complete coverage, resolved type
graphs, capability tests, or a project ownership ledger can pass those facts as
an optional evidence bundle tied to the scan's `input_sha256`:

```sh
smells check \
  --path /path/to/project \
  --policy quality-policy.json \
  --evidence provider-evidence.json \
  --format json
```

An external entry replaces the built-in observations for that rule; omitted rules
continue to use their built-in collectors. Supplied entries still fail closed when
stale, incomplete, malformed, duplicated, or inconsistent with the selected rule
pack. Start with the [provider evidence guide](docs/provider-evidence.md) and its
[JSON Schema](schemas/provider-evidence.schema.json).

## Policies and rule reference

### Choose which policy groups run

The selection model follows uv's dependency-group behavior. Persistent defaults
come from `default_groups` in the checked-in policy; the starter policies use
`["all"]`. Inspect the exact resolved policy before scanning:

```sh
smells policy show --policy quality-policy.json --format table
smells policy show --policy quality-policy.json --only-group source --format json
```

Built-in groups are `all`, `source`, `evidence`, and the five canonical smell
categories: `bloaters`, `object-orientation-abusers`, `change-preventers`,
`dispensables`, and `couplers`. The `evidence` name is retained for policy
compatibility; in 0.4.0 those rules have built-in collectors and merely *allow*
external evidence overrides.

| Selector | Resolution behavior |
| --- | --- |
| `--group NAME` | Add a group to the configured defaults; repeatable |
| `--only-group NAME` | Replace defaults with only these groups; repeatable |
| `--all-groups` | Include every rule |
| `--no-default-groups` | Start without configured defaults |
| `--no-group NAME` | Remove a group after every inclusion; exclusions always win |

`--only-group` conflicts with `--group`, `--all-groups`, and
`--no-default-groups`. Unknown or duplicate group selectors return exit `2`.
The resolved selection, group membership, selected/excluded rule IDs, and rule
mode appear in `policy show` and every scan report. The resolved selection is
also part of `input_sha256`, so evidence for one selection cannot be replayed
against another.

List the registered rules for one language pack:

```sh
smells rules --rule-pack rust-v1
smells rules --rule-pack python-v1
smells rules --rule-pack typescript-v1
```

Exact thresholds and measurement contracts are documented here:

- [Rust rule contracts](docs/rust-rule-contracts.md)
- [Python rule contracts](docs/python-rule-contracts.md)
- [TypeScript rule contracts](docs/typescript-rule-contracts.md)
- [Report interface](docs/report-interface.md)
- [Provider evidence](docs/provider-evidence.md)
- [Project integration](docs/project-integration.md)

Analysis limits such as `maximum_files`, `maximum_pairs`, and
`maximum_group_combinations` are deterministic safety ceilings. Exceeding a
limit returns exit code `2`; it never produces a silently truncated success.

## What `smells` does not do

- It does not automatically fix or refactor source code.
- It does not execute application code, builds, compilers, tests, or build scripts.
- It does not claim that its source-level type, coverage, or architecture
  approximations are equivalent to a compiler, coverage runner, or project contract.
- It does not treat a structural indicator as proof of a design defect.
- It does not guess active Rust features, Python environments, or TypeScript configs.
- It does not authenticate optional external evidence producers; the surrounding quality
  pipeline must invoke and verify the intended pinned provider.
- It does not currently scan Java, Kotlin, JavaScript, JSX, or other languages.

The tool supplies deterministic evidence. A human or coding agent still decides
whether the detected structure is intentional and which behavior-preserving
change, if any, is appropriate.

## Performance and benchmarking

Python and TypeScript parsing uses bounded Rayon workers. Duplicate detection
uses exact prefix, size, and positional filters before charging the pair budget,
avoiding comparisons that cannot reach the configured similarity threshold.

Measure your own repository instead of assuming one machine or corpus represents
another:

```sh
./scripts/benchmark-scan.sh . quality-policy.json 5 cold default
./scripts/benchmark-scan.sh . quality-policy.json 5 warm default
```

The benchmark discards the report but records wall time, cache hits/misses,
syntax work, candidate comparisons, output bytes, and peak memory. Always compare
cold and warm results; cache hits are deterministic but are not guaranteed to be
faster for every corpus. See [performance and algorithm research](docs/performance-algorithm-research.md)
for the algorithm and benchmark design.

## Developing `smells`

Run the deterministic test suite:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Install this repository's own pre-commit and pre-push hooks:

```sh
make install-hooks
make pre-commit-push
```

The shared gate runs formatting, build, Clippy, tests, the scanner against
itself, CRAP analysis, Gitleaks, and OSV. CRAP has a target of `5` and a hard
blocking limit of `10`; scores above the target remain visible to coding agents.

Pull requests run that same gate in GitHub Actions and additionally build, install,
and execute a binary wheel and standalone archive. Pull requests must originate
from a fork. Only repository owner `mindful-time` can dispatch release publication
from `main`; read
[repository governance](docs/repository-governance.md) and the
[release process](docs/release-process.md) before changing either workflow.

The optional live E2E test verifies the hook-to-finding-to-Refactoring.Guru
research path and therefore requires network access:

```sh
cargo test --locked --test e2e_live -- --ignored --nocapture
```

Ordinary tests remain offline and deterministic.

## Project status

`smells` source is currently version `0.4.0`. Standalone GitHub archives, binary
PyPI wheels, and native npm packages on npmjs.com are published. The owner-gated
workflow publishes the same signed npm artifacts to GitHub Packages and uses an
idempotent recovery workflow when a registry fails after release creation.
crates.io still requires its documented one-time ownership bootstrap before future
releases can use short-lived OIDC credentials. No Python sdist is produced. Smells
is open-source software licensed under the [MIT License](LICENSE).
Repository:
[mindful-time/smells](https://github.com/mindful-time/smells).
