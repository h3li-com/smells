# Deterministic TypeScript patterns — typescript-v1

This is the normative contract for `typescript-v1`. It maps the exact 23-item Refactoring.Guru catalog to 28 deterministic rules. Every rule has a built-in collector. Eighteen rules also accept a higher-fidelity per-rule override through the optional [provider-evidence contract](provider-evidence.md).

## Shared source contract

- The policy selects `typescript-v1` and `authored_source`. Full scans include `.ts`, `.tsx`, `.mts`, and `.cts`; staged scans read policy and source from regular Git-index blobs. Both modes also capture `Cargo.toml`, `pyproject.toml`, and `package.json` runtime markers so findings are grouped under the nearest repository implementation before the repository rollup.
- The locked Tree-sitter TypeScript grammar parses `.ts`, `.mts`, and `.cts`; the locked TSX grammar parses `.tsx`. If the initial tree contains an error, the scanner may retry once after replacing only a generic-call type argument shaped exactly as `<typeof import("literal")>` or its single-quoted equivalent with a same-length `<T ...>` form that preserves every byte offset and line break. This compensates for a pinned grammar gap around zero-argument calls and trailing commas. The compatibility tree supplies structure while locations and excerpts use the original source; its duplicate fingerprint represents the replacement type argument as one normalized identifier. The retry is accepted only when its entire tree is error-free. Any other syntax error makes the scan incomplete. No import execution, module loading, TypeScript compiler, test runner, or application code is run.
- A callable is a function/generator declaration or expression, arrow function, or concrete class method. Parameters are named children of `formal_parameters`; an unparenthesized single arrow parameter counts once. Defaulted, optional, rest, and constructor parameter-properties each count once.
- Body code lines are distinct zero-based syntax rows occupied by non-comment named leaves inside the body, reported as a count. Blank lines, comment-only rows, and delimiter-only rows are excluded; multiline syntax occupies every row spanned by its leaf.
- A class is a `class`, `class_declaration`, or `abstract_class_declaration`. Its field set is unique direct `public_field_definition` names plus constructor parameters marked by accessibility, `readonly`, or `override`. Its method set includes direct concrete methods, abstract method signatures, and class method signatures; only concrete bodies add method lines. Nested classes/functions do not add members to an enclosing class. Members are source-owned syntax, not declaration merging, prototype mutation, decorators, emitted parameter-property assignments, or inherited members.
- Ratios use integer cross-products; findings and inputs are sorted; resource-budget exhaustion errors. A structural match is evidence for review, not proof of a semantic defect.
- Every matched finding includes source location/excerpt, observed value, match operator, threshold, evidence, smell identity, remediation guidance, and a mandatory Refactoring.Guru research URL. Before reviewing or remediating a finding, the agent must make an external research/tool call to that exact URL and read the page; if it cannot, it reports the research as incomplete and stops.

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

## Built-in full-scan collectors and optional overrides

The rules below run without `--evidence`. Their built-in observations are conservative authored-source or bounded-Git-history indicators and identify their exact collector in each finding. A supplied external entry replaces the built-in observations for that rule, using the same measurement map and scanner-owned threshold evaluation.

<a id="typescript-primitive-slots"></a>
### `typescript.primitive_slots@1` — built in
For each class with authored fields, count exact `string`, `number`, `boolean`, `bigint`, and `symbol` types. Match `minimum_raw_slots` and `minimum_share_percent`; aliases and containers are not unwrapped.

<a id="typescript-alternative-interfaces"></a>
### `typescript.alternative_interfaces@1` — built in
Compare distinct classes with different method-name sets using identifier-normalized lexical body-token multisets. Match both token minima and configured Jaccard similarity. This is behavioral-shape evidence, not substitutability proof.

<a id="typescript-repeated-dispatch"></a>
### `typescript.repeated_dispatch@1` — built in
Group callables by their authored `case` label sequence. Match `minimum_sites` sharing at least `minimum_arms`; discriminated-union identity is not inferred.

