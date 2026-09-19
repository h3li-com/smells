use serde_json::{Map, Value, json};
use std::{
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
    fn new(file: &str, source: &str, policy: &Value) -> Self {
        let path = std::env::temp_dir().join(format!(
            "smells-multilanguage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create workspace");
        fs::write(path.join(file), source).expect("write source");
        fs::write(
            path.join("quality-policy.json"),
            serde_json::to_vec(policy).expect("serialize policy"),
        )
        .expect("write policy");
        Self { path }
    }

    fn check_path(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_smells"))
            .args([
                "check",
                "--path",
                ".",
                "--policy",
                "quality-policy.json",
                "--format",
                "json",
            ])
            .current_dir(&self.path)
            .output()
            .expect("run scanner")
    }

    fn stage(&self) {
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&self.path)
            .output()
            .expect("initialize Git repository");
        assert!(
            init.status.success(),
            "{}",
            String::from_utf8_lossy(&init.stderr)
        );
        let add = Command::new("git")
            .args(["add", "."])
            .current_dir(&self.path)
            .output()
            .expect("stage inputs");
        assert!(
            add.status.success(),
            "{}",
            String::from_utf8_lossy(&add.stderr)
        );
    }

    fn check_staged(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_smells"))
            .args([
                "check",
                "--staged",
                "--policy",
                "quality-policy.json",
                "--format",
                "json",
            ])
            .current_dir(&self.path)
            .output()
            .expect("run staged scanner")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("remove workspace");
    }
}

fn selection(mode: &str, parameters: Value) -> Value {
    json!({"version": 1, "mode": mode, "parameters": parameters})
}

fn portable_policy(language: &str) -> Value {
    let prefix = language;
    let mut rules = Map::new();
    let mut add = |name: &str, mode: &str, parameters: Value| {
        rules.insert(format!("{prefix}.{name}"), selection(mode, parameters));
    };
    add("function_lines", "required", json!({"maximum": 3}));
    add("function_arguments", "required", json!({"maximum": 3}));
    add("class_fields", "report", json!({"maximum": 15}));
    add("class_methods", "report", json!({"maximum": 20}));
    add("class_method_lines", "report", json!({"maximum": 500}));
    add(
        "primitive_slots",
        "off",
        json!({"minimum_raw_slots": 5, "minimum_share_percent": 80}),
    );
    add(
        "data_clumps",
        "report",
        json!({"minimum_group_size": 3, "minimum_declarations": 3}),
    );
    add(
        "alternative_interfaces",
        "off",
        json!({"minimum_similarity_basis_points": 8200, "minimum_tokens": 20}),
    );
    add(
        "repeated_dispatch",
        "off",
        json!({"minimum_sites": 3, "minimum_arms": 4}),
    );
    add(
        "temporary_fields",
        "off",
        json!({"minimum_fields": 3, "minimum_methods": 4, "maximum_use_percent": 25}),
    );
    add(
        "comment_share",
        "report",
        json!({"minimum_code_lines": 20, "minimum_share_percent": 30}),
    );
    add(
        "duplicate_functions",
        "report",
        json!({"minimum_similarity_basis_points": 8200, "minimum_tokens": 20}),
    );
    add(
        "data_class",
        "report",
        json!({"minimum_fields": 2, "maximum_operations": 0}),
    );
    add(
        "lazy_class",
        "report",
        json!({"maximum_fields": 1, "maximum_functions": 1, "maximum_lines": 5}),
    );
    add(
        "forwarding_share",
        "off",
        json!({"minimum_methods": 5, "minimum_share_percent": 80}),
    );
    add("function_crap", "off", json!({"maximum": 30}));
    add("unused_code", "off", json!({"maximum_findings": 0}));
    add(
        "unused_type_parameters",
        "off",
        json!({"maximum_findings": 0}),
    );
    add(
        "nominal_slot_contract",
        "off",
        json!({"maximum_mismatches": 0}),
    );
    add("port_conformance", "off", json!({"maximum_failures": 0}));
    add(
        "refused_bequest",
        "off",
        json!({"minimum_inherited_members": 3, "minimum_unused_percent": 80}),
    );
    add(
        "divergent_change",
        "off",
        json!({"minimum_changes": 3, "minimum_responsibilities": 3}),
    );
    add(
        "parallel_inheritance",
        "off",
        json!({"minimum_parallel_pairs": 3}),
    );
    add("shotgun_surgery", "off", json!({"maximum_owners": 3}));
    add(
        "foreign_accesses",
        "off",
        json!({"minimum_foreign_accesses": 5, "minimum_share_percent_exclusive": 60}),
    );
    add(
        "dependency_contract",
        "off",
        json!({"maximum_forbidden_accesses": 0}),
    );
    add(
        "library_capabilities",
        "off",
        json!({"maximum_failures": 0}),
    );
    add(
        "navigation_chains",
        "off",
        json!({"minimum_transitions": 3}),
    );
    json!({
        "schema_version": 2,
        "rule_pack": format!("{language}-v1"),
        "scanner_version": "0.1.0",
        "scope": "authored_source",
        "exclude_directories": [".git", "target", "node_modules", ".venv", "__pycache__"],
        "limits": {
            "maximum_files": 10000,
            "maximum_pairs": 100000,
            "maximum_group_combinations": 100000
        },
        "rules": rules,
        "exceptions": []
    })
}

fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON report ({error}): {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn require_maximum(policy: &mut Value, rule: &str, maximum: u64) {
    policy["rules"][rule] = selection("required", json!({"maximum": maximum}));
}

fn require(policy: &mut Value, rule: &str, parameters: Value) {
    policy["rules"][rule] = selection("required", parameters);
}

#[cfg(unix)]
#[test]
fn working_tree_skips_non_source_symlinks_without_following_them() {
    use std::os::unix::fs::symlink;

    let mut python_policy = portable_policy("python");
    python_policy["exclude_directories"] = json!([".git", "target"]);
    let python = Workspace::new("app.py", "def healthy():\n    return 1\n", &python_policy);
    fs::create_dir_all(python.path.join("target/external-package")).unwrap();
    fs::write(
        python.path.join("target/external-package/poison.py"),
        "def broken(",
    )
    .unwrap();
    fs::create_dir_all(python.path.join("frontend/.next/standalone/vendor")).unwrap();
    symlink(
        python.path.join("target/external-package"),
        python.path.join("frontend/.next/standalone/vendor/package"),
    )
    .unwrap();
    let output = python.check_path();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(report(&output)["scanned_files"], json!(["app.py"]));

    let mut typescript_policy = portable_policy("typescript");
    typescript_policy["exclude_directories"] = json!([".git", "target"]);
    let typescript = Workspace::new(
        "app.ts",
        "function healthy() { return 1; }\n",
        &typescript_policy,
    );
    fs::create_dir_all(typescript.path.join("target")).unwrap();
    fs::write(typescript.path.join("target/poison.ts"), "function broken(").unwrap();
    fs::create_dir_all(typescript.path.join("backend/environment/bin")).unwrap();
    symlink(
        typescript.path.join("target/poison.ts"),
        typescript.path.join("backend/environment/bin/python"),
    )
    .unwrap();
    let output = typescript.check_path();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(report(&output)["scanned_files"], json!(["app.ts"]));
}

#[cfg(unix)]
#[test]
fn working_tree_rejects_selected_source_symlinks_for_portable_languages() {
    use std::os::unix::fs::symlink;

    for (language, source, link) in [
        ("python", "app.py", "linked.py"),
        ("typescript", "app.ts", "linked.ts"),
    ] {
        let workspace = Workspace::new(source, "", &portable_policy(language));
        symlink(source, workspace.path.join(link)).unwrap();
        let output = workspace.check_path();
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("symlink in source corpus"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn example_policies_exclude_common_monorepo_caches() {
    let cases = [
        (
            "app.py",
            "def healthy():\n    return 1\n",
            serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap(),
            "frontend/.next/generated.py",
            "def broken(",
        ),
        (
            "app.ts",
            "function healthy() { return 1; }\n",
            serde_json::from_str(include_str!("../examples/typescript-quality-policy.json"))
                .unwrap(),
            "backend/.venv/generated.ts",
            "function broken(",
        ),
        (
            "lib.rs",
            "fn healthy() {}\n",
            serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap(),
            "frontend/node_modules/generated.rs",
            "fn broken(",
        ),
    ];

    for (source_path, source, policy, generated_path, generated) in cases {
        let workspace = Workspace::new(source_path, source, &policy);
        fs::create_dir_all(workspace.path.join(generated_path).parent().unwrap()).unwrap();
        fs::write(workspace.path.join(generated_path), generated).unwrap();
        let output = workspace.check_path();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            report(&output)["scanned_files"],
            json!([source_path]),
            "{generated_path} should be excluded"
        );
    }
}

#[test]
fn python_policy_scans_a_full_directory_through_the_public_cli() {
    let source = format!(
        "def overloaded(a, b, c, d):\n{}",
        "    value = 1\n".repeat(4)
    );
    let workspace = Workspace::new("app.py", &source, &portable_policy("python"));
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let data = report(&output);
    assert_eq!(data["rule_pack"], "python-v1");
    assert_eq!(data["language"], "python");
    assert_eq!(data["scanned_files"], json!(["app.py"]));
    for (rule, observed, threshold) in [
        ("python.function_lines", 4, 3),
        ("python.function_arguments", 4, 3),
    ] {
        let finding = data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["rule_id"] == rule)
            .unwrap_or_else(|| panic!("missing {rule}"));
        assert_eq!(finding["evaluation"]["observed"], observed);
        assert_eq!(finding["evaluation"]["threshold"], threshold);
        assert_eq!(finding["evaluation"]["matched"], true);
        assert_eq!(finding["blocking"], true);
        assert!(finding["source_excerpt"] != Value::Null);
    }
}

#[test]
fn python_class_metrics_combine_declared_state_and_owned_methods() {
    let source = r#"class Account:
    def __init__(self, a, b, c):
        self.a = a
        self.b = b
        self.c = c

    def total(self):
        value = self.a + self.b
        return value
"#;
    let mut policy = portable_policy("python");
    require_maximum(&mut policy, "python.class_fields", 2);
    require_maximum(&mut policy, "python.class_methods", 1);
    require_maximum(&mut policy, "python.class_method_lines", 4);
    let workspace = Workspace::new("account.py", source, &policy);
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    for (rule, observed, threshold) in [
        ("python.class_fields", 3, 2),
        ("python.class_methods", 2, 1),
        ("python.class_method_lines", 5, 4),
    ] {
        let finding = data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["rule_id"] == rule)
            .unwrap_or_else(|| panic!("missing {rule}"));
        assert_eq!(finding["symbol"], "Account");
        assert_eq!(finding["evaluation"]["observed"], observed);
        assert_eq!(finding["evaluation"]["threshold"], threshold);
        assert_eq!(finding["evaluation"]["matched"], true);
        assert_eq!(finding["blocking"], true);
        assert!(finding["source_excerpt"] != Value::Null);
    }
}

#[test]
fn typescript_class_metrics_cover_fields_methods_and_method_lines() {
    let source = r#"class Service {
  private a: number;
  private b: number;
  private c: number;

  constructor(a: number, b: number, c: number) {
    this.a = a;
    this.b = b;
    this.c = c;
  }

  run(a: number, b: number, c: number, d: number): number {
    const value = a + b + c;
    return value + d;
  }
}
"#;
    let mut policy = portable_policy("typescript");
    require_maximum(&mut policy, "typescript.class_fields", 2);
    require_maximum(&mut policy, "typescript.class_methods", 1);
    require_maximum(&mut policy, "typescript.class_method_lines", 4);
    let workspace = Workspace::new("service.ts", source, &policy);
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    assert_eq!(data["rule_pack"], "typescript-v1");
    assert_eq!(data["language"], "typescript");
    for (rule, observed, threshold) in [
        ("typescript.class_fields", 3, 2),
        ("typescript.class_methods", 2, 1),
        ("typescript.class_method_lines", 5, 4),
    ] {
        let finding = data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["rule_id"] == rule)
            .unwrap_or_else(|| panic!("missing {rule}"));
        assert_eq!(finding["symbol"], "Service");
        assert_eq!(finding["evaluation"]["observed"], observed);
        assert_eq!(finding["evaluation"]["threshold"], threshold);
        assert_eq!(finding["blocking"], true);
    }
}

