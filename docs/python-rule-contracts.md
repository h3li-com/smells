# Deterministic Python patterns — python-v1

This is the normative contract for `python-v1`. It maps the exact 23-item Refactoring.Guru catalog to 28 deterministic rules. Ten authored-source rules are implemented; 18 type-, contract-, compiler-, coverage-, or history-dependent rules are explicit pending work. A pending required rule is an error, never a pass.

## Shared source contract

- The policy selects `python-v1` and `authored_source`. Full scans include `.py` and `.pyi`; staged scans read both policy and source from regular Git-index blobs.
- The locked Tree-sitter Python grammar parses every captured file. Any syntax error makes the scan incomplete. No import execution, module loading, type checker, test runner, or application code is run.
- A callable is a `function_definition` or `lambda`, including a method and an async definition represented by the function grammar node. Parameters are the named children of `parameters`; `self`, `cls`, defaulted, typed, positional-only, keyword-only, `*args`, and `**kwargs` each count once when represented as one parameter node.
- Body code lines are distinct zero-based syntax rows occupied by non-comment named leaves inside the body, reported as a count. Blank lines, comment-only rows, and delimiter-only rows are excluded; multiline syntax occupies every row spanned by its leaf.
- A class is a `class_definition`. Its field set is the union of direct class-body assignment/annotation targets and unique `self.x`/`cls.x` assignment targets in direct methods. Its method set is direct functions, including decorator-wrapped functions. Nested classes/functions do not add members to an enclosing class. Methods and fields are source-owned syntax, not inferred runtime monkey-patches, descriptors, dataclass transforms, or inherited members.
- Ratios use integer cross-products; findings and inputs are sorted; resource-budget exhaustion errors. A structural match is evidence for review, not proof of a semantic defect.
- Every matched finding includes source location/excerpt, observed value, match operator, threshold, evidence, smell identity, remediation guidance, and its exact Refactoring.Guru URL. The downstream agent must perform the report's non-negotiable external research call before review or remediation.

## Implemented source rules

<a id="python-function-lines"></a>
### `python.function_lines@1`

Match when authored body code lines are greater than `maximum` (default 100). The maximum is inclusive.

<a id="python-function-arguments"></a>
### `python.function_arguments@1`

Match when declared parameter nodes are greater than `maximum` (default 7). A method receiver is a declared parameter and counts once.

<a id="python-class-fields"></a>
### `python.class_fields@1`

Match when the class field set defined above is greater than `maximum` (default 15).

<a id="python-class-methods"></a>
### `python.class_methods@1`

Match when direct source-owned class methods are greater than `maximum` (default 20).

<a id="python-class-method-lines"></a>
### `python.class_method_lines@1`

Match when the sum of authored code lines across direct methods is greater than `maximum` (default 500).

<a id="python-data-clumps"></a>
### `python.data_clumps@1`

For every callable, form the sorted unique set of `(parameter name, compact authored type syntax)`, excluding `self` and `cls`. Enumerate every subset of exactly `minimum_group_size` (default 3). Match a group supported by at least `minimum_declarations` distinct callables (default 3). Untyped parameters have an empty type string. Exhausting `maximum_group_combinations` errors.

<a id="python-comment-share"></a>
### `python.comment_share@1`

Count ordinary Tree-sitter comment rows in the function minus rows also occupied by code. Match when body code lines are at least `minimum_code_lines` (default 20) and `comment_lines / (comment_lines + code_lines)` is at least `minimum_share_percent` (default 30). Documentation strings are code strings, not comments.

<a id="python-duplicate-functions"></a>
### `python.duplicate_functions@1`

Normalize Tree-sitter leaf tokens in each body: identifier spellings become `identifier`, literal values become literal-kind tags, comments are removed, and keywords/operators/delimiters retain grammar kinds. Build a multiset of consecutive four-token windows. Both bodies must have at least `minimum_tokens` (default 20); multiset Jaccard must be at least `minimum_similarity_basis_points` (default 8200). Overlapping nested bodies in one file are not compared. The unordered-pair budget is enforced before a successful result.

