# Deterministic Python patterns — python-v1

This is the normative contract for `python-v1`. It maps the exact 23-item Refactoring.Guru catalog to 28 deterministic rules. Every rule has a built-in collector. Eighteen rules also accept a higher-fidelity per-rule override through the optional [provider-evidence contract](provider-evidence.md).

## Shared source contract

- The policy selects `python-v1` and `authored_source`. Full scans include `.py` and `.pyi`; staged scans read policy and source from regular Git-index blobs. Both modes also capture `Cargo.toml`, `pyproject.toml`, and `package.json` runtime markers so findings are grouped under the nearest repository implementation before the repository rollup.
- The locked Tree-sitter Python grammar parses every captured file. Any syntax error makes the scan incomplete. No import execution, module loading, type checker, test runner, or application code is run.
- A callable is a `function_definition` or `lambda`, including a method and an async definition represented by the function grammar node. Parameters are the named children of `parameters`; `self`, `cls`, defaulted, typed, positional-only, keyword-only, `*args`, and `**kwargs` each count once when represented as one parameter node.
- Body code lines are distinct zero-based syntax rows occupied by non-comment named leaves inside the body, reported as a count. Blank lines, comment-only rows, and delimiter-only rows are excluded; multiline syntax occupies every row spanned by its leaf.
- A class is a `class_definition`. Its field set is the union of direct class-body assignment/annotation targets and unique `self.x`/`cls.x` assignment targets in direct methods, except dunder class attributes such as `__slots__` and properties with an `@name.setter`, whose assignments run the setter. Its method set is direct functions, including decorator-wrapped functions. Nested classes/functions do not add members to an enclosing class. Methods and fields are source-owned syntax, not inferred runtime monkey-patches, descriptors, dataclass transforms, or inherited members.
- Ratios use integer cross-products; findings and inputs are sorted; resource-budget exhaustion errors. A structural match is evidence for review, not proof of a semantic defect.
- Every matched finding includes source location/excerpt, observed value, match operator, threshold, evidence, smell identity, remediation guidance, and a mandatory Refactoring.Guru research URL. Before reviewing or remediating a finding, the agent must make an external research/tool call to that exact URL and read the page; if it cannot, it reports the research as incomplete and stops.

## Implemented source rules

<a id="python-function-lines"></a>
### `python.function_lines@2`

Match when authored body code lines are greater than `maximum` (default 100). The maximum is inclusive.

An exact zero-argument Alembic `upgrade` or `downgrade` Declaration Role in a high-confidence `database_migration` Source Role retains the matched Measurement and Finding but is reported as a nonblocking `reported_by_role` review signal. Inferred roles require Alembic import and revision-graph evidence; an explicit anchored Alembic role supplies the file-level framework evidence but still requires revision metadata and the canonical signature. Helpers and same-named functions without that evidence use normal blocking semantics.

<a id="python-function-arguments"></a>
### `python.function_arguments@2`

Match when declared parameter nodes are greater than `maximum` (default 7). A method receiver is a declared parameter and counts once. Keyword-only parameters that have a default (after `*` or `*args`) are not counted: callers must name them and may omit them, so they do not lengthen a call. Keyword-only parameters without a default still count. Comments between parameters are not parameters.

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
### `python.data_clumps@2`

For every callable, form the sorted unique set of `(parameter name, compact authored type syntax)`, excluding `self` and `cls`. Enumerate every subset of exactly `minimum_group_size` (default 3) and keep those supported by at least `minimum_declarations` distinct callables (default 3). Report one finding per closed group: the slots shared by every supporter of a qualifying subset, so a wider shared population yields one maximal group rather than one finding per subset. Untyped parameters have an empty type string. Exhausting `maximum_group_combinations` errors.

<a id="python-comment-share"></a>
### `python.comment_share@1`

Count ordinary Tree-sitter comment rows in the function minus rows also occupied by code. Match when body code lines are at least `minimum_code_lines` (default 20) and `comment_lines / (comment_lines + code_lines)` is at least `minimum_share_percent` (default 30). Documentation strings are code strings, not comments. Sphinx `#:` attribute doc comments are documentation, and tool directives (`noqa`, `type: ignore`, `pragma: no cover`, `fmt: off`, and the other lint, type-checker and coverage suppressions the history rules recognise) are not ordinary comments.