#[test]
fn typescript_staged_scan_uses_staged_source_and_staged_policy() {
    let source = "export function overloaded(a: number, b: number, c: number, d: number) {\n  return a + b + c + d;\n}\n";
    let policy = portable_policy("typescript");
    let workspace = Workspace::new("service.ts", source, &policy);
    workspace.stage();
    fs::write(
        workspace.path.join("service.ts"),
        "export function clean() { return 1; }\n",
    )
    .expect("replace working-tree source");
    fs::write(
        workspace.path.join("quality-policy.json"),
        serde_json::to_vec(&portable_policy("python")).unwrap(),
    )
    .expect("replace working-tree policy");

    let output = workspace.check_staged();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    assert_eq!(data["rule_pack"], "typescript-v1");
    assert_eq!(data["source_mode"], "staged_snapshot");
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "typescript.function_arguments")
        .expect("staged TypeScript argument finding");
    assert_eq!(finding["evaluation"]["observed"], 4);
    assert_eq!(finding["evaluation"]["matched"], true);
}

#[test]
fn python_staged_scan_uses_staged_source_and_staged_policy() {
    let source = "def overloaded(a, b, c, d):\n    return a + b + c + d\n";
    let policy = portable_policy("python");
    let workspace = Workspace::new("service.py", source, &policy);
    workspace.stage();
    fs::write(
        workspace.path.join("service.py"),
        "def clean():\n    return 1\n",
    )
    .expect("replace working-tree source");
    fs::write(
        workspace.path.join("quality-policy.json"),
        serde_json::to_vec(&portable_policy("typescript")).unwrap(),
    )
    .expect("replace working-tree policy");

    let output = workspace.check_staged();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    assert_eq!(data["rule_pack"], "python-v1");
    assert_eq!(data["source_mode"], "staged_snapshot");
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "python.function_arguments")
        .expect("staged Python argument finding");
    assert_eq!(finding["evaluation"]["observed"], 4);
    assert_eq!(finding["evaluation"]["matched"], true);
}

