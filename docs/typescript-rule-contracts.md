# Deterministic TypeScript patterns — typescript-v1

This is the normative contract for `typescript-v1`. It maps the exact 23-item Refactoring.Guru catalog to 28 deterministic rules. Ten authored-source rules are implemented; 18 type-, contract-, compiler-, coverage-, or history-dependent rules are explicit pending work. A pending required rule is an error, never a pass.

## Shared source contract

- The policy selects `typescript-v1` and `authored_source`. Full scans include `.ts`, `.tsx`, `.mts`, and `.cts`; staged scans read policy and source from regular Git-index blobs. Both modes also capture `Cargo.toml`, `pyproject.toml`, and `package.json` runtime markers so findings are grouped under the nearest repository implementation before the repository rollup.
- The locked Tree-sitter TypeScript grammar parses `.ts`, `.mts`, and `.cts`; the locked TSX grammar parses `.tsx`. If the initial tree contains an error, the scanner may retry once after replacing only a generic-call type argument shaped exactly as `<typeof import("literal")>` or its single-quoted equivalent with a same-length `<T ...>` form that preserves every byte offset and line break. This compensates for a pinned grammar gap around zero-argument calls and trailing commas. The compatibility tree supplies structure while locations and excerpts use the original source; its duplicate fingerprint represents the replacement type argument as one normalized identifier. The retry is accepted only when its entire tree is error-free. Any other syntax error makes the scan incomplete. No import execution, module loading, TypeScript compiler, test runner, or application code is run.
- A callable is a function/generator declaration or expression, arrow function, or concrete class method. Parameters are named children of `formal_parameters`; an unparenthesized single arrow parameter counts once. Defaulted, optional, rest, and constructor parameter-properties each count once.
- Body code lines are distinct zero-based syntax rows occupied by non-comment named leaves inside the body, reported as a count. Blank lines, comment-only rows, and delimiter-only rows are excluded; multiline syntax occupies every row spanned by its leaf.
- A class is a `class`, `class_declaration`, or `abstract_class_declaration`. Its field set is unique direct `public_field_definition` names plus constructor parameters marked by accessibility, `readonly`, or `override`. Its method set includes direct concrete methods, abstract method signatures, and class method signatures; only concrete bodies add method lines. Nested classes/functions do not add members to an enclosing class. Members are source-owned syntax, not declaration merging, prototype mutation, decorators, emitted parameter-property assignments, or inherited members.
- Ratios use integer cross-products; findings and inputs are sorted; resource-budget exhaustion errors. A structural match is evidence for review, not proof of a semantic defect.
- Every matched finding includes source location/excerpt, observed value, match operator, threshold, evidence, smell identity, remediation guidance, and its exact Refactoring.Guru URL. The downstream agent must perform the report's non-negotiable external research call before review or remediation.

## Implemented source rules

<a id="typescript-function-lines"></a>
### `typescript.function_lines@1`

Match when authored body code lines are greater than `maximum` (default 100). The maximum is inclusive.

<a id="typescript-function-arguments"></a>
### `typescript.function_arguments@1`

Match when declared parameter nodes are greater than `maximum` (default 7). JavaScript's implicit `this` is not a declared parameter and is not counted; an explicit TypeScript `this` parameter is counted.

<a id="typescript-class-fields"></a>
### `typescript.class_fields@1`

Match when the class field set defined above is greater than `maximum` (default 15).

<a id="typescript-class-methods"></a>
### `typescript.class_methods@1`

Match when direct source-owned class methods are greater than `maximum` (default 20).

<a id="typescript-class-method-lines"></a>
### `typescript.class_method_lines@1`

Match when the sum of authored code lines across direct methods is greater than `maximum` (default 500).

<a id="typescript-data-clumps"></a>
### `typescript.data_clumps@1`

For every callable, form the sorted unique set of `(parameter name, compact authored type syntax)`, excluding an explicit parameter named `this`. Enumerate every subset of exactly `minimum_group_size` (default 3). Match a group supported by at least `minimum_declarations` distinct callables (default 3). Untyped parameters have an empty type string. Exhausting `maximum_group_combinations` errors.

<a id="typescript-comment-share"></a>
### `typescript.comment_share@1`

Count ordinary Tree-sitter comment rows in the callable minus rows also occupied by code. Match when body code lines are at least `minimum_code_lines` (default 20) and `comment_lines / (comment_lines + code_lines)` is at least `minimum_share_percent` (default 30). JSDoc is comment syntax and is counted when it falls inside the callable node.

<a id="typescript-duplicate-functions"></a>
### `typescript.duplicate_functions@1`

