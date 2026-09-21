# Deterministic Python patterns — python-v1

This is the normative contract for `python-v1`. It maps the exact 23-item Refactoring.Guru catalog to 28 deterministic rules. Every rule has a built-in collector. Eighteen rules also accept a higher-fidelity per-rule override through the optional [provider-evidence contract](provider-evidence.md).

## Shared source contract

- The policy selects `python-v1` and `authored_source`. Full scans include `.py` and `.pyi`; staged scans read policy and source from regular Git-index blobs. Both modes also capture `Cargo.toml`, `pyproject.toml`, and `package.json` runtime markers so findings are grouped under the nearest repository implementation before the repository rollup.
- The locked Tree-sitter Python grammar parses every captured file. Any syntax error makes the scan incomplete. No import execution, module loading, type checker, test runner, or application code is run.
- A callable is a `function_definition` or `lambda`, including a method and an async definition represented by the function grammar node. Parameters are the named children of `parameters`; `self`, `cls`, defaulted, typed, positional-only, keyword-only, `*args`, and `**kwargs` each count once when represented as one parameter node.
- Body code lines are distinct zero-based syntax rows occupied by non-comment named leaves inside the body, reported as a count. Blank lines, comment-only rows, and delimiter-only rows are excluded; multiline syntax occupies every row spanned by its leaf.
- A class is a `class_definition`. Its field set is the union of direct class-body assignment/annotation targets and unique `self.x`/`cls.x` assignment targets in direct methods. Its method set is direct functions, including decorator-wrapped functions. Nested classes/functions do not add members to an enclosing class. Methods and fields are source-owned syntax, not inferred runtime monkey-patches, descriptors, dataclass transforms, or inherited members.
- Ratios use integer cross-products; findings and inputs are sorted; resource-budget exhaustion errors. A structural match is evidence for review, not proof of a semantic defect.
- Every matched finding includes source location/excerpt, observed value, match operator, threshold, evidence, smell identity, remediation guidance, and a mandatory Refactoring.Guru research URL. Before reviewing or remediating a finding, the agent must make an external research/tool call to that exact URL and read the page; if it cannot, it reports the research as incomplete and stops.

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

Normalize Tree-sitter leaf tokens in each body: identifier spellings become `identifier`, literal values become literal-kind tags, comments are removed, and keywords/operators/delimiters retain grammar kinds. Build a multiset of consecutive four-token windows. Both bodies must have at least `minimum_tokens` (default 20); multiset Jaccard must be at least `minimum_similarity_basis_points` (default 8200). Overlapping nested bodies in one file are not compared. Expand multiset occurrences into globally document-frequency-ordered features, then apply exact Jaccard prefix, size-ratio, and positional-overlap filters. These are necessary conditions, so they cannot remove a threshold-matching pair. `maximum_pairs` counts the remaining unique pairs whose full multiset intersection/union is evaluated; exhaustion errors rather than returning a truncated success.

<a id="python-data-class"></a>
### `python.data_class@1`

Match when fields are at least `minimum_fields` (default 2) and non-accessor operations are at most `maximum_operations` (default 0). `__init__` is excluded. A strict accessor is one statement that either returns exactly `self.x`/`cls.x`, or assigns its only ordinary identifier parameter directly to such a field. Decorator semantics, generated dataclass behavior, and inheritance are not inferred.

<a id="python-lazy-class"></a>
### `python.lazy_class@1`

Match only when field count, method count, and summed method lines are all at most their configured maxima (defaults 1, 1, and 5). Small nominal, schema, or transport classes can legitimately match.

## Built-in full-scan collectors and optional overrides

The rules below run without `--evidence`. Their built-in observations are conservative authored-source or bounded-Git-history indicators and identify their exact collector in each finding. A supplied external entry replaces the built-in observations for that rule, using the same measurement map and scanner-owned threshold evaluation.

<a id="python-primitive-slots"></a>
### `python.primitive_slots@1` — built in
For each class with annotated fields, count exact authored `str`, `int`, `float`, `bool`, and `bytes` types. Match `minimum_raw_slots` and `minimum_share_percent`; aliases and containers are not unwrapped.

