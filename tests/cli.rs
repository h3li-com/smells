#![expect(
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::format_collect,
    clippy::format_push_string,
    reason = "CLI test helpers fail loudly, and fixtures read best as one block"
)]

use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Workspace {
    path: PathBuf,
}
impl Workspace {
    fn new(source: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "smells-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("src")).unwrap();
        let workspace = Self { path };
        workspace.source("src/lib.rs", source);
        workspace.policy(
            &serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap(),
        );
        workspace
    }
    fn source(&self, path: &str, source: &str) {
        let path = self.path.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, source).unwrap();
    }
    fn policy(&self, value: &Value) {
        fs::write(
            self.path.join("quality-policy.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
    fn modify(&self, pointer: &str, value: Value) {
        let mut policy: Value =
            serde_json::from_slice(&fs::read(self.path.join("quality-policy.json")).unwrap())
                .unwrap();
        *policy.pointer_mut(pointer).unwrap() = value;
        self.policy(&policy);
    }
    fn check(&self) -> Output {
        self.command(&[
            "check",
            "--path",
            ".",
            "--policy",
            "quality-policy.json",
            "--only-group",
            "source",
            "--format",
            "json",
        ])
    }
    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_smells"))
            .args(args)
            .current_dir(&self.path)
            .output()
            .unwrap()
    }
    fn git(&self, args: &[&str]) {
        let result = Command::new("git")
            .args(args)
            .current_dir(&self.path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn refinery_rust_migration_builder_is_inferred_and_reported_by_role() {
    let workspace =
        Workspace::new("use refinery::embed_migrations;\nembed_migrations!(\"migrations\");\n");
    workspace.source(
        "migrations/V1__create_accounts.rs",
        concat!(
            "pub fn migration() -> String {\n",
            "    let mut sql = String::new();\n",
            "    sql.push_str(\"CREATE TABLE accounts (id INTEGER);\");\n",
            "    sql.push_str(\"CREATE INDEX ix_accounts_id ON accounts(id);\");\n",
            "    sql.push_str(\"ALTER TABLE accounts ADD COLUMN name TEXT;\");\n",
            "    sql\n",
            "}\n",
        ),
    );
    workspace.modify("/rules/rust.function_lines/parameters/maximum", json!(3));

    let output = workspace.check();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let data = report(&output);
    let role = data["source_roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["path"] == "migrations/V1__create_accounts.rs")
        .expect("Refinery source role");
    assert_eq!(role["framework"], "refinery");
    assert_eq!(role["source"], "inferred");
    assert_eq!(
        role["evidence"],
        json!([
            "versioned_filename:refinery",
            "canonical_signature:migration_to_string",
            "embedding_call:embed_migrations"
        ])
    );
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["rule_id"] == "rust.function_lines"
                && finding["location"]["path"] == "migrations/V1__create_accounts.rs"
        })
        .expect("long Refinery builder");
    assert_eq!(finding["status"], "reported_by_role");
    assert_eq!(finding["blocking"], false);
    assert_eq!(
        finding["role_context"]["declaration_role"],
        "migration_sql_builder"
    );
}

#[test]
fn refinery_embedding_does_not_classify_a_different_directory() {
    let workspace =
        Workspace::new("use refinery::embed_migrations;\nembed_migrations!(\"migrations\");\n");
    workspace.source(
        "legacy/V1__not_embedded.rs",
        concat!(
            "pub fn migration() -> String {\n",
            "    let mut sql = String::new();\n",
            "    sql.push_str(\"CREATE TABLE legacy (id INTEGER);\");\n",
            "    sql.push_str(\"CREATE INDEX legacy_id ON legacy(id);\");\n",
            "    sql\n",
            "}\n",
        ),
    );
    workspace.modify("/rules/rust.function_lines/parameters/maximum", json!(2));

    let output = workspace.check();
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    assert_eq!(data["source_roles"], json!([]));
    assert_eq!(
        data["source_role_candidates"][0]["evidence"],
        json!(["unresolved_versioned_filename:refinery"])
    );
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["location"]["path"] == "legacy/V1__not_embedded.rs")
        .expect("unembedded Refinery-like builder remains blocking");
    assert_eq!(finding["status"], "violation");
}

#[test]
fn seaorm_migration_type_reports_aggregate_function_lines_by_role() {
    let workspace = Workspace::new(concat!(
        "use sea_orm_migration::prelude::*;\n",
        "mod m20261009_accounts;\n",
        "pub struct Migrator;\n",
        "impl MigratorTrait for Migrator {\n",
        "    fn migrations() -> Vec<Box<dyn MigrationTrait>> {\n",
        "        vec![Box::new(m20261009_accounts::Migration)]\n",
        "    }\n",
        "}\n",
    ));
    workspace.source(
        "src/m20261009_accounts.rs",
        concat!(
            "use sea_orm_migration::prelude::*;\n",
            "#[derive(DeriveMigrationName)]\n",
            "pub struct Migration;\n\n",
            "#[async_trait::async_trait]\n",
            "impl MigrationTrait for Migration {\n",
            "    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {\n",
            "        let table = Table::create();\n",
            "        manager.create_table(table).await?;\n",
            "        manager.create_index(Index::create()).await?;\n",
            "        Ok(())\n",
            "    }\n\n",
            "    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {\n",
            "        manager.drop_table(Table::drop()).await?;\n",
            "        Ok(())\n",
            "    }\n",
            "}\n",
        ),
    );
    workspace.modify(
        "/rules/rust.type_function_lines/parameters/maximum",
        json!(4),
    );

    let output = workspace.check();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    assert_eq!(data["source_roles"][0]["framework"], "sea_orm");
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["rule_id"] == "rust.type_function_lines"
                && finding["location"]["path"] == "src/m20261009_accounts.rs"
        })
        .expect("SeaORM aggregate function lines");
    assert_eq!(finding["status"], "reported_by_role");
    assert_eq!(finding["blocking"], false);
    assert_eq!(
        finding["role_context"]["declaration_role"],
        "migration_shell"
    );
}

#[test]
fn seaorm_shape_without_registry_stays_an_unresolved_candidate() {
    let workspace = Workspace::new(concat!(
        "use sea_orm_migration::prelude::*;\n",
        "mod m20261009_accounts;\n",
        "pub struct Migrator;\n",
        "impl MigratorTrait for Migrator {\n",
        "    fn migrations() -> Vec<Box<dyn MigrationTrait>> { vec![] }\n",
        "}\n",
    ));
    workspace.source(
        "src/m20261009_accounts.rs",
        concat!(
            "use sea_orm_migration::prelude::*;\n",
            "#[derive(DeriveMigrationName)]\n",
            "pub struct Migration;\n",
            "#[async_trait::async_trait]\n",
            "impl MigrationTrait for Migration {\n",
            "    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {\n",
            "        manager.create_table(Table::create()).await?;\n",
            "        manager.create_index(Index::create()).await?;\n",
            "        manager.alter_table(Table::alter()).await?;\n",
            "        Ok(())\n",
            "    }\n",
            "}\n",
        ),
    );
    workspace.modify(
        "/rules/rust.type_function_lines/parameters/maximum",
        json!(3),
    );

    let output = workspace.check();
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    assert_eq!(data["source_roles"], json!([]));
    assert_eq!(
        data["source_role_candidates"][0]["evidence"],
        json!(["unresolved_framework_signal:sea_orm_migration"])
    );
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "rust.type_function_lines")
        .expect("unregistered SeaORM shape remains blocking");
    assert_eq!(finding["status"], "violation");
}

#[test]
fn rust_sql_migration_inventory_honors_framework_evidence_and_refinery_path() {
    let workspace = Workspace::new(concat!(
        "use diesel_migrations::{embed_migrations, EmbeddedMigrations};\n",
        "const DIESEL: EmbeddedMigrations = embed_migrations!();\n",
        "static SQLX: sqlx::migrate::Migrator = sqlx::migrate!(\"./sqlx\");\n",
        "refinery::embed_migrations!(\"refinery\");\n",
    ));
    workspace.source(
        "migrations/20261009000000_accounts/up.sql",
        "CREATE TABLE accounts(id integer);\n",
    );
    workspace.source(
        "sqlx/20261009000000_accounts.up.sql",
        "CREATE TABLE accounts(id integer);\n",
    );
    workspace.source(
        "refinery/V1__accounts.sql",
        "CREATE TABLE accounts(id integer);\n",
    );
    workspace.source(
        "legacy/V2__not_embedded.sql",
        "CREATE TABLE legacy(id integer);\n",
    );
    workspace.source(
        "legacy/20261009000000_not_embedded.sql",
        "CREATE TABLE legacy_sqlx(id integer);\n",
    );
    workspace.source(
        "archive/20261009000000_not_embedded/up.sql",
        "CREATE TABLE legacy_diesel(id integer);\n",
    );

    let output = workspace.check();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let data = report(&output);
    let artifacts = data["migration_artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 3, "{artifacts:?}");
    assert_eq!(artifacts[0]["framework"], "diesel");
    assert_eq!(artifacts[1]["framework"], "refinery");
    assert_eq!(artifacts[2]["framework"], "sqlx");
    assert!(
        artifacts
            .iter()
            .all(|artifact| !artifact["path"].as_str().unwrap().contains("not_embedded"))
    );
}

#[test]
fn rust_sql_migrations_are_discovered_from_project_and_cli_configuration() {
    let workspace = Workspace::new(concat!(
        "use refinery::load_sql_migrations;\n",
        "pub fn migrations() { let _ = load_sql_migrations(\"database/refinery\"); }\n",
    ));
    workspace.source(
        "Cargo.toml",
        concat!(
            "[package]\nname = \"migration-runner\"\nversion = \"0.1.0\"\n",
            "[dependencies]\n",
            "diesel_migrations = \"2\"\n",
            "refinery = \"0.9\"\n",
            "sqlx = \"0.9\"\n",
        ),
    );
    workspace.source(
        "sqlx.toml",
        concat!(
            "[migrate]\n",
            "migrations-dir = \"database/sqlx\"\n",
            "ignored-chars = [\" \", \"\\t\", \"\\r\", \"\\n\"]\n",
        ),
    );
    workspace.source(
        "Justfile",
        concat!(
            "migrate:\n    refinery migrate -p ./database/refinery-cli\n",
            "announce:\n    @echo \"run refinery migrate -p ./database/echo-only\"\n",
            "usage := '''\n  refinery migrate -p ./database/string-only\n'''\n",
            "help:\n    #!/usr/bin/env sh\n    cat <<'EOF'\n    refinery migrate -p ./database/heredoc-only\n    EOF\n",
            "continued:\n    printf '%s\\n' \\\n      refinery migrate -p ./database/continued-only\n",
        ),
    );
    workspace.source(
        "tools/Justfile",
        "migrate:\n    refinery migrate --path ./database/not-root\n",
    );
    workspace.source(
        "migrations/202610110001_accounts/up.sql",
        "CREATE TABLE diesel_accounts(id integer);\n",
    );
    workspace.source(
        "migrations/202610110001_accounts/down.sql",
        "DROP TABLE diesel_accounts;\n",
    );
    workspace.source(
        "migrations/V2__refinery_accounts.sql",
        "CREATE TABLE refinery_accounts(id integer);\n",
    );
    workspace.source(
        "database/refinery/V3__refinery_audit.sql",
        "CREATE TABLE refinery_audit(id integer);\n",
    );
    workspace.source(
        "database/refinery-cli/V4__refinery_cli.sql",
        "CREATE TABLE refinery_cli(id integer);\n",
    );
    workspace.source(
        "database/not-root/V5__not_owned.sql",
        "CREATE TABLE not_owned(id integer);\n",
    );
    workspace.source(
        "database/echo-only/V6__not_a_command.sql",
        "CREATE TABLE not_a_command(id integer);\n",
    );
    workspace.source(
        "database/string-only/V7__not_a_recipe.sql",
        "CREATE TABLE not_a_recipe(id integer);\n",
    );
    workspace.source(
        "database/heredoc-only/V8__not_a_command.sql",
        "CREATE TABLE not_a_command(id integer);\n",
    );
    workspace.source(
        "database/continued-only/V9__not_a_command.sql",
        "CREATE TABLE not_a_command(id integer);\n",
    );
    workspace.source(
        "database/sqlx/202610110001_sqlx_accounts.sql",
        "CREATE TABLE sqlx_accounts(id integer);\n",
    );

    let output = workspace.check();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let data = report(&output);
    let artifacts = data["migration_artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 6, "{artifacts:?}");
    let sqlx = artifacts
        .iter()
        .find(|artifact| artifact["framework"] == "sqlx")
        .expect("configured SQLx migration");
    assert_eq!(
        sqlx["integrity"],
        json!({
            "mechanism": "sqlx_content_checksum",
            "content_normalization": "configured_ignored_characters",
            "configuration_path": "sqlx.toml"
        })
    );
    assert_eq!(sqlx["declaration_path"], "sqlx.toml");
    assert_eq!(sqlx["declaration_span"]["start_line"], 2);
    assert!(artifacts.iter().any(|artifact| {
        artifact["framework"] == "refinery"
            && artifact["path"] == "database/refinery/V3__refinery_audit.sql"
    }));
    let refinery_cli = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "database/refinery-cli/V4__refinery_cli.sql")
        .expect("Refinery CLI literal path migration");
    assert_eq!(refinery_cli["declaration_path"], "Justfile");
    assert_eq!(refinery_cli["declaration_span"]["start_line"], 2);
    let candidates = data["migration_artifact_candidates"].as_array().unwrap();
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate["path"] == "database/not-root/V5__not_owned.sql")
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate["path"] == "database/echo-only/V6__not_a_command.sql")
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate["path"] == "database/string-only/V7__not_a_recipe.sql")
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate["path"] == "database/heredoc-only/V8__not_a_command.sql")
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate["path"] == "database/continued-only/V9__not_a_command.sql")
    );
}

#[test]
fn refinery_justfile_working_directory_overrides_fail_closed() {
    let workspace = Workspace::new("pub fn migration_runner() {}\n");
    workspace.source(
        "Cargo.toml",
        concat!(
            "[package]\nname = \"migration-runner\"\nversion = \"0.1.0\"\n",
            "[dependencies]\nrefinery = \"0.9\"\n",
        ),
    );
    workspace.source(
        "Justfile",
        concat!(
            "set no-cd\n",
            "migrate:\n    refinery migrate -p ./database/history\n",
        ),
    );
    workspace.source(
        "database/history/V1__accounts.sql",
        "CREATE TABLE accounts(id integer);\n",
    );

    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    assert_eq!(data["migration_artifacts"], json!([]));
    assert_eq!(
        data["migration_artifact_candidates"][0]["path"],
        "database/history/V1__accounts.sql"
    );
}

