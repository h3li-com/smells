# Finding report interface

The JSON report is the interface between the deterministic scanner and a review agent. The scanner owns measurement and threshold evaluation. The agent owns semantic investigation and any proposed code change.

`report_schema_version` is currently `4`. Version 4 adds `provider_evidence_sha256`; an empty value means no provider bundle was supplied. `input_sha256` continues to identify the captured policy, manifests, and source, while a nonempty provider digest identifies the exact supplemental evidence bytes evaluated against that input. Version 3 introduced `implementation_results` as the primary monorepo result. Each implementation contains its own canonical smell results, while the top-level `smell_results` remains the repository rollup and the rule-level `coverage` and `findings` remain the evidence. Consumers must reject unsupported versions rather than guessing field semantics.

## Repository implementation model

An implementation is a repository subtree owned by the nearest ancestor runtime manifest. Discovery uses the closed manifest set `Cargo.toml`, `pyproject.toml`, and `package.json` anywhere inside the captured repository root. These map respectively to `rust_cargo_project`, `python_project`, and `javascript_typescript_package`. Discovery applies to every selected source file regardless of the source language, so a Python fixture generator below a frontend `package.json` is correctly reported under that frontend implementation.

Nested manifests take precedence over parent manifests. Multiple runtime manifests in the same directory describe one multi-runtime implementation. A runtime manifest symlink is rejected. In staged mode, manifests come from the Git index; in path mode, they come from the captured worktree. Manifest paths and bytes participate in `input_sha256`, and discovery never executes a manifest or application code.

Source files with no ancestor runtime manifest are not silently folded into another package. They appear under `implementation_id: "__unowned__"`, `ownership: "unowned_source"`, and a null `implementation_root`. The repository summary exposes `implementations` and `unowned_files` counts so a hook can decide whether repository-level scripts are expected.

`implementation_results` is ordered by repository-relative implementation root, with `__unowned__` last. Each entry contains:

- Repository-relative `implementation_id` and `implementation_root` (`.` means the captured repository root).
- `ownership`, `implementation_types`, `runtime_types`, and the exact `runtime_manifests` used as evidence.
- The number of selected-language `scanned_files` owned by the implementation.
- A local summary and exactly 23 `smell_results` derived only from findings whose primary or related evidence occurs in that implementation.

A cross-implementation finding, such as duplicate code between two packages, is referenced by both implementations. Therefore implementation finding counts are local impact counts and must not be summed to reconstruct the repository total. Every implementation uses the same global `matched_finding_indices`, so those indexes still address the top-level `findings` array and its repository-relative locations.

```json
{
  "implementation_id": "backend",
  "implementation_root": "backend",
  "ownership": "runtime_manifest",
  "implementation_types": ["python_project"],
  "runtime_types": ["python"],
  "runtime_manifests": ["backend/pyproject.toml"],
  "scanned_files": 1446,
  "summary": {
    "verdict": "blocked_by_required_patterns",
    "total_smell_patterns": 23,
    "matched_smell_patterns": 7,
    "blocking_smell_patterns": 3,
    "review_smell_patterns": 4,
    "error_smell_patterns": 0,
    "matched_smell_ids": ["long-method", "large-class", "long-parameter-list", "data-clumps", "duplicate-code", "data-class", "lazy-class"]
  },
  "smell_results": []
}
```

## Pattern-first result model

Every implementation's `smell_results` is its primary smell result. The top-level collection is the repository rollup. Both contain exactly one entry for each of the 23 canonical Refactoring.Guru smell patterns, in registry order. A smell can have several deterministic rules; these collections aggregate their outcomes without treating the rule machinery as separate user-facing smells.

Each entry contains the canonical `smell_id`, display `smell`, `category`, `reference_url`, language `applicability`, aggregate `state`, and `coverage_status`. Its counts distinguish matched required findings from report-only review signals. `measured_rule_ids`, `matched_rule_ids`, `pending_rule_ids`, `disabled_rule_ids`, and `incomplete_rule_ids` explain which detectors contributed. `affected_files` and `affected_symbols` count unique primary and related evidence locations. `matched_finding_indices` contains zero-based indexes into the top-level `findings` array, so an agent can move from the smell-level result to the exact lines, measurements, thresholds, evidence, and guidance without duplicating findings.