<a id="python-duplicate-functions"></a>
### `python.duplicate_functions@1`

Normalize Tree-sitter leaf tokens in each body: identifier spellings become `identifier`, literal values become literal-kind tags, comments are removed, and keywords/operators/delimiters retain grammar kinds. Build a multiset of consecutive four-token windows. Both bodies must have at least `minimum_tokens` (default 20); multiset Jaccard must be at least `minimum_similarity_basis_points` (default 8200). Overlapping nested bodies in one file are not compared. Expand multiset occurrences into globally document-frequency-ordered features, then apply exact Jaccard prefix, size-ratio, and positional-overlap filters. These are necessary conditions, so they cannot remove a threshold-matching pair. `maximum_pairs` counts the remaining unique pairs whose full multiset intersection/union is evaluated; exhaustion errors rather than returning a truncated success. Passing pairs join into connected families, and each family is one finding (see `docs/report-interface.md`).

<a id="python-data-class"></a>
### `python.data_class@1`

Match when fields are at least `minimum_fields` (default 2) and non-accessor operations are at most `maximum_operations` (default 0). `__init__` is excluded. A strict accessor is one statement that either returns exactly `self.x`/`cls.x`, or assigns its only ordinary identifier parameter directly to such a field. A subclass of a standard-library enumeration base (`Enum`, `IntEnum`, `StrEnum`, `Flag`, `IntFlag`, `ReprEnum`) holds enumeration values, not fields, and is not observed. Decorator semantics, generated dataclass behavior, and inheritance are not inferred.

<a id="python-lazy-class"></a>
### `python.lazy_class@1`

Match only when field count, method count, and summed method lines are all at most their configured maxima (defaults 1, 1, and 5). Small nominal, schema, or transport classes can legitimately match.

## Built-in full-scan collectors and optional overrides

The rules below run without `--evidence`. Their built-in observations are conservative authored-source or bounded-Git-history indicators and identify their exact collector in each finding. A supplied external entry replaces the built-in observations for that rule, using the same measurement map and scanner-owned threshold evaluation.

<a id="python-primitive-slots"></a>
### `python.primitive_slots@1` — built in
For each class with annotated fields, count exact authored `str`, `int`, `float`, `bool`, and `bytes` types. Match `minimum_raw_slots` and `minimum_share_percent`; aliases and containers are not unwrapped.

<a id="python-alternative-interfaces"></a>
### `python.alternative_interfaces@2` — built in
Compare distinct classes with different effective method-name sets. A class's effective set is its own methods plus those it inherits from authored bases, resolved as the declared-base rules resolve them, so subclasses that override different subsets of one authored base expose that base's one interface. Each class's concrete method bodies, in source order, become one code-only lexical token sequence (comments and strings masked; identifiers and numbers normalized; a boundary between methods), compared as multisets of ordered 4-token shingles. Match both token minima and configured shingle Jaccard similarity (default 6200 basis points). Python data-model special methods such as `__init__`, `__eq__`, `__hash__` and `__bool__` are protocol implementations and are left out of the method-name sets; library protocol hooks such as `__rich_console__` still count. Similar-sized classes whose token counts merely overlap no longer match. This is behavioral-shape evidence, not substitutability proof.

<a id="python-repeated-dispatch"></a>
### `python.repeated_dispatch@2` — built in
Group callables by the sorted authored text of their `case` labels (string labels keep their literal text; a `case` with several patterns is one label). Each `switch`/`match` statement counts once, for its innermost enclosing function, and a function with several identical statements is one site. Match `minimum_sites` sharing at least `minimum_arms`; type identity is not inferred.

<a id="python-temporary-fields"></a>
### `python.temporary_fields@1` — built in
Treat fields annotated with `Optional`/`None` or assigned `None` as lifecycle candidates and count direct `self.field` use across non-constructor class methods; `__init__` initializes fields and is not lifecycle use. Apply the configured field, method, and maximum-use thresholds.