#[test]
fn version_is_machine_checkable_for_release_smoke_tests() {
    let output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("smells {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn json_report_identifies_embedded_catalog_versions() {
    let workspace = Workspace::new("fn main() {}\n");
    let output = workspace.check();

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = report(&output);
    assert_eq!(data["catalog_version"], "1.1.0");
    assert_eq!(data["source_manifest_version"], "1.1.0");
}

#[test]
fn starter_policies_activate_every_rule_and_default_to_all_groups() {
    for policy in [
        "examples/quality-policy.json",
        "examples/python-quality-policy.json",
        "examples/typescript-quality-policy.json",
    ] {
        let value: Value = serde_json::from_slice(&fs::read(policy).unwrap()).unwrap();
        assert_eq!(value["default_groups"], json!(["all"]), "{policy}");
        let rules = value["rules"].as_object().unwrap();
        assert_eq!(rules.len(), 28, "{policy}");
        assert!(
            rules.values().all(|rule| rule["mode"] != "off"),
            "{policy} contains an inactive rule"
        );
    }
}

#[test]
fn policy_show_explains_the_resolved_all_active_policy() {
    let output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "policy",
            "show",
            "--policy",
            "examples/quality-policy.json",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value = report(&output);
    assert_eq!(value["status"], "resolved_policy");
    assert_eq!(value["selection"]["default_groups"], json!(["all"]));
    assert_eq!(
        value["selection"]["selected_rule_ids"]
            .as_array()
            .unwrap()
            .len(),
        28
    );
    assert_eq!(
        value["rules"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|rule| rule["active"] == true)
            .count(),
        28
    );
    for rule in value["rules"].as_array().unwrap() {
        assert_eq!(
            rule["guidance_ref"],
            format!("{}@1", rule["smell_id"].as_str().unwrap()),
            "{} must inherit versioned smell guidance",
            rule["rule_id"]
        );
        let when_to_ignore = rule["when_to_ignore"].as_object().unwrap_or_else(|| {
            panic!(
                "{} must resolve its smell-level when_to_ignore guidance",
                rule["rule_id"]
            )
        });
        assert_eq!(when_to_ignore["version"], 1, "{}", rule["rule_id"]);
        assert!(
            matches!(
                when_to_ignore["provenance"].as_str(),
                Some("refactoring_guru_verbatim" | "smells_authored")
            ),
            "{} has invalid guidance provenance",
            rule["rule_id"]
        );
        assert!(
            when_to_ignore["source_url"]
                .as_str()
                .is_some_and(|url| url.starts_with("https://refactoring.guru/")),
            "{} must identify its source page",
            rule["rule_id"]
        );
        assert_eq!(
            when_to_ignore["checked_on"], "2026-09-25",
            "{}",
            rule["rule_id"]
        );
        assert!(
            when_to_ignore["items"]
                .as_array()
                .is_some_and(|items| !items.is_empty()
                    && items
                        .iter()
                        .all(|item| item.as_str().is_some_and(|text| !text.trim().is_empty()))),
            "{} must provide ordered non-empty guidance items",
            rule["rule_id"]
        );
        if rule["required_inputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|input| input == "built_in_collector")
        {
            assert_eq!(
                rule["required_inputs"],
                json!(["built_in_collector"]),
                "optional evidence is not a required input for {}",
                rule["rule_id"]
            );
        }
    }

    let table = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args(["policy", "show", "--policy", "examples/quality-policy.json"])
        .output()
        .unwrap();
    assert_eq!(table.status.code(), Some(0), "{table:?}");
    let table = String::from_utf8(table.stdout).unwrap();
    assert!(table.contains("Resolved policy: examples/quality-policy.json (rust-v1 / rust)"));
    assert!(table.contains("Active rules: 28/28 | defaults: all"));
    assert!(table.contains("Guidance: long-method@1"));
    assert!(table.contains("When to ignore:"));
}

#[test]
fn uv_style_group_selection_is_deterministic_and_exclusions_win() {
    let workspace = Workspace::new("fn concise() {}");
    for arguments in [
        vec!["--only-group", "source"],
        vec!["--all-groups", "--no-group", "evidence"],
        vec!["--no-default-groups", "--group", "source"],
    ] {
        let mut command = vec![
            "check",
            "--path",
            ".",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ];
        command.extend(arguments);
        let output = workspace.command(&command);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let value = report(&output);
        assert_eq!(
            value["policy_selection"]["selected_rule_ids"]
                .as_array()
                .unwrap()
                .len(),
            17
        );
        assert_eq!(
            value["policy_selection"]["excluded_rule_ids"]
                .as_array()
                .unwrap()
                .len(),
            11
        );
        assert!(value["coverage"].as_array().unwrap().iter().any(|smell| {
            smell["rules"].as_array().unwrap().iter().any(|rule| {
                rule["rule_id"] == "rust.function_crap"
                    && rule["selected"] == false
                    && rule["measurement_status"] == "excluded_by_group"
            })
        }));
    }
}

#[test]
fn checked_in_default_groups_are_used_until_cli_replaces_them() {
    let workspace = Workspace::new("pub fn concise() {}");
    workspace.git(&["init", "-q"]);
    workspace.modify("/default_groups", json!(["source"]));
    let configured = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ]);
    assert_eq!(configured.status.code(), Some(0));
    let configured = report(&configured);
    assert_eq!(
        configured["policy_selection"]["default_groups"],
        json!(["source"])
    );
    assert_eq!(
        configured["policy_selection"]["selected_rule_ids"]
            .as_array()
            .unwrap()
            .len(),
        17
    );
    let configured_digest = configured["input_sha256"].as_str().unwrap().to_string();

    let overridden = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--only-group",
        "evidence",
        "--format",
        "json",
    ]);
    assert_eq!(overridden.status.code(), Some(0));
    let overridden = report(&overridden);
    assert_eq!(
        overridden["policy_selection"]["only_groups"],
        json!(["evidence"])
    );
    assert_eq!(
        overridden["policy_selection"]["selected_rule_ids"]
            .as_array()
            .unwrap()
            .len(),
        11
    );
    assert_eq!(overridden["errors"], json!([]));
    assert_ne!(overridden["input_sha256"], configured_digest);
}

#[test]
fn all_active_default_uses_built_in_collectors_without_provider_evidence() {
    let workspace = Workspace::new("pub fn concise() {}");
    workspace.git(&["init", "-q"]);
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(0));
    let value = report(&output);
    assert_eq!(
        value["policy_selection"]["selected_rule_ids"]
            .as_array()
            .unwrap()
            .len(),
        28
    );
    assert_eq!(value["errors"], json!([]));
    assert_eq!(value["summary"]["error_smell_patterns"], 0);
}

#[test]
fn default_rust_scan_detects_compiler_and_type_shape_signals_without_evidence() {
    let workspace = Workspace::new(
        "trait OutputPort { fn save(&self); fn load(&self); }\n\
         struct BrokenOutput { customer_id: u64 }\n\
         impl OutputPort for BrokenOutput { fn save(&self) {} }\n\
         fn branching(value: i32) -> i32 { if value > 0 { value } else { 0 } }\n\
         fn generic<T>(value: i32) -> i32 { value }\n",
    );
    workspace.git(&["init", "-q"]);
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    for rule in [
        "rust.function_crap",
        "rust.unused_code",
        "rust.unused_type_parameters",
        "rust.nominal_slot_contract",
        "rust.port_conformance",
    ] {
        assert!(matched(&data, rule), "missing built-in match for {rule}");
        let finding = data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["rule_id"] == rule)
            .unwrap();
        let measurement = finding_measurement(&data, finding);
        let provider_ref = measurement["evidence"]["provider_ref"].as_u64().unwrap() as usize;
        assert_eq!(
            data["evidence_provider_catalog"][provider_ref]["name"],
            "smells-built-in"
        );
    }
    assert_eq!(
        finding(&data, "rust.unused_code", "unused-private-declarations")["evaluation"]["metric"],
        "unused-code findings"
    );
    assert_eq!(
        finding(&data, "rust.unused_type_parameters", "src/lib.rs::generic")["evaluation"]["metric"],
        "unused type-parameter findings"
    );
}

#[test]
fn built_in_rust_model_uses_authored_tokens_not_literal_or_comment_text() {
    let workspace = Workspace::new(
        "fn unused<T>() -> &'static str { \"T if while case catch && || => unused\" }\n\
         // unused and T are prose, not references\n",
    );
    workspace.git(&["init", "-q"]);
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let data = report(&output);
    let crap = finding(&data, "rust.function_crap", "src/lib.rs::unused");
    assert_eq!(
        crap["evaluation"]["observed"],
        json!({"numerator": 2, "denominator": 1})
    );
    assert_eq!(crap["evaluation"]["matched"], false);
    assert!(matched(&data, "rust.unused_code"));
    assert!(matched(&data, "rust.unused_type_parameters"));
}

#[test]
fn default_rust_scan_uses_rust_match_privacy_and_extension_signals() {
    let workspace = Workspace::new(
        "mod leak;\n\
         pub struct Secret { token: String }\n\
         pub fn own(secret: Secret) -> String { secret.token }\n\
         pub trait PathExt { fn normalized(&self) -> bool; }\n\
         impl PathExt for std::path::Path { fn normalized(&self) -> bool { true } }\n\
         pub fn dispatch(value: u8) -> u8 { match value { 0 => 0, 1 => 1, 2 => 2, _ => 3 } }\n",
    );
    workspace.source(
        "src/leak.rs",
        "pub fn leak(secret: crate::Secret) -> String { secret.token }\n",
    );
    workspace.git(&["init", "-q"]);
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    let intimacy = finding(
        &data,
        "rust.dependency_contract",
        "src/leak.rs::foreign-private-field-accesses",
    );
    assert_eq!(intimacy["evaluation"]["observed"], 1);
    for rule in [
        "rust.function_crap",
        "rust.dependency_contract",
        "rust.library_capabilities",
    ] {
        assert!(
            matched(&data, rule),
            "missing Rust built-in match for {rule}"
        );
    }
}

#[test]
fn built_in_model_excludes_const_and_static_initializer_declarations() {
    let workspace = Workspace::new(
        "const _: () = {\n\
             struct HiddenConst { a: i32, b: i32, c: i32, d: i32, e: i32 }\n\
             fn hidden_const() { if true {} }\n\
         };\n\
         static VALUE: () = {\n\
             struct HiddenStatic { a: i32, b: i32, c: i32, d: i32, e: i32 }\n\
             fn hidden_static() { if true {} }\n\
         };\n",
    );
    workspace.git(&["init", "-q"]);
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = report(&output);
    assert!(data["findings"].as_array().unwrap().iter().all(|finding| {
        let symbol = finding["symbol"].as_str().unwrap_or_default();
        !symbol.contains("HiddenConst")
            && !symbol.contains("HiddenStatic")
            && !symbol.contains("hidden_const")
            && !symbol.contains("hidden_static")
    }));
}

#[test]
fn unknown_and_conflicting_group_selectors_fail_clearly() {
    let workspace = Workspace::new("fn concise() {}");
    for arguments in [
        vec!["--only-group", "unknown"],
        vec!["--only-group", "source", "--group", "evidence"],
        vec!["--only-group", "source", "--all-groups"],
    ] {
        let mut command = vec!["check", "--path", ".", "--policy", "quality-policy.json"];
        command.extend(arguments);
        let output = workspace.command(&command);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
    }

    for invalid in ["-", ""] {
        let output = workspace.command(&[
            "check",
            "--path",
            ".",
            "--policy",
            "quality-policy.json",
            "--group",
            invalid,
        ]);
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("invalid value for --group"),
            "{output:?}"
        );
    }
}

#[test]
fn output_formats_are_limited_to_scan_and_policy_display_commands() {
    let workspace = Workspace::new("fn concise() {}");
    let invalid_scan_format = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--format",
        "yaml",
    ]);
    assert_eq!(invalid_scan_format.status.code(), Some(2));

    let invalid_contract_format = workspace.command(&[
        "contracts",
        "validate",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ]);
    assert_eq!(invalid_contract_format.status.code(), Some(2));
}
impl Drop for Workspace {
    fn drop(&mut self) {
        // This is the exact unique fixture directory successfully created by this test.
        fs::remove_dir_all(&self.path).unwrap();
    }
}

fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}
fn finding<'a>(report: &'a Value, rule: &str, suffix: &str) -> &'a Value {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .chain(report["measurements"].as_array().unwrap().iter())
        .find(|f| f["rule_id"] == rule && f["symbol"].as_str().unwrap().ends_with(suffix))
        .unwrap_or_else(|| panic!("missing {rule} {suffix}"))
}
fn finding_measurement<'a>(report: &'a Value, finding: &Value) -> &'a Value {
    &report["measurements"][finding["measurement_index"].as_u64().unwrap() as usize]
}
fn matched(report: &Value, rule: &str) -> bool {
    report["findings"].as_array().unwrap().iter().any(|f| {
        f["rule_id"] == rule && matches!(f["status"].as_str(), Some("indicator" | "violation"))
    })
}

fn smell_result<'a>(report: &'a Value, smell_id: &str) -> &'a Value {
    report["smell_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["smell_id"] == smell_id)
        .unwrap_or_else(|| panic!("missing smell result {smell_id}"))
}

fn implementation_result<'a>(report: &'a Value, implementation_id: &str) -> &'a Value {
    report["implementation_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["implementation_id"] == implementation_id)
        .unwrap_or_else(|| panic!("missing implementation result {implementation_id}"))
}

#[test]
fn registry_covers_all_23_smells_and_exposes_readiness() {
    let output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .arg("rules")
        .output()
        .unwrap();
    assert!(output.status.success());
    let catalog = report(&output);
    assert_eq!(
        catalog["catalog"]["source"],
        "https://refactoring.guru/refactoring/smells"
    );
    assert_eq!(catalog["catalog"]["checked_on"], "2026-09-19");
    assert_eq!(catalog["catalog"]["item_count"], 23);
    assert_eq!(catalog["smells"].as_array().unwrap().len(), 23);
    assert_eq!(
        catalog["smells"]
            .as_array()
            .unwrap()
            .iter()
            .map(|smell| smell["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "long-method",
            "large-class",
            "primitive-obsession",
            "long-parameter-list",
            "data-clumps",
            "alternative-classes-with-different-interfaces",
            "refused-bequest",
            "switch-statements",
            "temporary-field",
            "divergent-change",
            "parallel-inheritance-hierarchies",
            "shotgun-surgery",
            "comments",
            "duplicate-code",
            "data-class",
            "dead-code",
            "lazy-class",
            "speculative-generality",
            "feature-envy",
            "inappropriate-intimacy",
            "incomplete-library-class",
            "message-chains",
            "middle-man",
        ]
    );
    assert_eq!(catalog["rules"].as_array().unwrap().len(), 28);
    assert_eq!(
        catalog["rules"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["implementation"] == "implemented")
            .count(),
        28
    );
    assert_eq!(
        catalog["smells"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["applicability"] == "not_applicable_native_rust")
            .count(),
        2
    );
    assert!(catalog["smells"].as_array().unwrap().iter().all(|smell| {
        smell["applicability"] == "not_applicable_native_rust"
            || !smell["rules"].as_array().unwrap().is_empty()
    }));
}

#[test]
fn no_arguments_prints_help_without_panicking() {
    let output = Command::new(env!("CARGO_BIN_EXE_smells")).output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("smells check"));
}