#[test]
fn tsx_classes_and_constructor_parameter_properties_are_scanned() {
    let source = r#"class Widget {
  constructor(private count: number, readonly label: string) {}

  render() {
    return <div>{this.label}: {this.count}</div>;
  }
}
"#;
    let mut policy = portable_policy("typescript");
    require_maximum(&mut policy, "typescript.class_fields", 1);
    let workspace = Workspace::new("widget.tsx", source, &policy);
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let data = report(&output);
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "typescript.class_fields")
        .expect("TypeScript class field finding");
    assert_eq!(finding["symbol"], "Widget");
    assert_eq!(finding["evaluation"]["observed"], 2);
    assert_eq!(finding["evaluation"]["threshold"], 1);
    assert_eq!(finding["blocking"], true);
}

#[test]
fn python_portable_source_patterns_emit_deterministic_evidence() {
    let source = r#"class Record:
    name: str
    age: int

class Tiny:
    pass

def first(x, y, z):
    # explain the calculation
    # preserve the invariant
    value = x + y
    return value + z

def second(x, y, z):
    value = x + y
    return value + z

def third(x, y, z):
    return x + y + z
"#;
    let mut policy = portable_policy("python");
    require(
        &mut policy,
        "python.comment_share",
        json!({"minimum_code_lines":2,"minimum_share_percent":40}),
    );
    require(
        &mut policy,
        "python.duplicate_functions",
        json!({"minimum_similarity_basis_points":10000,"minimum_tokens":4}),
    );
    require(
        &mut policy,
        "python.data_clumps",
        json!({"minimum_group_size":3,"minimum_declarations":3}),
    );
    require(
        &mut policy,
        "python.data_class",
        json!({"minimum_fields":2,"maximum_operations":0}),
    );
    require(
        &mut policy,
        "python.lazy_class",
        json!({"maximum_fields":0,"maximum_functions":0,"maximum_lines":0}),
    );
    let workspace = Workspace::new("patterns.py", source, &policy);
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    for rule in [
        "python.comment_share",
        "python.duplicate_functions",
        "python.data_clumps",
        "python.data_class",
        "python.lazy_class",
    ] {
        let finding = data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["rule_id"] == rule && finding["evaluation"]["matched"] == true)
            .unwrap_or_else(|| panic!("missing matched {rule}"));
        assert_eq!(finding["blocking"], true);
        assert!(finding["source_excerpt"] != Value::Null);
        assert!(
            finding["diagnostic"]["reference_url"]
                .as_str()
                .unwrap()
                .starts_with("https://refactoring.guru/smells/")
        );
        assert_eq!(
            finding["diagnostic"]["reference_check"]["non_negotiable"],
            true
        );
    }
}