<a id="python-forwarding-share"></a>
### `python.forwarding_share@1` — built in
Count non-constructor methods whose complete body is one optional `return` of `self.delegate.method(...)`. Match the configured method minimum and forwarding share.

<a id="python-function-crap"></a>
### `python.function_crap@3` — built in
Calculate cyclomatic complexity on the syntax tree and a conservative zero-coverage CRAP upper bound `C² + C`. Match scores greater than `warning_maximum` (default 5). A match blocks only when the rule is `required`, the score is greater than `maximum` (default 10), and it was measured from supplied LCOV coverage. Scores at most `maximum` and every zero-coverage upper bound are non-blocking warnings, and the explanation says which. Complete external coverage may replace this pessimistic observation. Decisions follow radon: `if`, `elif`, conditional expressions, `for`, `while`, an `else` on a loop or `try`, `except`, `assert`, comprehension `for`/`if`, `and`, `or`, and each `match` `case` except a wildcard or bare capture. A `case` guard and `finally` are not decisions. Unlike radon, decisions inside an `assert` count, and lambdas, like nested functions and classes, are their own scope. Text in strings and comments never counts.

<a id="python-unused-code"></a>
### `python.unused_code@2` — built in
Count single-underscore private function/class declarations with no second identifier occurrence in the captured corpus. Match findings greater than `maximum_findings`; public and dynamic reachability are outside the built-in scope. Single-underscore protocol hooks such as `_missing_` or `_repr_markdown_` are called by a framework and are not private. Functions registered by a decorator that a framework invokes without naming them are reachable: a decorator whose last name ends in `validator` or `serializer` (pydantic), `validates`, `validates_schema`, `pre_load`, `post_load`, `pre_dump`, or `post_dump` (marshmallow, SQLAlchemy), and a pytest `fixture` called with `name=` or with `autouse` not `False`. Other decorators, including a bare `@pytest.fixture`, do not make a declaration reachable. Declarations under a conventional test path are grouped as `<path>::unused-test-declarations` and are a non-blocking indicator.

<a id="python-unused-type-parameters"></a>
### `python.unused_type_parameters@1` — built in
Count authored generic parameters declared in bracket syntax but not referenced elsewhere in the declaration. Match findings greater than `maximum_findings`.

<a id="python-nominal-slot-contract"></a>
### `python.nominal_slot_contract@2` — built in
Count primitive, non-boolean class fields with common domain-role names such as IDs, email, phone, money, dates, addresses, and country/postcode values. A role matches the whole name or a `_`-separated suffix (`user_id`, `zip_code`); the ambiguous `zip` counts only as the whole name, so `search_zip` is not a postcode. Match mismatches greater than `maximum_mismatches`; a project contract may replace this naming heuristic.

<a id="python-port-conformance"></a>
### `python.port_conformance@2` — built in
For classes inheriting an authored `Protocol`/`ABC`, compare direct method names with the port's required methods and count missing methods. A `Protocol` requires every declared method; an `ABC` requires only methods decorated with `abstractmethod` (or another `abc.abstract*` decorator), because concrete methods are inherited. Match failures greater than `maximum_failures`; behavioral tests remain an optional stronger override.

<a id="python-refused-bequest"></a>
### `python.refused_bequest@1` — built in
For same-corpus inheritance, count base methods and derived overrides that explicitly reject them using `pass`, `NotImplemented`, or `NotImplementedError`. Apply the inherited-member and unused-share thresholds.