#[test]
fn policy_show_consumes_selectors_and_value_options_in_sequence() {
    let workspace = Workspace::new("fn concise() {}");
    let output = workspace.command(&[
        "policy",
        "show",
        "--all-groups",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(report(&output)["status"], "resolved_policy");
}

#[test]
fn rules_rejects_an_unknown_flag_even_when_its_arity_is_valid() {
    let output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args(["rules", "--unknown", "rust-v1"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("usage: smells rules [--rule-pack PACK]")
    );
}

#[test]
fn shared_modules_do_not_invent_distinct_clumps_or_duplicate_bodies() {
    let workspace = Workspace::new("mod shared;");
    workspace.source("src/main.rs", "mod shared;");
    workspace.source("src/shared.rs", "fn a(x:i32,y:i32,z:i32)->i32{let p=x+1;let q=y+p;let r=z*q;r}\nfn b(x:i32,y:i32,z:i32)->i32{let p=x+2;let q=y+p;let r=z*q;r}");
    workspace.modify(
        "/rules/rust.duplicate_functions/parameters/minimum_tokens",
        json!(20),
    );
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    assert!(!matched(&data, "rust.data_clumps"));
    let pairs: Vec<_> = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule_id"] == "rust.duplicate_functions")
        .collect();
    assert_eq!(pairs.len(), 1);
    assert_ne!(pairs[0]["location"], pairs[0]["related_locations"][0]);
}

#[test]
fn duplicate_test_functions_outside_test_modules_are_test_code() {
    let workspace = Workspace::new(
        "#[test] fn a(){let x=1;let p=x+1;let q=p*2;let r=q-3;assert_eq!(r,1);}\n#[test] fn b(){let x=1;let p=x+2;let q=p*2;let r=q-3;assert_eq!(r,3);}",
    );
    workspace.modify(
        "/rules/rust.duplicate_functions/parameters/minimum_tokens",
        json!(20),
    );
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = report(&output);
    let duplicate = finding(&data, "rust.duplicate_functions", "::a");
    let measurement = finding_measurement(&data, duplicate);
    assert_eq!(measurement["evidence"]["test_code"], true, "{measurement}");
    assert_eq!(duplicate["blocking"], false);
}

#[test]
fn qualified_module_reexports_and_glob_reexports_preserve_one_owner_budget() {
    let workspace = Workspace::new(
        "mod original{pub struct A;impl A{fn a(){}}}mod facade{pub use crate::original as renamed;pub use crate::original::*;}mod consumer{impl crate::facade::renamed::A{fn b(){}}impl crate::facade::A{fn c(){}}}",
    );
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        finding(&report(&output), "rust.type_functions", "::original::A")["evaluation"]["observed"],
        3
    );
}

#[test]
fn report_required_and_off_modes_have_distinct_verdicts_and_coverage() {
    let workspace = Workspace::new(include_str!("fixtures/catalog/lib.rs"));
    workspace.modify("/rules/rust.duplicate_functions/mode", json!("report"));
    workspace.modify("/rules/rust.unused_code/mode", json!("report"));
    assert_eq!(workspace.check().status.code(), Some(0));
    workspace.modify("/rules/rust.data_class/mode", json!("required"));
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    assert!(
        data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule_id"] == "rust.data_class"
                && f["status"] == "violation"
                && f["blocking"] == true)
    );
    workspace.modify("/rules/rust.data_class/mode", json!("off"));
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    assert!(
        !data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule_id"] == "rust.data_class")
    );
    assert!(
        data["coverage"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["rules"].as_array().unwrap())
            .any(|r| r["rule_id"] == "rust.data_class" && r["measurement_status"] == "disabled")
    );
}

#[test]
fn path_attributes_follow_containing_file_and_inline_module_directories() {
    let workspace = Workspace::new("mod parent;");
    workspace.source("src/parent.rs", "#[path=\"sibling.rs\"] mod child;\n#[path=\"custom\"] mod inline{#[path=\"inside.rs\"] mod nested;}");
    workspace.source("src/sibling.rs", "struct Local;");
    fs::create_dir(workspace.path.join("src/custom")).unwrap();
    workspace.source("src/custom/inside.rs", "fn f(){}");
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = report(&output);
    finding(&data, "rust.type_functions", "::parent::child::Local");
    finding(&data, "rust.function_lines", "::parent::inline::nested::f");
}

#[test]
fn catalog_fixture_exercises_every_authored_source_rule() {
    let workspace = Workspace::new(include_str!("fixtures/catalog/lib.rs"));
    workspace.modify(
        "/rules/rust.duplicate_functions/parameters/minimum_tokens",
        json!(20),
    );
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    for id in [
        "rust.function_lines",
        "rust.function_arguments",
        "rust.type_fields",
        "rust.enum_variants",
        "rust.type_functions",
        "rust.type_function_lines",
        "rust.trait_functions",
    ] {
        assert!(
            data["measurements"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["rule_id"] == id),
            "{id}"
        );
    }
    for id in [
        "rust.primitive_slots",
        "rust.data_clumps",
        "rust.alternative_interfaces",
        "rust.repeated_dispatch",
        "rust.temporary_fields",
        "rust.comment_share",
        "rust.duplicate_functions",
        "rust.data_class",
        "rust.lazy_class",
        "rust.forwarding_share",
    ] {
        assert!(matched(&data, id), "{id}");
    }
    assert_eq!(data["coverage"].as_array().unwrap().len(), 23);
    for smell in data["coverage"].as_array().unwrap() {
        assert!(
            smell["reference_url"]
                .as_str()
                .unwrap()
                .starts_with("https://refactoring.guru/smells/")
        );
        for rule in smell["rules"].as_array().unwrap() {
            assert_eq!(rule["smell_id"], smell["smell_id"]);
            assert_eq!(rule["smell"], smell["smell"]);
            assert_eq!(rule["category"], smell["category"]);
            assert_eq!(rule["pattern_type"], rule["kind"]);
            for field in ["signal", "why_it_matters", "review", "remediation"] {
                assert!(
                    !rule[field].as_str().unwrap().is_empty(),
                    "{} {field}",
                    rule["rule_id"]
                );
            }
            assert_eq!(rule["reference_url"], smell["reference_url"]);
            assert!(
                rule["review"]
                    .as_str()
                    .unwrap()
                    .starts_with("NON-NEGOTIABLE RESEARCH:")
            );
            assert!(
                rule["review"]
                    .as_str()
                    .unwrap()
                    .contains(rule["reference_url"].as_str().unwrap())
            );
            assert_eq!(rule["reference_check"]["required"], true);
            assert_eq!(rule["reference_check"]["non_negotiable"], true);
            assert_eq!(
                rule["reference_check"]["action"],
                "perform_external_research_call_to_reference_url"
            );
            assert_eq!(
                rule["reference_check"]["required_before"],
                "review_remediation_or_suppression"
            );
            assert_eq!(
                rule["reference_check"]["unavailable_action"],
                "report_reference_research_incomplete_and_do_not_review_remediate_or_suppress"
            );
        }
    }
}

#[test]
fn report_aggregates_rule_evidence_by_canonical_smell_pattern() {
    let workspace = Workspace::new(
        "struct Account { first: i32, second: i32 } impl Account {\nfn open(&self) {\nlet _ = self.first;\n}\nfn close(&self) {\nlet _ = self.second;\n}\n}",
    );
    for pointer in [
        "/rules/rust.type_fields/parameters/maximum",
        "/rules/rust.type_functions/parameters/maximum",
        "/rules/rust.type_function_lines/parameters/maximum",
    ] {
        workspace.modify(pointer, json!(0));
    }
    workspace.modify("/rules/rust.type_function_lines/mode", json!("report"));

    let output = workspace.check();
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    assert_eq!(data["report_schema_version"], 11);
    assert_eq!(data["guidance_catalog"].as_array().unwrap().len(), 23);
    assert!(
        data["measurements"].as_array().unwrap().len() > data["findings"].as_array().unwrap().len(),
        "unmatched evaluations must remain compact measurements, not findings"
    );
    assert!(
        data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["evaluation"]["matched"] == true),
        "schema-8 findings contain only matched evaluations"
    );
    let results = data["smell_results"].as_array().unwrap();
    assert_eq!(results.len(), 23);
    assert_eq!(results[0]["smell_id"], "long-method");
    assert_eq!(results[1]["smell_id"], "large-class");

    let large_class = smell_result(&data, "large-class");
    assert_eq!(large_class["smell"], "Large Class");
    assert_eq!(large_class["category"], "bloaters");
    assert_eq!(large_class["state"], "blocking_match");
    assert_eq!(large_class["coverage_status"], "measured_defined_scope");
    assert_eq!(large_class["matched_findings"], 3);
    assert_eq!(large_class["blocking_findings"], 2);
    assert_eq!(large_class["review_signals"], 1);
    assert_eq!(large_class["affected_files"], 1);
    assert_eq!(large_class["affected_symbols"], 1);
    assert_eq!(
        large_class["matched_rule_ids"],
        json!([
            "rust.type_fields",
            "rust.type_function_lines",
            "rust.type_functions"
        ])
    );
    assert_eq!(
        large_class["reference_url"],
        "https://refactoring.guru/smells/large-class"
    );
    assert_eq!(large_class["reference_check"]["required"], true);
    assert_eq!(large_class["reference_check"]["non_negotiable"], true);
    assert_eq!(
        large_class["reference_check"]["action"],
        "perform_external_research_call_to_reference_url"
    );
    for index in large_class["matched_finding_indices"].as_array().unwrap() {
        let finding = &data["findings"][index.as_u64().unwrap() as usize];
        assert_eq!(finding["smell_id"], "large-class");
        assert_eq!(finding["evaluation"]["matched"], true);
    }

    assert_eq!(data["summary"]["matched_smell_patterns"], 1);
    assert_eq!(data["summary"]["blocking_smell_patterns"], 1);
    assert_eq!(data["summary"]["review_smell_patterns"], 0);
    assert_eq!(data["summary"]["matched_smell_ids"], json!(["large-class"]));
}

#[test]
fn table_output_is_actionable_while_json_report_is_saved() {
    let workspace = Workspace::new(
        "pub struct Account {\n    pub name: String,\n    pub email: String,\n}\n\npub struct Invoice {\n    pub number: u64,\n    pub total: u64,\n}\n",
    );
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--only-group",
        "source",
        "--format",
        "table",
        "--report",
        "smells-report.json",
    ]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Scan summary | verdict:",
        "Data Class | data-class | review_match",
        "Finding | Smell | Repository symbols | Metric | Value | Matches when | Threshold | Status | Repository evidence locations",
        "0 | Data Class | src/lib.rs::Account | fields and non-accessor operations",
        "Actionable findings for __unowned__:",
        "Actionable finding",
        "Issue: Data Class: rust.data_class indicator",
        "Observed versus threshold:",
        "Source excerpt:",
        "src/lib.rs:1:12",
        "pub struct Account {",
        "Why it matters:",
        "Remediation:",
        "Reference URL: https://refactoring.guru/smells/data-class",
        "Review guidance: NON-NEGOTIABLE RESEARCH:",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in:\n{stdout}"
        );
    }

    let saved: Value =
        serde_json::from_slice(&fs::read(workspace.path.join("smells-report.json")).unwrap())
            .unwrap();
    assert_eq!(saved["summary"]["matched_smell_patterns"], 1);
    assert_eq!(saved["summary"]["review_smell_patterns"], 1);
    assert_eq!(saved["summary"]["blocking_smell_patterns"], 0);
    assert_eq!(saved["summary"]["matched_smell_ids"], json!(["data-class"]));
    assert_eq!(
        stdout
            .matches("Reference URL: https://refactoring.guru/smells/data-class")
            .count(),
        2,
        "every finding must carry its own reference and guidance"
    );
}

#[test]
fn table_output_caps_related_excerpts_and_reports_the_omitted_count() {
    let workspace = Workspace::new(concat!(
        "fn one(a: i32, b: i32, c: i32) {}\n",
        "fn two(a: i32, b: i32, c: i32) {}\n",
        "fn three(a: i32, b: i32, c: i32) {}\n",
        "fn four(a: i32, b: i32, c: i32) {}\n",
        "fn five(a: i32, b: i32, c: i32) {}\n",
        "fn six(a: i32, b: i32, c: i32) {}\n",
        "fn seven(a: i32, b: i32, c: i32) {}\n",
    ));
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--only-group",
        "source",
        "--format",
        "table",
    ]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Related source excerpt 5:"), "{stdout}");
    assert!(!stdout.contains("Related source excerpt 6:"), "{stdout}");
    assert!(
        stdout.contains("Related source excerpts omitted: 1"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("Related source excerpts omitted: 0"),
        "{stdout}"
    );
    assert!(stdout.contains("    >      1 | fn one"), "{stdout}");
}

#[test]
fn hook_mode_saves_one_indexed_finding_log_and_keeps_stdout_compact() {
    let workspace =
        Workspace::new("pub struct Account {\n    pub name: String,\n    pub email: String,\n}\n");
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--only-group",
        "source",
        "--format",
        "table",
        "--report",
        "smells-report.json",
        "--log",
        "smells-findings.log",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("verdict: required_checks_passed_with_review_signals"));
    assert!(stdout.contains("Finding Log: smells-findings.log"));
    assert!(stdout.contains("Start at the Issue Index"));
    assert!(!stdout.contains("Actionable finding"));
    assert!(!stdout.contains("more in the Finding Log"), "{stdout}");

    let log = fs::read_to_string(workspace.path.join("smells-findings.log")).unwrap();
    let lines: Vec<_> = log.lines().collect();
    assert_eq!(lines[0], "Smells Finding Log");
    assert!(lines.contains(&"Issue Index"));
    let index_line = lines
        .iter()
        .find(|line| {
            line.starts_with("F000001 | review | rust.data_class | src/lib.rs:1:12 | detail line ")
        })
        .expect("indexed finding");
    let detail_line: usize = index_line.rsplit_once(' ').unwrap().1.parse().unwrap();
    assert_eq!(lines[detail_line - 1], "Finding F000001");
    assert!(log.contains("When to ignore [data-class@1]"));
    assert!(log.contains("Provenance: smells_authored"));
    assert!(
        log.contains("Suppression form: smells: ignore[rust.data_class] -- <non-empty reason>")
    );
    assert!(log.contains("inspect the complete finding, reference, source, callers, and tests"));

    let saved: Value =
        serde_json::from_slice(&fs::read(workspace.path.join("smells-report.json")).unwrap())
            .unwrap();
    assert_eq!(saved["findings"][0]["finding_id"], "F000001");
}

#[test]
fn finding_log_keeps_blocking_ignored_and_review_targets_with_focus_markers() {
    let mut source = String::from("pub fn oversized() {\n");
    for index in 0..170 {
        source.push_str(&format!("    let value_{index} = {index};\n"));
    }
    source.push_str("}\n\n");
    source.push_str("// smells: ignore[rust.function_arguments] -- external callback shape\n");
    source.push_str(
        "fn callback(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32, i: i32, j: i32, k: i32, l: i32) {}\n\n",
    );
    source.push_str("pub struct Account { pub name: String, pub email: String }\n");
    let workspace = Workspace::new(&source);
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--only-group",
        "source",
        "--format",
        "table",
        "--log",
        "smells-findings.log",
    ]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let log = fs::read_to_string(workspace.path.join("smells-findings.log")).unwrap();
    for expected in [
        "Status: blocking",
        "Status: ignored",
        "Status: review",
        "Accepted suppression:",
        "  >      1 | pub fn oversized() {",
    ] {
        assert!(log.contains(expected), "missing {expected:?} in:\n{log}");
    }
}

#[test]
fn finding_log_indexes_source_local_scanner_errors_at_their_location() {
    let workspace = Workspace::new(concat!(
        "// smells: ignore[rust.function_arguments] --\n",
        "fn publish(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32) {}\n",
    ));
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--only-group",
        "source",
        "--format",
        "table",
        "--report",
        "smells-report.json",
        "--log",
        "smells-findings.log",
    ]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let log = fs::read_to_string(workspace.path.join("smells-findings.log")).unwrap();
    assert!(
        log.contains("E000001 | scanner_error | scanner | src/lib.rs:1:4 | detail line "),
        "{log}"
    );
    assert!(log.contains("Primary location: src/lib.rs:1:4"), "{log}");
    let saved: Value =
        serde_json::from_slice(&fs::read(workspace.path.join("smells-report.json")).unwrap())
            .unwrap();
    assert!(saved["errors"][0].is_string());
}

#[test]
fn finding_log_uses_the_rust_parser_failure_span() {
    let workspace = Workspace::new("fn valid() {}\n\nfn broken( {\n");
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--only-group",
        "source",
        "--format",
        "table",
        "--log",
        "smells-findings.log",
        "--report",
        "smells-report.json",
    ]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let log = fs::read_to_string(workspace.path.join("smells-findings.log")).unwrap();
    assert!(
        log.contains("E000001 | scanner_error | scanner | src/lib.rs:3:"),
        "{log}"
    );
    assert!(!log.contains("src/lib.rs:1:1"), "{log}");
}

#[test]
fn next_declaration_suppression_covers_a_finding_inside_its_body() {
    let workspace = Workspace::new(
        "pub struct Root;\n// smells: ignore[rust.navigation_chains] -- fluent traversal is the public API\npub fn read(root: Root) { let _ = root.head.second.third; }\n",
    );
    workspace.git(&["init", "-q"]);
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = report(&output);
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "rust.navigation_chains")
        .expect("navigation finding remains visible");
    assert_eq!(finding["status"], "ignored_match");
    assert_eq!(finding["suppression"]["target_location"]["line"], 3);
    assert_eq!(finding["suppression"]["target_end_line"], 3);
}

#[test]
fn self_smell_script_keeps_actionable_output_visible() {
    let output_directory = std::env::temp_dir().join(format!(
        "smells-self-check-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&output_directory).unwrap();
    let report_path = output_directory.join("smells-report.json");
    let log_path = output_directory.join("smells-findings.log");
    let output = Command::new("sh")
        .arg("scripts/self-smell-check.sh")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("SMELLS_BIN", env!("CARGO_BIN_EXE_smells"))
        .env("SMELLS_REPORT", &report_path)
        .env("SMELLS_FINDING_LOG", &log_path)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Scan summary | verdict:",
        "Finding Log:",
        "Start at the Issue Index",
        "JSON Evidence Report:",
        "self smell scan passed; complete Finding Log:",
    ] {
        assert!(stdout.contains(expected), "missing {expected:?}");
    }
    let log = fs::read_to_string(&log_path).unwrap();
    for expected in [
        "Issue Index",
        "Policy evaluation:",
        "Source excerpt:",
        "Remediation:",
        "Reference URL: https://refactoring.guru/smells/",
        "When to ignore [",
        "Review guidance: NON-NEGOTIABLE RESEARCH:",
    ] {
        assert!(log.contains(expected), "missing {expected:?}");
    }
    let saved: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert!(saved["summary"]["matched_findings"].as_u64().unwrap() > 0);
}

#[test]
fn cargo_manifest_marks_the_repository_rust_implementation() {
    let workspace = Workspace::new("fn concise() {}");
    workspace.source("Cargo.toml", "[package]\nname='fixture'\nversion='0.1.0'\n");
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    assert_eq!(data["summary"]["implementations"], 1);
    assert_eq!(data["summary"]["unowned_files"], 0);
    let implementation = implementation_result(&data, ".");
    assert_eq!(implementation["implementation_root"], ".");
    assert_eq!(implementation["ownership"], "runtime_manifest");
    assert_eq!(
        implementation["implementation_types"],
        json!(["rust_cargo_project"])
    );
    assert_eq!(implementation["runtime_types"], json!(["rust"]));
    assert_eq!(implementation["runtime_manifests"], json!(["Cargo.toml"]));
    assert_eq!(implementation["scanned_files"], 1);
}

#[test]
fn smell_results_distinguish_checked_disabled_and_inapplicable_patterns() {
    let workspace = Workspace::new("fn concise() {}");
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);

    let parameters = smell_result(&data, "long-parameter-list");
    assert_eq!(parameters["state"], "checked_no_match_in_measured_scope");
    assert_eq!(parameters["coverage_status"], "measured_defined_scope");
    assert_eq!(parameters["matched_findings"], 0);

    let long_method = smell_result(&data, "long-method");
    assert_eq!(long_method["state"], "checked_no_match_in_measured_scope");
    assert_eq!(long_method["coverage_status"], "measured_defined_scope");
    assert_eq!(long_method["pending_rule_ids"], json!([]));
    assert_eq!(long_method["disabled_rule_ids"], json!([]));
    assert_eq!(
        long_method["excluded_rule_ids"],
        json!(["rust.function_crap"])
    );

    let dead_code = smell_result(&data, "dead-code");
    assert_eq!(dead_code["state"], "excluded");
    assert_eq!(dead_code["coverage_status"], "excluded");
    assert_eq!(dead_code["disabled_rule_ids"], json!([]));
    assert_eq!(dead_code["excluded_rule_ids"], json!(["rust.unused_code"]));

    let refused_bequest = smell_result(&data, "refused-bequest");
    assert_eq!(refused_bequest["state"], "not_applicable");
    assert_eq!(refused_bequest["coverage_status"], "not_applicable");
}

