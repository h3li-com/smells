# Database migration code and generic smell rules

Research date: 2026-10-09

This note uses primary sources only: official framework documentation, API
documentation, and framework source. Statements labelled **Recommendation** or
**Inference** are conclusions for Smells; the linked material establishes the
framework facts on which they depend.

## Implementation status

The initial recommendation is implemented:

- optional, validated, repository-root-anchored `source_roles` policy declarations;
- high-confidence built-in source classifiers for Alembic, Django, TypeORM,
  MikroORM, Kysely, Knex, database-backed Sequelize/Umzug, SeaORM, and Refinery
  shapes; `rusqlite_migration` remains explicit-only until declaration-level
  sequence extraction is available;
- `reported_by_role` dispositions for canonical migration entrypoint length and
  migration-shell aggregate method/function length, while helpers and complexity
  rules remain unchanged;
- identical, Implementation-scoped role resolution in working-tree and staged
  snapshots; and
- SQL artifact inventory with `migration_body_not_analyzed` for recognized Drizzle,
  Prisma 7, SQLx, Diesel, and Refinery SQL bodies.

The dialect-aware SQL rule pack, trusted applied-state/provider evidence, and
`pre_existing_migration_modified` lifecycle rule remain future phases. Automatic
Prisma 8 package-version resolution, declaration-level `rusqlite_migration` sequence
extraction, framework-configuration resolution, and role-local declaration spans are
also deferred; Prisma 8 and `rusqlite_migration` can currently be declared explicitly.

The code follows the repository's [hexagonal architecture](architecture.md): the
scanner and migration policy are inside `core`, the CLI is a driving/inbound adapter,
and filesystem/Git capture is a driven/outbound adapter. Cockburn's “application” is
the whole inside; it is not represented by another `application/` directory.

## Executive decision

Smells should **not** add `versions`, `migrations`, or `migration` to
`exclude_directories` to silence long migration entrypoints.

The cross-language pattern is broader and more precise:

1. A migration is an artifact role, not a directory name. Every reviewed ecosystem
   permits non-default locations, direct registration, or custom providers.
2. Long, low-branching sequences are normal inside canonical migration entrypoints.
   Length alone is weak evidence there, but helpers, callbacks, backfills, branching,
   raw SQL, and transaction logic remain ordinary risk-bearing code.
3. The file must remain in the corpus. Smells should assign a
   `database_migration` **source role**, then assign only recognized entrypoints a
   **declaration role** such as `migration_up`, `migration_down`,
   `migration_operations`, or `migration_sql_builder`.
4. Applied state is external deployment state. Git age cannot prove that a migration
   ran anywhere. Some frameworks enforce hashes, some store only identity, and some
   officially support squashing or pruning.
5. Drizzle Kit, Prisma 7, SQLx, and Diesel put migration bodies in SQL, which the
   current Python/TypeScript/Rust packs do not scan. Their support needs a future SQL
   artifact role and SQL-specific rules, not a source-directory exemption.

The first implementation should preserve measurements and emit an auditable
`reported_by_role` or `not_applicable_by_role` disposition. It should never silently
remove a migration file or finding.

## Why Smells' current exclusion mechanism is unsafe

Smells recognizes `.rs`, `.py`/`.pyi`, and `.ts`/`.tsx`/`.mts`/`.cts` as source
([`source_file`](../src/input.rs)). `exclude_directories` is matched against **every
normal path component**, not an anchored repository-relative path
([`excluded`](../src/input.rs)). Working-tree traversal and staged-index capture both
drop excluded paths before corpus construction
([`walk_entry`](../src/input.rs), [`staged_corpus`](../src/input.rs)).

Therefore excluding `versions` does not merely suppress `function_lines`. It removes
matching files from every rule, Implementation ownership, source counts, snapshot
digests, and the paths used for bounded 200-commit history. It can also remove an
unrelated `versions` directory anywhere in the repository.

The current policy cannot narrow that exclusion to one anchored path or one rule.
Entries must be unique bare directory components, and `exceptions` must be empty
([policy validation](../src/policy.rs),
[`quality-policy.schema.json`](../schemas/quality-policy.schema.json)). The in-progress
Change Check / Debt Envelope design in [`CONTEXT.md`](../CONTEXT.md) and
[`ADR 0001`](adr/0001-separate-strict-and-staged-checks.md) is complementary: it can
grandfather existing measured debt, but only if migration files remain measurable in
both snapshots.