<a id="python-alternative-interfaces"></a>
### `python.alternative_interfaces@1` — built in
Compare distinct classes with different method-name sets using identifier-normalized lexical body-token multisets. Match both token minima and configured Jaccard similarity. This is behavioral-shape evidence, not substitutability proof.

<a id="python-repeated-dispatch"></a>
### `python.repeated_dispatch@1` — built in
Group callables by their authored `case` label sequence. Match `minimum_sites` sharing at least `minimum_arms`; type identity is not inferred.

<a id="python-temporary-fields"></a>
### `python.temporary_fields@1` — built in
Treat fields annotated with `Optional`/`None` or assigned `None` as lifecycle candidates and count direct `self.field` use across class methods. Apply the configured field, method, and maximum-use thresholds.

<a id="python-forwarding-share"></a>
### `python.forwarding_share@1` — built in
Count non-constructor methods whose complete body is one optional `return` of `self.delegate.method(...)`. Match the configured method minimum and forwarding share.

<a id="python-function-crap"></a>
### `python.function_crap@1` — built in
Calculate authored lexical cyclomatic complexity and a conservative zero-coverage CRAP upper bound `C² + C`. Match scores greater than `maximum`. Complete external coverage may replace this pessimistic observation.

<a id="python-unused-code"></a>
### `python.unused_code@1` — built in
Count single-underscore private function/class declarations with no second identifier occurrence in the captured corpus. Match findings greater than `maximum_findings`; public and dynamic reachability are outside the built-in scope.

<a id="python-unused-type-parameters"></a>
### `python.unused_type_parameters@1` — built in
Count authored generic parameters declared in bracket syntax but not referenced elsewhere in the declaration. Match findings greater than `maximum_findings`.

<a id="python-nominal-slot-contract"></a>
### `python.nominal_slot_contract@1` — built in
Count primitive class fields with common domain-role names such as IDs, email, phone, money, dates, addresses, and country/postcode values. Match mismatches greater than `maximum_mismatches`; a project contract may replace this naming heuristic.

<a id="python-port-conformance"></a>
### `python.port_conformance@1` — built in
For classes inheriting an authored `Protocol`/`ABC`, compare direct method names with the port and count missing methods. Match failures greater than `maximum_failures`; behavioral tests remain an optional stronger override.

<a id="python-refused-bequest"></a>
### `python.refused_bequest@1` — built in
For same-corpus inheritance, count base methods and derived overrides that explicitly reject them using `pass`, `NotImplemented`, or `NotImplementedError`. Apply the inherited-member and unused-share thresholds.

<a id="python-divergent-change"></a>
### `python.divergent_change@1` — built in
Inspect up to 200 commits and measure each current file's change count plus distinct top-level co-change responsibilities. Apply both configured minima. Without Git history this rule is incomplete, never clean.

<a id="python-parallel-inheritance"></a>
### `python.parallel_inheritance@1` — built in
Compare authored inheritance families and count shared child-role prefixes/suffixes across different bases. Match at least `minimum_parallel_pairs`; this detects parallel structure without claiming synchronized historical addition.

<a id="python-shotgun-surgery"></a>
### `python.shotgun_surgery@1` — built in
Inspect up to 200 commits and count distinct top-level source responsibilities changed in each commit. Match owner count greater than `maximum_owners`. Without Git history this rule is incomplete, never clean.

<a id="python-foreign-accesses"></a>
### `python.foreign_accesses@1` — built in
Count dotted member accesses whose receiver is not `self`, `cls`, `this`, `Self`, or `super`. Apply the foreign count and exclusive-share thresholds. External type ownership can replace this lexical classification.

<a id="python-dependency-contract"></a>
### `python.dependency_contract@1` — built in
Count foreign accesses to underscore/private-style members. Match forbidden accesses greater than `maximum_forbidden_accesses`; an explicit dependency contract may replace this visibility heuristic.

<a id="python-library-capabilities"></a>
### `python.library_capabilities@1` — built in
Count authored foreign/prototype attribute mutation and `setattr` workarounds as missing-capability signals. Match failures greater than `maximum_failures`; capability tests may replace the heuristic.

<a id="python-navigation-chains"></a>
### `python.navigation_chains@1` — built in
Count authored dotted transitions in contiguous navigation expressions and match at least `minimum_transitions`. Fluent APIs may legitimately match; resolved domain-owner evidence may replace the lexical result.
