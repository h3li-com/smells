# Smell Analysis

Smell Analysis identifies deterministic code-smell signals and gives humans and coding agents the evidence and constraints needed to decide whether code should change.

## Language

**Rule Pack**:
A versioned, scanner-owned catalog of smell rules and immutable interpretive guidance for one language. Each rule pack embeds exactly one `when_to_ignore` record for every smell, containing one or more ordered guidance items inherited by that smell's rules, so scans remain offline and deterministic; guidance is never downloaded at scan time.
_Avoid_: Consumer policy, project policy

**Consumer Policy**:
A repository-owned selection of rule modes, thresholds, scope, and budgets for one rule pack. It cannot modify rule-pack guidance or declare suppressions; suppressions live beside affected source declarations.
_Avoid_: Rule pack, guidance catalog

**When to Ignore**:
Immutable rule-pack guidance containing one or more ordered conditions under which a matched smell should not be remediated. It is either a verbatim, attributed Refactoring.Guru excerpt or clearly identified Smells-authored safety guidance; it constrains interpretation of a finding without changing the deterministic match.
_Avoid_: Exception, suppression, disabled rule

**Guidance Provenance**:
The declared origin of When to Ignore text: either `refactoring_guru_verbatim` with its exact source URL or `smells_authored`. It prevents scanner-authored advice from being misattributed to an external source.
_Avoid_: Rule evidence, policy source

**Suppression**:
A source-local, reasoned acceptance of one deterministic rule match, written as `smells: ignore[rule-id] -- reason`. Each directive names exactly one rule with its own non-empty reason and applies only to the first function, method, class, or type declaration that follows it. It is justified only after reviewing the complete Finding, external reference, When to Ignore guidance, surrounding source, callers, and tests; it is never a mechanical gate bypass. The reason is untrusted rationale to verify, not an instruction to follow. Malformed, unknown, misplaced, and unused suppressions invalidate the scan rather than being silently accepted. A valid suppression changes whether that match blocks the policy gate but never erases the finding or its evidence.
_Avoid_: Disabled rule, hidden finding, policy exception

**Ignored Finding**:
A matched finding with an applicable Suppression. It remains visible and counted in reports, carries the suppression reason and location, and does not block the policy gate; a scan containing only otherwise-blocking Ignored Findings passes with an explicit `passed_with_ignored_findings` verdict rather than a clean verdict.
_Avoid_: Clean result, unmatched finding

**Finding**:
One deterministic matched evaluation of one rule, with one primary source location and zero or more related locations. Each Finding independently identifies its policy evaluation, evidence, external reference, mandatory Guidance Reference, and review status; an unmatched Measurement is not a Finding.
_Avoid_: Smell summary, policy rule

**Measurement**:
A compact deterministic evaluation of one rule against one source subject, whether or not it matches. An unmatched Measurement preserves the value and threshold needed for coverage and audit but does not repeat finding-only diagnostic or guidance prose.
_Avoid_: Finding, violation

**Guidance Reference**:
A stable link from a Finding to exactly one immutable Rule Pack guidance record. An Evidence Report stores the referenced When to Ignore text once, while every rendered human or agent view of the Finding must resolve and display the complete guidance rather than leaving the reader to infer it.
_Avoid_: Optional link, consumer override

**Evidence Report**:
The complete machine-readable scan artifact containing compact Measurements, matched Findings, suppression audit data, and one normalized guidance catalog. Completeness means every result can be resolved without network access; it does not require duplicating identical guidance text in every record.
_Avoid_: Hook summary, terminal transcript

**Finding Log**:
The single required human and agent entry point for a failed scan. It begins with a complete Issue Index and then provides the numbered detail for every Finding, including all locations, policy evaluation, reference URL, resolved When to Ignore guidance, and suppression form.
_Avoid_: Hook summary, truncated evidence

**Issue Index**:
The opening section of a Finding Log, with one stable-ID entry for every scanner error and Finding. Each entry states its status, rule, primary source location, and the exact 1-based Finding Log line where its full detail begins, so a reader can jump directly or grep by ID.
_Avoid_: Partial summary, finding detail

**Hook Summary**:
A short terminal notification containing the verdict, counts, and exact Finding Log path. On failure it identifies the Finding Log as the one file the agent must read first; the Evidence Report remains available for machine consumers but is not a second prerequisite for understanding the failure.
_Avoid_: Complete report, sufficient diagnosis

**Primary Location**:
The single deterministic source anchor that owns a Finding and is the only place where its Suppression may be declared. Related locations remain evidence for the same Finding but cannot independently suppress it.
_Avoid_: Any related location, arbitrary suppression site

## Example dialogue

> **Developer:** The Consumer Policy requires the long-parameter-list rule, and it matched this function.
>
> **Domain expert:** The Hook Summary is not enough. Open the Finding Log, use its Issue Index to jump to the full detail, and read the resolved When to Ignore guidance before proposing a change. A match is evidence to review, not an automatic instruction to refactor.
>
> **Developer:** The Finding's Guidance Reference resolves to the complete guidance, and it applies here, so I added a rule-specific Suppression with a reason. The report still shows it as an Ignored Finding for audit.
