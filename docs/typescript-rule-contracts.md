# Deterministic TypeScript patterns — typescript-v1

This is the normative contract for `typescript-v1`. It maps the exact 23-item Refactoring.Guru catalog to 28 deterministic rules. Every rule has a built-in collector. Eighteen rules also accept a higher-fidelity per-rule override through the optional [provider-evidence contract](provider-evidence.md).

## Shared source contract

- The policy selects `typescript-v1` and `authored_source`. Full scans include `.ts`, `.tsx`, `.mts`, and `.cts`; staged scans read policy and source from regular Git-index blobs. Both modes also capture `Cargo.toml`, `pyproject.toml`, and `package.json` runtime markers so findings are grouped under the nearest repository implementation before the repository rollup.
- The locked Tree-sitter TypeScript grammar parses `.ts`, `.mts`, and `.cts`; the locked TSX grammar parses `.tsx`. If the initial tree contains an error, the scanner may retry once after replacing only a generic-call type argument shaped exactly as `<typeof import("literal")>` or its single-quoted equivalent with a same-length `<T ...>` form that preserves every byte offset and line break. This compensates for a pinned grammar gap around zero-argument calls and trailing commas. The compatibility tree supplies structure while locations and excerpts use the original source; its duplicate fingerprint represents the replacement type argument as one normalized identifier. The retry is accepted only when its entire tree is error-free. Any other syntax error makes the scan incomplete. No import execution, module loading, TypeScript compiler, test runner, or application code is run.
- A callable is a function/generator declaration or expression, arrow function, or concrete class method. Parameters are named children of `formal_parameters`; an unparenthesized single arrow parameter counts once. Defaulted, optional, rest, and constructor parameter-properties each count once.
- Body code lines are distinct zero-based syntax rows occupied by non-comment named leaves inside the body, reported as a count. Blank lines, comment-only rows, and delimiter-only rows are excluded; multiline syntax occupies every row spanned by its leaf.
- A class is a `class`, `class_declaration`, or `abstract_class_declaration`. Its field set is unique direct `public_field_definition` names whose value is not an arrow function or function expression, plus constructor parameters marked by accessibility, `readonly`, or `override`; a property holding a function is a method, not a field. Its method set includes direct concrete methods, direct properties holding an arrow function or function expression, abstract method signatures, and class method signatures; overload signatures of one name and their implementation count as one method, and only concrete bodies add method lines. Nested classes/functions do not add members to an enclosing class. Members are source-owned syntax, not declaration merging, prototype mutation, decorators, emitted parameter-property assignments, or inherited members.
- Ratios use integer cross-products; findings and inputs are sorted; resource-budget exhaustion errors. A structural match is evidence for review, not proof of a semantic defect.
- Every matched finding includes source location/excerpt, observed value, match operator, threshold, evidence, smell identity, remediation guidance, and a mandatory Refactoring.Guru research URL. Before reviewing or remediating a finding, the agent must make an external research/tool call to that exact URL and read the page; if it cannot, it reports the research as incomplete and stops.

## Implemented source rules

<a id="typescript-function-lines"></a>
### `typescript.function_lines@2`

Match when authored body code lines are greater than `maximum` (default 100). The maximum is inclusive.

Exact canonical `up`, `down`, or Prisma `operations` Declaration Roles in a high-confidence framework migration Source Role retain matched Measurements and Findings but are reported as nonblocking `reported_by_role` review signals. Inferred roles require actual framework imports/contracts and framework-bound parameters. An explicit anchored role supplies file-level framework evidence for wrappers or custom providers, but the scanner still requires the framework's exact top-level export or direct class relationship, entrypoint set, and nonempty parameters where that framework uses them. Helpers and sibling declarations use normal blocking semantics.

<a id="typescript-function-arguments"></a>
### `typescript.function_arguments@2`

Match when declared parameter nodes are greater than `maximum` (default 7). JavaScript's implicit `this` is not a declared parameter and is not counted; an explicit TypeScript `this` parameter is counted. Comments between parameters are not parameters.

<a id="typescript-class-fields"></a>
### `typescript.class_fields@1`

Match when the class field set defined above is greater than `maximum` (default 15).

<a id="typescript-class-methods"></a>
### `typescript.class_methods@1`

Match when direct source-owned class methods are greater than `maximum` (default 20).

<a id="typescript-class-method-lines"></a>
### `typescript.class_method_lines@2`

