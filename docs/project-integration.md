# Planned project integration

This is the interface contract for implementation. No `smells` executable or hook installer exists yet.

## Project policy

Each project checks in `quality-policy.json`, based on the generic example when useful. It owns its limits, required rules, exact exceptions, target/features, and any optional architecture/domain/history inputs. `tiny` keeps its own dependency map; another project supplies its own map. The scanner must not infer one from a repository's name or absolute checkout location.

The example policy records shared initial metrics. Required compiler/coverage measurements must complete or fail with an explicit error. Optional boundary or contract checks need declared project rules before activation. Missing optional configuration receives an explicit coverage status, not a fabricated passing measurement.

## CLI

```text
smells check --staged --policy quality-policy.json [--format table|json]
```

- The current Git repository is the scan target. `--staged` analyzes an isolated staged snapshot, including the policy and source; it never substitutes an unstaged policy file.
- The policy path is relative to that repository's root. Paths escaping the snapshot are rejected; staged symlinks must not let the scanner read unstaged or external source as staged input. The tool does not assume a sibling repository layout.
- Terminal output is the default; JSON uses the same measurements and statuses. Findings sort by relative path, line, qualified symbol, and rule ID. Missing values are null with an explicit reason. Reports include all 23 catalogue coverage statuses and exclude timestamps from verdict data.
- Exit 0 means all configured required checks completed and passed. Exit 1 means at least one blocking violation. Exit 2 means an input, configuration, tool, or required-measurement error. Errors take precedence when both errors and violations occur. These exact exits require fixtures before release.
- The installed scanner version and evidence/tool versions are reported. Consuming projects must pin a tested release or revision before relying on it in a hook; no automatic tool upgrade changes a commit verdict silently.

## Hook composition

The inactive `hooks/pre-commit.example` demonstrates one future invocation. It checks that the executable exists and propagates its exit code. Do not install it before the executable and staged-snapshot checks work. Do not overwrite an existing hook; add the scanner invocation to the project's existing quality runner instead.

A project's broader quality runner continues to own OSV, Gitleaks, formatting, Clippy, application tests, coverage generation, and any other required checks. Shared scanner code lives here; application-specific policy and quality-runner composition live with the application.