#[test]
fn matched_findings_resolve_normalized_measurements_and_llm_diagnostics() {
    let workspace = Workspace::new(&format!(
        "{}fn overloaded(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32){{}}",
        " ".repeat(500)
    ));
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    assert_eq!(data["summary"]["verdict"], "blocked_by_required_patterns");
    let finding = finding(&data, "rust.function_arguments", "::overloaded");
    assert_eq!(finding["guidance_ref"], "long-parameter-list@1");
    let inherited_guidance = data["guidance_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|guidance| guidance["guidance_ref"] == finding["guidance_ref"])
        .expect("finding guidance reference must resolve offline");
    assert_eq!(
        inherited_guidance["provenance"],
        "refactoring_guru_verbatim"
    );
    assert_eq!(
        inherited_guidance["items"][0],
        "Don’t get rid of parameters if doing so would cause unwanted dependency between classes."
    );
    assert_eq!(finding["smell_id"], "long-parameter-list");
    assert_eq!(finding["smell"], "Long Parameter List");
    assert_eq!(finding["category"], "bloaters");
    assert_eq!(finding["pattern_type"], "metric");
    assert_eq!(finding["certainty"], "exact_source_metric");
    assert_eq!(finding["policy_mode"], "required");
    assert_eq!(
        finding["evaluation"]["metric"],
        "signature inputs including receiver"
    );
    assert_eq!(finding["evaluation"]["observed"], 8);
    assert_eq!(finding["evaluation"]["match_condition"], ">");
    assert_eq!(finding["evaluation"]["threshold"], 7);
    assert_eq!(finding["evaluation"]["matched"], true);
    assert_eq!(finding["status"], "violation");
    assert_eq!(finding["blocking"], true);
    assert_eq!(
        finding["diagnostic"]["reference_url"],
        "https://refactoring.guru/smells/long-parameter-list"
    );
    assert_eq!(finding["diagnostic"]["reference_check"]["required"], true);
    assert_eq!(
        finding["diagnostic"]["reference_check"]["non_negotiable"],
        true
    );
    assert_eq!(
        finding["diagnostic"]["reference_check"]["action"],
        "perform_external_research_call_to_reference_url"
    );
    assert_eq!(
        finding["diagnostic"]["reference_check"]["required_before"],
        "review_remediation_or_suppression"
    );
    assert_eq!(
        finding["diagnostic"]["reference_check"]["unavailable_action"],
        "report_reference_research_incomplete_and_do_not_review_remediate_or_suppress"
    );
    assert!(
        finding["diagnostic"]["review"]
            .as_str()
            .unwrap()
            .starts_with("NON-NEGOTIABLE RESEARCH:")
    );
    assert!(
        finding["diagnostic"]["review"]
            .as_str()
            .unwrap()
            .contains(finding["diagnostic"]["reference_url"].as_str().unwrap())
    );
    for field in [
        "headline",
        "explanation",
        "signal",
        "why_it_matters",
        "review",
        "remediation",
        "contract",
    ] {
        assert!(!finding["diagnostic"][field].as_str().unwrap().is_empty());
    }
    assert!(finding.get("source_excerpt").is_none());
    assert!(finding.get("evidence").is_none());
    let measurement = finding_measurement(&data, finding);
    assert_eq!(measurement["symbol"], finding["symbol"]);
    assert_eq!(measurement["location"], finding["location"]);
    assert_eq!(measurement["evaluation"]["observed"], 8);
    assert_eq!(measurement["evaluation"]["matched"], true);
}

#[test]
fn function_lines_include_the_maximum_and_fail_above_it() {
    for lines in [99, 100, 101] {
        let workspace = Workspace::new(&format!("fn f() {{\n{}}}\n", "let _ = 0;\n".repeat(lines)));
        let output = workspace.check();
        let data = report(&output);
        assert_eq!(
            finding(&data, "rust.function_lines", "::f")["evaluation"]["observed"],
            lines
        );
        assert_eq!(output.status.code(), Some(i32::from(lines > 100)));
    }
}

#[test]
fn argument_limits_count_the_receiver_as_one() {
    for count in [6, 7, 8] {
        let parameters = (0..count - 1)
            .map(|i| format!("p{i}:i32"))
            .collect::<Vec<_>>()
            .join(",");
        let workspace = Workspace::new(&format!(
            "struct A; impl A {{ fn f(&self,{parameters}) {{}} }}"
        ));
        let output = workspace.check();
        let data = report(&output);
        assert_eq!(
            finding(&data, "rust.function_arguments", "::f")["evaluation"]["observed"],
            count
        );
        assert_eq!(output.status.code(), Some(i32::from(count > 7)));
    }
}

#[test]
fn type_field_limits_are_per_declaration_or_variant() {
    for count in [14, 15, 16] {
        let fields = (0..count)
            .map(|i| format!("f{i}:i32"))
            .collect::<Vec<_>>()
            .join(",");
        let workspace = Workspace::new(&format!(
            "struct A {{{fields}}} enum E {{ V {{{fields}}}, Unit }}"
        ));
        let output = workspace.check();
        let data = report(&output);
        assert_eq!(
            finding(&data, "rust.type_fields", "::A")["evaluation"]["observed"],
            count
        );
        assert_eq!(
            finding(&data, "rust.type_fields", "::E::V")["evaluation"]["observed"],
            count
        );
        assert_eq!(
            finding(&data, "rust.type_fields", "::E::Unit")["evaluation"]["observed"],
            0
        );
        assert_eq!(output.status.code(), Some(i32::from(count > 15)));
    }
}

#[test]
fn enum_and_trait_limits_are_inclusive() {
    for count in [19, 20, 21] {
        let variants = (0..count)
            .map(|i| format!("V{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let methods = (0..count)
            .map(|i| format!("fn f{i}(&self);"))
            .collect::<String>();
        let workspace = Workspace::new(&format!("enum E {{{variants}}} trait T {{{methods}}}"));
        let output = workspace.check();
        let data = report(&output);
        assert_eq!(
            finding(&data, "rust.enum_variants", "::E")["evaluation"]["observed"],
            count
        );
        assert_eq!(
            finding(&data, "rust.trait_functions", "::T")["evaluation"]["observed"],
            count
        );
        assert_eq!(output.status.code(), Some(i32::from(count > 20)));
    }
}

#[test]
fn methods_aggregate_across_files_imports_aliases_and_generic_impls() {
    let methods_a = (0..12)
        .map(|i| format!("fn a{i}(&self) {{}} "))
        .collect::<String>();
    let methods_b = (0..12)
        .map(|i| format!("fn b{i}(&self) {{}} "))
        .collect::<String>();
    let workspace = Workspace::new(&format!(
        "mod helpers; struct A<T> {{value:T}} impl<T> A<T> {{{methods_a}}}"
    ));
    workspace.source("src/helpers.rs",&format!("use crate::A as Original; type Alias<T> = Original<T>; impl<T> Alias<T> {{{methods_b}}}"));
    let output = workspace.check();
    let data = report(&output);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        finding(&data, "rust.type_functions", "::A")["evaluation"]["observed"],
        24
    );
}

#[test]
fn method_counts_and_type_line_sums_have_exact_boundaries() {
    for count in [19, 20, 21] {
        let methods = (0..count)
            .map(|i| format!("fn f{i}(&self) {{}} "))
            .collect::<String>();
        let workspace = Workspace::new(&format!("struct A; impl A {{{methods}}}"));
        let output = workspace.check();
        let data = report(&output);
        assert_eq!(
            finding(&data, "rust.type_functions", "::A")["evaluation"]["observed"],
            count
        );
        assert_eq!(output.status.code(), Some(i32::from(count > 20)));
    }
    for lines in [499, 500, 501] {
        let workspace = Workspace::new(&format!(
            "struct A; impl A {{fn f(&self) {{\n{}}}}}",
            "let _=0;\n".repeat(lines)
        ));
        workspace.modify("/rules/rust.function_lines/parameters/maximum", json!(1000));
        let output = workspace.check();
        let data = report(&output);
        assert_eq!(
            finding(&data, "rust.type_function_lines", "::A")["evaluation"]["observed"],
            lines
        );
        assert_eq!(output.status.code(), Some(i32::from(lines > 500)));
    }
}

#[test]
fn unrelated_same_named_types_and_trait_members_are_not_merged() {
    let workspace = Workspace::new(
        "mod a {pub struct A;impl A {fn f(&self){}}} mod b {pub struct A;impl A {fn g(&self){}}} trait T {fn inherited(&self) {}} impl T for a::A {} ",
    );
    let output = workspace.check();
    let data = report(&output);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        finding(&data, "rust.type_functions", "::a::A")["evaluation"]["observed"],
        1
    );
    assert_eq!(
        finding(&data, "rust.type_functions", "::b::A")["evaluation"]["observed"],
        1
    );
}

#[test]
fn comments_strings_blank_lines_and_unicode_follow_lexical_rules() {
    let workspace = Workspace::new(
        "fn café() { let _ = \"🦀\"; }\nfn f(){\n// ordinary\n/// doc\n\nlet café = \"// not a comment\";\nlet _ = r#\"line\n/* literal */\n\"#;\n}\n",
    );
    let output = workspace.check();
    let data = report(&output);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        finding(&data, "rust.function_lines", "::café")["evaluation"]["observed"],
        1
    );
    assert_eq!(
        finding(&data, "rust.function_lines", "::f")["evaluation"]["observed"],
        4
    );
    assert_eq!(
        finding(&data, "rust.comment_share", "::f")["evaluation"]["observed"],
        json!({"comment_lines":1,"code_lines":4})
    );
}

#[test]
fn source_allow_cannot_disable_scanner_limits() {
    let workspace = Workspace::new(
        "#[allow(clippy::too_many_arguments)] fn f(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32){} ",
    );
    assert_eq!(workspace.check().status.code(), Some(1));
}

#[test]
fn identical_pinned_inputs_produce_identical_reports() {
    let first = Workspace::new(include_str!("fixtures/catalog/lib.rs"));
    let second = Workspace::new(include_str!("fixtures/catalog/lib.rs"));
    assert_eq!(first.check().stdout, first.check().stdout);
    assert_eq!(first.check().stdout, second.check().stdout);
}