Match when the sum of authored code lines across direct methods is greater than `maximum` (default 500).

A TypeORM, MikroORM, or Prisma migration shell bound to its actual imported interface/base—or to the corresponding direct `implements`/`extends` structure inside an explicit anchored framework role—retains the aggregate Measurement and Finding but is reported as a nonblocking `reported_by_role` review signal. Unrelated sibling classes remain blocking.

<a id="typescript-data-clumps"></a>
### `typescript.data_clumps@2`

For every callable, form the sorted unique set of `(parameter name, compact authored type syntax)`, excluding an explicit parameter named `this`. Enumerate every subset of exactly `minimum_group_size` (default 3) and keep those supported by at least `minimum_declarations` distinct callables (default 3). Report one finding per closed group: the slots shared by every supporter of a qualifying subset, so a wider shared population yields one maximal group rather than one finding per subset. Untyped parameters have an empty type string. Exhausting `maximum_group_combinations` errors.

<a id="typescript-comment-share"></a>
### `typescript.comment_share@1`

Count ordinary Tree-sitter comment rows in the callable minus rows also occupied by code. Tool directives (`@ts-expect-error`, `@ts-ignore`, `eslint-disable`, `prettier-ignore`, `istanbul ignore`, and the other lint, type-checker and coverage suppressions the history rules recognise) are not ordinary comments. Match when body code lines are at least `minimum_code_lines` (default 20) and `comment_lines / (comment_lines + code_lines)` is at least `minimum_share_percent` (default 30). JSDoc is comment syntax and is counted when it falls inside the callable node.

<a id="typescript-duplicate-functions"></a>
### `typescript.duplicate_functions@1`

Normalize Tree-sitter leaf tokens in each body: identifier spellings become `identifier`, literal values become literal-kind tags, comments are removed, and keywords/operators/delimiters retain grammar kinds. Build a multiset of consecutive four-token windows. Both bodies must have at least `minimum_tokens` (default 20); multiset Jaccard must be at least `minimum_similarity_basis_points` (default 8200). Overlapping nested bodies in one file are not compared. Expand multiset occurrences into globally document-frequency-ordered features, then apply exact Jaccard prefix, size-ratio, and positional-overlap filters. These are necessary conditions, so they cannot remove a threshold-matching pair. `maximum_pairs` counts the remaining unique pairs whose full multiset intersection/union is evaluated; exhaustion errors rather than returning a truncated success. Passing pairs join into connected families, and each family is one finding (see `docs/report-interface.md`).

<a id="typescript-data-class"></a>
### `typescript.data_class@1`

Match when fields are at least `minimum_fields` (default 2) and non-accessor operations are at most `maximum_operations` (default 0). `constructor` is excluded, and a property holding an arrow function or function expression is an operation. A strict accessor is one statement that either returns exactly `this.x`, or assigns its only ordinary identifier parameter directly to such a member. Decorator semantics, emitted code, declaration merging, and inheritance are not inferred.

<a id="typescript-lazy-class"></a>
### `typescript.lazy_class@1`

Match only when field count, method count, and summed method lines are all at most their configured maxima (defaults 1, 1, and 5). Small nominal, schema, or transport classes can legitimately match.

## Built-in full-scan collectors and optional overrides

The rules below run without `--evidence`. Their built-in observations are conservative authored-source or bounded-Git-history indicators and identify their exact collector in each finding. A supplied external entry replaces the built-in observations for that rule, using the same measurement map and scanner-owned threshold evaluation.

<a id="typescript-primitive-slots"></a>
### `typescript.primitive_slots@2` — built in
For each class with authored fields (interface declarations are not classes and are not observed), count exact `string`, `number`, `boolean`, `bigint`, and `symbol` types. Match `minimum_raw_slots` and `minimum_share_percent`; aliases and containers are not unwrapped.

<a id="typescript-alternative-interfaces"></a>
### `typescript.alternative_interfaces@2` — built in
Compare distinct classes with different effective method-name sets. A class's effective set is its own methods plus those it inherits from authored bases, resolved as the declared-base rules resolve them, so subclasses that override different subsets of one authored base expose that base's one interface. Each class's concrete method bodies, in source order, become one code-only lexical token sequence (comments and strings masked; identifiers and numbers normalized; a boundary between methods), compared as multisets of ordered 4-token shingles. Match both token minima and configured shingle Jaccard similarity (default 6200 basis points). Python data-model special methods such as `__init__`, `__eq__`, `__hash__` and `__bool__` are protocol implementations and are left out of the method-name sets; library protocol hooks such as `__rich_console__` still count. Similar-sized classes whose token counts merely overlap no longer match. This is behavioral-shape evidence, not substitutability proof.