## Cross-framework evidence

| Ecosystem | Canonical body and discovery | History integrity and official validation | Smells consequence |
| --- | --- | --- | --- |
| Python: Alembic | A revision module has revision metadata and top-level `upgrade()` plus normally `downgrade()`; operations may also live in helpers called by them ([tutorial](https://alembic.sqlalchemy.org/en/latest/tutorial.html#create-a-migration-script), [operations API](https://alembic.sqlalchemy.org/en/latest/ops.html)). `script_location` and one or more recursive `version_locations` are configurable ([configuration](https://alembic.sqlalchemy.org/en/latest/tutorial.html#editing-the-ini-file)). | `alembic check` runs the autogenerate comparison, whose candidates require manual review; `current --check-heads`, upgrade/downgrade, and offline SQL exercise other properties ([autogenerate/check](https://alembic.sqlalchemy.org/en/latest/autogenerate.html#running-alembic-check-to-test-for-new-upgrade-operations), [head check](https://alembic.sqlalchemy.org/en/latest/cookbook.html#test-current-database-revision-is-at-head-s)). The default table records `version_num`, not content checksum ([DDL API](https://alembic.sqlalchemy.org/en/latest/api/ddl.html#alembic.ddl.impl.DefaultImpl.version_table_impl)). | Strong Python priority: metadata + Alembic import + entrypoint shape + configured path. Report linear entrypoint length; scan helpers normally. SQLAlchemy delegates schema migrations to Alembic, while FastAPI/Pydantic imports do not define a migration format ([SQLAlchemy](https://docs.sqlalchemy.org/en/21/core/metadata.html#altering-database-objects-through-migrations), [FastAPI](https://fastapi.tiangolo.com/tutorial/sql-databases/#create-the-tables)). |
| Python: Django | A `Migration` subclass contains `dependencies` and an `operations` list; `RunPython` callables are executable Python. `MIGRATION_MODULES` can move each app's migrations to an arbitrary package ([migration files](https://docs.djangoproject.com/en/6.0/topics/migrations/#migration-files), [`MIGRATION_MODULES`](https://docs.djangoproject.com/en/5.2/ref/settings/#migration-modules)). | `makemigrations --check`, `showmigrations --plan`, and replay tests cover distinct properties. Django supports staged squashing and later deletion of replaced history ([squashing](https://docs.djangoproject.com/en/6.0/topics/migrations/#squashing-migrations)). | Treat the operations list as declarative volume; treat `RunPython` bodies as ordinary code. A `migrations` basename is neither necessary nor sufficient. |
| TypeScript: TypeORM, Knex, MikroORM | TypeORM implements `MigrationInterface.up/down(QueryRunner)` ([creation](https://typeorm.io/docs/migrations/creating/)); Knex exports `up(knex)` and `down(knex)` ([migrations](https://knexjs.org/guide/migrations)); MikroORM extends `Migration`, with required `up()` and optional `down()` ([docs](https://mikro-orm.io/docs/migrations), [API](https://mikro-orm.io/api/migrations/class/Migration)). All support configured paths/globs/lists; Knex also supports a custom source. | Their status/list/check and up/down/fresh commands validate ordering or execution, not one universal property. TypeORM's executor matches timestamp/name rather than a body hash ([executor](https://github.com/typeorm/typeorm/blob/master/src/migration/MigrationExecutor.ts)); Knex's list validation checks executed names remain present and can be disabled; MikroORM supports snapshots and intentionally concatenates methods during `migration:rollup`. | Canonical entrypoints are strong structural roles. MikroORM rollup is direct evidence that a generic line ceiling is unsuitable. Entity/model dependencies, callbacks, helpers, and control flow retain normal rules. |
| TypeScript: Kysely | A `Migration` has `up(db: Kysely)` and optional `down(db)`; `FileMigrationProvider` accepts a configured folder, while a custom `MigrationProvider` can return migrations from anywhere ([official source/API](https://github.com/kysely-org/kysely/blob/master/src/migration/migrator.ts), [`FileMigrationProvider`](https://kysely-org.github.io/kysely-apidoc/interfaces/migration.FileMigrationProviderProps.html)). The official CLI also permits a custom folder, provider, or migrator ([`kysely-ctl`](https://github.com/kysely-org/kysely-ctl)). | `getMigrations`, `migrateUp`, `migrateDown`, `migrateTo`, and `migrateToLatest` report execution outcomes and reject missing or out-of-order executed names by default. The migration table source stores only `name` and `timestamp`, not a content hash ([migrator source](https://github.com/kysely-org/kysely/blob/master/src/migration/migrator.ts)). | Require Kysely package/import or provider evidence plus the exports; `up`/`down` alone is too generic. Preserve transaction, raw SQL, and data logic checks. |
| TypeScript/JavaScript: Sequelize CLI and Umzug | Sequelize CLI migration modules export `up(queryInterface, Sequelize)` and `down(...)`; `.sequelizerc` can configure `migrations-path` ([Sequelize migrations](https://sequelize.org/docs/v6/other-topics/migrations/)). Umzug v3 explicitly supports TypeScript exports typed as `Migration`, glob/cwd/ignore discovery, direct migration arrays, custom resolvers, mixed SQL/source formats, and migrations located anywhere ([Umzug](https://github.com/sequelize/umzug)). | Sequelize CLI exposes `db:migrate:status`, migrate, and undo. Umzug exposes pending/executed/up/down and a CLI. Default Sequelize storage records a unique migration name (optionally timestamps), not body content ([storage source](https://github.com/sequelize/umzug/blob/main/src/storage/sequelize.ts)). | Sequelize's default generated `.js` may be outside the TypeScript pack, but Umzug `.ts` is in scope. Umzug is database-agnostic, so Umzug shape alone must not imply `database_migration`; require database context/storage or an explicit role. |
| TypeScript tooling: Drizzle Kit | Current `drizzle-kit generate` writes SQL migrations, snapshots, and journal metadata; `generate --custom` also creates an SQL migration. Official docs say JavaScript/TypeScript custom migration scripts are upcoming, not current ([fundamentals](https://orm.drizzle.team/docs/migrations), [`generate`](https://orm.drizzle.team/docs/drizzle-kit-generate), [custom migrations](https://orm.drizzle.team/docs/kit-custom-migrations)). `out` is configurable and defaults to `./drizzle`. | `drizzle-kit check` detects generated-history collisions, not database parity; `migrate` applies pending SQL. Runtime source computes/stores SHA-256, but current dialect migration code selects newer entries by `created_at` rather than validating the stored hash against applied content ([reader](https://github.com/drizzle-team/drizzle-orm/blob/main/drizzle-orm/src/migrator.ts), [PostgreSQL dialect](https://github.com/drizzle-team/drizzle-orm/blob/main/drizzle-orm/src/pg-core/dialect.ts)). | There is no current TypeScript migration-body entrypoint to exempt. Keep schema, `drizzle.config.ts`, and runtime `migrate()` orchestration under ordinary TypeScript rules; inventory the SQL bundle for a future SQL pack. |
| TypeScript tooling: Prisma 7 and 8 | Prisma 7 uses timestamped `migration.sql` files, not TypeScript bodies ([v7 history](https://www.prisma.io/docs/orm/v7/prisma-migrate/understanding-prisma-migrate/migration-histories)). Prisma 8's early system uses a default-exported TypeScript `Migration` class whose `operations` getter emits `ops.json` and `migration.json` ([editing](https://www.prisma.io/docs/orm/migrations/editing-a-migration)). | Prisma 7 detects changed/deleted applied SQL. Prisma 8 `migration check` hashes compiled artifacts; deployment executes `ops.json`, and an uncompiled `migration.ts` is deliberately not checked ([how it works](https://www.prisma.io/docs/orm/migrations/how-migrations-work)). | Prisma 7 is a SQL-pack concern. Version-gate Prisma 8 detection; give only the `operations` declaration declarative-volume treatment and scan data-transform callbacks normally. |
| Rust: SQLx and Diesel | SQLx uses versioned `.sql` and optional paired `.up.sql`/`.down.sql`; `migrate!()` embeds a configurable directory ([source](https://docs.rs/sqlx/latest/sqlx/migrate/trait.MigrationSource.html), [types](https://docs.rs/sqlx/latest/sqlx/migrate/enum.MigrationType.html), [macro](https://docs.rs/sqlx/latest/sqlx/macro.migrate.html)). Diesel uses versioned folders with `up.sql`, `down.sql`, and optional metadata, and can embed them ([crate](https://docs.rs/diesel_migrations/latest/diesel_migrations/), [file layout](https://docs.rs/diesel_migrations/latest/diesel_migrations/struct.FileBasedMigrations.html)). | SQLx validates applied checksums ([`Migrator`](https://docs.rs/sqlx/latest/sqlx/migrate/struct.Migrator.html)); Diesel's official guide recommends `redo` and says generated diffs need manual correction, while its harness records version/run time rather than body hash ([guide](https://diesel.rs/guides/getting-started.html#database-migrations), [harness source](https://docs.diesel.rs/main/src/diesel_migrations/migration_harness.rs.html)). | Standard migration bodies are not `.rs`. Keep `migrate!()`, embedding, and startup orchestration under ordinary Rust rules. |
| Rust: SeaORM | A `.rs` module defines `struct Migration`, derives `DeriveMigrationName`, and implements `MigrationTrait::up` plus optional `down`; `MigratorTrait::migrations()` registers the modules ([writing](https://www.sea-ql.org/SeaORM/docs/migration/writing-migration/), [trait](https://docs.rs/sea-orm-migration/latest/sea_orm_migration/trait.MigrationTrait.html)). | Status/up/down/fresh/refresh/reset are built in; the tracking model has `version` and `applied_at`, not a checksum ([migrator](https://docs.rs/sea-orm-migration/latest/sea_orm_migration/migrator/trait.MigratorTrait.html), [model](https://docs.rs/sea-orm-migration/latest/sea_orm_migration/seaql_migrations/struct.Model.html)). | This is a high-confidence Rust-source case: Cargo dependency + trait/derive + preferably registry membership. Conditions, seeding, and helpers stay ordinary. |
| Rust: Refinery | Refinery accepts SQL **or Rust modules** named `[U\|V]<version>__<name>.rs`; a Rust module must define `migration() -> String`. `embed_migrations!()` uses the default crate-root `migrations` folder or an explicit relative directory, and CLI `-p` also selects a path ([macro](https://docs.rs/refinery/latest/refinery/macro.embed_migrations.html), [CLI](https://docs.rs/crate/refinery_cli/latest/source/README.md), [official repository](https://github.com/rust-db/refinery)). Rust bodies often build SQL with Barrel, whose builder emits SQL but does not itself provide Refinery's history runner ([Barrel](https://docs.rs/barrel/latest/barrel/)). | The runner records a checksum derived from name/version/SQL and, by default, aborts on divergent or missing applied migrations; those abort controls are configurable ([`Migration`](https://docs.rs/refinery/latest/refinery/struct.Migration.html), [`Runner`](https://docs.rs/refinery/latest/refinery/struct.Runner.html)). Official rollback guidance is to add a new forward migration rather than define `down` ([Refinery](https://github.com/rust-db/refinery)). Library runner and CLI `migrate` exercise the path; there is no separate documented `check` command. | Add `migration_sql_builder`, not `up/down`. Require filename + exact function signature + Refinery dependency/embedding evidence. Long linear builder code can be reported by role; all other functions stay normal. Flag disabled divergent/missing guards as separate migration-specific evidence rather than assuming integrity is unconditional. |
| Rust: `rusqlite_migration` | Migrations may be an ordinary Rust constant/slice containing ordered `M::up(...)` and optional down SQL; SQL files are also supported. Feature-gated up/down hooks can contain Rust closures ([crate docs](https://docs.rs/rusqlite_migration/latest/rusqlite_migration/), [`M` API](https://docs.rs/rusqlite_migration/latest/rusqlite_migration/struct.M.html)). | `Migrations::validate()` is designed for tests; `to_latest`/`to_version` execute the sequence. The crate tracks only SQLite `user_version`, so it does not provide applied-body checksums. | Do not tag the entire containing file. Recognize only a `migration_sequence` declaration backed by the crate dependency and constructors; ordinary sibling code and hook closures retain normal rules. |

## Lifecycle: history-oriented does not mean universally immutable

The documented evidence divides frameworks into several behaviors:

| Behavior | Examples |
| --- | --- |
| Applied-content integrity is enforced in the standard workflow | SQLx validates checksums; Refinery detects divergent/missing applied migrations using recorded metadata/checksum by default (its abort flags can be disabled); Prisma 7 detects changed/deleted applied SQL; Prisma 8 hashes compiled artifacts. |
| Applied identity/order is tracked without body validation | Alembic records revision identifiers; TypeORM, Knex, Kysely, Sequelize/Umzug, Diesel, SeaORM, and `rusqlite_migration` track a name/version/order without a documented source-body checksum. |
| A hash exists but current application logic does not use it as an applied-body guard | Drizzle records a hash, while its current dialect migrator selects pending entries from `created_at`; `drizzle-kit check` checks generated-history consistency rather than deployed-body equality. |
| History compaction or repair is officially supported | Alembic documents pruning after old environments no longer need revisions ([cookbook](https://alembic.sqlalchemy.org/en/latest/cookbook.html#building-an-up-to-date-database-from-scratch)); Django documents squash-and-delete; MikroORM provides rollup. |

**Recommendation:** a future VCS-only rule may report
`pre_existing_migration_modified`. It must not claim
`applied_migration_modified` without trusted database/provider evidence for that exact
migration. New, pending, applied-somewhere, and applied-everywhere are different states
that static source cannot infer.

## The SQL artifact gap needs a separate design

Documented fact: Drizzle Kit, Prisma 7, SQLx, and Diesel place the executable migration
body in SQL. Refinery can use SQL or a Rust function that returns SQL. Those SQL files
are absent from Smells' present language corpus, so a clean TypeScript or Rust scan is
not evidence that the migration body was assessed.

**Recommendation:** add a future `database_migration_sql` artifact kind under the same
language-neutral source-role model, but give it a SQL parser and framework-aware
declaration roles. Reusing `function_lines` or pretending every SQL file has `up/down`
would be incorrect:

- Drizzle and Prisma use forward SQL plus journal/snapshot/directory metadata;
- Diesel uses `up.sql`/`down.sql` in one version folder;
- SQLx supports simple forward files or paired reversible files; and
- Refinery uses versioned forward SQL and recommends a new migration for rollback.

Candidate SQL rules should be statement/operation based: destructive DDL/DML,
transaction opt-outs, reverse-pair consistency where the framework promises a pair,
raw data transforms, and replay ordering. They require dialect-aware validation before
standardization. The scanner must never autoformat or rewrite checksum-enforced SQL.
Until that pack exists, reports should explicitly say `migration_body_not_analyzed`
rather than imply coverage from scanning only configuration or orchestration source.

## Proposed role and detection model

### Source role and declaration role are separate

An explicit role should use normalized, repository-root-anchored,
Implementation-scoped paths, for example:

```json
{
  "role": "database_migration",
  "framework": "alembic",
  "paths": ["/backend/alembic/versions/**/*.py"],
  "configuration": ["/backend/alembic.ini"]
}
```

The leading `/` means repository root, not filesystem root. Role membership must not
remove the file from `Input.files`, counts, ownership, digest, or history.

Inside a confirmed source, classify only the exact framework declarations:

- `migration_up`, `migration_down`, and `migration_operations`;
- `migration_sql_builder` for Refinery's `migration() -> String`; and
- `migration_sequence` for an inline `rusqlite_migration` declaration.

Config modules, registries, helpers, callbacks, data transforms, and sibling code do
not inherit the declaration disposition.

### Evidence hierarchy

Automatic classification should combine independent evidence and retain it in the
report:

1. an explicit anchored role declaration;
2. a statically resolved framework configuration, registry, provider, or embedding
   call;
3. a framework-specific AST signature with imports/base classes/traits resolved to the
   package where feasible;
4. the owning manifest's framework dependency; and
5. a conventional path or filename as supporting evidence only.

Configuration discovery includes Alembic locations, Django `MIGRATION_MODULES`,
TypeORM/Knex/MikroORM globs and lists, Kysely folders/providers, Sequelize
`migrations-path`, Umzug globs/resolvers, Drizzle `out`, SQLx sources/macros, and Rust
registry/embedding calls. Smells must not execute application configuration. Dynamic
or custom providers stay unresolved unless the user declares an anchored role.

### False-positive and false-negative boundary

The following are **implementation inferences**, not framework guarantees:

- Path-only, manifest-only, import-only, and `up`/`down`-name-only matches must not
  change a rule disposition. Each is common outside database migration bodies.
- Umzug is database-agnostic. An Umzug task migration needs database context such as
  `SequelizeStorage`/`QueryInterface`, SQL resolver evidence, or an explicit
  `database_migration` declaration.
- A Refinery-like `migration() -> String` is not enough without its versioned filename
  and dependency/embedding evidence. A SeaORM-like `Migration` name is not enough
  without the trait/package evidence.
- Tests, examples, fixtures, generated build output, schema/config files, and runtime
  migration launchers must not inherit the body role merely because they sit nearby.
- Aliases, re-exports, wrapper types, custom Knex/Kysely/Umzug providers, dynamic
  JavaScript configuration, inline migration arrays, and monorepo dependency
  indirection can defeat inference. Low-confidence candidates should receive only a
  `possible_database_migration` annotation; their rules remain unchanged.
- Explicit anchored declarations are the deterministic escape hatch for valid cases
  that static inference cannot prove. Reports should show whether a role was declared
  or inferred and list every signal used.

## Rule applicability matrix

| Rule family | Confirmed canonical declaration | Helpers, callbacks, and sibling code | SQL artifact today |
| --- | --- | --- | --- |
| Function/method lines and aggregate class lines | Measure and retain; default to `reported_by_role` for predominantly sequential operations/builders | Normal evaluation | Not analyzed; report coverage gap |
| Cyclomatic complexity / CRAP | Normal evaluation; branching and data migration logic remain the risk | Normal evaluation | Future SQL-specific control/data-flow rules |
| Duplicate functions | Contextualize/report for entrypoints; shared helpers can create temporal coupling across history | Normal evaluation | Future statement-level analysis |
| Long parameters | `not_applicable_by_role` only for an exact framework-mandated signature | Normal evaluation | Not applicable |
| Large/lazy/data class or unit struct | Do not diagnose a framework-mandated protocol shell solely from its shape; retain measurements | Normal evaluation | Not applicable |
| Unused/dependency/navigation | Keep unless a narrow framework contract proves inapplicability | Normal evaluation | Future artifact ownership/registration checks |
| Divergent change / shotgun surgery | Keep the file in corpus and history; decide per detector | Normal evaluation | Include once SQL is captured |
| Migration-specific safety | Separate rules/evidence for replay, reversibility, historical-model use, checksum drift, transaction mode, and framework registration | Apply where relevant | Core future SQL-pack purpose |

No reviewed framework specifies a universal source-line ceiling. The useful distinction
is declarative volume versus behavioral complexity, not migration versus ordinary code.

## Acceptance criteria and test matrix

These are **recommendations for implementation**, derived from the repository behavior
and framework contracts above.

| Area | Required acceptance tests |
| --- | --- |
| Policy validation | Accept normalized root-anchored paths scoped to one Implementation. Reject `..`, filesystem-absolute paths, escaping symlinks, ambiguous duplicate declarations, unsupported framework/language pairs, and empty matches when strict validation is requested. |
| Corpus preservation | In working-tree and staged modes, a role-tagged file remains in `Input.files`, source count, nearest-manifest ownership, snapshot digest, and history input. Adding a role changes disposition metadata, not corpus membership. |
| Positive Python fixtures | Alembic revision at a configured non-`versions` path; Django migration at a `MIGRATION_MODULES` path. Only canonical declarations receive roles. |
| Positive TypeScript fixtures | TypeORM/MikroORM class, Knex exports, Kysely exports/provider, database-backed Umzug/Sequelize module, and version-gated Prisma 8 class. Long linear entrypoints are reported by role. |
| Positive Rust fixtures | SeaORM trait module and registry; Refinery `V1__name.rs` with `migration() -> String` plus dependency/embedding; inline `rusqlite_migration` sequence. Only the exact methods/function/declaration are tagged. |
| Negative controls | Ordinary files under directories named `versions`/`migrations`; generic functions named `up`, `down`, `upgrade`, or `migration`; manifest dependency without structure; structure without package evidence; non-database Umzug tasks; schema/config/orchestrator files; tests and copied examples. All retain original evaluation. |
| Unresolved discovery | Dynamic config, custom providers, aliases the resolver cannot prove, and missing registry evidence produce a deterministic `possible_database_migration` annotation with no rule reclassification. An explicit anchored role resolves the case without executing code. |
| Rule boundary | A long linear canonical entrypoint retains its observed value and becomes `reported_by_role`; an equally long helper fails normally. A branch-heavy or high-CRAP entrypoint still fails its complexity rule. Exact required signatures alone can be `not_applicable_by_role`. |
| SQL coverage | Drizzle, Prisma 7, SQLx, and Diesel fixtures say `migration_body_not_analyzed`; their `.ts`/`.rs` config and launchers are scanned normally. Refinery SQL files are likewise identified as unscanned while Refinery `.rs` builders are scanned. |
| Lifecycle wording | Changing a reference-snapshot migration emits at most `pre_existing_migration_modified` without provider evidence. Checksum-aware severity varies by framework, but the report never invents applied state. |
| Determinism and output | JSON/text output includes role, framework/version, declared-or-inferred source, evidence, confidence, path scope, declaration span, original measurement, and adjusted disposition. Results and digests are stable across file traversal order. |
| Regression parity | Each fixture runs in both working and staged corpus paths. Change Check evaluates reference and change with identical role policy and preserves existing debt-envelope comparisons. |

## Phased implementation recommendation

### Phase 0: preserve evidence

Do not broaden `exclude_directories`. Until roles exist, keep current findings. A
source suppression may be added when a new migration is reviewed, but do not edit an
already-deployed or checksum-protected migration merely to silence Smells.

### Phase 1: add the role/report contract

Add anchored, Implementation-scoped source roles and declaration roles. Keep files in
the corpus and include evidence/confidence/disposition in reports. Support explicit
declarations first so users can handle dynamic framework configurations safely.

### Phase 2: add high-confidence classifiers

Prioritize signatures with low collision risk:

1. Alembic metadata/imports/entrypoints;
2. TypeORM and MikroORM class contracts;
3. SeaORM trait/derive/registry;
4. Refinery versioned filename + exact `migration() -> String` + embedding/dependency;
5. Kysely imports/provider + `up`/optional `down`; and
6. Knex and database-backed Sequelize/Umzug only when configuration/context evidence
   resolves their generic export shapes.

Django can follow with settings mapping. Keep Prisma 8 behind a versioned detector
while its documented system is early. `rusqlite_migration` should begin as a
declaration-only classifier because its sequence often shares a normal Rust module.

### Phase 3: change only narrow rule dispositions

Start with exact framework signatures/shells and long linear entrypoints. Keep
complexity, helper, callback, data-transform, dependency, raw-SQL-surrounding, and
history collectors active. Integrate with Change Check so existing measured findings
can remain debt without disappearing from the baseline.

### Phase 4: inventory SQL, then design a SQL pack

Recognize external SQL bundles and report the coverage gap for Drizzle, Prisma 7,
SQLx, Diesel, and Refinery SQL. Only after adding dialect-aware parsing should Smells
evaluate SQL migration bodies. Do not autoformat them.

### Phase 5: accept framework validation and lifecycle evidence

Do not run migrations implicitly. Accept or document explicit, snapshot-bound evidence
from framework check/status/replay commands. Add `pre_existing_migration_modified`
first; reserve applied-state language for trusted provider/database evidence.

## Final decision

Migration handling should be a first-class, language-neutral source/artifact role with
framework-specific declaration classifiers and rule applicability. The immediate
Alembic `versions` exception should not become a global directory exclusion. The same
mistake would miss configured TypeScript layouts, hide SeaORM and Refinery Rust source,
misclassify Kysely or Umzug functions, and still leave the shared SQL migration bodies
entirely unexamined.

## Verification strategy

Tests follow architectural responsibility rather than framework branding:

- Core unit tests exercise classification, disposition, report state, and scan
  orchestration through in-memory ports. They do not boot Alembic, TypeORM, Drizzle,
  or SeaORM.
- Property tests exercise domain invariants such as fail-closed canonicality and
  provenance preservation.
- Integration tests feed framework-shaped source, manifests, policies, working-tree
  snapshots, and staged snapshots through the public scanner boundary.
- Mutation tests run against changed Rust on pull requests and the full source tree on
  a weekly/manual schedule. Reports separate caught, survived, unviable, and timed-out
  mutants instead of hiding untested outcomes in one score.
- `make real-repository-e2e` performs static scans of pinned FastAPI/Alembic,
  SeaORM, TypeORM, and Drizzle repositories. It evaluates all 23 smell categories,
  verifies both positive and fail-closed migration behavior, installs no repository
  dependencies, and never executes a migration.

Database upgrade/downgrade/replay tests belong to applications that own a schema and
database. Smells verifies its static interpretation of those artifacts; it must not
turn a source scan into an implicit framework or database execution.