<a id="python-divergent-change"></a>
### `python.divergent_change@2` — built in
Inspect up to 200 commits and measure each current file's change count plus distinct co-change responsibilities. A responsibility is the first module below the owning implementation root, after a leading `src` or `lib` and a top-level Python package (a directory with `__init__.py`); a file directly in that root is its own responsibility, except that files directly in a flat directory (one with no production files below it) share that directory as one responsibility. Only production code counts: test code (`tests`/`test`/`__tests__`/`benches` directories, `conftest.py`, `test_*.py`, `*_test.py`, `.test.`/`.spec.` files), files under `docs`, `docs_src`, `examples`, `fixtures`, `__fixtures__`, `scripts`, `bench(mark)s`, `playground`, `e2e` or `*-tests`/`*_tests` directories, `*.config.*` files, `setup.py` and `noxfile.py` are never responsibilities or finding locations. A file whose first comment lines carry a generated marker (`@generated`, `do not edit`, `auto-generated`, `generated by`, ...) is never a finding location. Logical changes exclude the repository's initial commit (an import), mechanical sweeps, and reverts: a commit is a sweep when its subject starts with `bump`, `format` or `reformat`, has a conventional type of `build`, `chore`, `ci`, `doc`, `docs`, `fmt`, `format`, `lint`, `style`, `test` or `typo`, or names a formatter, linter, type checker, typing, whitespace or spelling sweep (`rustfmt`, `clippy`, `ruff`, `isort`, `mypy`, `prettier`, `typos`, `msrv`, `typing`, `whitespace`, `ws`, `__future__`, ...), or when it touches at least max(20, 10% of production files) production files. A `Revert "X"` commit never counts and the change it reverts counts only when it was later re-applied, so a revert pair counts once at most. A file whose only change is a move or rename with unchanged content, or whose only edit replaces string literals assigned to `version`, `VERSION` or `__version__`, is not changed. A commit subject naming type hints, type annotations, Python support or Python versions is a sweep. A file change whose only edit adds, removes or moves lint, formatter, type-checker or coverage suppressions (`eslint-disable`, `@ts-expect-error`, `noqa`, `type: ignore`, `pragma: no cover`, Rust `#[allow(...)]`/`#[expect(...)]` lines, ...) is not a change of that file, and a commit left with no other change does not count. Suppressions are read from the commit's zero-context patch without fetching missing objects; when the patch cannot be read, such as in a blobless partial clone, every changed file still counts. Commit subjects are matched only against these fixed structural words; no other commit prose or LLM inference is used. Apply both configured minima. Without Git history this rule is incomplete, never clean.

<a id="python-parallel-inheritance"></a>
### `python.parallel_inheritance@2` — built in
Compare inheritance families and pair child roles one-to-one across different bases. A role is the child name minus the base name's words, ignoring `Base`/`Sync`/`Async` markers; roles pair exactly first, then when one role's words begin the other's (`Catch` with `CatchClause`). Sync/async twin bases (`SyncByteStream`/`AsyncByteStream`) pair children by their marker-free names. Affix roles pair only between differently named bases that are authored or external without a stripped marker, so copies of one hierarchy and external bases such as `BaseModel` do not pair; a class inheriting both bases is not counted. Match at least `minimum_parallel_pairs`; this detects parallel structure without claiming synchronized historical addition.

