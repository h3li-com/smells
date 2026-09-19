# Rust smell checks

The Rust-first scanner recognizes fixed source patterns. Its [versioned registry](../rules/rust-v1.json) inventories all 23 [Refactoring.Guru smells](https://refactoring.guru/refactoring/smells); the [rule contracts](rust-rule-contracts.md) define every implemented and pending rule separately.

## Current implementation

Seventeen source rules are executable. Seven size rules can block by policy; ten structural indicators are report-only in the example. Findings include rule/version, smell, symbol, relative location, value, comparison, threshold, status, blocking flag and evidence. Function/type metrics below limits are included. Pair/group patterns emit qualifying matches rather than every nonmatching combination.

Eleven rules requiring compiler, full type resolution, coverage, domain/interface contracts or frozen history are pending. Their coverage remains visible when disabled. Requiring an unavailable detector errors. Refused Bequest and Parallel Inheritance Hierarchies are native-Rust exclusions, never passing measurements of the original inheritance smells.

## Determinism is a bounded claim

Identical captured source and policy with the same tested scanner build produces identical findings and verdict. Report digests identify input and implementation. Explicit scope, local ownership, tokenization, exact ratio comparisons, canonical ordering and fail-closed errors make the patterns reproducible.

The source pass does not determine all domain meaning, evaluate cfg like rustc, expand macros, infer missing requirements, or prove all semantic smells absent. Its scope includes tests and inactive branches. Some legitimate data messages, nominal types, algorithms, adapters and proxies match structural patterns. Report the evidence consistently; choose whether a project adopts a pattern as blocking separately.

## Implementation order

1. Source capture, parser, local owner resolution, function/type metrics and deterministic source indicators: first pass implemented with CLI fixtures.
2. Full compiler configuration and verified compiler/coverage evidence: required before activating Dead Code/CRAP and unused-type-parameter provider rules.
3. Full type resolution and explicit domain/interface/dependency/capability contracts: required for foreign accesses, navigation and capability checks.
4. Frozen logical-change ledger, ownership and responsibility contracts: required for Divergent Change/Shotgun Surgery.

Exact algorithms, per-rule test cases, limitations and parameter defaults live in one normative document: [rust-rule-contracts.md](rust-rule-contracts.md). No LLM is part of the detection or verdict path. Automatic refactoring is outside the current task.