#[test]
fn default_check_summary_names_the_first_blocking_findings() {
    let long = |name: &str| {
        format!("pub fn {name}(a:u8,b:u8,c:u8,d:u8,e:u8,f:u8,g:u8,h:u8)->u8{{a+b+c+d+e+f+g+h}}\n")
    };
    let workspace = Workspace::new(&["one", "two", "three", "four"].map(long).concat());
    let output = workspace.command(&["check", "--only-group", "source"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let named = stdout
        .lines()
        .filter(|line| {
            line.starts_with("  F")
                && line.contains(" | blocking | rust.function_arguments | src/lib.rs:")
        })
        .count();
    assert_eq!(named, 3, "{stdout}");
    assert!(
        stdout.contains("  +1 more in the Finding Log\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Finding Log: smells-findings.log"),
        "{stdout}"
    );
}

#[test]
fn staged_scan_ignores_unstaged_source_and_policy() {
    let workspace = Workspace::new("fn f(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32){} ");
    workspace.git(&["init", "-q"]);
    workspace.git(&["add", "--", "quality-policy.json", "src/lib.rs"]);
    workspace.source("src/lib.rs", "fn f(){}");
    workspace.modify(
        "/rules/rust.function_arguments/parameters/maximum",
        json!(100),
    );
    let output = workspace.command(&[
        "check",
        "--staged",
        "--only-group",
        "source",
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    assert_eq!(data["source_mode"], "staged_snapshot");
    assert_eq!(
        finding(&data, "rust.function_arguments", "::f")["evaluation"]["threshold"],
        7
    );
    assert_eq!(
        finding(&data, "rust.function_arguments", "::f")["evaluation"]["observed"],
        8
    );
}

#[test]
fn unstaged_policy_and_escaping_staged_paths_error() {
    let workspace = Workspace::new("fn f(){}");
    workspace.git(&["init", "-q"]);
    workspace.git(&["add", "--", "src/lib.rs"]);
    for path in ["quality-policy.json", "../quality-policy.json"] {
        let output =
            workspace.command(&["check", "--staged", "--policy", path, "--format", "json"]);
        assert_eq!(output.status.code(), Some(2));
    }
    let output = workspace.command(&["check", "--staged", "--format", "json"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn required_evidence_group_rules_use_built_in_collectors() {
    let catalog: Value = serde_json::from_str(include_str!("../rules/rust-v1.json")).unwrap();
    let provider_rules: Vec<_> = catalog["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rule| {
            !rule["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|input| input == "authored_source")
        })
        .map(|rule| rule["id"].as_str().unwrap())
        .collect();
    assert_eq!(provider_rules.len(), 11);
    for rule in provider_rules {
        let workspace = Workspace::new("fn f(){}");
        workspace.git(&["init", "-q"]);
        workspace.modify(&format!("/rules/{rule}/mode"), json!("required"));
        let output = workspace.command(&[
            "check",
            "--path",
            ".",
            "--policy",
            "quality-policy.json",
            "--only-group",
            "evidence",
            "--format",
            "json",
        ]);
        let data = report(&output);
        assert!(
            matches!(output.status.code(), Some(0 | 1)),
            "{rule}: {data}"
        );
        assert_eq!(data["errors"], json!([]), "{rule}");
        let smell_id = catalog["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|definition| definition["id"] == rule)
            .unwrap()["smell"]
            .as_str()
            .unwrap();
        assert!(
            smell_result(&data, smell_id)["measured_rule_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|measured| measured == rule),
            "{rule}"
        );
    }
}

#[test]
fn malformed_unknown_duplicate_and_out_of_range_policy_values_error() {
    let workspace = Workspace::new("fn f(){}");
    for (pointer, value) in [
        ("/schema_version", json!(1)),
        ("/scanner_version", json!("latest")),
        ("/rules/rust.function_lines/version", json!(3)),
        (
            "/rules/rust.primitive_slots/parameters/minimum_share_percent",
            json!(101),
        ),
        ("/exceptions", json!([{"reason":"ignore everything"}])),
        (
            "/source_roles",
            json!([{"role":"database_migration","framework":"alembic","paths":["/migrations/*.rs"]}]),
        ),
        (
            "/source_roles",
            json!([{"role":"database_migration","framework":"refinery","paths":["migrations/*.rs"]}]),
        ),
        (
            "/source_roles",
            json!([{"role":"database_migration","framework":"refinery","paths":["/../migrations/*.rs"]}]),
        ),
    ] {
        workspace.policy(
            &serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap(),
        );
        workspace.modify(pointer, value);
        assert_eq!(workspace.check().status.code(), Some(2), "{pointer}");
    }
    let duplicate = include_str!("../examples/quality-policy.json").replacen(
        "\"maximum\":100",
        "\"maximum\":100,\"maximum\":1",
        1,
    );
    fs::write(workspace.path.join("quality-policy.json"), duplicate).unwrap();
    assert_eq!(workspace.check().status.code(), Some(2));
}

#[test]
fn parser_unresolved_owner_and_missing_module_fail_closed() {
    for source in [
        "fn broken(",
        "impl Unknown {fn f(){}}",
        "mod missing;",
        "type A=B;type B=A;impl A{fn f(){}}",
        "mod a{pub use crate::b::*;}mod b{pub use crate::a::*;}impl a::Unknown{fn f(){}}",
        "mod a{pub struct A;}mod b{pub struct A;}mod facade{pub use crate::a::*;pub use crate::b::*;}mod consumer{use crate::facade::*;impl A{fn f(){}}}",
        "#[cfg_attr(unix,path=\"absent.rs\")]mod a;",
        "mod a{#[cfg_attr(unix,path=\"x\")]mod b{}}",
        "mod a{pub struct A;}mod b{pub struct A;}#[cfg(test)]use a::A;#[cfg(not(test))]use b::A;impl A{fn f(){}}",
        "mod a{}mod b{}#[cfg(test)]use crate::a::A;#[cfg(not(test))]use crate::b::A;impl A{fn f(){}}",
        "mod a{pub use crate::b::A;}mod b{pub use crate::a::A;}mod c{pub struct A;}#[cfg(test)]use a::A;#[cfg(not(test))]use c::A;impl A{fn f(){}}",
        "macro_rules! named{struct A;}impl A{fn f(){}}",
    ] {
        let workspace = Workspace::new(source);
        assert_eq!(workspace.check().status.code(), Some(2), "{source}");
    }
}

#[test]
fn cargo_target_roots_and_unused_same_name_imports_scan_cleanly() {
    let workspace = Workspace::new(
        "pub mod extract { pub trait FromRef<T> { fn from_ref(input: &T) -> Self; } }\n\
         pub mod macros { #[macro_export] macro_rules! from_ref { () => {}; } pub use from_ref as FromRef; }\n\
         pub use extract::FromRef;\n\
         pub use macros::FromRef;\n",
    );
    workspace.source(
        "tests/cli.rs",
        "mod util;\n#[test]\nfn works() { util::helper(); }\n",
    );
    workspace.source("tests/util.rs", "pub fn helper() {}\n");
    workspace.source(
        "examples/demo/main.rs",
        "mod args;\nfn main() { args::parse(); }\n",
    );
    workspace.source("examples/demo/args.rs", "pub fn parse() {}\n");
    let output = workspace.check();
    assert_eq!(report(&output)["errors"], json!([]));
}

#[test]
fn foreign_generic_macro_and_local_impl_targets_scan_cleanly() {
    let workspace = Workspace::new(
        "pub trait Describe { fn describe(&self) -> u8; }\n\
         impl<T> Describe for Box<T> { fn describe(&self) -> u8 { 1 } }\n\
         impl<T, E> Describe for Result<T, E> { fn describe(&self) -> u8 { 2 } }\n\
         impl<T: Clone> Describe for T { fn describe(&self) -> u8 { 3 } }\n\
         pin_project! { pub struct Pinned<F> { #[pin] future: F } }\n\
         impl<F> Pinned<F> { pub fn new(future: F) -> Self { Self { future } } }\n\
         pub mod cookies { pub use cookie::Key; }\n\
         pub mod tests { use crate::cookies::*; impl crate::Describe for Key { fn describe(&self) -> u8 { 4 } } }\n\
         pub fn local() -> u8 { struct Local; impl Local { fn value(&self) -> u8 { 5 } } Local.value() }\n",
    );
    let output = workspace.check();
    assert_eq!(report(&output)["errors"], json!([]));
}

#[cfg(unix)]
#[test]
fn modules_cross_in_root_directory_links_once_and_outside_links_fail_closed() {
    let overloaded = "pub fn f(a:u8,b:u8,c:u8,d:u8,e:u8,g:u8,h:u8,i:u8){}";
    let workspace = Workspace::new("mod linked;");
    workspace.source("common/mod.rs", overloaded);
    std::os::unix::fs::symlink("../common", workspace.path.join("src/linked")).unwrap();
    let data = report(&workspace.check());
    assert_eq!(data["errors"], json!([]));
    let paths: Vec<_> = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["rule_id"] == "rust.function_arguments")
        .map(|finding| finding["location"]["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["common/mod.rs"]);

    workspace.git(&["init", "-q"]);
    workspace.git(&["add", "--", "."]);
    let staged = workspace.command(&[
        "check",
        "--staged",
        "--policy",
        "quality-policy.json",
        "--only-group",
        "source",
        "--format",
        "json",
    ]);
    let data = report(&staged);
    assert_eq!(data["errors"], json!([]));
    assert_eq!(
        finding(&data, "rust.function_arguments", "::f")["location"]["path"],
        "common/mod.rs"
    );

    let outside = Workspace::new("");
    let escaping = Workspace::new("mod linked;");
    outside.source("mod.rs", overloaded);
    std::os::unix::fs::symlink(&outside.path, escaping.path.join("src/linked")).unwrap();
    assert_eq!(escaping.check().status.code(), Some(2));
}

#[test]
fn a_file_shared_by_two_crates_is_extracted_once_and_resolves_in_both() {
    let workspace = Workspace::new("");
    workspace.source("tests/a.rs", "mod common;");
    workspace.source(
        "tests/b.rs",
        "mod common;\nfn g(x: common::Bag, y: &common::Bag) {}\nimpl common::Bag { fn h(&self) {} }",
    );
    workspace.source(
        "tests/common/mod.rs",
        "pub struct Bag;\npub fn f(a:u8,b:u8,c:u8,d:u8,e:u8,g:u8,h:u8,i:u8){}",
    );
    let data = report(&workspace.check());
    assert_eq!(data["errors"], json!([]));
    let matches = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["rule_id"] == "rust.function_arguments")
        .count();
    assert_eq!(matches, 1);
}

#[test]
fn every_configuration_variant_is_scanned_at_its_own_file() {
    let overloaded = "pub fn f(a:u8,b:u8,c:u8,d:u8,e:u8,g:u8,h:u8,i:u8){}";
    let workspace = Workspace::new(
        "#[cfg_attr(feature=\"alternate\",path=\"other.rs\")]pub mod child;\n\
         #[cfg(unix)]pub mod imp{pub struct Unix;}\n\
         #[cfg(windows)]pub mod imp{pub struct Windows;}\n\
         impl imp::Windows{pub fn new()->Self{Self}}\n\
         #[cfg(test)]pub struct Twin;\n\
         #[cfg(not(test))]pub enum Twin{One}\n\
         impl Twin{pub fn twin(&self){}}\n",
    );
    workspace.source("src/child.rs", overloaded);
    workspace.source("src/other.rs", overloaded);
    let output = workspace.check();
    let data = report(&output);
    assert_eq!(data["errors"], json!([]));
    let mut paths: Vec<_> = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["rule_id"] == "rust.function_arguments")
        .map(|finding| finding["location"]["path"].as_str().unwrap())
        .collect();
    paths.sort_unstable();
    assert_eq!(paths, ["src/child.rs", "src/other.rs"]);
    assert!(data["findings"].as_array().unwrap().iter().any(|finding| {
        finding["symbol"]
            .as_str()
            .unwrap()
            .contains("child [cfg variant 2]::f")
    }));

    let conditional_only =
        Workspace::new("#[cfg_attr(feature=\"alternate\",path=\"other.rs\")]pub mod child;");
    conditional_only.source("src/other.rs", overloaded);
    let data = report(&conditional_only.check());
    assert_eq!(data["errors"], json!([]));
    assert!(matched(&data, "rust.function_arguments"));
}

#[test]
fn analysis_budget_exhaustion_cannot_return_success() {
    let workspace = Workspace::new(include_str!("fixtures/catalog/lib.rs"));
    for pointer in [
        "/limits/maximum_files",
        "/limits/maximum_pairs",
        "/limits/maximum_group_combinations",
    ] {
        workspace.policy(
            &serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap(),
        );
        workspace.modify(pointer, json!(1));
        if pointer.ends_with("maximum_files") {
            workspace.source("src/other.rs", "fn other(){}");
        }
        if pointer.ends_with("maximum_group_combinations") {
            // Identical field sets are enumerated once, so the budget needs two
            // distinct populations to run out.
            workspace.source(
                "src/lib.rs",
                &format!(
                    "{}{}",
                    include_str!("fixtures/catalog/lib.rs"),
                    "pub struct D { pub w: i32, pub x: i32, pub y: i32, pub z: i32 }\n\
                 pub struct E { pub w: i32, pub x: i32, pub y: i32, pub z: i32 }\n\
                 pub struct F { pub w: i32, pub x: i32, pub y: i32, pub z: i32 }\n",
                ),
            );
        }
        let output = workspace.check();
        assert_eq!(output.status.code(), Some(2), "{pointer}");
        if pointer.ends_with("maximum_group_combinations") {
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .contains("maximum_group_combinations budget exceeded")
            );
        }
    }
}

#[test]
fn all_authored_cfg_branches_are_explicitly_the_scan_scope() {
    let workspace = Workspace::new(
        "#[cfg(test)] fn test_only(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32){}",
    );
    // Inactive cfg branches are scanned; test-only code is reported but never blocks.
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    assert_eq!(data["scope"], "authored_all_cfg");
    let test_only = finding(&data, "rust.function_arguments", "test_only");
    assert_eq!(test_only["status"], "indicator");
    assert_eq!(test_only["blocking"], false);
}

#[cfg(unix)]
#[test]
fn staged_and_working_symlinks_cannot_substitute_external_source() {
    use std::os::unix::fs::symlink;
    let workspace = Workspace::new("fn f(){}");
    symlink("lib.rs", workspace.path.join("src/link.rs")).unwrap();
    assert_eq!(workspace.check().status.code(), Some(2));
    workspace.git(&["init", "-q"]);
    workspace.git(&["add", "--", "quality-policy.json", "src"]);
    assert_eq!(
        workspace
            .command(&[
                "check",
                "--staged",
                "--policy",
                "quality-policy.json",
                "--format",
                "json"
            ])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn example_policy_contract_validation_is_not_a_source_scan() {
    let workspace = Workspace::new("fn broken(");
    let output = workspace.command(&["contracts", "validate", "--policy", "quality-policy.json"]);
    assert_eq!(output.status.code(), Some(0));
    let validated = report(&output);
    assert_eq!(validated["status"], "valid_contracts");
    assert_eq!(validated["active_rules"], 28);
    assert_eq!(workspace.check().status.code(), Some(2));
}

#[test]
fn primitive_slot_count_share_and_spelled_type_scope_are_independent() {
    for (fields, expected) in [
        ("a:i32,b:i32,c:i32,d:i32,e:i32", true),
        ("a:i32,b:i32,c:i32,d:i32,e:Wrapped", false),
        ("a:i32,b:i32,c:i32,d:i32,e:i32,f:Wrapped,g:Wrapped", false),
        ("a:Alias,b:Alias,c:Alias,d:Alias,e:Alias", false),
    ] {
        let workspace = Workspace::new(&format!(
            "struct Wrapped(i32);type Alias=i32;struct A {{{fields}}}"
        ));
        assert_eq!(
            matched(&report(&workspace.check()), "rust.primitive_slots"),
            expected,
            "{fields}"
        );
    }
    let workspace =
        Workspace::new("struct Wrapped(i32);struct A {a:i32,b:i32,c:i32,d:i32,e:Wrapped}");
    workspace.modify(
        "/rules/rust.primitive_slots/parameters/minimum_raw_slots",
        json!(4),
    );
    assert!(matched(&report(&workspace.check()), "rust.primitive_slots"));
}

#[test]
fn named_groups_require_distinct_declarations_and_exact_syntax() {
    for count in [2, 3] {
        let source = (0..count)
            .map(|i| format!("struct A{i} {{x:i32,y:i32,z:i32,extra{i}:bool}} "))
            .collect::<String>();
        let workspace = Workspace::new(&source);
        assert_eq!(
            matched(&report(&workspace.check()), "rust.data_clumps"),
            count == 3
        );
    }
    let workspace = Workspace::new(
        "type Alias=i32;struct A{x:i32,y:i32,z:i32}struct B{x:Alias,y:Alias,z:Alias}struct C{x:i32,y:i32,z:i32}",
    );
    assert!(!matched(&report(&workspace.check()), "rust.data_clumps"));
}

#[test]
fn named_groups_do_not_count_constructors_of_a_supporting_struct() {
    let pair = "struct A{x:i32,y:i32,z:i32}struct B{x:i32,y:i32,z:i32}";
    for (constructor, expected) in [
        ("fn make(x:i32,y:i32,z:i32)->A{A{x,y,z}}", false),
        ("impl A{fn new(x:i32,y:i32,z:i32)->Self{A{x,y,z}}}", false),
        (
            "impl A{fn try_new(x:i32,y:i32,z:i32)->Result<Self,()>{Ok(A{x,y,z})}}",
            false,
        ),
        ("fn make(x:i32,y:i32,z:i32)->Option<A>{None}", false),
        ("fn make(x:i32,y:i32,z:i32)->i32{x}", true),
        (
            "struct C{x:i32,y:i32,z:i32}fn make(x:i32,y:i32,z:i32)->A{A{x,y,z}}",
            true,
        ),
    ] {
        let workspace = Workspace::new(&format!("{pair}{constructor}"));
        assert_eq!(
            matched(&report(&workspace.check()), "rust.data_clumps"),
            expected,
            "{constructor}"
        );
    }
}

#[test]
fn duplicate_similarity_compares_the_fraction_without_rounding() {
    let source =
        "fn a()->i32{let x=1;let y=x+2;let z=y*3;z} fn b(){let x=9;let y=x+8;let z=y*7;z;}";
    for (threshold, expected) in [(9444, true), (9445, false)] {
        let workspace = Workspace::new(source);
        workspace.modify(
            "/rules/rust.duplicate_functions/parameters/minimum_tokens",
            json!(20),
        );
        workspace.modify(
            "/rules/rust.duplicate_functions/parameters/minimum_similarity_basis_points",
            json!(threshold),
        );
        let data = report(&workspace.check());
        assert_eq!(matched(&data, "rust.duplicate_functions"), expected);
        if expected {
            let finding = finding(&data, "rust.duplicate_functions", "::a");
            assert_eq!(
                finding["evaluation"]["observed"],
                json!({"intersection":17,"union":18})
            );
            assert_eq!(finding["related_locations"].as_array().unwrap().len(), 1);
            assert!(finding.get("related_source_excerpts").is_none());
            let measurement = finding_measurement(&data, finding);
            assert_eq!(
                measurement["related_locations"].as_array().unwrap().len(),
                1
            );
        }
    }
    let workspace = Workspace::new(source);
    workspace.modify(
        "/rules/rust.duplicate_functions/parameters/minimum_tokens",
        json!(21),
    );
    assert!(!matched(
        &report(&workspace.check()),
        "rust.duplicate_functions"
    ));
}

#[test]
fn rust_duplicate_budget_counts_only_exactly_pruned_candidates() {
    let mut source = String::new();
    for index in 0..450 {
        source.push_str(&format!("fn tiny_{index}()->i32{{{index}}}"));
    }
    let body = "let first=value+1;let second=first*2;let third=second-3;let fourth=third/4;fourth";
    source.push_str(&format!(
        "fn duplicate_a(value:i32)->i32{{{body}}}fn duplicate_b(item:i32)->i32{{{body}}}"
    ));
    let workspace = Workspace::new(&source);
    workspace.modify(
        "/rules/rust.duplicate_functions/parameters/minimum_tokens",
        json!(20),
    );
    workspace.modify("/limits/maximum_pairs", json!(1));
    workspace.modify(
        "/rules/rust.duplicate_functions/parameters/minimum_similarity_basis_points",
        json!(10_000),
    );
    let output = workspace.check();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    assert!(data["errors"].as_array().unwrap().is_empty());
    let duplicate = finding(&data, "rust.duplicate_functions", "::duplicate_a");
    assert_eq!(
        duplicate["related_symbols"],
        json!(["src/lib.rs::duplicate_b"])
    );
}

#[test]
fn rust_duplicate_budget_still_fails_closed_for_too_many_exact_candidates() {
    let body = "let first=value+1;let second=first*2;let third=second-3;let fourth=third/4;fourth";
    let workspace = Workspace::new(&format!(
        "fn first(value:i32)->i32{{{body}}}fn second(value:i32)->i32{{{body}}}fn third(value:i32)->i32{{{body}}}"
    ));
    workspace.modify("/limits/maximum_pairs", json!(1));
    workspace.modify(
        "/rules/rust.duplicate_functions/parameters/minimum_similarity_basis_points",
        json!(10_000),
    );
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        report(&output)["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| {
                error
                    == "rule rust.duplicate_functions: maximum_pairs budget exceeded: charged 2 exact comparisons; policy limit is 1"
            })
    );
}

#[test]
fn overlapping_nested_bodies_are_not_reported_as_duplicate_occurrences() {
    let repeated = "x+=1;".repeat(40);
    let workspace = Workspace::new(&format!(
        "fn outer(){{fn nested()->i32{{let mut x=0;{repeated}x}}let _=nested();}}"
    ));
    assert!(!matched(
        &report(&workspace.check()),
        "rust.duplicate_functions"
    ));
}

#[test]
fn alternative_interfaces_need_distinct_owners_and_different_interfaces() {
    let body = "let y=x+1;let z=y*2;let w=z-3;w+4";
    for (name, expected) in [("a", false), ("b", true)] {
        let workspace = Workspace::new(&format!(
            "struct A;struct B;impl A{{fn a(&self,x:i32)->i32{{{body}}}}}impl B{{fn {name}(&self,x:i32)->i32{{{body}}}}}"
        ));
        assert_eq!(
            matched(&report(&workspace.check()), "rust.alternative_interfaces"),
            expected
        );
    }
}

#[test]
fn temporary_fields_count_distinct_receivers_not_static_functions() {
    for (reads, expected) in [(1, true), (2, false)] {
        let methods = (0..4)
            .map(|i| {
                if i < reads {
                    format!("fn f{i}(&self){{let _=self.a;let _=self.b;let _=self.c;}} ")
                } else {
                    format!("fn f{i}(&self){{}} ")
                }
            })
            .collect::<String>();
        let workspace = Workspace::new(&format!(
            "struct A{{a:Option<i32>,b:Option<i32>,c:Option<i32>}}impl A{{{methods}fn static_a(){{}}fn static_b(){{}}}}"
        ));
        assert_eq!(
            matched(&report(&workspace.check()), "rust.temporary_fields"),
            expected
        );
    }
}

#[test]
fn forwarding_share_has_a_method_minimum_and_excludes_additional_behavior() {
    for (methods, forward, expected) in [(4, 4, false), (5, 4, true), (5, 3, false)] {
        let functions = (0..methods)
            .map(|i| {
                if i < forward {
                    format!("fn f{i}(&self,x:i32)->i32{{self.backend.send(x)}}")
                } else {
                    format!("fn f{i}(&self,x:i32)->i32{{self.backend.send(x)+1}}")
                }
            })
            .collect::<String>();
        let workspace = Workspace::new(&format!(
            "struct Backend;struct A{{backend:Backend}}impl A{{{functions}}}"
        ));
        assert_eq!(
            matched(&report(&workspace.check()), "rust.forwarding_share"),
            expected
        );
    }
    let unrelated = Workspace::new(
        "fn calculate(x:i32)->i32{x+1}struct A;impl A{fn a(&self,x:i32)->i32{calculate(x)}fn b(&self,x:i32)->i32{calculate(x)}fn c(&self,x:i32)->i32{calculate(x)}fn d(&self,x:i32)->i32{calculate(x)}fn e(&self,x:i32)->i32{calculate(x)}}",
    );
    assert!(!matched(
        &report(&unrelated.check()),
        "rust.forwarding_share"
    ));
    let ufcs = Workspace::new(
        "struct Backend;impl Backend{fn send(_: &Backend,x:i32)->i32{x}}struct A{backend:Backend}impl A{fn a(&self,x:i32)->i32{Backend::send(&self.backend,x)}fn b(&self,x:i32)->i32{Backend::send(&self.backend,x)}fn c(&self,x:i32)->i32{Backend::send(&self.backend,x)}fn d(&self,x:i32)->i32{Backend::send(&self.backend,x)}fn e(&self,x:i32)->i32{Backend::send(&self.backend,x)}}",
    );
    assert!(matched(&report(&ufcs.check()), "rust.forwarding_share"));
}

#[test]
fn data_class_accessors_are_narrow_and_constructors_are_operations() {
    for (methods, expected) in [
        ("fn x(&self)->i32{self.x}fn y(&self)->&i32{&self.y}", true),
        ("fn x(&self)->i32{self.x+1}", false),
        ("fn new(x:i32,y:i32)->Self{Self{x,y}}", false),
    ] {
        let workspace = Workspace::new(&format!("struct A{{x:i32,y:i32}}impl A{{{methods}}}"));
        assert_eq!(
            matched(&report(&workspace.check()), "rust.data_class"),
            expected
        );
    }
}

#[test]
fn tiny_type_indicator_requires_all_three_limits() {
    for (fields, functions, lines, expected) in [
        (1, 1, 5, true),
        (2, 1, 5, false),
        (1, 2, 5, false),
        (1, 1, 6, false),
    ] {
        let fields = (0..fields)
            .map(|i| format!("f{i}:i32"))
            .collect::<Vec<_>>()
            .join(",");
        let methods = (0..functions)
            .map(|i| format!("fn f{i}(&self){{\n{}}}", "let _=0;\n".repeat(lines)))
            .collect::<String>();
        let workspace = Workspace::new(&format!("struct A{{{fields}}}impl A{{{methods}}}"));
        assert_eq!(
            matched(&report(&workspace.check()), "rust.lazy_class"),
            expected
        );
    }
    for enumeration in ["enum A{X}", "enum A{X(i32),Y}"] {
        let workspace = Workspace::new(enumeration);
        assert!(!matched(&report(&workspace.check()), "rust.lazy_class"));
    }
}

#[test]
fn comment_share_obeys_both_exact_share_and_code_minimum() {
    for (comments, code, expected) in [(9, 21, true), (8, 21, false), (9, 19, false)] {
        let workspace = Workspace::new(&format!(
            "fn f(){{\n{}{}}}",
            "// why\n".repeat(comments),
            "let _=0;\n".repeat(code)
        ));
        assert_eq!(
            matched(&report(&workspace.check()), "rust.comment_share"),
            expected
        );
    }
}

#[test]
fn repeated_dispatch_counts_callables_not_repeated_matches_in_one_body() {
    let dispatch = "match value{E::A=>1,E::B=>2,E::C=>3,E::D=>4};";
    for (functions, expected) in [(2, false), (3, true)] {
        let methods = (0..functions)
            .map(|i| format!("fn f{i}(value:E){{{dispatch}{dispatch}}}"))
            .collect::<String>();
        let workspace = Workspace::new(&format!("enum E{{A,B,C,D}}{methods}"));
        assert_eq!(
            matched(&report(&workspace.check()), "rust.repeated_dispatch"),
            expected
        );
    }
}

#[test]
fn repeated_dispatch_supports_self_and_requires_real_declared_variants() {
    let valid = Workspace::new(
        "enum E{A,B,C,D}impl E{fn a(&self){match self{Self::A=>(),Self::B=>(),Self::C=>(),Self::D=>()}}fn b(&self){match self{Self::A=>(),Self::B=>(),Self::C=>(),Self::D=>()}}fn c(&self){match self{Self::A=>(),Self::B=>(),Self::C=>(),Self::D=>()}}}",
    );
    assert!(matched(&report(&valid.check()), "rust.repeated_dispatch"));
    let invalid = Workspace::new(
        "enum E{A,B,C,D}fn a(x:E){match x{E::A=>(),E::B=>(),E::C=>(),E::Missing=>()}}fn b(x:E){match x{E::A=>(),E::B=>(),E::C=>(),E::Missing=>()}}fn c(x:E){match x{E::A=>(),E::B=>(),E::C=>(),E::Missing=>()}}",
    );
    assert!(!matched(
        &report(&invalid.check()),
        "rust.repeated_dispatch"
    ));
}

#[test]
fn nested_named_functions_are_measured_once_at_every_depth() {
    let workspace = Workspace::new("fn outer(){fn middle(){fn inner(){} inner();} middle();}");
    let data = report(&workspace.check());
    assert_eq!(
        data["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["rule_id"] == "rust.function_lines")
            .count(),
        3
    );
}

#[test]
fn built_in_rust_callable_population_matches_body_metrics_and_excludes_required_methods() {
    let workspace = Workspace::new(
        "pub trait Port {\n\
             fn required<T>(&self);\n\
             fn defaulted(&self) { fn nested_trait() { if true {} } nested_trait(); }\n\
         }\n\
         pub struct Owner;\n\
         impl Owner {\n\
             pub fn method(&self) { fn nested_method() { if true {} } nested_method(); }\n\
         }\n\
         pub struct Foo;\n\
         pub fn value(_items: Vec<Foo>) {}\n\
         mod first { pub fn duplicate() { if true {} } }\n\
         mod second { pub fn duplicate() { if true {} } }\n\
         pub fn outer() { fn nested_free() { if true {} } nested_free(); }\n",
    );
    workspace.git(&["init", "-q"]);
    let output = workspace.command(&[
        "check",
        "--path",
        ".",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = report(&output);
    let locations = |rule: &str| {
        data["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["rule_id"] == rule)
            .map(|finding| {
                (
                    finding["location"]["line"].as_u64().unwrap(),
                    finding["location"]["column"].as_u64().unwrap(),
                )
            })
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        locations("rust.function_crap"),
        locations("rust.function_lines")
    );
    for name in ["nested_trait", "nested_method", "nested_free"] {
        assert!(
            data["measurements"]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| {
                    finding["rule_id"] == "rust.function_crap"
                        && finding["symbol"]
                            .as_str()
                            .is_some_and(|symbol| symbol.contains(name))
                })
        );
    }
    assert!(
        data["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| {
                finding["rule_id"] != "rust.function_crap"
                    || !finding["symbol"]
                        .as_str()
                        .is_some_and(|symbol| symbol.contains("required"))
            })
    );
    assert_eq!(
        finding(
            &data,
            "rust.unused_type_parameters",
            "src/lib.rs::Port.required"
        )["evaluation"]["matched"],
        true
    );
    assert!(data["findings"].as_array().unwrap().iter().all(|finding| {
        finding["rule_id"] != "rust.unused_type_parameters"
            || !finding["symbol"].as_str().is_some_and(|symbol| {
                symbol.ends_with("src/lib.rs::Port") || symbol.ends_with("src/lib.rs::value")
            })
    }));
    let duplicate_symbols = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| {
            finding["rule_id"] == "rust.function_crap"
                && finding["symbol"]
                    .as_str()
                    .is_some_and(|symbol| symbol.ends_with("::duplicate"))
        })
        .map(|finding| finding["symbol"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        duplicate_symbols,
        BTreeSet::from([
            "src/lib.rs::first::duplicate",
            "src/lib.rs::second::duplicate"
        ])
    );
    assert_eq!(
        finding(&data, "rust.type_functions", "::Owner")["evaluation"]["observed"],
        1
    );
}

#[test]
fn every_registered_contract_links_to_its_matching_definition() {
    let output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .arg("rules")
        .output()
        .unwrap();
    let data = report(&output);
    let contracts = include_str!("../docs/rust-rule-contracts.md");
    for rule in data["rules"].as_array().unwrap() {
        assert!(contracts.contains(&format!("id=\"{}\"", rule["contract"].as_str().unwrap())));
    }
}

#[test]
fn modern_raw_c_strings_do_not_turn_literal_contents_into_comments() {
    let workspace = Workspace::new(
        "fn f(){\nlet _ = cr#\"an unescaped \" quote\n// literal, not comment\n\"#;\n}",
    );
    let output = workspace.check();
    let data = report(&output);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        finding(&data, "rust.function_lines", "::f")["evaluation"]["observed"],
        3
    );
    assert_eq!(
        finding(&data, "rust.comment_share", "::f")["evaluation"]["observed"],
        json!({"code_lines":3,"comment_lines":0})
    );
}

#[test]
fn opt_in_scan_metrics_explain_the_exact_join_without_changing_the_report() {
    let workspace = Workspace::new(
        "fn first(value:i32)->i32{let next=value+1;next*2}\nfn second(item:i32)->i32{let next=item+1;next*2}",
    );
    workspace.modify(
        "/rules/rust.duplicate_functions/parameters/minimum_tokens",
        json!(4),
    );
    let metrics_path = workspace.path.join("scan-metrics.json");
    let output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--path",
            ".",
            "--policy",
            "quality-policy.json",
            "--only-group",
            "source",
            "--format",
            "json",
        ])
        .env("SMELLS_METRICS_FILE", &metrics_path)
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _: Value = serde_json::from_slice(&output.stdout).expect("unchanged report JSON");
    let metrics: Value =
        serde_json::from_slice(&fs::read(metrics_path).expect("metrics file")).unwrap();
    assert_eq!(metrics["schema_version"], 2);
    assert_eq!(metrics["files"], 1);
    assert_eq!(metrics["functions"], 2);
    assert_eq!(metrics["json_bytes"], output.stdout.len());
    assert_eq!(metrics["syntax_nodes"], 0);
    assert!(metrics["syntax_tokens"].as_u64().unwrap() > 0);
    assert!(metrics["normalized_tokens"].as_u64().unwrap() > 0);
    assert!(metrics["eligible_fingerprints"].as_u64().unwrap() > 0);
    assert!(
        metrics["unique_candidates"].as_u64().unwrap()
            + metrics["candidates_rejected_by_length"].as_u64().unwrap()
            + metrics["candidates_rejected_by_position"].as_u64().unwrap()
            > 0
    );
    assert!(metrics["exact_comparisons_charged"].as_u64().unwrap() > 0);
    assert!(metrics["peak_resident_bytes"].as_u64().unwrap() > 1);
}

fn init_git_repo(root: &std::path::Path) {
    for args in [
        &["init", "-q"][..],
        &["config", "user.email", "test@example.com"][..],
        &["config", "user.name", "Test User"][..],
    ] {
        let out = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?} failed");
    }
}

fn git_stage_and_commit(root: &std::path::Path, message: &str) {
    let add = Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(add.status.success(), "git add failed: {add:?}");
    let commit = Command::new("git")
        .args(["commit", "-q", "-m", message])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(commit.status.success(), "git commit failed: {commit:?}");
}

#[test]
fn git_reference_ratchet_passes_legacy_blocks_regression_and_records_burndown() {
    let workspace = Workspace::new(
        "pub fn legacy(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> i32 { a + b + c + d + e + f + g + h }\n",
    );
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "initial legacy debt");

    // 1. Touch unrelated clean function: legacy debt fits inside HEAD's Debt Envelope -> exit 0
    fs::write(
        workspace.path.join("src/lib.rs"),
        "pub fn legacy(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> i32 { a + b + c + d + e + f + g + h }\npub fn helper(x: i32) -> i32 { x + 1 }\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&workspace.path)
            .status()
            .unwrap()
            .success()
    );
    let log_path = workspace.path.join("findings.log");
    let pass_out = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
            "--log",
            log_path.to_str().unwrap(),
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(pass_out.status.code(), Some(0));
    let pass_data = report(&pass_out);
    assert_eq!(pass_data["ratchet"]["reference_ref"], "HEAD");
    assert_eq!(pass_data["ratchet"]["reference_blockers"], 1);
    assert_eq!(pass_data["ratchet"]["candidate_blockers"], 1);
    assert_eq!(pass_data["ratchet"]["regressed_blockers"], 0);
    assert_eq!(pass_data["ratchet"]["legacy_within_envelope"], 1);
    assert_eq!(pass_data["ratchet"]["burndown_delta"], 0);
    assert_eq!(pass_data["attestation"]["complete"], true);
    assert_eq!(pass_data["attestation"]["exit_code"], 0);
    assert_eq!(pass_data["attestation"]["ratchet_active"], true);
    assert_eq!(pass_data["attestation"]["rules_selected"], 28);
    assert_eq!(pass_data["attestation"]["rules_measured"], 28);
    assert_eq!(pass_data["attestation"]["rules_incomplete"], 0);
    assert_eq!(pass_data["attestation"]["history_complete"], true);
    assert_eq!(pass_data["attestation"]["suppressions_total"], 0);
    assert_eq!(
        pass_data["attestation"]["policy_sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    let log_text = fs::read_to_string(&log_path).unwrap();
    assert!(log_text.contains("base: HEAD | legacy: 1 | delta: +0 | new_ignores: 0"));

    // 2. Worsen legacy function from 8 -> 9 parameters -> exceeds Debt Envelope -> exit 1
    fs::write(
        workspace.path.join("src/lib.rs"),
        "pub fn legacy(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32, i: i32) -> i32 { a + b + c + d + e + f + g + h + i }\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&workspace.path)
            .status()
            .unwrap()
            .success()
    );
    let fail_out = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(fail_out.status.code(), Some(1));
    let fail_data = report(&fail_out);
    assert_eq!(fail_data["ratchet"]["regressed_blockers"], 1);
    assert_eq!(fail_data["attestation"]["exit_code"], 1);

    // 3. Fix legacy function -> burndown_delta == -1, exit 0
    fs::write(
        workspace.path.join("src/lib.rs"),
        "pub fn legacy(a: i32) -> i32 { a }\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&workspace.path)
            .status()
            .unwrap()
            .success()
    );
    let burn_out = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(burn_out.status.code(), Some(0));
    let burn_data = report(&burn_out);
    assert_eq!(burn_data["ratchet"]["reference_blockers"], 1);
    assert_eq!(burn_data["ratchet"]["candidate_blockers"], 0);
    assert_eq!(burn_data["ratchet"]["burndown_delta"], -1);

    // 4. Verify burndown summary appears on stdout in both table and compact --log modes
    let table_out = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "table",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(table_out.status.code(), Some(0));
    let table_stdout = String::from_utf8(table_out.stdout).unwrap();
    assert!(
        table_stdout.contains("base: HEAD | legacy: 0 | delta: -1 | new_ignores: 0"),
        "{table_stdout}"
    );

    let hook_out = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "table",
            "--log",
            log_path.to_str().unwrap(),
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(hook_out.status.code(), Some(0));
    let hook_stdout = String::from_utf8(hook_out.stdout).unwrap();
    assert!(
        hook_stdout.contains("base: HEAD | legacy: 0 | delta: -1 | new_ignores: 0"),
        "{hook_stdout}"
    );
}

fn staged_ratchet_against_head(root: &std::path::Path, source: &str) -> Value {
    fs::write(root.join("src/lib.rs"), source).unwrap();
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(root)
            .status()
            .unwrap()
            .success(),
        "git add failed"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    let data = report(&output);
    assert_eq!(
        output
            .status
            .code()
            .and_then(|code| u64::try_from(code).ok()),
        data["attestation"]["exit_code"].as_u64(),
        "the attested exit code must match the process exit code"
    );
    data
}

const DUPLICATED_BODY: &str = "let mut total = 0; for value in values { if *value > 10 { total += value * 2; } else if *value < 0 { total -= value; } else { total += 1; } } total";

fn duplicate_pair(alpha_prefix: &str, beta_prefix: &str) -> String {
    format!(
        "pub fn alpha(values: &[i32]) -> i32 {{ {alpha_prefix} {DUPLICATED_BODY} }}\npub fn beta(values: &[i32]) -> i32 {{ {beta_prefix} {DUPLICATED_BODY} }}\n"
    )
}

#[test]
fn git_reference_ratchet_reports_the_added_finding_when_burdens_tie() {
    let legacy = "pub fn legacy(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> i32 { a + b + c + d + e + f + g + h }\n";
    let workspace = Workspace::new(legacy);
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "initial legacy debt");

    // `src/added.rs` sorts before `src/lib.rs`, so a path-order tie-break would
    // blame the legacy function instead of the one this change adds.
    fs::write(
        workspace.path.join("src/added.rs"),
        "pub fn added(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8, h: u8) -> u8 { a ^ b ^ c ^ d ^ e ^ f ^ g ^ h }\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&workspace.path)
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    assert_eq!(data["ratchet"]["regressed_blockers"], 1);
    let arguments: Vec<_> = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["rule_id"] == "rust.function_arguments")
        .map(|finding| {
            (
                finding["location"]["path"].as_str().unwrap(),
                finding["status"].as_str().unwrap(),
                finding["blocking"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        arguments,
        vec![
            ("src/added.rs", "violation", true),
            ("src/lib.rs", "legacy_within_envelope", false),
        ]
    );
}

#[test]
fn git_reference_ratchet_gates_duplicate_code_by_shared_token_mass() {
    let workspace = Workspace::new(&duplicate_pair("", ""));
    workspace.modify("/rules/rust.duplicate_functions/mode", json!("required"));
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "legacy duplicate pair");

    // Diverging one copy grows the pair's union but not its shared tokens.
    let diverged =
        staged_ratchet_against_head(&workspace.path, &duplicate_pair("", "let scale = 3;"));
    let duplicate = finding(&diverged, "rust.duplicate_functions", "alpha");
    assert_eq!(
        duplicate["evaluation"]["observed"],
        json!({"intersection":45,"union":50})
    );
    assert_eq!(duplicate["status"], "legacy_within_envelope");
    assert_eq!(diverged["ratchet"]["regressed_blockers"], 0);
    assert_eq!(diverged["attestation"]["exit_code"], 0);

    // Growing both copies in lockstep adds duplicated mass and regresses.
    let grown = staged_ratchet_against_head(
        &workspace.path,
        &duplicate_pair("let scale = 3;", "let scale = 3;"),
    );
    assert_eq!(grown["ratchet"]["regressed_blockers"], 1);
    assert_eq!(grown["attestation"]["exit_code"], 1);

    // A newly pasted copy joins the family and raises its shared mass.
    let pasted = staged_ratchet_against_head(
        &workspace.path,
        &format!(
            "{}pub fn gamma(values: &[i32]) -> i32 {{ {DUPLICATED_BODY} }}\n",
            duplicate_pair("", "")
        ),
    );
    let family = finding(&pasted, "rust.duplicate_functions", "alpha");
    assert_eq!(family["related_symbols"].as_array().unwrap().len(), 2);
    assert_eq!(pasted["ratchet"]["reference_blockers"], 1);
    assert_eq!(pasted["ratchet"]["candidate_blockers"], 1);
    assert_eq!(pasted["ratchet"]["regressed_blockers"], 1);
    assert_eq!(pasted["attestation"]["exit_code"], 1);
}

#[test]
fn git_reference_ratchet_gates_dead_code_without_line_drift() {
    let workspace = Workspace::new("fn orphan() -> i32 { 1 }\npub fn used() -> i32 { 2 }\n");
    workspace.modify("/rules/rust.unused_code/mode", json!("required"));
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "legacy dead code");

    // Moving the legacy declaration does not turn it into new debt.
    let moved = staged_ratchet_against_head(
        &workspace.path,
        "pub fn used() -> i32 { 2 }\n\n\npub fn more() -> i32 { 3 }\nfn orphan() -> i32 { 1 }\n",
    );
    assert_eq!(moved["ratchet"]["legacy_within_envelope"], 1);
    assert_eq!(moved["attestation"]["exit_code"], 0);

    // Another unused private declaration in the same file raises the file's count.
    let added = staged_ratchet_against_head(
        &workspace.path,
        "fn orphan() -> i32 { 1 }\nfn stray() -> i32 { 3 }\npub fn used() -> i32 { 2 }\n",
    );
    let dead = finding(&added, "rust.unused_code", "unused-private-declarations");
    assert_eq!(dead["evaluation"]["observed"], 2);
    assert_eq!(added["ratchet"]["regressed_blockers"], 1);
    assert_eq!(added["attestation"]["exit_code"], 1);
}

#[test]
fn git_reference_ratchet_blocks_cross_implementation_debt_transfer() {
    let workspace = Workspace::new("pub fn root_ok() -> i32 { 1 }\n");
    fs::create_dir_all(workspace.path.join("pkg-a/src")).unwrap();
    fs::create_dir_all(workspace.path.join("pkg-b/src")).unwrap();
    fs::write(
        workspace.path.join("pkg-a/Cargo.toml"),
        "[package]\nname = \"pkg-a\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        workspace.path.join("pkg-b/Cargo.toml"),
        "[package]\nname = \"pkg-b\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        workspace.path.join("pkg-a/src/lib.rs"),
        "pub fn clean(a: i32) -> i32 { a }\n",
    )
    .unwrap();
    fs::write(
        workspace.path.join("pkg-b/src/lib.rs"),
        "pub fn legacy(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> i32 { a + b + c + d + e + f + g + h }\n",
    )
    .unwrap();
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "initial monorepo");

    // Fix legacy smell in pkg-b, but introduce an identical smell in pkg-a -> MUST block (exit 1)
    fs::write(
        workspace.path.join("pkg-b/src/lib.rs"),
        "pub fn legacy(a: i32) -> i32 { a }\n",
    )
    .unwrap();
    fs::write(
        workspace.path.join("pkg-a/src/lib.rs"),
        "pub fn moved(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> i32 { a + b + c + d + e + f + g + h }\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&workspace.path)
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    assert_eq!(data["ratchet"]["regressed_blockers"], 1);
    assert_eq!(data["ratchet"]["legacy_within_envelope"], 0);
}

#[test]
fn git_reference_ratchet_blocks_new_suppressions_and_policy_weakening() {
    let workspace = Workspace::new("fn clean(a: i32) -> i32 { a }\n");
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "clean baseline");

    // 1. Agent adds a new smell AND tries to hide it with an ignore directive -> blocked by suppression guardrail
    fs::write(
        workspace.path.join("src/lib.rs"),
        "// smells: ignore[rust.function_arguments] -- trying to bypass hook\nfn sneaked(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> i32 { a + b + c + d + e + f + g + h }\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&workspace.path)
            .status()
            .unwrap()
            .success()
    );
    let sup_out = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(sup_out.status.code(), Some(1));
    let sup_data = report(&sup_out);
    assert_eq!(
        sup_data["summary"]["verdict"],
        "blocked_by_suppression_growth"
    );
    assert_eq!(sup_data["ratchet"]["new_suppressions"], 1);
    assert_eq!(sup_data["ratchet"]["suppression_guardrail_triggered"], true);

    // 2. Agent restores clean code but relaxes a required rule threshold in policy -> blocked by policy weakening
    fs::write(
        workspace.path.join("src/lib.rs"),
        "fn clean(a: i32) -> i32 { a }\n",
    )
    .unwrap();
    workspace.modify(
        "/rules/rust.function_arguments/parameters/maximum",
        json!(12),
    );
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&workspace.path)
            .status()
            .unwrap()
            .success()
    );
    let pol_out = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(pol_out.status.code(), Some(1));
    let pol_data = report(&pol_out);
    assert_eq!(
        pol_data["summary"]["verdict"],
        "blocked_by_policy_weakening"
    );
    assert_eq!(pol_data["ratchet"]["policy_changed"], true);
    assert_eq!(
        pol_data["ratchet"]["policy_weakened_rules"],
        json!(["rust.function_arguments"])
    );
}

#[test]
fn git_reference_ratchet_rejects_unknown_base_ref_and_supports_working_tree() {
    let workspace = Workspace::new("fn clean(a: i32) -> i32 { a }\n");
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "clean baseline");

    let bad_ref = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--path",
            ".",
            "--base",
            "refs/heads/does-not-exist",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(bad_ref.status.code(), Some(2));

    let wt_ok = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--path",
            ".",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(wt_ok.status.code(), Some(0));
    let data = report(&wt_ok);
    assert_eq!(data["ratchet"]["reference_ref"], "HEAD");
    assert_eq!(data["attestation"]["ratchet_active"], true);
}

#[test]
fn git_reference_ratchet_preserves_existing_suppressions_and_fails_closed_on_reference_errors() {
    let workspace = Workspace::new(
        "// smells: ignore[rust.function_arguments] -- reviewed external API\nfn legacy(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> i32 { a + b + c + d + e + f + g + h }\n",
    );
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "baseline with one reviewed ignore");

    // Keeping the exact existing suppression passes (new_suppressions == 0)
    let keep_out = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(keep_out.status.code(), Some(0));
    let keep_data = report(&keep_out);
    assert_eq!(keep_data["ratchet"]["reference_suppressions"], 1);
    assert_eq!(keep_data["ratchet"]["candidate_suppressions"], 1);
    assert_eq!(keep_data["ratchet"]["new_suppressions"], 0);
    assert_eq!(
        keep_data["ratchet"]["suppression_guardrail_triggered"],
        false
    );
    assert_eq!(keep_data["attestation"]["suppressions_total"], 1);

    // Commit a broken file as HEAD so the reference snapshot itself has scan errors, then stage a clean fix.
    // Comparing against a broken reference must fail closed (exit 2) and report incomplete rules in attestation.
    fs::write(workspace.path.join("src/lib.rs"), "fn broken( {\n").unwrap();
    git_stage_and_commit(&workspace.path, "broken reference");
    fs::write(
        workspace.path.join("src/lib.rs"),
        "fn fixed() -> i32 { 1 }\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&workspace.path)
            .status()
            .unwrap()
            .success()
    );
    let broken_ref = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--staged",
            "--base",
            "HEAD",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert_eq!(broken_ref.status.code(), Some(2));
    let broken_data = report(&broken_ref);
    assert_eq!(broken_data["attestation"]["complete"], false);
    assert_eq!(broken_data["attestation"]["exit_code"], 2);
    assert!(
        broken_data["attestation"]["rules_incomplete"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(broken_data["errors"].as_array().unwrap().iter().any(|err| {
        err.as_str()
            .unwrap_or("")
            .contains("reference snapshot HEAD")
    }));
}

#[test]
fn rust_test_suite_source_role_reports_test_functions_by_role_and_helpers_as_test_code() {
    let long_body = "    let x = 1;\n".repeat(12);
    let test_file = format!(
        "#[test]\nfn long_integration_case() {{\n{long_body}}}\n\nfn long_helper() {{\n{long_body}}}\n"
    );
    let workspace = Workspace::new("pub fn ok() -> i32 { 1 }\n");
    fs::create_dir_all(workspace.path.join("tests")).unwrap();
    fs::write(workspace.path.join("tests/integration.rs"), test_file).unwrap();
    workspace.modify("/rules/rust.function_lines/parameters/maximum", json!(5));
    workspace.modify(
        "/source_roles",
        json!([{
            "role": "test_suite",
            "framework": "cargo_test",
            "paths": ["/tests/**/*.rs"]
        }]),
    );

    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    let test_finding = finding(&data, "rust.function_lines", "long_integration_case");
    assert_eq!(test_finding["status"], "reported_by_role");
    assert_eq!(test_finding["blocking"], false);
    assert_eq!(test_finding["role_context"]["source_role"], "test_suite");
    assert_eq!(
        test_finding["role_context"]["declaration_role"],
        "test_case"
    );
    assert_eq!(test_finding["role_context"]["framework"], "cargo_test");

    // A helper is not a test case, but it is still test code, so it never blocks.
    let helper_finding = finding(&data, "rust.function_lines", "long_helper");
    assert_eq!(helper_finding["status"], "indicator");
    assert_eq!(helper_finding["blocking"], false);
    assert!(helper_finding["role_context"].is_null());
    assert!(
        helper_finding["diagnostic"]["explanation"]
            .as_str()
            .unwrap()
            .contains("test code")
    );
}

fn doctor(workspace: &Workspace, extra: &[&str]) -> (Option<i32>, Value) {
    let mut args = vec![
        "doctor",
        "--policy",
        "quality-policy.json",
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    let output = workspace.command(&args);
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        output.status.code().map(i64::from),
        data["exit_code"].as_i64(),
        "the JSON exit code must match the process exit code"
    );
    (output.status.code(), data)
}

fn doctor_check<'a>(data: &'a Value, id: &str) -> &'a Value {
    data["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == id)
        .unwrap()
}

#[test]
fn doctor_reports_governance_gaps_as_warnings_on_a_complete_repository() {
    let workspace = Workspace::new("pub fn value() -> u8 { 1 }\n");
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "initial");
    let (code, data) = doctor(&workspace, &[]);
    assert_eq!(code, Some(0), "{data}");
    assert_eq!(data["status"], "checks_passed");
    assert_eq!(data["next_steps"], json!(["smells check"]), "{data}");
    assert_eq!(doctor_check(&data, "policy")["status"], "ok");
    assert_eq!(doctor_check(&data, "git_history")["status"], "ok");
    assert_eq!(doctor_check(&data, "suppressions")["status"], "ok");
    assert_eq!(doctor_check(&data, "coverage")["status"], "warn");
    assert_eq!(doctor_check(&data, "agents_contract")["status"], "warn");
    assert_eq!(doctor_check(&data, "codeowners")["status"], "warn");

    workspace.source("AGENTS.md", "# Review contract\n");
    workspace.source("CLAUDE.md", "@AGENTS.md\n");
    workspace.source(".github/CODEOWNERS", "*quality-policy.json @quality\n");
    workspace.source("lcov.info", "SF:src/lib.rs\nDA:1,1\nend_of_record\n");
    let (code, data) = doctor(&workspace, &["--path", "."]);
    assert_eq!(code, Some(0));
    for check in data["checks"].as_array().unwrap() {
        assert_eq!(check["status"], "ok", "{check}");
    }
    assert_eq!(
        doctor_check(&data, "codeowners")["detail"],
        ".github/CODEOWNERS assigns quality-policy.json to @quality"
    );
    let table = workspace.command(&["doctor", "--policy", "quality-policy.json"]);
    assert_eq!(table.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&table.stdout);
    assert!(
        stdout.contains("Setup & Governance\n  ✓ policy"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("\x1b["),
        "piped output stays plain: {stdout}"
    );
}

#[test]
fn doctor_fails_closed_on_missing_history_invalid_policy_and_bad_arguments() {
    let workspace = Workspace::new("pub fn value() -> u8 { 1 }\n");
    let (code, data) = doctor(&workspace, &[]);
    assert_eq!(code, Some(2));
    assert_eq!(data["status"], "checks_failed");
    assert_eq!(data["next_steps"], json!([]), "{data}");
    assert_eq!(doctor_check(&data, "git_history")["status"], "error");

    workspace.modify("/scanner_version", json!("0.0.0"));
    let (code, data) = doctor(&workspace, &[]);
    assert_eq!(code, Some(2));
    assert_eq!(doctor_check(&data, "policy")["status"], "error");
    let ids: Vec<_> = data["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| check["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["policy", "agents_contract", "claude_import", "codeowners"]
    );

    for args in [
        &["doctor", "--policy"][..],
        &[
            "doctor",
            "--policy",
            "quality-policy.json",
            "--format",
            "yaml",
        ][..],
        &["doctor", "--policy", "a", "--policy", "b"][..],
        &["doctor", "--policy", "quality-policy.json", "--staged", "x"][..],
    ] {
        let output = workspace.command(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn doctor_works_with_zero_flags_and_auto_discovers_policy_or_starter_and_default_base() {
    let workspace = Workspace::new("fn dead_helper() -> u8 { 1 }\n");
    workspace.source(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "initial");

    // 1. Zero flags with a checked-in quality-policy.json auto-discovers it.
    let output = workspace.command(&["doctor", "--format", "json"]);
    assert_eq!(output.status.code(), Some(0));
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(doctor_check(&data, "policy")["status"], "ok");
    assert_eq!(data["diagnostics"]["blocking_findings"], 1);
    assert!(data["burndown"].is_null());

    // 2. When origin/main exists, zero-flag doctor automatically reports burndown against it.
    workspace.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    workspace.source("src/lib.rs", "pub fn healthy() -> u8 { 1 }\n");
    let output = workspace.command(&["doctor", "--format", "json"]);
    assert_eq!(output.status.code(), Some(0));
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(data["burndown"]["base_ref"], "origin/main");
    assert_eq!(data["burndown"]["blocking_delta"], -1);
    assert_eq!(data["burndown"]["score_delta"], 10);

    // 3. Removing quality-policy.json falls back to the built-in Rust starter with a warning.
    fs::remove_file(workspace.path.join("quality-policy.json")).unwrap();
    let output = workspace.command(&["doctor", "--format", "json"]);
    assert_eq!(output.status.code(), Some(0));
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(doctor_check(&data, "policy")["status"], "warn");
    assert!(
        doctor_check(&data, "policy")["detail"]
            .as_str()
            .unwrap()
            .contains("built-in rust-v1 starter policy")
    );
    assert_eq!(data["diagnostics"]["score"], 100);
    assert_eq!(data["burndown"]["base_ref"], "origin/main");

    // 4. Multiple policy files without --policy fail closed.
    workspace.source("quality-policy.json", "{}");
    workspace.source("python-quality-policy.json", "{}");
    let output = workspace.command(&["doctor", "--format", "json"]);
    assert_eq!(output.status.code(), Some(2));
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(doctor_check(&data, "policy")["status"], "error");
    assert!(
        doctor_check(&data, "policy")["detail"]
            .as_str()
            .unwrap()
            .contains("multiple policy files found")
    );
}

#[test]
fn doctor_reports_compact_smell_diagnostics_and_fails_closed_on_scan_errors() {
    let workspace = Workspace::new(
        "pub struct Payload { pub a: u8, pub b: u8 }\n\
         fn dead_helper() -> u8 { 1 }\n",
    );
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "initial smells");

    let (code, data) = doctor(&workspace, &[]);
    assert_eq!(code, Some(0), "{data}");
    assert_eq!(data["status"], "checks_passed");
    assert_eq!(data["diagnostics"]["scanned_files"], 1);
    assert_eq!(data["diagnostics"]["blocking_findings"], 1);
    assert!(data["diagnostics"]["review_signals"].as_u64().unwrap() >= 1);
    assert_eq!(data["diagnostics"]["smells"][0]["smell"], "Dead Code");
    assert_eq!(data["diagnostics"]["smells"][0]["blocking"], 1);
    assert_eq!(data["diagnostics"]["hotspots"][0]["path"], "src/lib.rs");

    let table = workspace.command(&["doctor", "--policy", "quality-policy.json"]);
    assert_eq!(table.status.code(), Some(0));
    let stdout = String::from_utf8(table.stdout).unwrap();
    assert!(stdout.contains("Codebase Health\n"), "{stdout}");
    assert!(
        stdout.contains("1 files · 1 blocking · 0 ignored"),
        "{stdout}"
    );
    assert!(stdout.contains("Dead Code"), "{stdout}");
    assert!(stdout.contains("Top Hotspot Files\n"), "{stdout}");
    assert!(stdout.contains("src/lib.rs"), "{stdout}");

    // CLICOLOR_FORCE emits ANSI styling; NO_COLOR always disables it.
    let colored = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args(["doctor", "--policy", "quality-policy.json"])
        .current_dir(&workspace.path)
        .env("CLICOLOR_FORCE", "1")
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    assert_eq!(colored.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&colored.stdout).contains("\x1b["));

    let no_color = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args(["doctor", "--policy", "quality-policy.json"])
        .current_dir(&workspace.path)
        .env("CLICOLOR_FORCE", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(no_color.status.code(), Some(0));
    assert!(!String::from_utf8_lossy(&no_color.stdout).contains("\x1b["));

    // Paying down the dead helper shows burndown against HEAD in both JSON and table output.
    workspace.source(
        "src/lib.rs",
        "pub struct Payload { pub a: u8, pub b: u8 }\n",
    );
    let (code, burned) = doctor(&workspace, &["--base", "HEAD"]);
    assert_eq!(code, Some(0), "{burned}");
    assert_eq!(burned["burndown"]["base_ref"], "HEAD");
    assert_eq!(burned["burndown"]["base_blocking"], 1);
    assert_eq!(burned["burndown"]["current_blocking"], 0);
    assert_eq!(burned["burndown"]["blocking_delta"], -1);
    assert_eq!(burned["burndown"]["score_delta"], 10);
    assert_eq!(burned["burndown"]["smell_deltas"][0]["smell"], "Dead Code");
    assert_eq!(burned["burndown"]["smell_deltas"][0]["blocking_delta"], -1);

    let burn_table = workspace.command(&[
        "doctor",
        "--policy",
        "quality-policy.json",
        "--base",
        "HEAD",
    ]);
    assert_eq!(burn_table.status.code(), Some(0));
    let burn_stdout = String::from_utf8(burn_table.stdout).unwrap();
    assert!(
        burn_stdout.contains("Burndown since HEAD ("),
        "{burn_stdout}"
    );
    assert!(
        burn_stdout.contains("Blockers: 1 -> 0 (-1)"),
        "{burn_stdout}"
    );
    assert!(
        burn_stdout.contains("Dead Code     1        0     -1      +0"),
        "{burn_stdout}"
    );

    // Unknown --base ref fails closed with exit 2.
    let (code, bad_base) = doctor(&workspace, &["--base", "refs/heads/does-not-exist"]);
    assert_eq!(code, Some(2), "{bad_base}");
    assert_eq!(doctor_check(&bad_base, "base_ref")["status"], "error");

    // Parser error during diagnostic scan fails closed with exit 2.
    workspace.source("src/lib.rs", "pub fn broken( {\n");
    let (code, data) = doctor(&workspace, &[]);
    assert_eq!(code, Some(2), "{data}");
    assert_eq!(data["status"], "checks_failed");
    assert_eq!(doctor_check(&data, "scan")["status"], "error");
    assert_eq!(data["diagnostics"]["grade"], "Incomplete", "{data}");
    let steps = data["next_steps"].as_array().unwrap();
    assert_eq!(steps.len(), 1, "{data}");
    assert!(
        steps[0]
            .as_str()
            .unwrap()
            .starts_with("fix the errors above"),
        "{data}"
    );

    // A base commit with a parser error also fails closed with exit 2.
    git_stage_and_commit(&workspace.path, "broken base");
    workspace.source("src/lib.rs", "pub fn healthy() -> u8 { 1 }\n");
    let (code, broken_base) = doctor(&workspace, &["--base", "HEAD"]);
    assert_eq!(code, Some(2), "{broken_base}");
    assert_eq!(doctor_check(&broken_base, "base_ref")["status"], "error");
}

#[test]
fn policy_defaults_to_quality_policy_json_in_the_scanned_directory() {
    let workspace = Workspace::new("fn concise() {}");
    let explicit = workspace.check();
    let implicit = workspace.command(&[
        "check",
        "--path",
        ".",
        "--only-group",
        "source",
        "--format",
        "json",
    ]);
    assert_eq!(
        implicit.status.code(),
        explicit.status.code(),
        "{implicit:?}"
    );
    assert_eq!(report(&implicit)["findings"], report(&explicit)["findings"]);

    fs::create_dir(workspace.path.join("nested")).unwrap();
    fs::rename(
        workspace.path.join("quality-policy.json"),
        workspace.path.join("nested/quality-policy.json"),
    )
    .unwrap();
    workspace.source("nested/src/lib.rs", "fn concise() {}");
    let nested = workspace.command(&["check", "--path", "nested", "--only-group", "source"]);
    assert_eq!(nested.status.code(), Some(0), "{nested:?}");

    for args in [
        &["contracts", "validate"][..],
        &["policy", "show"][..],
        &["check", "--path", ".", "--only-group", "source"][..],
    ] {
        let output = workspace.command(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("no --policy given and ./quality-policy.json not found"),
            "{args:?}: {output:?}"
        );
    }
}

#[test]
fn crap_warns_from_six_and_blocks_only_measured_scores_above_ten() {
    let workspace = Workspace::new(
        "pub fn pick(a: bool, b: bool) -> u8 {\n    if a {\n        1\n    } else if b {\n        2\n    } else {\n        3\n    }\n}\n",
    );
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "initial");
    let lcov = |hits: [u8; 9]| {
        let lines = hits
            .iter()
            .enumerate()
            .map(|(index, hit)| format!("DA:{},{hit}\n", index + 1))
            .collect::<String>();
        workspace.source(
            "lcov.info",
            &format!("SF:src/lib.rs\n{lines}end_of_record\n"),
        );
    };
    let check = |coverage: bool| {
        let mut args = vec!["check", "--path", ".", "--policy", "quality-policy.json"];
        if coverage {
            args.extend(["--coverage", "lcov.info"]);
        }
        args.extend(["--format", "json"]);
        let output = workspace.command(&args);
        (output.status.code(), report(&output))
    };
    let crap = |data: &Value| {
        let found = finding(data, "rust.function_crap", "::pick");
        (
            found["status"].as_str().unwrap().to_owned(),
            found["blocking"].as_bool().unwrap(),
            found["diagnostic"]["explanation"]
                .as_str()
                .unwrap()
                .to_owned(),
        )
    };

    let (code, data) = check(false);
    let (status, blocking, explanation) = crap(&data);
    assert_eq!(
        (code, status.as_str(), blocking),
        (Some(0), "indicator", false)
    );
    assert!(explanation.contains("assumes 0% coverage"), "{explanation}");

    lcov([1, 1, 1, 0, 0, 0, 0, 0, 0]);
    let (code, data) = check(true);
    let (status, blocking, explanation) = crap(&data);
    assert_eq!(
        (code, status.as_str(), blocking),
        (Some(0), "indicator", false)
    );
    assert!(explanation.contains("warning band"), "{explanation}");

    lcov([0; 9]);
    let (code, data) = check(true);
    let (status, blocking, _) = crap(&data);
    assert_eq!(
        (code, status.as_str(), blocking),
        (Some(1), "violation", true)
    );

    lcov([1; 9]);
    let (code, data) = check(true);
    assert_eq!(code, Some(0));
    assert!(!matched(&data, "rust.function_crap"));
}

#[test]
fn coverage_replaces_estimated_crap_and_rejects_stale_artifacts() {
    let workspace = Workspace::new(
        "pub fn pick(ready: bool) -> u8 {\n    if ready {\n        1\n    } else {\n        2\n    }\n}\n",
    );
    init_git_repo(&workspace.path);
    git_stage_and_commit(&workspace.path, "initial");
    let check = |extra: &[&str]| {
        let mut args = vec![
            "check",
            "--path",
            ".",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ];
        args.extend_from_slice(extra);
        workspace.command(&args)
    };

    let estimated = check(&[]);
    assert_eq!(estimated.status.code(), Some(0));
    let estimated = report(&estimated);
    let crap = finding(&estimated, "rust.function_crap", "::pick");
    assert_eq!(
        crap["evaluation"]["observed"],
        json!({"numerator": 6, "denominator": 1})
    );
    let (code, data) = doctor(&workspace, &[]);
    assert_eq!(code, Some(0), "{data}");
    let coverage = doctor_check(&data, "coverage");
    assert_eq!(coverage["status"], "warn");
    let detail = coverage["detail"].as_str().unwrap();
    assert!(
        detail.contains("hid 1 non-blocking CRAP estimate"),
        "{detail}"
    );
    assert!(detail.contains("cargo llvm-cov --lcov"), "{detail}");
    assert_eq!(data["diagnostics"]["estimated_crap_hidden"], 1);
    assert!(
        data["diagnostics"]["smells"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["smell"] != "Complex Untested Code")
    );

    workspace.source(
        "coverage/lcov.info",
        "SF:src/lib.rs\nDA:1,1\nDA:2,1\nDA:3,1\nDA:5,0\nend_of_record\n",
    );
    let measured = check(&["--coverage", "coverage/lcov.info"]);
    assert_eq!(measured.status.code(), Some(0));
    let measured = report(&measured);
    let crap = finding(&measured, "rust.function_crap", "::pick");
    assert_eq!(
        crap["evaluation"]["observed"],
        json!({"numerator": 58, "denominator": 27})
    );
    assert!(!matched(&measured, "rust.function_crap"));
    let found = check(&[]);
    assert_eq!(found.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&found.stderr).contains(
            "smells: reading coverage from ./coverage/lcov.info (found automatically; pass --coverage FILE to choose another)"
        ),
        "{found:?}"
    );
    let crap = finding(&report(&found), "rust.function_crap", "::pick").clone();
    assert_eq!(
        crap["evaluation"]["observed"],
        json!({"numerator": 58, "denominator": 27}),
        "check finds coverage without --coverage"
    );
    let based = check(&["--base", "HEAD"]);
    assert_eq!(based.status.code(), Some(0), "{based:?}");
    assert!(based.stderr.is_empty(), "{based:?}");
    let crap = finding(&report(&based), "rust.function_crap", "::pick").clone();
    assert_eq!(
        crap["evaluation"]["observed"],
        json!({"numerator": 6, "denominator": 1}),
        "--base scans skip discovered coverage the reference cannot match"
    );
    let (code, data) = doctor(&workspace, &[]);
    assert_eq!(code, Some(0), "{data}");
    let coverage = doctor_check(&data, "coverage");
    assert_eq!(coverage["status"], "ok", "{coverage}");
    assert!(
        coverage["detail"]
            .as_str()
            .unwrap()
            .starts_with("coverage/lcov.info covers 1 scanned file(s)")
    );
    assert_eq!(data["diagnostics"]["estimated_crap_hidden"], 0);

    workspace.source(
        "coverage/lcov.info",
        "SF:src/lib.rs\nDA:99,1\nend_of_record\n",
    );
    let (code, data) = doctor(&workspace, &[]);
    assert_eq!(code, Some(2), "{data}");
    let coverage = doctor_check(&data, "coverage");
    assert_eq!(coverage["status"], "error");
    assert!(
        coverage["detail"]
            .as_str()
            .unwrap()
            .contains("coverage is stale for src/lib.rs")
    );
    let stale = check(&["--coverage", "coverage/lcov.info"]);
    assert_eq!(stale.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&stale.stderr).contains("coverage is stale"));
    let missing = check(&["--coverage", "absent.info"]);
    assert_eq!(missing.status.code(), Some(2));
    let (code, data) = doctor(&workspace, &["--coverage", "absent.info"]);
    assert_eq!(code, Some(2), "{data}");
    assert!(
        doctor_check(&data, "coverage")["detail"]
            .as_str()
            .unwrap()
            .contains("cannot read coverage")
    );
}