<a id="python-shotgun-surgery"></a>
### `python.shotgun_surgery@2` — built in
Inspect up to 200 commits and count distinct source responsibilities changed in each commit. A responsibility is the first module below the owning implementation root, after a leading `src` or `lib` and a top-level Python package (a directory with `__init__.py`); a file directly in that root is its own responsibility, except that files directly in a flat directory (one with no production files below it) share that directory as one responsibility. Only production code counts: test code (`tests`/`test`/`__tests__`/`benches` directories, `conftest.py`, `test_*.py`, `*_test.py`, `.test.`/`.spec.` files), files under `docs`, `docs_src`, `examples`, `fixtures`, `__fixtures__`, `scripts`, `bench(mark)s`, `playground`, `e2e` or `*-tests`/`*_tests` directories, `*.config.*` files, `setup.py` and `noxfile.py` are never responsibilities or finding locations. A file whose first comment lines carry a generated marker (`@generated`, `do not edit`, `auto-generated`, `generated by`, ...) is never a finding location. Logical changes exclude the repository's initial commit (an import), mechanical sweeps, and reverts: a commit is a sweep when its subject starts with `bump`, `format` or `reformat`, has a conventional type of `build`, `chore`, `ci`, `doc`, `docs`, `fmt`, `format`, `lint`, `style`, `test` or `typo`, or names a formatter, linter, type checker, typing, whitespace or spelling sweep (`rustfmt`, `clippy`, `ruff`, `isort`, `mypy`, `prettier`, `typos`, `msrv`, `typing`, `whitespace`, `ws`, `__future__`, ...), or when it touches at least max(20, 10% of production files) production files. A `Revert "X"` commit never counts and the change it reverts counts only when it was later re-applied, so a revert pair counts once at most. A file whose only change is a move or rename with unchanged content, or whose only edit replaces string literals assigned to `version`, `VERSION` or `__version__`, is not changed. A commit subject naming type hints, type annotations, Python support or Python versions is a sweep. A file change whose only edit adds, removes or moves lint, formatter, type-checker or coverage suppressions (`eslint-disable`, `@ts-expect-error`, `noqa`, `type: ignore`, `pragma: no cover`, Rust `#[allow(...)]`/`#[expect(...)]` lines, ...) is not a change of that file, and a commit left with no other change does not count. Suppressions are read from the commit's zero-context patch without fetching missing objects; when the patch cannot be read, such as in a blobless partial clone, every changed file still counts. Commit subjects are matched only against these fixed structural words; no other commit prose or LLM inference is used. Match owner count greater than `maximum_owners`. Without Git history this rule is incomplete, never clean.

<a id="python-foreign-accesses"></a>
### `python.foreign_accesses@3` — built in
Measure each method that has an owned receiver: count its dotted member accesses whose receiver is not `self`, `cls`, `this`, `Self`, or `super` against all of its dotted accesses, and apply the foreign count and exclusive-share thresholds. A local bound directly from owned state (`x = self.items`, `const def = this._def`) is owned, and a dotted chain of names inherits its root's ownership, so `this._def.innerType` reads owned state. Statement annotations such as `values: dict[str, t.Any] = {}` name types and are not accesses. Free functions, static methods, and methods without dotted accesses have no owned data to compare and are not observed. External type ownership can replace this lexical classification.

<a id="python-dependency-contract"></a>
### `python.dependency_contract@2` — built in
Count foreign accesses to underscore/private-style members. Dunder names that begin and end with double underscores (`__html__`, `__version__`) and sunder names that begin and end with one underscore (the `Enum` members `_value_` and `_name_`) are public protocol names, not private members. Match forbidden accesses greater than `maximum_forbidden_accesses`; an explicit dependency contract may replace this visibility heuristic. An access inside a class to a member that the innermost enclosing class declares as a field or method, or uses on its own receiver, is the class's own (`other._key` in `__eq__`).

<a id="python-library-capabilities"></a>
### `python.library_capabilities@2` — built in
Count authored foreign/prototype attribute assignments (not `==` comparisons) and `setattr` workarounds as missing-capability signals. `setattr(self, ...)` and `setattr(cls, ...)` target the object's own receiver and are not counted. A dotted assignment whose final attribute an authored class declares as assignable, by an annotated class field or an `@name.setter` property, uses that authored API and is not counted. A mutation of an authored class object or its methods (`setattr(Flask, ...)`, `Thing.__doc__ = ...`, `Foo.__rich_repr__.angular = True`) changes project code, not a library, and is not counted; a field reached through the receiver, such as `self.session.headers`, still counts. Match failures greater than `maximum_failures`; capability tests may replace the heuristic.

<a id="python-navigation-chains"></a>
### `python.navigation_chains@3` — built in
Measure the longest run of consecutive navigation hops in a contiguous dotted expression and match at least `minimum_transitions`. A hop is a member read or a zero-argument call; a call with any argument text, including nested parentheses or arguments continued on the following lines, starts a new run; standard conversions (`items`, `keys`, `values`, `copy`, `strip`, `lower`, `upper`, `encode`, `decode`, `split`, and similar) are transparent; the first hop from `self`, `this`, or `Self` reads owned state and is not counted. Remaining fluent APIs may legitimately match; resolved domain-owner evidence may replace the lexical result.
