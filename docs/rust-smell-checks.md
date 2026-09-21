# Rust smell checks

The Rust-first scanner recognizes fixed source and Git-history patterns and can optionally evaluate pinned external provider facts. Its [versioned registry](../rules/rust-v1.json) inventories all 23 [Refactoring.Guru smells](https://refactoring.guru/refactoring/smells); the [rule contracts](rust-rule-contracts.md) define all 28 executable rules separately.

## Current implementation

All 28 rules have built-in collectors. Seven size rules can block by policy; structural, type-shape, coverage-risk, contract, and history indicators are report-only in the example. Findings include rule/version, smell, symbol, relative location, value, comparison, threshold, status, blocking flag and evidence. Function/type metrics below limits are included. Pair/group patterns emit qualifying matches rather than every nonmatching combination.

Eleven rules accept optional compiler, type-resolution, coverage, domain/interface, or curated-history replacements through the [provider-evidence interface](provider-evidence.md). Omitting those entries uses conservative built-in observations. Refused Bequest and Parallel Inheritance Hierarchies are native-Rust exclusions, never passing measurements of the original subclass smells.

## Determinism is a bounded claim

Identical captured source and policy with the same tested scanner build produces identical findings and verdict. Report digests identify input and implementation. Explicit scope, local ownership, tokenization, exact ratio comparisons, canonical ordering and fail-closed errors make the patterns reproducible.

The source pass does not determine all domain meaning, evaluate cfg like rustc, expand macros, infer missing requirements, or prove all semantic smells absent. Its scope includes tests and inactive branches. Some legitimate data messages, nominal types, algorithms, adapters and proxies match structural patterns. Report the evidence consistently; choose whether a project adopts a pattern as blocking separately.

## Fidelity layers

1. Source capture, parser, local owner resolution, function/type metrics, and deterministic structural indicators run by default.
2. Conservative source-level dead-code, CRAP, type, interface, dependency, and capability indicators run by default and state their limitations in evidence.
3. Bounded Git co-change history runs automatically when the scan root is inside a repository.
4. Optional external compiler diagnostics, complete coverage, resolved type graphs, tests, and project ledgers can replace individual built-in observations before a team promotes a heuristic to a strict gate.

Exact algorithms, per-rule test cases, limitations and parameter defaults live in one normative document: [rust-rule-contracts.md](rust-rule-contracts.md). No LLM is part of the detection or verdict path. Automatic refactoring is outside the current task.