<a id="typescript-temporary-fields"></a>
### `typescript.temporary_fields@1` — built in
Treat optional, nullable, or `undefined` fields as lifecycle candidates and count direct `this.field` use across class methods. Apply the configured field, method, and maximum-use thresholds.

<a id="typescript-forwarding-share"></a>
### `typescript.forwarding_share@1` — built in
Count non-constructor methods whose complete body is one optional `return` of `this.delegate.method(...)`. Match the configured method minimum and forwarding share.

<a id="typescript-function-crap"></a>
### `typescript.function_crap@1` — built in
Calculate authored lexical cyclomatic complexity and a conservative zero-coverage CRAP upper bound `C² + C`. Match scores greater than `maximum`. Complete external coverage may replace this pessimistic observation.

<a id="typescript-unused-code"></a>
### `typescript.unused_code@1` — built in
Count single-underscore private function/class declarations with no second identifier occurrence in the captured corpus. Match findings greater than `maximum_findings`; exported and dynamic reachability are outside the built-in scope.

<a id="typescript-unused-type-parameters"></a>
### `typescript.unused_type_parameters@1` — built in
Count authored generic parameters declared in angle-bracket syntax but not referenced elsewhere in the declaration. Match findings greater than `maximum_findings`.

<a id="typescript-nominal-slot-contract"></a>
### `typescript.nominal_slot_contract@1` — built in
Count primitive class fields with common domain-role names such as IDs, email, phone, money, dates, addresses, and country/postcode values. Match mismatches greater than `maximum_mismatches`; a project contract may replace this naming heuristic.

<a id="typescript-port-conformance"></a>
### `typescript.port_conformance@1` — built in
For classes declaring an authored interface, compare direct method names with that interface and count missing methods. Match failures greater than `maximum_failures`; compiler and behavioral-test evidence remain optional stronger overrides.

<a id="typescript-refused-bequest"></a>
### `typescript.refused_bequest@1` — built in
For same-corpus inheritance, count base methods and derived overrides that explicitly reject them by throwing an error. Apply the inherited-member and unused-share thresholds.

<a id="typescript-divergent-change"></a>
### `typescript.divergent_change@1` — built in
Inspect up to 200 commits and measure each current file's change count plus distinct top-level co-change responsibilities. Apply both configured minima. Without Git history this rule is incomplete, never clean.

<a id="typescript-parallel-inheritance"></a>
### `typescript.parallel_inheritance@1` — built in
Compare authored inheritance families and count shared child-role prefixes/suffixes across different bases. Match at least `minimum_parallel_pairs`; this detects parallel structure without claiming synchronized historical addition.

<a id="typescript-shotgun-surgery"></a>
### `typescript.shotgun_surgery@1` — built in
Inspect up to 200 commits and count distinct top-level source responsibilities changed in each commit. Match owner count greater than `maximum_owners`. Without Git history this rule is incomplete, never clean.

<a id="typescript-foreign-accesses"></a>
### `typescript.foreign_accesses@1` — built in
Count dotted member accesses whose receiver is not `self`, `cls`, `this`, `Self`, or `super`. Apply the foreign count and exclusive-share thresholds. External type ownership can replace this lexical classification.

<a id="typescript-dependency-contract"></a>
### `typescript.dependency_contract@1` — built in
Count foreign accesses to underscore/private-style members. Match forbidden accesses greater than `maximum_forbidden_accesses`; an explicit dependency contract may replace this visibility heuristic.

<a id="typescript-library-capabilities"></a>
### `typescript.library_capabilities@1` — built in
Count authored prototype or foreign attribute mutation workarounds as missing-capability signals. Match failures greater than `maximum_failures`; capability tests may replace the heuristic.

<a id="typescript-navigation-chains"></a>
### `typescript.navigation_chains@1` — built in
Count authored dotted transitions in contiguous navigation expressions and match at least `minimum_transitions`. Fluent APIs may legitimately match; resolved domain-owner evidence may replace the lexical result.