#[test]
fn portable_registries_examples_schemas_and_contract_anchors_are_complete() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for language in ["python", "typescript"] {
        let pack = format!("{language}-v1");
        let output = Command::new(env!("CARGO_BIN_EXE_smells"))
            .args(["rules", "--rule-pack", &pack])
            .output()
            .expect("print registry");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let registry: Value = serde_json::from_slice(&output.stdout).expect("registry JSON");
        assert_eq!(registry["language"], language);
        assert_eq!(registry["smells"].as_array().unwrap().len(), 23);
        assert_eq!(registry["rules"].as_array().unwrap().len(), 28);
        assert_eq!(
            registry["rules"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|rule| rule["implementation"] == "implemented")
                .count(),
            10
        );

        let policy = root.join(format!("examples/{language}-quality-policy.json"));
        let validate = Command::new(env!("CARGO_BIN_EXE_smells"))
            .args(["contracts", "validate", "--policy"])
            .arg(&policy)
            .output()
            .expect("validate example policy");
        assert!(
            validate.status.success(),
            "{}",
            String::from_utf8_lossy(&validate.stderr)
        );
        let status: Value = serde_json::from_slice(&validate.stdout).expect("validation JSON");
        assert_eq!(status["status"], "valid_contracts");
        assert_eq!(status["smells"], 23);
        assert_eq!(status["rules"], 28);
        assert_eq!(status["implemented_source_rules"], 10);

        let schema =
            fs::read_to_string(root.join(format!("schemas/{language}-quality-policy.schema.json")))
                .expect("read policy schema");
        let _: Value = serde_json::from_str(&schema).expect("policy schema JSON");
        let contracts = fs::read_to_string(root.join(format!("docs/{language}-rule-contracts.md")))
            .expect("read rule contracts");
        for rule in registry["rules"].as_array().unwrap() {
            let anchor = rule["contract"].as_str().unwrap();
            assert!(
                contracts.contains(&format!("id=\"{anchor}\"")),
                "missing {anchor}"
            );
        }
    }
}

#[test]
fn python_lambdas_and_typescript_arrows_are_callable_metrics() {
    for (language, file, source, rule) in [
        (
            "python",
            "callable.py",
            "overloaded = lambda a, b, c, d: a + b + c + d\n",
            "python.function_arguments",
        ),
        (
            "typescript",
            "callable.mts",
            "const overloaded = (a: number, b: number, c: number, d: number) => a + b + c + d;\n",
            "typescript.function_arguments",
        ),
    ] {
        let workspace = Workspace::new(file, source, &portable_policy(language));
        let output = workspace.check_path();
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let data = report(&output);
        let finding = data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["rule_id"] == rule)
            .unwrap_or_else(|| panic!("missing {rule}"));
        assert_eq!(finding["evaluation"]["observed"], 4);
        assert_eq!(finding["blocking"], true);
    }
}

#[test]
fn python_parameter_separators_are_not_arguments() {
    let workspace = Workspace::new(
        "parameters.py",
        "def exact(a, /, b, *, c):\n    return a + b + c\n",
        &portable_policy("python"),
    );
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "python.function_arguments")
        .expect("Python argument measurement");
    assert_eq!(finding["evaluation"]["observed"], 3);
    assert_eq!(finding["evaluation"]["matched"], false);
}

#[test]
fn portable_reports_replay_byte_for_byte() {
    for (language, file, source) in [
        ("python", "replay.py", "def value(a):\n    return a\n"),
        (
            "typescript",
            "replay.ts",
            "export function value(a: number) { return a; }\n",
        ),
    ] {
        let workspace = Workspace::new(file, source, &portable_policy(language));
        let first = workspace.check_path();
        let second = workspace.check_path();
        assert_eq!(first.status.code(), second.status.code());
        assert_eq!(first.stdout, second.stdout);
        assert_eq!(first.stderr, second.stderr);
    }
}

#[test]
fn required_portable_pending_rule_fails_closed() {
    let mut policy = portable_policy("python");
    require(
        &mut policy,
        "python.refused_bequest",
        json!({"minimum_inherited_members":3,"minimum_unused_percent":80}),
    );
    let workspace = Workspace::new("model.py", "class Child:\n    pass\n", &policy);
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(2));
    let data = report(&output);
    assert_eq!(data["summary"]["verdict"], "incomplete_due_to_errors");
    assert!(
        data["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error == "required detector not implemented: python.refused_bequest")
    );
}