<a id="typescript-repeated-dispatch"></a>
### `typescript.repeated_dispatch@2` — built in
Group callables by the sorted authored text of their `case` labels (string labels keep their literal text; `default` is not a label). Each `switch`/`match` statement counts once, for its innermost enclosing function, and a function with several identical statements is one site. Match `minimum_sites` sharing at least `minimum_arms`; discriminated-union identity is not inferred.

<a id="typescript-temporary-fields"></a>
### `typescript.temporary_fields@2` — built in
Treat optional, nullable, or `undefined` fields as lifecycle candidates. A field is a name the class itself declares: a direct property declaration in the class body, a constructor parameter (including parameter properties), or a `this.name = ...` assignment. Members of inline object types, parameter lists and object literals nested inside a member are not class fields. Count direct `this.field` use across non-constructor class methods; `constructor` initializes fields and is not lifecycle use. Apply the configured field, method, and maximum-use thresholds.

<a id="typescript-forwarding-share"></a>
### `typescript.forwarding_share@1` — built in
Count non-constructor methods whose complete body is one optional `return` of `this.delegate.method(...)`. Match the configured method minimum and forwarding share.

<a id="typescript-function-crap"></a>
### `typescript.function_crap@3` — built in
Calculate cyclomatic complexity on the syntax tree and a conservative zero-coverage CRAP upper bound `C² + C`. Match scores greater than `warning_maximum` (default 5). A match blocks only when the rule is `required`, the score is greater than `maximum` (default 10), and it was measured from supplied LCOV coverage. Scores at most `maximum` and every zero-coverage upper bound are non-blocking warnings, and the explanation says which. Complete external coverage may replace this pessimistic observation. Decisions follow ESLint's `complexity` rule: `if`, ternaries, `for`, `for…in/of`, `while`, `do`, each `case` (not `default`), `catch`, `&&`, `||`, `??`, the logical assignments `&&=`, `||=`, `??=`, each optional chain `?.`, and each default value in a parameter or destructuring pattern. Arrow functions, function expressions and nested classes are their own scope; an expression-bodied arrow counts its body. Text in strings, comments and templates never counts.

<a id="typescript-unused-code"></a>
### `typescript.unused_code@1` — built in
Count single-underscore private function/class declarations with no second identifier occurrence in the captured corpus. Match findings greater than `maximum_findings`; exported and dynamic reachability are outside the built-in scope. An exported `_` declaration (`export function _union`) is public. Declarations under a conventional test path are grouped as `<path>::unused-test-declarations` and are a non-blocking indicator.

<a id="typescript-unused-type-parameters"></a>
### `typescript.unused_type_parameters@1` — built in
Count authored generic parameters declared in angle-bracket syntax but not referenced elsewhere in the declaration. Match findings greater than `maximum_findings`.

<a id="typescript-nominal-slot-contract"></a>
### `typescript.nominal_slot_contract@2` — built in
Count primitive, non-boolean class fields (not `interface` declarations, which describe shapes such as wire payloads) with common domain-role names such as IDs, email, phone, money, dates, addresses, and country/postcode values. A role matches the whole name or a `_`-separated suffix (`user_id`, `zip_code`); the ambiguous `zip` counts only as the whole name, so `search_zip` is not a postcode. Match mismatches greater than `maximum_mismatches`; a project contract may replace this naming heuristic.

<a id="typescript-port-conformance"></a>
### `typescript.port_conformance@2` — built in
For classes declaring an authored interface in their `extends`/`implements` heritage clause (not type-parameter constraints or defaults), compare direct method names with that interface and count missing methods. A port name resolves to a same-file declaration, else to the single declaration of that name; an ambiguous name is not resolved. Match failures greater than `maximum_failures`; compiler and behavioral-test evidence remain optional stronger overrides.

<a id="typescript-refused-bequest"></a>
### `typescript.refused_bequest@1` — built in
For same-corpus inheritance, count base methods and derived overrides that explicitly reject them by throwing an error. Apply the inherited-member and unused-share thresholds.

