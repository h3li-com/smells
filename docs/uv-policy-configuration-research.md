# uv configuration patterns for selectable smell policies

Research date: 2026-09-20
uv source revision inspected: [`7b090fba99bc89a6670a23de5368d48d2418a756`](https://github.com/astral-sh/uv/tree/7b090fba99bc89a6670a23de5368d48d2418a756)

## Conclusion

The useful uv pattern is not a collection of preset policy files. It is a small,
predictable configuration system:

1. discover one project configuration;
2. merge it over user and system defaults;
3. let environment variables override persistent configuration;
4. let command-line arguments override everything else;
5. start with declared default groups, then apply explicit inclusions and exclusions;
6. make exclusions win.

`smells` should adopt that model while keeping its stronger audit requirements. The
recommended default is `default-groups = "all"`, so all 28 rules in the selected
language pack are active. Rules may still be either `required` or `report`; “active”
must never be conflated with “blocking.” A provider-backed rule without the evidence
needed to evaluate it should make an all-active scan incomplete (exit 2), not silently
become disabled or clean.

Users should be able to narrow the resolved rules with uv-like `--group`,
`--only-group`, `--all-groups`, `--no-group`, and `--no-default-groups` selectors.
The report must record those choices and the source of every resolved setting. This
last part is an intentional improvement over uv's public interface: uv has a hidden
debug flag that prints resolved settings, but it does not document a first-class
configuration explanation command.

## What uv actually does

Everything in this section is a sourced uv fact. The recommendations for `smells` are
separated below.

### Project, user, and system configuration

uv searches the current directory and its parents for the nearest `uv.toml` or
`pyproject.toml`. A `pyproject.toml` without `[tool.uv]` does not stop the search. If
both formats exist in one directory, `uv.toml` wins and the `[tool.uv]` table is
ignored. In a workspace, uv starts at the workspace root and ignores member-level uv
configuration because the workspace is locked as one unit
([configuration-file documentation](https://docs.astral.sh/uv/concepts/configuration-files/#configuration-files)).
The implementation walks ancestors and returns the first usable configuration, rather
than merging arbitrary project files from every parent directory
([`FilesystemOptions::find`](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-settings/src/lib.rs#L95-L127)).

uv also reads user configuration from locations such as
`~/.config/uv/uv.toml` and system configuration from locations such as
`/etc/uv/uv.toml`. User and system configuration must use `uv.toml`, not
`pyproject.toml`. The persistent precedence is project over user over system
([configuration-file documentation](https://docs.astral.sh/uv/concepts/configuration-files/#configuration-files)).
The command implementation performs exactly that ordered combination
([configuration loading](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv/src/lib.rs#L299-L347)).

For scalars, the higher-precedence value replaces the lower-precedence value. Arrays
are concatenated, with higher-precedence elements placed first
([configuration-file documentation](https://docs.astral.sh/uv/concepts/configuration-files/#configuration-files));
[`Combine` implementation](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-settings/src/combine.rs#L25-L38),
[`Vec` combination](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-settings/src/combine.rs#L127-L138)).

Commands that operate on user-level tools ignore local project configuration and use
only user/system configuration. This is a scope-specific choice, not a universal rule
for every uv command
([configuration-file documentation](https://docs.astral.sh/uv/concepts/configuration-files/#configuration-files)).

### Explicit selection and precedence

uv's complete documented precedence is:

```text
command line > environment > project config > user config > system config > defaults
```

The first two layers are explicitly documented as command line over environment over
persistent configuration
([configuration-file documentation](https://docs.astral.sh/uv/concepts/configuration-files/#configuration-files)).

`--config-file PATH` or `UV_CONFIG_FILE` selects one `uv.toml` and replaces all
discovered persistent files, including user configuration. `--no-config` or
`UV_NO_CONFIG` disables persistent discovery
([configuration-file documentation](https://docs.astral.sh/uv/concepts/configuration-files/#configuration-files),
[`UV_CONFIG_FILE`](https://docs.astral.sh/uv/reference/environment/#uv_config_file),
[`UV_NO_CONFIG`](https://docs.astral.sh/uv/reference/environment/#uv_no_config)).
The CLI declares these as global options and makes `UV_CONFIG_FILE` and
`UV_NO_CONFIG` exact environment counterparts
([top-level arguments](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-cli/src/lib.rs#L150-L176)).

`--project PATH` changes where project discovery begins, while unrelated relative
command-line paths remain relative to the working directory
([`--project` CLI reference](https://docs.astral.sh/uv/reference/cli/#uv-run--project)).

### Defaults plus opt-in and opt-out groups

uv dependency groups provide the closest model for selectable smell rules. The project
declares defaults with `default-groups`. uv defaults to `['dev']`, while the literal
`"all"` makes every declared group a default
([default-group documentation](https://docs.astral.sh/uv/concepts/projects/dependencies/#default-groups),
[`default-groups` settings reference](https://docs.astral.sh/uv/reference/settings/#default-groups)).

At invocation time:

- `--group NAME` adds one or more named groups;
- `--only-group NAME` selects only named groups and implies no default groups;
- `--all-groups` includes every group;
- `--no-default-groups` removes declared defaults but still permits explicit groups;
- `--no-group NAME` removes a group and always wins over defaults, `--all-groups`,
  and `--group`.

These semantics are documented together, including the rule that exclusions beat
inclusions
([syncing development dependencies](https://docs.astral.sh/uv/concepts/projects/sync/#syncing-development-dependencies)).
The Clap declarations enforce conflicts such as `--only-group` versus `--group` or
`--all-groups`, and state the final-precedence behavior of `--no-group`
([group argument definitions](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-cli/src/lib.rs#L6995-L7073)).

uv exposes environment opt-outs through `UV_NO_DEFAULT_GROUPS` and the space-delimited
`UV_NO_GROUP`
([`UV_NO_DEFAULT_GROUPS`](https://docs.astral.sh/uv/reference/environment/#uv_no_default_groups),
[`UV_NO_GROUP`](https://docs.astral.sh/uv/reference/environment/#uv_no_group)).
The official environment reference does not currently document corresponding positive
`UV_GROUP` or `UV_ALL_GROUPS` variables. That asymmetry is worth preserving if an
environment override is intended mainly as an emergency opt-out.

### Defaults, versions, and validation

uv's generated settings reference states each setting's type, default, and examples.
For example, `default-groups` is `str | list[str]` with default `['dev']`
([settings reference](https://docs.astral.sh/uv/reference/settings/#default-groups)).
This gives users an authoritative contract instead of forcing them to infer defaults
from examples.

`required-version` lets a project constrain the uv version and causes a runtime error
when the executable is incompatible
([`required-version` reference](https://docs.astral.sh/uv/reference/settings/#required-version)).
The implementation deliberately checks a version mismatch even when full settings
parsing fails, so an old executable can produce the more useful version error
([version check](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-settings/src/lib.rs#L240-L276)).

uv's implementation deserializes its flattened option wire format with
`deny_unknown_fields`, which rejects unrecognized keys and improves error messages
([`OptionsWire`](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-settings/src/settings.rs#L2554-L2573)).
It also rejects settings that are legal only in `pyproject.toml` when they appear in
`uv.toml`
([format-specific validation](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-settings/src/lib.rs#L292-L349)).
These are implementation facts; the public documentation does not promise a general
unknown-key validation command as a stable user-facing feature.

### How uv exposes resolved settings

Normal verbose logging can identify an explicitly selected configuration file
([configuration debug log](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv/src/lib.rs#L515-L518)).
The source also contains a hidden `--show-settings` option intended for debugging and
development
([CLI declaration](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-cli/src/lib.rs#L360-L364)).
It prints the resolved settings structures and exits successfully
([printing implementation](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv/src/lib.rs#L569-L584)).

The official command reference does not document `uv config show`, `uv config explain`,
or the hidden `--show-settings` flag
([official CLI reference](https://docs.astral.sh/uv/reference/cli/)). Therefore uv
has an internal resolved-value view, but not a public, provenance-rich explanation
interface that `smells` should copy verbatim.

## Recommendations for `smells`

Everything in this section is a design recommendation or inference from the sourced uv
behavior above. It is not a claim about uv.

### 1. Make every rule active by default

Separate **selection** from **severity**:

- selected + `required`: evaluated and blocking when matched;
- selected + `report`: evaluated and review-only when matched;
- unselected: deliberately excluded by the resolved group selection;
- selected but unevaluable: incomplete scan, exit 2.

The checked-in starter policies should have no rule whose base mode is `off`. Preserve
the current objective source thresholds as `required`, preserve judgement-heavy smells
as `report`, and give provider-backed rules an explicit `required` or `report` mode.
Then use:

```toml
default-groups = "all"
```

This makes the default promise simple: all 28 rules are selected. It also forces the
remaining provider workflow to become usable. Until provider evidence can be generated,
an all-active scan should say exactly which selected rule lacks which evidence and exit
2. Quietly treating missing evidence as an empty finding set would invalidate the
coverage claim.

### 2. Add uv-style group algebra without overloading `--policy`

Keep `--policy FILE` as the path to the deterministic policy contract. Add repeatable
selectors with uv's names and semantics:

```text
--all-groups
--group NAME
--only-group NAME
--no-default-groups
--no-group NAME
```

Resolve them in this exact order:

```text
selected = configured defaults
selected += --all-groups and every --group
selected = only-groups when any --only-group is present
selected -= every --no-group
```

As in uv, exclusions must win. Reject ambiguous combinations such as `--only-group`
with `--group` or `--all-groups` instead of inventing an order. Environment variables
should be limited to `SMELLS_NO_DEFAULT_GROUPS` and space-delimited
`SMELLS_NO_GROUP`; positive selection belongs in the checked-in file or visible command
line.

Provide stable, non-overlapping primary groups:

```text
source       rules evaluated directly from authored source
evidence     rules requiring pinned provider evidence
```

Optionally provide canonical Refactoring.Guru category aliases (`bloaters`,
`object-orientation-abusers`, `change-preventers`, `dispensables`, `couplers`) as
secondary groups. Selection is a set union, so a rule appearing through more than one
alias still executes once. Unknown group names and duplicate rule identifiers must be
errors.

Do not add a second “profile” system initially. `strict`, `recommended`, and
`source-only` profiles would overlap with rule modes and groups, creating multiple ways
to express the same state. A checked-in policy plus group algebra is sufficient.

### 3. Discover configuration, but keep verdict inputs reproducible

Use one cross-language project filename, `smells.toml`. Discover the nearest file by
walking upward from the **scan root**, not an unrelated process working directory. In
`--staged` mode, discovery must read the Git index snapshot and begin at the repository
root; it must never mix a staged source snapshot with an unstaged policy.

Recommended persistent layers:

```text
project: <nearest scan-root ancestor>/smells.toml
user:    $XDG_CONFIG_HOME/smells/smells.toml
system:  platform configuration directory
```

Adopt uv's broad precedence:

```text
CLI > environment > project > user > system > built-in defaults
```

But restrict user/system configuration to operational preferences such as output format,
cache directory, color, and concurrency. Rule selection, thresholds, evidence
requirements, and severity affect the verdict and should come only from the checked-in
project policy or an explicit command-line replacement. Otherwise two developers could
scan the same commit and get different pass/fail results from invisible home-directory
settings.

Treat the existing `--policy FILE` like uv's `--config-file`: it replaces project policy
discovery instead of merging an arbitrary extra file. Add `SMELLS_POLICY_FILE` as its
environment counterpart and `--no-config`/`SMELLS_NO_CONFIG` for hermetic runs. Always
record an explicit policy path and digest in the report.

Do not copy uv's array concatenation blindly for rule and group definitions. Merging two
lists containing the same rule can hide contradictions. Use named-set semantics and
reject duplicate or conflicting definitions.

### 4. Make the resolved policy explainable

Add a public command rather than relying on verbose logs:

```sh
smells policy show --resolved --path . --format table
smells policy show --resolved --path . --format json
```

For every rule it should show:

- rule ID, smell ID, language pack, and evaluator kind;
- selected or excluded state;
- `required` or `report` mode;
- fully resolved thresholds;
- groups that included it and any exclusion that removed it;
- configuration source for every value (`default`, `system`, `user`, `project`,
  environment, or CLI);
- required evidence provider and whether compatible evidence is present;
- scanner version and policy schema version.

The ordinary scan report should embed the same resolved-policy digest and selection
provenance. This lets an LLM, hook, or human answer “why did this rule run?” and “why did
this threshold win?” without reproducing resolution logic.

### 5. Keep validation fail-closed and make generation easy

Retain the current strict rejection of unknown fields, rule IDs, parameters, versions,
and nonempty unsupported exceptions. Extend it to reject:

- unknown or empty groups;
- references to rules from a different language pack;
- duplicate rule membership where the resolved mode or parameters conflict;
- incompatible selector combinations;
- an all-active selection with missing or stale provider evidence;
- user/system attempts to override verdict-affecting settings.

Pair strictness with generation and discovery commands:

```sh
smells policy init --rule-pack python-v1
smells policy list
smells policy show --rule-pack python-v1
smells policy validate
```

`init` should generate `default-groups = "all"` and all 28 explicit rule contracts. The
settings reference should list each option's type, default, legal values, evidence
provider, and CLI/environment equivalents, following uv's generated settings-reference
approach.

## Recommended acceptance criteria

1. A generated policy selects all 28 rules; no rule has mode `off`.
2. With complete pinned evidence, the resolved report shows 28 selected and evaluated
   rules.
3. Without required evidence, the default all-active scan exits 2 and names every
   unevaluable rule.
4. `--only-group source` evaluates exactly the source-backed rules.
5. `--all-groups --no-group evidence` excludes evidence rules because exclusion wins.
6. CLI selection overrides environment selection, which overrides the checked-in
   defaults, and the report records that provenance.
7. User/system preferences cannot change rule modes, thresholds, selected rules, input
   scope, evidence requirements, or the verdict.
8. `--policy FILE` suppresses discovered project/user/system policy content.
9. Unknown fields, groups, rules, parameters, and conflicting selectors exit 2.
10. Repeated scans over identical source, evidence, resolved configuration, and scanner
    version emit byte-identical JSON.

## Primary sources

- [uv configuration files](https://docs.astral.sh/uv/concepts/configuration-files/)
- [uv dependency groups and defaults](https://docs.astral.sh/uv/concepts/projects/dependencies/#dependency-groups)
- [uv sync group selection](https://docs.astral.sh/uv/concepts/projects/sync/#syncing-development-dependencies)
- [uv settings reference](https://docs.astral.sh/uv/reference/settings/)
- [uv environment-variable reference](https://docs.astral.sh/uv/reference/environment/)
- [uv CLI reference](https://docs.astral.sh/uv/reference/cli/)
- [uv configuration loading source](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv/src/lib.rs#L299-L347)
- [uv filesystem-settings source](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-settings/src/lib.rs#L50-L209)
- [uv CLI group arguments](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/crates/uv-cli/src/lib.rs#L6995-L7073)