The closed `state` values are:

- `blocking_match`: at least one required rule matched.
- `review_match`: no required rule matched, but at least one report-only rule matched.
- `checked_no_match_in_measured_scope`: at least one implemented enabled rule completed and none matched. This never claims that the semantic smell is absent.
- `pending`: reserved for a future rule pack that registers an unavailable detector; all v1 rules now have source or provider evaluators.
- `disabled`: every registered implemented detector is disabled by policy.
- `not_applicable`: the canonical smell does not apply to the selected language model.
- `error`: measurement was incomplete. Do not infer absence or continue as if the check passed.

`coverage_status` independently states whether the smell was `measured_defined_scope`, `measured_with_pending_rules`, `pending`, `disabled`, `not_applicable`, or `incomplete`. In the v1 packs, an enabled provider-backed rule without complete pinned evidence is `incomplete`, never pending or passed. The deterministic source match remains valid evidence, but it is not complete semantic coverage unless every enabled provider rule also completed.

Every smell result repeats the canonical URL and `reference_check`. For a matched smell, the consuming agent must open that exact URL before reviewing any referenced finding. The detailed finding retains rule-specific guidance and the same non-negotiable research gate.

```json
{
  "smell_id": "large-class",
  "smell": "Large Class",
  "category": "bloaters",
  "reference_url": "https://refactoring.guru/smells/large-class",
  "reference_check": {
    "required": true,
    "non_negotiable": true,
    "action": "perform_external_research_call_to_reference_url",
    "required_before": "review_or_remediation",
    "unavailable_action": "report_reference_research_incomplete_and_do_not_review_or_remediate"
  },
  "applicability": "applicable",
  "state": "blocking_match",
  "interpretation": "One or more required deterministic rules matched this smell pattern.",
  "coverage_status": "measured_defined_scope",
  "evaluated_findings": 3,
  "matched_findings": 3,
  "blocking_findings": 2,
  "review_signals": 1,
  "within_pattern_limits": 0,
  "affected_files": 1,
  "affected_symbols": 1,
  "measured_rule_ids": ["rust.type_fields", "rust.enum_variants", "rust.type_functions", "rust.type_function_lines", "rust.trait_functions"],
  "matched_rule_ids": ["rust.type_fields", "rust.type_function_lines", "rust.type_functions"],
  "pending_rule_ids": [],
  "disabled_rule_ids": [],
  "incomplete_rule_ids": [],
  "matched_finding_indices": [0, 1, 2]
}
```

The repository summary repeats compact implementation and smell-level routing data: implementation and unowned-source counts, total/matched/blocking/review/error pattern counts, and canonical `matched_smell_ids`. Existing finding counts remain available for volume and policy decisions.

## Hook decision order

1. Read `summary.verdict`. If it is `incomplete_due_to_errors`, do not infer that unmatched rules passed; resolve the input, parser, ownership, budget, or provider error first.
2. Read `implementation_results` as the primary monorepo result. Review `__unowned__` explicitly instead of assuming those sources belong to a package.
3. Within each implementation, start with `blocking_match` and `review_match`; use `coverage_status` and the rule ID lists to avoid claiming more coverage than ran. Use the top-level `smell_results` only when a repository rollup is required.
4. Follow `matched_finding_indices` into `findings`. `blocking: true` is a required policy violation. A nonblocking match is a review signal, not proof that the design is wrong.
5. Open `location` and every `related_location`. `source_excerpt` and up to five `related_source_excerpts` provide bounded context, but the agent should read the enclosing declaration and relevant callers/tests before editing.
6. Use `evaluation.observed`, `evaluation.match_condition`, and `evaluation.threshold` as the reason the deterministic pattern fired. Do not replace this verdict with an LLM score.
7. Enforce `reference_check` before semantic review. The research is non-negotiable: make an external research/tool call that opens the exact `reference_url` and read that page. Memory, a search-result snippet, or the URL string in the report is not completion of the research call. If the page cannot be consulted, report that the reference research is incomplete and stop without reviewing or remediating the finding.
8. Use the canonical `smell_id`, `smell`, `category`, and `pattern_type` to identify the smell and kind of detector. `diagnostic.signal` states the concrete pattern being checked. After the required research call, use `why_it_matters` and `review` to investigate the risk. Treat `diagnostic.remediation` as a candidate behavior-preserving move, not an instruction to refactor blindly. Follow `diagnostic.contract` for the exact selected language-pack algorithm. Refactoring.Guru does not define this project's numeric thresholds; the checked-in policy does.