<a id="typescript-divergent-change"></a>
### `typescript.divergent_change@2` — built in
Inspect up to 200 commits and measure each current file's change count plus distinct co-change responsibilities. A responsibility is the first module below the owning implementation root, after a leading `src` or `lib` and a top-level Python package (a directory with `__init__.py`); a file directly in that root is its own responsibility, except that files directly in a flat directory (one with no production files below it) share that directory as one responsibility. Only production code counts: test code (`tests`/`test`/`__tests__`/`benches` directories, `conftest.py`, `test_*.py`, `*_test.py`, `.test.`/`.spec.` files), files under `docs`, `docs_src`, `examples`, `fixtures`, `__fixtures__`, `scripts`, `bench(mark)s`, `playground`, `e2e` or `*-tests`/`*_tests` directories, `*.config.*` files, `setup.py` and `noxfile.py` are never responsibilities or finding locations. A file whose first comment lines carry a generated marker (`@generated`, `do not edit`, `auto-generated`, `generated by`, ...) is never a finding location. Logical changes exclude the repository's initial commit (an import), mechanical sweeps, and reverts: a commit is a sweep when its subject starts with `bump`, `format` or `reformat`, has a conventional type of `build`, `chore`, `ci`, `doc`, `docs`, `fmt`, `format`, `lint`, `style`, `test` or `typo`, or names a formatter, linter, type checker, typing, whitespace or spelling sweep (`rustfmt`, `clippy`, `ruff`, `isort`, `mypy`, `prettier`, `typos`, `msrv`, `typing`, `whitespace`, `ws`, `__future__`, ...), or when it touches at least max(20, 10% of production files) production files. A `Revert "X"` commit never counts and the change it reverts counts only when it was later re-applied, so a revert pair counts once at most. A file whose only change is a move or rename with unchanged content, or whose only edit replaces string literals assigned to `version`, `VERSION` or `__version__`, is not changed. A commit subject naming type hints, type annotations, Python support or Python versions is a sweep. A file change whose only edit adds, removes or moves lint, formatter, type-checker or coverage suppressions (`eslint-disable`, `@ts-expect-error`, `noqa`, `type: ignore`, `pragma: no cover`, Rust `#[allow(...)]`/`#[expect(...)]` lines, ...) is not a change of that file, and a commit left with no other change does not count. Suppressions are read from the commit's zero-context patch without fetching missing objects; when the patch cannot be read, such as in a blobless partial clone, every changed file still counts. Commit subjects are matched only against these fixed structural words; no other commit prose or LLM inference is used. Apply both configured minima. Without Git history this rule is incomplete, never clean.

<a id="typescript-parallel-inheritance"></a>
### `typescript.parallel_inheritance@2` — built in
Compare inheritance families and pair child roles one-to-one across different bases. A role is the child name minus the base name's words, ignoring `Base`/`Sync`/`Async` markers; roles pair exactly first, then when one role's words begin the other's (`Catch` with `CatchClause`). Sync/async twin bases (`SyncByteStream`/`AsyncByteStream`) pair children by their marker-free names. Affix roles pair only between differently named bases that are authored or external without a stripped marker, so copies of one hierarchy and external bases such as `BaseModel` do not pair; a class inheriting both bases is not counted; `interface` declarations describe shapes, not subclass hierarchies, and are not family members. Match at least `minimum_parallel_pairs`; this detects parallel structure without claiming synchronized historical addition.