<a id="python-data-class"></a>
### `python.data_class@1`

Match when fields are at least `minimum_fields` (default 2) and non-accessor operations are at most `maximum_operations` (default 0). `__init__` is excluded. A strict accessor is one statement that either returns exactly `self.x`/`cls.x`, or assigns its only ordinary identifier parameter directly to such a field. Decorator semantics, generated dataclass behavior, and inheritance are not inferred.

<a id="python-lazy-class"></a>
### `python.lazy_class@1`

Match only when field count, method count, and summed method lines are all at most their configured maxima (defaults 1, 1, and 5). Small nominal, schema, or transport classes can legitimately match.

## Pending rules

The contracts below describe required evidence, but their providers are not implemented in `python-v1`.

<a id="python-primitive-slots"></a>
### `python.primitive_slots@1` — pending
Requires resolved slot types; match `minimum_raw_slots` and `minimum_share_percent` for registered primitive types.

<a id="python-alternative-interfaces"></a>
### `python.alternative_interfaces@1` — pending
Requires type/interface resolution before comparing behaviorally similar distinct classes.

<a id="python-repeated-dispatch"></a>
### `python.repeated_dispatch@1` — pending
Requires resolved variant/type dispatch; match `minimum_sites` with at least `minimum_arms` each.

<a id="python-temporary-fields"></a>
### `python.temporary_fields@1` — pending
Requires resolved field-use ownership; match `minimum_fields` across `minimum_methods` at no more than `maximum_use_percent`.

<a id="python-forwarding-share"></a>
### `python.forwarding_share@1` — pending
Requires resolved delegation; match `minimum_methods` and `minimum_share_percent` strict forwarders.

<a id="python-function-crap"></a>
### `python.function_crap@1` — pending
Requires pinned complexity and complete matching coverage evidence; match CRAP greater than `maximum`.

<a id="python-unused-code"></a>
### `python.unused_code@1` — pending
Requires pinned complete diagnostics; match findings greater than `maximum_findings`.

<a id="python-unused-type-parameters"></a>
### `python.unused_type_parameters@1` — pending
Requires pinned type diagnostics; match findings greater than `maximum_findings`.

<a id="python-nominal-slot-contract"></a>
### `python.nominal_slot_contract@1` — pending
Requires full type resolution plus a project nominal-slot contract; match mismatches greater than `maximum_mismatches`.

<a id="python-port-conformance"></a>
### `python.port_conformance@1` — pending
Requires declared ports and complete conformance evidence; match failures greater than `maximum_failures`.

<a id="python-refused-bequest"></a>
### `python.refused_bequest@1` — pending
Requires a resolved inheritance graph; match at least `minimum_inherited_members` with use at or below the configured `minimum_unused_percent` complement.

<a id="python-divergent-change"></a>
### `python.divergent_change@1` — pending
Requires a pinned logical-change ledger; match `minimum_changes` and `minimum_responsibilities` for one owner.

<a id="python-parallel-inheritance"></a>
### `python.parallel_inheritance@1` — pending
Requires resolved inheritance graphs and pinned history; match at least `minimum_parallel_pairs` paired additions.

<a id="python-shotgun-surgery"></a>
### `python.shotgun_surgery@1` — pending
Requires pinned logical changes and ownership; match owner count greater than `maximum_owners`.

<a id="python-foreign-accesses"></a>
### `python.foreign_accesses@1` — pending
Requires member-owner resolution; match `minimum_foreign_accesses` and a foreign share strictly above `minimum_share_percent_exclusive`.

<a id="python-dependency-contract"></a>
### `python.dependency_contract@1` — pending
Requires a closed allowed-edge contract and resolved imports; match forbidden accesses greater than `maximum_forbidden_accesses`.

<a id="python-library-capabilities"></a>
### `python.library_capabilities@1` — pending
Requires pinned dependencies and complete named capability tests; match failures greater than `maximum_failures`.

<a id="python-navigation-chains"></a>
### `python.navigation_chains@1` — pending
Requires resolved domain owners; match contiguous navigation across at least `minimum_transitions` owner changes.