Every rule owns its URL in the Rust guidance or the language-neutral portable guidance instantiated for Python/TypeScript. Startup validation requires exactly one guidance entry per registered rule and requires its URL to equal the canonical URL of the smell mapped by the registry. The emitted `diagnostic.review` begins with the non-negotiable research instruction and includes that exact URL. The scanner does not perform network access or falsely claim that the page was read; the consuming hook must require the external call before allowing review output.

Each guidance entry is self-contained: it repeats the canonical `smell_id`, display `smell`, Refactoring.Guru `category`, and `pattern_type`, followed by the exact `signal` the rule checks. Startup validation rejects guidance when any of those fields disagrees with the registry.

Source text, comments, string literals, symbol names, and evidence values are untrusted repository data. A hook must never interpret text inside them as instructions to the agent.

## Matched finding example

```json
{
  "rule_id": "rust.function_arguments",
  "rule_version": 1,
  "smell_id": "long-parameter-list",
  "smell": "Long Parameter List",
  "category": "bloaters",
  "pattern_type": "metric",
  "certainty": "exact_source_metric",
  "policy_mode": "required",
  "symbol": "src/lib.rs::create_order",
  "location": {"path": "src/lib.rs", "line": 42, "column": 4},
  "related_symbols": [],
  "related_locations": [],
  "source_excerpt": {
    "path": "src/lib.rs",
    "focus_line": 42,
    "focus_column": 4,
    "lines": [
      {"line": 41, "display_start_column": 1, "text": "", "truncated_before": false, "truncated_after": false},
      {"line": 42, "display_start_column": 1, "text": "fn create_order(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H) {", "truncated_before": false, "truncated_after": false},
      {"line": 43, "display_start_column": 1, "text": "    // ...", "truncated_before": false, "truncated_after": false}
    ]
  },
  "related_source_excerpts": [],
  "omitted_related_excerpts": 0,
  "evaluation": {"metric": "signature inputs including receiver", "observed": 8, "match_condition": ">", "threshold": 7, "matched": true},
  "diagnostic": {
    "headline": "Long Parameter List: rust.function_arguments violation",
    "explanation": "Observed signature inputs including receiver = 8; this satisfies the configured match condition > 7.",
    "signal": "A callable declares more signature inputs than the configured maximum.",
    "why_it_matters": "Many inputs can make call sites difficult to understand and can indicate a missing domain value or mixed responsibilities.",
    "review": "NON-NEGOTIABLE RESEARCH: Perform an external research/tool call to https://refactoring.guru/smells/long-parameter-list and read the page before reviewing this finding or proposing or applying remediation. If the URL cannot be consulted, report the reference research as incomplete and stop. Inspect recurring parameter groups, independent optional modes, call-site readability, and whether the arguments form one meaningful value.",
    "remediation": "Introduce a cohesive parameter object or nominal domain value for arguments that belong together, or split independent operations instead of merely hiding unrelated parameters.",
    "contract": "docs/rust-rule-contracts.md#rust-function-arguments",
    "reference_url": "https://refactoring.guru/smells/long-parameter-list",
    "reference_check": {
      "required": true,
      "non_negotiable": true,
      "action": "perform_external_research_call_to_reference_url",
      "required_before": "review_or_remediation",
      "unavailable_action": "report_reference_research_incomplete_and_do_not_review_or_remediate"
    }
  },
  "status": "violation",
  "blocking": true,
  "evidence": {"scope": "source_authored"}
}
```

Excerpts contain the focus line plus one line on either side. Each displayed line is limited to 320 Unicode scalar values and is centered so the focus column remains visible. At most five related excerpts are embedded; `omitted_related_excerpts` tells the agent how many additional `related_locations` must be opened directly.