<a id="typescript-shotgun-surgery"></a>
### `typescript.shotgun_surgery@2` — built in
Inspect up to 200 commits and count distinct source responsibilities changed in each commit. A responsibility is the first module below the owning implementation root, after a leading `src` or `lib` and a top-level Python package (a directory with `__init__.py`); a file directly in that root is its own responsibility, except that files directly in a flat directory (one with no production files below it) share that directory as one responsibility. Only production code counts: test code (`tests`/`test`/`__tests__`/`benches` directories, `conftest.py`, `test_*.py`, `*_test.py`, `.test.`/`.spec.` files), files under `docs`, `docs_src`, `examples`, `fixtures`, `__fixtures__`, `scripts`, `bench(mark)s`, `playground`, `e2e` or `*-tests`/`*_tests` directories, `*.config.*` files, `setup.py` and `noxfile.py` are never responsibilities or finding locations. A file whose first comment lines carry a generated marker (`@generated`, `do not edit`, `auto-generated`, `generated by`, ...) is never a finding location. Logical changes exclude the repository's initial commit (an import), mechanical sweeps, and reverts: a commit is a sweep when its subject starts with `bump`, `format` or `reformat`, has a conventional type of `build`, `chore`, `ci`, `doc`, `docs`, `fmt`, `format`, `lint`, `style`, `test` or `typo`, or names a formatter, linter, type checker, typing, whitespace or spelling sweep (`rustfmt`, `clippy`, `ruff`, `isort`, `mypy`, `prettier`, `typos`, `msrv`, `typing`, `whitespace`, `ws`, `__future__`, ...), or when it touches at least max(20, 10% of production files) production files. A `Revert "X"` commit never counts and the change it reverts counts only when it was later re-applied, so a revert pair counts once at most. A file whose only change is a move or rename with unchanged content, or whose only edit replaces string literals assigned to `version`, `VERSION` or `__version__`, is not changed. A commit subject naming type hints, type annotations, Python support or Python versions is a sweep. A file change whose only edit adds, removes or moves lint, formatter, type-checker or coverage suppressions (`eslint-disable`, `@ts-expect-error`, `noqa`, `type: ignore`, `pragma: no cover`, Rust `#[allow(...)]`/`#[expect(...)]` lines, ...) is not a change of that file, and a commit left with no other change does not count. Suppressions are read from the commit's zero-context patch without fetching missing objects; when the patch cannot be read, such as in a blobless partial clone, every changed file still counts. Commit subjects are matched only against these fixed structural words; no other commit prose or LLM inference is used. Match owner count greater than `maximum_owners`. Without Git history this rule is incomplete, never clean.

<a id="typescript-foreign-accesses"></a>
### `typescript.foreign_accesses@3` — built in
Measure each method that has an owned receiver: count its dotted member accesses whose receiver is not `self`, `cls`, `this`, `Self`, or `super` against all of its dotted accesses, and apply the foreign count and exclusive-share thresholds. A local bound directly from owned state (`x = self.items`, `const def = this._def`) is owned, and a dotted chain of names inherits its root's ownership, so `this._def.innerType` reads owned state. The first parameter of a local closure is owned when every call of the closure in the method passes an argument from the same owned root (`removeFromMap(this.a); removeFromMap(this.b)`). Free functions, static methods, and methods without dotted accesses have no owned data to compare and are not observed. External type ownership can replace this lexical classification.

<a id="typescript-dependency-contract"></a>
### `typescript.dependency_contract@2` — built in
Count foreign accesses to underscore/private-style members. Dunder names such as the `__proto__` builtin and all-caps `__NAME` binding constants such as `__STATIC_CONTENT` are public by convention, not private members. Match forbidden accesses greater than `maximum_forbidden_accesses`; an explicit dependency contract may replace this visibility heuristic. An access inside a class to a member that the innermost enclosing class declares as a field or method, or uses on its own receiver, is the class's own (`other._key` in `__eq__`). ECMAScript `#private` members can only be reached from inside their own class and never count.

<a id="typescript-library-capabilities"></a>
### `typescript.library_capabilities@2` — built in
Count authored prototype or foreign attribute assignment workarounds (not `==`/`===` comparisons) as missing-capability signals. A non-prototype assignment counts only when the root of its target chain is an imported binding or a host global (`window`, `globalThis`, `global`, `process`, `navigator`); a local object such as `config.languageOptions.parser` is not a library class, a DOM write such as `document.head.innerHTML` and an environment write such as `process.env.NODE_ENV` are ordinary use, and a type in an annotation (`const x: ts.server.ServerHost =`, `| typeof fs.realpathSync.native =`) is not an assignment target. Match failures greater than `maximum_failures`; capability tests may replace the heuristic.

<a id="typescript-navigation-chains"></a>
### `typescript.navigation_chains@3` — built in
Measure the longest run of consecutive navigation hops in a contiguous dotted expression and match at least `minimum_transitions`. A hop is a member read or a zero-argument call; a call with any argument text, including nested parentheses or arguments continued on the following lines, starts a new run; standard conversions (`toString`, `valueOf`, `trim`, `keys`, `values`, `entries`, `length`, `slice`, and similar) are transparent; the first hop from `self`, `this`, or `Self` reads owned state and is not counted. Remaining fluent APIs may legitimately match; resolved domain-owner evidence may replace the lexical result.
