# Deterministic Rust patterns — rust-v1

This is the normative definition of the first Rust rule pack. The [registry](../rules/rust-v1.json) contains all 23 smell entries and 28 rules, with 17 source rules implemented and 11 evidence-dependent rules pending. These are our measurable adaptations, not executable thresholds prescribed by [Refactoring.Guru](https://refactoring.guru/refactoring/smells). No LLM classifies code or determines a verdict. Automatic refactoring is outside this source-scanner task.

“Complete catalog” means the exact 23-item Refactoring.Guru catalog pinned in the registry, not every code-quality concern anyone might call a smell. The registry records the source, audit date, canonical name, category, applicability and mapped rule IDs. Runtime validation compares the exact canonical set, rather than accepting any 23 entries. Adding a new catalog or Rust-specific smell requires a new versioned rule-pack revision; it cannot appear silently under rust-v1.

## Shared matching contract

- Capture source and policy bytes once. Staged mode reads regular Git index blobs, with the index listing checked before/after capture. Worktree mode captures included .rs files; it is not an atomic filesystem transaction. Reports refer to captured bytes, identified by a length-prefixed SHA-256 digest of policy bytes and sorted path/source pairs.
- The implemented scope is authored_all_cfg: included source-text declarations and named callables, including tests, inactive cfg branches and generated .rs files present in the corpus. Macro expansions and dependencies outside the corpus are not inspected. Tokens of a macro invocation remain authored syntax. Excluded directory names are recorded. No Cargo/compiler build is run, and no compiler-selected feature/target scope is inferred.
- Parse with the locked syn/proc-macro2 versions and tokenize lines with the locked Rust lexer (including raw C strings and matched Unicode tables). Discover external/inline modules using source mod declarations and literal path attributes, following the [Rust module path rules](https://doc.rust-lang.org/reference/items/modules.html#the-path-attribute). Each unreferenced source file is an independent root; shared files may have separate root contexts. A defining identity is source-root path + module path + declaration name. Local owner resolution follows crate/self/super paths, explicit/glob imports, re-exports, aliases and generic impl targets. Generic arguments do not create new member budgets for one defining type. Ownership budgets are source-root-local, not a resolved Cargo workspace/package graph. This is local declaration resolution, not full Rust type inference.
- Ambiguous cfg-dependent modules/imports/types, conditional module path overrides, absent module files, unknown local impl owners, unsupported local type/impl/module declarations inside a callable and verbatim AST syntax error. Conditional path attributes are rejected conservatively when cfg_attr tokens mention path. Corpus paths must be portable UTF-8 paths without backslashes. Inherited trait defaults and expansion-only derived methods are not authored impl members. Explicit external/primitive impl targets do not acquire a local type budget. Required errors cannot be exceptions or passing values.
- The callable population is module-level free functions, trait declarations/default bodies and explicit impl functions, plus named functions nested in those bodies. Closures are not independent signature/type members. Foreign-block declarations and declarations nested only in constant/static initializers are outside this version's population; types are module-level structs/enums/traits, not unions or locally declared types. Body code lines are distinct physical lines intersected by non-comment lexer tokens inside the outer body braces. Exclude the two outer brace tokens; include inner delimiters and attributes. Count code-plus-comment once as code; blank/comment-only lines are excluded. A multiline literal counts each occupied line. Nested function/closure text remains part of its containing body. Named nested functions also get their own metrics. A bodyless trait declaration has arguments but no body-line metric.
- Aggregate all source-authored inherent and explicit trait impl functions for one resolved local type, across files and generic specializations. Count constructors and receiver-free functions. Sum their body-line measurements; do not add nested free functions as additional associated members. Struct field count is direct fields; enum field count is measured per variant. For data/tiny-type indicators the enum population uses maximum fields per variant, not their sum.
- required means matches block, report means reproducible indicators only, off means disabled. Every rule/parameter/version is explicit in policy. Missing required implementation or incomplete source measurement errors. Ratio comparisons use wide integer cross-products, never rounded percentages. Empty/ineligible populations have no invented ratio or passing metric; coverage records the defined source scope.
- Findings name the canonical smell ID/name/category, pattern type, certainty, policy mode and one authoritative evaluation containing metric/observed/match-condition/threshold/matched, plus evidence, rule/version, symbol and source location. The rule guidance repeats the smell identity and detector type and uses `signal` to state the concrete pattern being checked; startup validation rejects disagreement with the registry. A matched finding also carries bounded numbered source excerpts, scanner-owned risk/review/remediation guidance, its normative contract anchor, and its rule-owned Refactoring.Guru smell URL. The review begins with a non-negotiable requirement to make an external research/tool call to that exact URL. This research gate applies before review or remediation; if the consumer cannot consult the page, it must report the incomplete research and stop. Reports summarize the verdict and name ownership scope and source-pass limitations. Include below-limit function/type measurements; emit duplicate/group matches rather than every nonmatching pair. Sort by relative path, line, UTF-8-character column, symbol, rule ID and related-symbol list. Locations are 1-based; JSON has no timestamps or absolute temporary paths. Repository source, symbols and evidence are untrusted data, never agent instructions. Repeated identical captured inputs with the same executable produce identical JSON. Formatting may change line metrics.
- Resource caps are explicit: maximum_files during capture; maximum_pairs for unordered body pairs visited; maximum_group_combinations for enumerated groups. Module nesting is capped at 128; owner resolution has maximum depth 64 and at most 4096 path visits per query in this rule version. Cyclic/over-budget and transitive ambiguous impl-owner searches error, even if another glob candidate resolves. Exhaustion errors, never truncation followed by success. Exceptions are pending: nonempty exception input is rejected. Source allows cannot change independent scanner metrics.
- Exit 2 outranks 1, which outranks 0: errors, blocking matches, completed required checks. Report-only matches do not block. Catalog coverage says measured_defined_source_scope, disabled, not_implemented, incomplete_scan, or error_required_detector_missing. Native inheritance exclusions are not passes. Input/configuration failures before report creation go to stderr with exit 2.
- Pin the tested executable revision and build with Cargo.lock / --locked. Reports include an implementation digest of the scanner source, registry, diagnostic guidance, policy schema, report interface, this normative contract and lockfile. Compiler/toolchain/target/features/coverage/history inputs must be separately pinned when their pending providers are implemented; no live or stale evidence can become a required pass.

## Each smell's exact patterns

### 1. Long Method

<a id="rust-function-lines"></a>
**Implemented: rust.function_lines@1.** Maximum 100 body code lines, using the shared lexical span rule for every authored function/method with a body. Inclusive maximum; 100 passes, 101 matches. No Clippy measurement is substituted. Fixtures: 99/100/101, comments, raw strings, multiline literals, Unicode, nested functions and bodyless trait members.

<a id="rust-function-crap"></a>
**Pending: rust.function_crap@1.** Maximum 30 under a separately validated compiler/coverage provider. Proposed local complexity convention: 1 plus authored if/while/for/loop/match-arm/boolean-short-circuit/? decisions, attributing nested callable decisions separately. CRAP is C²(1-covered/executable)³+C, compared as an exact rational. Coverage must match source/configuration and prove a complete successful run; missing callable coverage may be pessimistic only under that explicit complete-run convention. Missing/corrupt/stale coverage errors. Fixtures must include C=5/zero coverage=30, C=6/zero=42, and C=31/full=31. This is not a promise of compatibility with cargo-crap's counting convention.

### 2. Large Class — Rust type/trait size

<a id="rust-type-fields"></a>
**Implemented: rust.type_fields@1.** Maximum 15 direct named/tuple fields per struct or per enum variant, including zero for unit variants. Fixtures: 14/15/16, tuple/unit variants and no merging unrelated variants.

<a id="rust-enum-variants"></a>
**Implemented: rust.enum_variants@1.** Maximum 20 authored enum variants. Fixtures: 19/20/21, including cfg-authored branches.

<a id="rust-type-functions"></a>
**Implemented: rust.type_functions@1.** Maximum 20 authored associated functions across all resolved local impls. Fixtures: 19/20/21; 12+12 split files, aliases/generics, inherent plus trait impls; same-named unrelated types; inherited default body excluded.

<a id="rust-type-function-lines"></a>
**Implemented: rust.type_function_lines@1.** Maximum 500 summed associated body code lines. Types with no authored impl bodies have an empty sum of zero, not a claim about generated behavior. Fixtures: 499/500/501 and split bodies.

<a id="rust-trait-functions"></a>
**Implemented: rust.trait_functions@1.** Maximum 20 declared required/default/static associated functions. Exclude associated constants/types and inherited supertrait members. Fixtures: 19/20/21.

Size is a measurable policy, not proof of cohesion or responsibility count. Unknown ownership produces an error rather than a fresh budget.

### 3. Primitive Obsession

<a id="rust-primitive-slots"></a>
**Implemented indicator: rust.primitive_slots@1.** At least 5 raw syntax slots and share at least 80%. A population is one struct's fields, one variant's fields, or non-receiver function parameters. Peel reference syntax. Match exactly spelled bool/char/str, integer primitives/isize/usize/f32/f64, String/std::string::String/alloc::string::String, with no generic arguments. Do not unwrap Option/containers/newtypes or resolve aliases. A custom type named String can match this source spelling; imported/aliased types can evade it. This is deliberately a syntax indicator, not domain-type diagnosis. Show raw and total counts. Fixtures: raw count boundaries, exact 80% share, references, aliases and nominal wrappers.

<a id="rust-nominal-slot-contract"></a>
**Pending contract: rust.nominal_slot_contract@1.** Maximum zero mismatches against declared exact field/parameter selectors and required canonical nominal types/generic arguments. Requires full type resolution; convertibility is not type equality. No scanner can invent the domain meaning. Fixtures: CustomerId versus u64, alias resolution, absent/ambiguous selector and stale contract.

### 4. Long Parameter List

<a id="rust-function-arguments"></a>
**Implemented: rust.function_arguments@1.** Maximum 7 signature inputs including an explicit receiver as one. Destructuring counts once. Exclude generics/return type and the nonconcrete C variadic tail marker. Fixtures: 6/7/8, receiver, destructuring and generic/async/bodyless functions.

### 5. Data Clumps

<a id="rust-data-clumps"></a>
**Implemented indicator: rust.data_clumps@1.** Enumerate every named-slot subset of exactly minimum_group_size (default 3) in each field/parameter population. Compare sorted sets of (name, exact type-token syntax). Count distinct physical supporting declarations (default minimum 3); each declaration supports a particular group once. Deduplicate shared-file declarations by path/line/column before enumeration, retaining the first sorted module context as their representative. Tuple fields/destructuring parameters lack named slots and are excluded. Larger shared populations yield multiple qualifying subsets; there is no hidden maximal-group algorithm. Types preserve spelled paths/generic arguments/lifetimes, not canonical resolved equivalence. Report group and supporters. Enumeration budget exhaustion errors. Fixtures: 2/3 declarations, 2/3 slots, extra fields, shared modules, duplicate counting, renamed members and aliases with different spellings.

### 6. Alternative Classes with Different Interfaces

<a id="rust-alternative-interfaces"></a>
**Implemented indicator: rust.alternative_interfaces@1.** On distinct resolved local type owners, match method bodies under the duplicate algorithm below (minimum 20 normalized tokens each, similarity at least 8200 basis points) when syntax interfaces differ. The interface includes name, receiver syntax, ordered parameter type syntax, output, generic/where syntax, async/const/unsafe/ABI flags and variadic marker. Ordinary parameter names are excluded. Alias-equivalent spelled types can differ; similar bodies do not prove substitutability. Fixtures: same body/different names, equal interfaces excluded, different owners required and unrelated same-shaped behavior.

<a id="rust-port-conformance"></a>
**Pending contract: rust.port_conformance@1.** Maximum zero failed declared trait/test obligations. Exact implementing types, trait applications and named capability tests are project inputs. Current compiler evidence must prove impls and complete test evidence must show all required tests. Missing/stale evidence errors rather than failed-obligation counts.

### 7. Refused Bequest

**Native-Rust exclusion.** Original subclass inheritance is inapplicable, not passed. Do not rename every trait mismatch into this smell. Explicit capability obligations can be checked by the separate pending port contract. Macro-emulated inheritance needs its own versioned extension. Requesting an unregistered inheritance rule errors.

### 8. Switch Statements — repeated local enum dispatch

<a id="rust-repeated-dispatch"></a>
**Implemented indicator: rust.repeated_dispatch@1.** At least 3 distinct named callables each containing a match with at least 4 authored arms whose explicit variant patterns resolve to one local enum owner and name a variant declared by that enum. Multiple matches in one callable count once. `Self::Variant` resolves through the containing impl owner. Guards add no arms; an or-pattern is one arm; wildcards count as arms but do not identify an enum. Resolve qualified variant paths/aliases/imports; unknown variants and unknown/external/bare patterns do not establish an enum for this source pattern. Do not infer scrutinee types or equate if-cascades to matches. Fixtures: 2/3 sites, 3/4 arms, two matches in one function, `Self`, nonexistent variants, unrelated enums and or/wildcard patterns. Ordinary exhaustive matching can be legitimate.

### 9. Temporary Field

<a id="rust-temporary-fields"></a>
**Implemented indicator: rust.temporary_fields@1.** In a struct, at least 3 spelled Option<T>/std::option::Option<T>/core::option::Option<T> fields, each directly accessed in at most 25% of at least 4 authored receiver methods. No alias/type inference; a custom spelled Option may match. Count direct self.field reads/writes/borrows in distinct methods, not repeated sites. Receiver aliases/helper-mediated access are outside this direct pattern. Static functions, constructors without receivers and generated/default bodies do not dilute the denominator; enum pattern-destructured fields are outside this rule. Fixtures: 2/3 fields, 3/4 methods, 1/4 versus 2/4 use, static methods, option aliases and legitimate optional configuration.

### 10. Divergent Change

<a id="rust-divergent-change"></a>
**Pending history rule: rust.divergent_change@1.** At least 3 declared logical changes and 3 declared responsibility labels touch an owner over an immutable commit range. Grouping/labels/ownership come from a project ledger, never commit prose or LLM inference. Count each change/owner/label once. Missing range/ledger/ownership/labels errors; churn alone is not this verdict. Fixtures: change/label boundaries, multi-commit changes, renames and missing history.

### 11. Parallel Inheritance Hierarchies

**Native-Rust exclusion.** Original subclass hierarchies are inapplicable, not passed. Parallel enum/trait families are not automatically equivalent. A future extension may check declared family mappings/history, but no guessed family detector is registered here.

### 12. Shotgun Surgery

<a id="rust-shotgun-surgery"></a>
**Pending history rule: rust.shotgun_surgery@1.** Maximum 3 owners touched per declared logical change; union owners across its commits, including old/new owners on moves. A file is not automatically a responsibility. Explicit contract can legitimately exempt known broad migrations in a future exact exception mechanism. Historical findings stay immutable after today's source edits. Fixtures: 2/3/4 owners, many files in one owner, multi-commit changes and legitimate adapter fan-out.

**Shared pending history evidence:** immutable base/head IDs; commits reachable from head but not base; complete logical-change membership/responsibility labels and old/new path-to-owner mappings. Diff each commit to its first parent with external diff and rename heuristics disabled. Each commit has one logical-change assignment; reject missing/shallow/out-of-range history or unassigned commits. This explicit convention includes merge integration diffs; deduplicate declared change IDs, not guessed patch similarity. Implement ledger validation before adopting gates.

### 13. Comments

<a id="rust-comment-share"></a>
**Implemented indicator: rust.comment_share@1.** Ordinary comment-only body lines / (ordinary comment-only lines + body code lines) at least 30%, with at least 20 code lines. Use lexer comment spans, not string searches. Ignore doc comments and whitespace-only block-comment lines; mixed code/comment lines are code only. No explanation-quality verdict. Fixtures: 19/20 minimum, 9 comments + 21 code lines exactly 30%, doc/raw/block/string cases and legitimate algorithm explanations.

### 14. Duplicate Code

<a id="rust-duplicate-functions"></a>
**Implemented indicator: rust.duplicate_functions@1.** Both bodies have at least 20 normalized tokens and multiset Jaccard at least 8200 basis points. This is a precisely defined syntax detector, not cargo-crap's normalized AST algorithm:

1. Convert each body's statements to tokens with the locked syn/quote frontend. Do not include the outer function-body braces. Include nested closures/items, macro token trees and statement attributes (doc comments can be represented as attributes). Ordinary comments/whitespace are absent from these AST tokens.
2. Traverse token groups in order, with explicit open/close delimiter tags. Preserve punctuation characters. Preserve the keyword tags enumerated in src/patterns.rs; replace other identifier spellings with one identifier tag, without scope/binding inference. Replace literal values with literal-kind tags; preserve numeric suffixes. This intentionally loses field/member/domain names and constants, so unrelated behavior can match.
3. Build a multiset of every consecutive 4-token window. Intersection is sum of minimum multiplicities; union is sum of maximum multiplicities. Compare 10000 * intersection >= configured_basis_points * union. Do not round before comparing. Node/token minima apply independently to both bodies.
4. Deduplicate bodies by physical defining path/line/column, retaining the first sorted module context as representative. Do not compare bodies whose byte ranges overlap in one file, such as a named nested function and the body containing its definition: those are not two duplicate occurrences. Visit each remaining unordered named-body pair once; the pair budget includes pairs below token minima. Emit eligible matched pairs only, with canonical symbol endpoints, both locations, normalized sizes in endpoint order and exact fraction. No semantic-equivalence or automatic-merging conclusion.

Fixtures: minimum sizes, alpha-renaming/literal changes, operator changes, repeated-window multiplicity and a 17/18 similarity passing 9444 but failing 9445 basis points. Equivalent syntax across intentionally separate domain implementations can remain report-only.

### 15. Data Class

<a id="rust-data-class"></a>
**Implemented indicator: rust.data_class@1.** At least 2 fields and at most 0 non-accessor authored associated functions. An accessor is one receiver-method expression self.field, &self.field or &mut self.field, optionally wrapped in return; or one assignment of its single ordinary identifier parameter to self.field. No extra statement, call, conversion, validation, branching, nested projection, ? or await. Constructors/static functions are operations. Derived/default bodies are excluded; enum size uses maximum fields per variant. This source shape does not prescribe adding behavior to messages/configuration. Fixtures: field boundary, accessor-only, validating constructor, derives and intentional data messages.

### 16. Lazy Class

<a id="rust-lazy-class"></a>
**Implemented indicator: rust.lazy_class@1.** At most 1 field, 1 associated function and 5 summed body code lines; all three must match. Empty authored totals are zero, not proof the compiled type does nothing. Same ownership union and enum-field convention as above. Nominal newtypes, markers and small ports can intentionally match. Fixtures: every maximum and plus one, marker/newtype, generated/default scope.

### 17. Speculative Generality

<a id="rust-unused-type-parameters"></a>
**Pending provider rule: rust.unused_type_parameters@1.** Maximum zero pinned clippy::extra_unused_type_parameters findings for the declared compiler configuration. Missing provider/configuration/stale evidence errors. Unused-code diagnostics may provide separate limited evidence; one-implementation traits/public ports are not automatically speculative. Fixtures: genuinely unused parameter, used bound/type, public extension point and a required single-implementation port.

### 18. Feature Envy

<a id="rust-foreign-accesses"></a>
**Pending typed indicator: rust.foreign_accesses@1.** In receiver methods, at least 5 direct foreign field projections and foreign share strictly greater than 60%. Resolved field's defining owner differs from the method's type owner; own/foreign read/write/borrow projection sites form the denominator. Count each syntax site, including nested sites, not runtime executions. Ignore method/port calls. Unresolved required owners/types error. Fixtures: 4/5 accesses, 6/10 does not match and 7/10 does, nested owners, mappers and ports. Counts do not choose where behavior belongs.

### 19. Dead Code

<a id="rust-unused-code"></a>
**Pending compiler rule: rust.unused_code@1.** Maximum zero exactly dead_code/unused_imports/unreachable_code diagnostics under a frozen production compiler configuration. Enforce selected lints independently of unregistered allows. Compiler failure/missing diagnostic input errors. Public APIs/other features/runtime selection limit whole-program conclusions. Fixtures: used/private unused/public code, inactive features, suppressions, compiler failure and stale evidence. The source scanner does not guess reachability from text searches.

### 20. Inappropriate Intimacy

<a id="rust-dependency-contract"></a>
**Pending contract: rust.dependency_contract@1.** Maximum zero forbidden production package edges or declared public-access obligations. A project supplies exact package identities, closed allowed edges and permitted item access selectors; resolve renamed dependencies/re-exports to defining owners. Dev/build-only edges are a separately reported scope, not silently included in production metrics. Privacy/compiler failures are errors, not invented graph counts. This does not prove macro/build-time purity. Fixtures: allowed/renamed-forbidden edges, re-exports, missing contract and legitimate adapter edges.

### 21. Incomplete Library Class

<a id="rust-library-capabilities"></a>
**Pending contract: rust.library_capabilities@1.** Maximum zero failed declared capability tests. Pin external package/version, local adapter selectors and named observable requirements. Require complete current compiler/package/test evidence. Missing test/evidence errors; failing tests are violations. Never infer a library requirement. Fixtures: satisfied/failing/missing test, package version change, stale evidence and an intentional extension adapter.

### 22. Message Chains

<a id="rust-navigation-chains"></a>
**Pending typed indicator: rust.navigation_chains@1.** At least 3 transitions between distinct project-registered domain owners in one contiguous navigation expression. Edges are resolved direct field projections or exact registered navigation methods. Same-owner stays add zero; unregistered calls interrupt the chain. Explicit iterator/builder/stream roles are excluded only from this rule. Missing roles/type information errors if required, rather than dot counting or zero. Fixtures: 2/3/4 transitions, same owner, unknown method, aliases and legitimate pipelines.

### 23. Middle Man

<a id="rust-forwarding-share"></a>
**Implemented indicator: rust.forwarding_share@1.** At least 5 authored receiver methods, at least 80% syntactically forwarding. Entire body is one direct method call on `self.field`, or one path/UFCS call whose first argument is `self.field`, `&self.field` or `&mut self.field`, optionally return-wrapped. A receiver method that merely calls an unrelated path function is not forwarding. Every ordinary parameter is a simple identifier, forwarded once unchanged in declaration order after the explicit delegate argument, without additional args, parameter borrowing, conversion, setup, `?`, `await` or another statement. Static methods/generated/default bodies do not dilute the denominator. Full input/output type equivalence is not checked by this source pattern. Adapters/proxies can legitimately match. Fixtures: method minimum, 4/5 versus 3/5 share, unrelated helpers, UFCS delegation, argument reordering/conversion/validation/`?` and intentional port forwarding.

## Remaining coverage work

Pending rules are intentionally off in the example policy; requiring one errors. Implement pinned compiler/evidence ingestion next, followed by full type/project contracts and frozen history. Do not enable evidence-based gates before matching/nonmatching/error fixtures validate their providers. Exact exceptions, baselines and automated fixes are not implemented. All 23 catalog entries remain visible regardless of readiness; a measured syntax pattern never means every possible semantic smell has been checked.