Normalize Tree-sitter leaf tokens in each body: identifier spellings become `identifier`, literal values become literal-kind tags, comments are removed, and keywords/operators/delimiters retain grammar kinds. Build a multiset of consecutive four-token windows. Both bodies must have at least `minimum_tokens` (default 20); multiset Jaccard must be at least `minimum_similarity_basis_points` (default 8200). Overlapping nested bodies in one file are not compared. Expand multiset occurrences into globally document-frequency-ordered features, then apply exact Jaccard prefix, size-ratio, and positional-overlap filters. These are necessary conditions, so they cannot remove a threshold-matching pair. `maximum_pairs` counts the remaining unique pairs whose full multiset intersection/union is evaluated; exhaustion errors rather than returning a truncated success.

<a id="typescript-data-class"></a>
### `typescript.data_class@1`

Match when fields are at least `minimum_fields` (default 2) and non-accessor operations are at most `maximum_operations` (default 0). `constructor` is excluded. A strict accessor is one statement that either returns exactly `this.x`, or assigns its only ordinary identifier parameter directly to such a member. Decorator semantics, emitted code, declaration merging, and inheritance are not inferred.

<a id="typescript-lazy-class"></a>
### `typescript.lazy_class@1`

Match only when field count, method count, and summed method lines are all at most their configured maxima (defaults 1, 1, and 5). Small nominal, schema, or transport classes can legitimately match.

## Pending rules

The contracts below describe required evidence, but their providers are not implemented in `typescript-v1`.

<a id="typescript-primitive-slots"></a>
### `typescript.primitive_slots@1` — pending
Requires resolved slot types; match `minimum_raw_slots` and `minimum_share_percent` for registered primitive types.

<a id="typescript-alternative-interfaces"></a>
### `typescript.alternative_interfaces@1` — pending
Requires type/interface resolution before comparing behaviorally similar distinct classes.

<a id="typescript-repeated-dispatch"></a>
### `typescript.repeated_dispatch@1` — pending
Requires resolved variant/type dispatch; match `minimum_sites` with at least `minimum_arms` each.

<a id="typescript-temporary-fields"></a>
### `typescript.temporary_fields@1` — pending
Requires resolved field-use ownership; match `minimum_fields` across `minimum_methods` at no more than `maximum_use_percent`.

<a id="typescript-forwarding-share"></a>
### `typescript.forwarding_share@1` — pending
Requires resolved delegation; match `minimum_methods` and `minimum_share_percent` strict forwarders.

<a id="typescript-function-crap"></a>
### `typescript.function_crap@1` — pending
Requires pinned complexity and complete matching coverage evidence; match CRAP greater than `maximum`.

<a id="typescript-unused-code"></a>
### `typescript.unused_code@1` — pending
Requires pinned complete diagnostics; match findings greater than `maximum_findings`.

<a id="typescript-unused-type-parameters"></a>
### `typescript.unused_type_parameters@1` — pending
Requires pinned type diagnostics; match findings greater than `maximum_findings`.

<a id="typescript-nominal-slot-contract"></a>
### `typescript.nominal_slot_contract@1` — pending
Requires full type resolution plus a project nominal-slot contract; match mismatches greater than `maximum_mismatches`.

<a id="typescript-port-conformance"></a>
### `typescript.port_conformance@1` — pending
Requires declared ports and complete conformance evidence; match failures greater than `maximum_failures`.

<a id="typescript-refused-bequest"></a>
### `typescript.refused_bequest@1` — pending
Requires a resolved inheritance graph; match at least `minimum_inherited_members` with use at or below the configured `minimum_unused_percent` complement.

<a id="typescript-divergent-change"></a>
### `typescript.divergent_change@1` — pending
Requires a pinned logical-change ledger; match `minimum_changes` and `minimum_responsibilities` for one owner.

<a id="typescript-parallel-inheritance"></a>
### `typescript.parallel_inheritance@1` — pending
Requires resolved inheritance graphs and pinned history; match at least `minimum_parallel_pairs` paired additions.

<a id="typescript-shotgun-surgery"></a>
### `typescript.shotgun_surgery@1` — pending
Requires pinned logical changes and ownership; match owner count greater than `maximum_owners`.

<a id="typescript-foreign-accesses"></a>
### `typescript.foreign_accesses@1` — pending
Requires member-owner resolution; match `minimum_foreign_accesses` and a foreign share strictly above `minimum_share_percent_exclusive`.

<a id="typescript-dependency-contract"></a>
### `typescript.dependency_contract@1` — pending
Requires a closed allowed-edge contract and resolved imports; match forbidden accesses greater than `maximum_forbidden_accesses`.

<a id="typescript-library-capabilities"></a>
### `typescript.library_capabilities@1` — pending
Requires pinned dependencies and complete named capability tests; match failures greater than `maximum_failures`.

<a id="typescript-navigation-chains"></a>
### `typescript.navigation_chains@1` — pending
Requires resolved domain owners; match contiguous navigation across at least `minimum_transitions` owner changes.
