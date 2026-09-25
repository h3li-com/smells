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
        let source_path = path.join(file);
        if let Some(parent) = source_path.parent() {
            fs::create_dir_all(parent).expect("create source parent");
        }
        fs::write(source_path, source).expect("write source");
        fs::write(
            path.join("quality-policy.json"),
            serde_json::to_vec(policy).expect("serialize policy"),
        )
        .expect("write policy");
        Self { path }
    }

    fn source(&self, path: &str, source: &str) {
        let path = self.path.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create source parent");
        }
        fs::write(path, source).expect("write source");
    }

    fn check_path(&self) -> Output {
        self.check_path_with(&[])
    }

    fn check_path_source_only(&self) -> Output {
        self.check_path_with(&["--only-group", "source"])
    }

    fn check_path_with(&self, selection: &[&str]) -> Output {
        let mut arguments = vec!["check", "--path", ".", "--policy", "quality-policy.json"];
        arguments.extend_from_slice(selection);
        arguments.extend_from_slice(&["--format", "json"]);
        Command::new(env!("CARGO_BIN_EXE_smells"))
            .args(arguments)
            .current_dir(&self.path)
            .output()
            .expect("run scanner")
    }

    fn check_path_table(&self) -> Output {
        self.check_path_table_with(&[])
    }

    fn check_path_source_table(&self) -> Output {
        self.check_path_table_with(&["--only-group", "source"])
    }

    fn check_path_table_with(&self, selection: &[&str]) -> Output {
        let mut arguments = vec!["check", "--path", ".", "--policy", "quality-policy.json"];
        arguments.extend_from_slice(selection);
        arguments.extend_from_slice(&["--format", "table"]);
        Command::new(env!("CARGO_BIN_EXE_smells"))
            .args(arguments)
            .current_dir(&self.path)
            .output()
            .expect("run scanner table")
    }

    fn check_path_with_evidence(&self, evidence: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_smells"))
            .args([
                "check",
                "--path",
                ".",
                "--policy",
                "quality-policy.json",
                "--evidence",
                evidence,
                "--format",
                "json",
            ])
            .current_dir(&self.path)
            .output()
            .expect("run scanner with provider evidence")
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

    fn initialize_git(&self) {
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
    }

    fn commit(&self, message: &str) {
        let add = Command::new("git")
            .args(["add", "."])
            .current_dir(&self.path)
            .output()
            .expect("stage commit inputs");
        assert!(
            add.status.success(),
            "{}",
            String::from_utf8_lossy(&add.stderr)
        );
        let commit = Command::new("git")
            .args([
                "-c",
                "user.name=Smells Test",
                "-c",
                "user.email=smells@example.invalid",
                "commit",
                "--quiet",
                "-m",
                message,
            ])
            .current_dir(&self.path)
            .output()
            .expect("commit inputs");
        assert!(
            commit.status.success(),
            "{}",
            String::from_utf8_lossy(&commit.stderr)
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

    fn check_staged_with_evidence(&self, evidence: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_smells"))
            .args([
                "check",
                "--staged",
                "--policy",
                "quality-policy.json",
                "--evidence",
                evidence,
                "--format",
                "json",
            ])
            .current_dir(&self.path)
            .output()
            .expect("run staged scanner with provider evidence")
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
    add("function_crap", "off", json!({"maximum": 5}));
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
        "scanner_version": "0.5.0",
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

fn measurements(report: &Value) -> impl Iterator<Item = &Value> {
    report["measurements"].as_array().unwrap().iter()
}

fn finding_measurement<'a>(report: &'a Value, finding: &Value) -> &'a Value {
    &report["measurements"][finding["measurement_index"].as_u64().unwrap() as usize]
}

fn require_maximum(policy: &mut Value, rule: &str, maximum: u64) {
    policy["rules"][rule] = selection("required", json!({"maximum": maximum}));
}

fn require(policy: &mut Value, rule: &str, parameters: Value) {
    policy["rules"][rule] = selection("required", parameters);
}

fn require_only(policy: &mut Value, rule_ids: &[&str]) {
    for rule in policy["rules"].as_object_mut().expect("policy rules") {
        rule.1["mode"] = json!("off");
    }
    for rule_id in rule_ids {
        policy["rules"][*rule_id]["mode"] = json!("required");
    }
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

#[cfg(unix)]
#[test]
fn working_tree_rejects_runtime_manifest_symlinks() {
    use std::os::unix::fs::symlink;

    for (language, source, manifest) in [
        ("python", "app.py", "pyproject.toml"),
        ("typescript", "app.ts", "package.json"),
    ] {
        let workspace = Workspace::new(source, "", &portable_policy(language));
        symlink(source, workspace.path.join(manifest)).unwrap();
        let output = workspace.check_path();
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("symlink runtime manifest"),
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
        let output = workspace.check_path_source_only();
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
fn every_starter_runs_all_rules_without_external_evidence() {
    for (file, source, policy) in [
        (
            "lib.rs",
            "fn healthy() {}\n",
            serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap(),
        ),
        (
            "app.py",
            "def healthy():\n    return 1\n",
            serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap(),
        ),
        (
            "app.ts",
            "function healthy() { return 1; }\n",
            serde_json::from_str(include_str!("../examples/typescript-quality-policy.json"))
                .unwrap(),
        ),
    ] {
        let workspace = Workspace::new(file, source, &policy);
        workspace.initialize_git();
        let output = workspace.check_path();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{file}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let data = report(&output);
        assert_eq!(
            data["policy_selection"]["active_rule_ids"]
                .as_array()
                .unwrap()
                .len(),
            28,
            "{file}"
        );
        assert_eq!(data["smell_results"].as_array().unwrap().len(), 23);
        assert_eq!(data["errors"], json!([]));
        assert!(
            data["smell_results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|result| result["coverage_status"] != "incomplete"),
            "{file}: {data}"
        );
    }
}

#[test]
fn default_python_scan_detects_message_chains_without_external_evidence() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new(
        "orders.py",
        "def city(order):\n    return order.customer.address.city\n",
        &policy,
    );
    workspace.initialize_git();
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
        .find(|finding| finding["rule_id"] == "python.navigation_chains")
        .expect("built-in message-chain finding");
    assert_eq!(finding["evaluation"]["observed"], 3);
    assert_eq!(finding["evaluation"]["matched"], true);
    let measurement = finding_measurement(&data, finding);
    let provider_ref = measurement["evidence"]["provider_ref"].as_u64().unwrap() as usize;
    assert_eq!(
        data["evidence_provider_catalog"][provider_ref]["name"],
        "smells-built-in"
    );
}

#[test]
fn default_python_scan_detects_feature_envy_without_external_evidence() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new(
        "billing.py",
        concat!(
            "def invoice(customer):\n",
            "    return (customer.name, customer.address, customer.city, ",
            "customer.country, customer.account, customer.currency)\n",
        ),
        &policy,
    );
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "python.foreign_accesses")
        .expect("built-in feature-envy finding");
    assert_eq!(finding["evaluation"]["observed"]["foreign_accesses"], 6);
    assert_eq!(finding["evaluation"]["matched"], true);
}

#[test]
fn default_python_scan_detects_inappropriate_intimacy_without_contract_file() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new(
        "accounts.py",
        "def leak(account):\n    return account._private_token\n",
        &policy,
    );
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "python.dependency_contract")
        .expect("built-in inappropriate-intimacy finding");
    assert_eq!(finding["evaluation"]["observed"], 1);
    assert_eq!(finding["evaluation"]["matched"], true);
}

#[test]
fn default_python_scan_detects_primitive_obsession_without_type_provider() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new(
        "profile.py",
        concat!(
            "class Profile:\n",
            "    name: str\n",
            "    city: str\n",
            "    country: str\n",
            "    age: int\n",
            "    active: bool\n",
        ),
        &policy,
    );
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "python.primitive_slots")
        .expect("built-in primitive-obsession finding");
    assert_eq!(finding["evaluation"]["observed"]["raw_slots"], 5);
    assert_eq!(finding["evaluation"]["matched"], true);
}

fn matched_rule<'a>(data: &'a Value, rule_id: &str) -> &'a Value {
    data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == rule_id && finding["evaluation"]["matched"] == true)
        .unwrap_or_else(|| {
            let matched = data["findings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|finding| finding["evaluation"]["matched"] == true)
                .map(|finding| finding["rule_id"].as_str().unwrap_or_default())
                .collect::<Vec<_>>();
            panic!("missing built-in match for {rule_id}; matched rules: {matched:?}")
        })
}

#[test]
fn default_python_scan_runs_structural_class_collectors() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new(
        "patterns.py",
        concat!(
            "class First:\n",
            "    def calculate(self, value):\n",
            "        first = value + 1\n        second = first * 2\n",
            "        third = second - 3\n        fourth = third / 4\n",
            "        return fourth + value + first + second + third\n\n",
            "class Second:\n",
            "    def transform(self, value):\n",
            "        first = value + 1\n        second = first * 2\n",
            "        third = second - 3\n        fourth = third / 4\n",
            "        return fourth + value + first + second + third\n\n",
            "class Temporary:\n",
            "    a: int | None = None\n    b: int | None = None\n    c: int | None = None\n",
            "    def one(self):\n        return self.a\n",
            "    def two(self):\n        return 2\n",
            "    def three(self):\n        return 3\n",
            "    def four(self):\n        return 4\n\n",
            "class MiddleMan:\n",
            "    def one(self, x):\n        return self.backend.send(x)\n",
            "    def two(self, x):\n        return self.backend.send(x)\n",
            "    def three(self, x):\n        return self.backend.send(x)\n",
            "    def four(self, x):\n        return self.backend.send(x)\n",
            "    def five(self, x):\n        return self.backend.send(x)\n\n",
            "def dispatch_one(value):\n    match value:\n        case Kind.A: return 1\n",
            "        case Kind.B: return 2\n        case Kind.C: return 3\n        case Kind.D: return 4\n",
            "def dispatch_two(value):\n    match value:\n        case Kind.A: return 1\n",
            "        case Kind.B: return 2\n        case Kind.C: return 3\n        case Kind.D: return 4\n",
            "def dispatch_three(value):\n    match value:\n        case Kind.A: return 1\n",
            "        case Kind.B: return 2\n        case Kind.C: return 3\n        case Kind.D: return 4\n",
        ),
        &policy,
    );
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    for rule in [
        "python.alternative_interfaces",
        "python.repeated_dispatch",
        "python.temporary_fields",
        "python.forwarding_share",
    ] {
        matched_rule(&data, rule);
    }
}

#[test]
fn default_python_scan_runs_static_semantic_collectors() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new(
        "contracts.py",
        concat!(
            "class Account:\n    customer_id: int\n\n",
            "def branching(value):\n    if value > 0:\n        return value\n    return 0\n\n",
            "def _unused_helper():\n    return 1\n\n",
            "def generic[T](value: int):\n    return value\n\n",
            "External.Widget.patch = replacement\n",
        ),
        &policy,
    );
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    for rule in [
        "python.function_crap",
        "python.unused_code",
        "python.unused_type_parameters",
        "python.nominal_slot_contract",
        "python.library_capabilities",
    ] {
        matched_rule(&data, rule);
    }
}

#[test]
fn python_class_collectors_ignore_method_locals_and_nested_callables() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new(
        "locals.py",
        concat!(
            "class Wrapper:\n",
            "    def run(self):\n",
            "        first: str = 'a'\n        second: str = 'b'\n",
            "        third: str = 'c'\n        fourth: str = 'd'\n",
            "        fifth: str = 'e'\n",
            "        one = lambda x: self.backend.send(x)\n",
            "        two = lambda x: self.backend.send(x)\n",
            "        three = lambda x: self.backend.send(x)\n",
            "        four = lambda x: self.backend.send(x)\n",
            "        return (first, second, third, fourth, fifth, one, two, three, four)\n",
        ),
        &policy,
    );
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    for rule in ["python.primitive_slots", "python.forwarding_share"] {
        assert!(
            !data["findings"].as_array().unwrap().iter().any(|finding| {
                finding["rule_id"] == rule && finding["evaluation"]["matched"] == true
            }),
            "method-local syntax must not populate {rule}"
        );
    }
}

#[test]
fn default_python_scan_runs_inheritance_and_port_collectors() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new(
        "hierarchies.py",
        concat!(
            "class OutputPort(Protocol):\n",
            "    def save(self): ...\n    def load(self): ...\n\n",
            "class BrokenOutput(OutputPort):\n    def save(self): return None\n\n",
            "class Parent:\n",
            "    def first(self): return 1\n",
            "    def second(self): return 2\n",
            "    def third(self): return 3\n\n",
            "class Child(Parent):\n",
            "    def first(self): raise NotImplementedError\n",
            "    def second(self): raise NotImplementedError\n",
            "    def third(self): raise NotImplementedError\n\n",
            "class EmailValidator(Validator): pass\nclass SmsValidator(Validator): pass\n",
            "class PushValidator(Validator): pass\n",
            "class EmailFormatter(Formatter): pass\nclass SmsFormatter(Formatter): pass\n",
            "class PushFormatter(Formatter): pass\n",
        ),
        &policy,
    );
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    for rule in [
        "python.port_conformance",
        "python.refused_bequest",
        "python.parallel_inheritance",
    ] {
        matched_rule(&data, rule);
    }
}

#[test]
fn default_python_scan_collects_git_history_without_an_evidence_bundle() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new("app.py", "value = 1\n", &policy);
    workspace.source("accounts/model.py", "value = 1\n");
    workspace.source("billing/model.py", "value = 1\n");
    workspace.source("shipping/model.py", "value = 1\n");
    workspace.stage();
    workspace.commit("baseline fanout");

    workspace.source("app.py", "value = 2\n");
    workspace.source("accounts/model.py", "value = 2\n");
    workspace.commit("accounts change");
    workspace.source("app.py", "value = 3\n");
    workspace.source("billing/model.py", "value = 3\n");
    workspace.commit("billing change");

    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    matched_rule(&data, "python.divergent_change");
    matched_rule(&data, "python.shotgun_surgery");
}

#[test]
fn every_selected_history_rule_is_incomplete_when_git_history_is_unavailable() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new("app.py", "value = 1\n", &policy);
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(2));
    let data = report(&output);
    assert_eq!(data["history_scope"]["available"], false);
    for (rule, smell) in [
        ("python.divergent_change", "divergent-change"),
        ("python.shotgun_surgery", "shotgun-surgery"),
    ] {
        assert!(data["errors"].as_array().unwrap().iter().any(|error| {
            error
                == &format!(
                    "rule {rule}: Git history is unavailable for built-in collector: {rule}"
                )
        }));
        let result = smell_result(&data, smell);
        assert_eq!(result["state"], "error");
        assert_eq!(result["coverage_status"], "incomplete");
        assert!(
            result["incomplete_rule_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|incomplete| incomplete == rule)
        );
    }
    assert_ne!(smell_result(&data, "large-class")["state"], "error");
}

#[test]
fn default_typescript_scan_runs_every_built_in_source_collector() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/typescript-quality-policy.json")).unwrap();
    let workspace = Workspace::new(
        "patterns.ts",
        concat!(
            "interface OutputPort { save(): void; load(): void; }\n",
            "class BrokenOutput implements OutputPort { save(): void {} }\n",
            "class Profile { name: string; city: string; country: string; age: number; active: boolean; customer_id: number; }\n",
            "class First { calculate(value: number) { const a=value+1; const b=a*2; const c=b-3; const d=c/4; return d+value+a+b+c; } }\n",
            "class Second { transform(value: number) { const a=value+1; const b=a*2; const c=b-3; const d=c/4; return d+value+a+b+c; } }\n",
            "class Temporary { a?: number; b?: number; c?: number; one(){return this.a;} two(){return 2;} three(){return 3;} four(){return 4;} }\n",
            "class MiddleMan { one(x:number){return this.backend.send(x);} two(x:number){return this.backend.send(x);} three(x:number){return this.backend.send(x);} four(x:number){return this.backend.send(x);} five(x:number){return this.backend.send(x);} }\n",
            "function dispatchOne(value: Kind) { switch(value) {\ncase Kind.A: return 1;\ncase Kind.B: return 2;\ncase Kind.C: return 3;\ncase Kind.D: return 4;\n} }\n",
            "function dispatchTwo(value: Kind) { switch(value) {\ncase Kind.A: return 1;\ncase Kind.B: return 2;\ncase Kind.C: return 3;\ncase Kind.D: return 4;\n} }\n",
            "function dispatchThree(value: Kind) { switch(value) {\ncase Kind.A: return 1;\ncase Kind.B: return 2;\ncase Kind.C: return 3;\ncase Kind.D: return 4;\n} }\n",
            "class Parent { first(){return 1;} second(){return 2;} third(){return 3;} }\n",
            "class Child extends Parent { first(){throw new Error();} second(){throw new Error();} third(){throw new Error();} }\n",
            "class EmailValidator extends Validator {} class SmsValidator extends Validator {} class PushValidator extends Validator {}\n",
            "class EmailFormatter extends Formatter {} class SmsFormatter extends Formatter {} class PushFormatter extends Formatter {}\n",
            "function branching(value:number){ if(value>0){return value;} return 0; }\n",
            "function _unusedHelper(){ return 1; }\n",
            "function generic<T>(value:number){ return value; }\n",
            "function inspect(order:any, customer:any, account:any){ const city=order.customer.address.city; return [customer.name,customer.address,customer.city,customer.country,customer.account,customer.currency,account._secret,city]; }\n",
            "External.prototype.patch = replacement;\n",
        ),
        &policy,
    );
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    for rule in [
        "typescript.primitive_slots",
        "typescript.alternative_interfaces",
        "typescript.repeated_dispatch",
        "typescript.temporary_fields",
        "typescript.forwarding_share",
        "typescript.function_crap",
        "typescript.unused_code",
        "typescript.unused_type_parameters",
        "typescript.nominal_slot_contract",
        "typescript.port_conformance",
        "typescript.refused_bequest",
        "typescript.parallel_inheritance",
        "typescript.foreign_accesses",
        "typescript.dependency_contract",
        "typescript.library_capabilities",
        "typescript.navigation_chains",
    ] {
        matched_rule(&data, rule);
    }
}

#[test]
fn typescript_structural_collectors_ignore_non_code_text_and_bodyless_signatures() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/typescript-quality-policy.json")).unwrap();
    let repeated_literal =
        "case Kind.A: one\\ncase Kind.B: two\\ncase Kind.C: three\\ncase Kind.D: four";
    let workspace = Workspace::new(
        "patterns.ts",
        &format!(
            concat!(
                "interface Port {{ execute<T>(): void; }}\n",
                "abstract class Base {{ abstract required<U>(): void; concrete() {{ return \"if while case catch && ||\"; }} }}\n",
                "class FalseTemporary {{ one(){{return \"this.a = null\";}} two(){{return \"this.b = null\";}} three(){{return \"this.c = null\";}} four(){{return \"idle\";}} }}\n",
                "class First {{ one(){{return \"alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau\";}} }}\n",
                "class Second {{ two(){{return \"alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau\";}} }}\n",
                "function fakeOne() {{ return `{0}`; }}\n",
                "function fakeTwo() {{ return `{0}`; }}\n",
                "function fakeThree() {{ return `{0}`; }}\n",
                "function _unused<T>() {{ return \"_unused T\"; }} // _unused T\n",
                "function values(items: Array<Foo>) {{ return items; }}\n",
                "class Derived extends Base<Foo> {{}}\n",
                "class Middle {{ a(){{return this./* note */backend.send(1);}} b(){{return this.backend.send(2);}} c(){{return this.backend.send(3);}} d(){{return this.backend.send(4);}} e(){{return this.backend.send(5);}} }}\n",
                "class Parent {{ a(){{}} b(){{}} c(){{}} }}\n",
                "class Child extends Parent {{ a(){{throw /* note */ new Error();}} b(){{throw new Error();}} c(){{throw new Error();}} }}\n",
            ),
            repeated_literal,
        ),
        &policy,
    );
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = report(&output);
    for rule in [
        "typescript.temporary_fields",
        "typescript.alternative_interfaces",
        "typescript.repeated_dispatch",
    ] {
        assert!(data["findings"].as_array().unwrap().iter().all(|finding| {
            finding["rule_id"] != rule || finding["evaluation"]["matched"] == false
        }));
    }
    let concrete_crap = measurements(&data)
        .find(|finding| {
            finding["rule_id"] == "typescript.function_crap"
                && finding["symbol"]
                    .as_str()
                    .is_some_and(|symbol| symbol.ends_with("Base.concrete"))
        })
        .expect("concrete method CRAP observation");
    assert_eq!(
        concrete_crap["evaluation"]["observed"],
        json!({"numerator": 2, "denominator": 1})
    );
    assert!(data["findings"].as_array().unwrap().iter().all(|finding| {
        finding["rule_id"] != "typescript.function_crap"
            || !finding["symbol"].as_str().is_some_and(|symbol| {
                symbol.ends_with("Port.execute") || symbol.ends_with("Base.required")
            })
    }));
    for name in ["Port.execute", "Base.required"] {
        assert!(data["findings"].as_array().unwrap().iter().any(|finding| {
            finding["rule_id"] == "typescript.unused_type_parameters"
                && finding["evaluation"]["matched"] == true
                && finding["symbol"]
                    .as_str()
                    .is_some_and(|symbol| symbol.ends_with(name))
        }));
    }
    assert!(data["findings"].as_array().unwrap().iter().all(|finding| {
        finding["rule_id"] != "typescript.unused_type_parameters"
            || !finding["symbol"].as_str().is_some_and(|symbol| {
                symbol.ends_with("patterns.ts::Port")
                    || symbol.ends_with("patterns.ts::Base")
                    || symbol.ends_with("patterns.ts::values")
                    || symbol.ends_with("patterns.ts::Derived")
            })
    }));
    for rule in [
        "typescript.unused_code",
        "typescript.unused_type_parameters",
        "typescript.forwarding_share",
        "typescript.refused_bequest",
    ] {
        matched_rule(&data, rule);
    }
}

#[test]
fn python_structural_collectors_ignore_literal_and_comment_text() {
    let policy: Value =
        serde_json::from_str(include_str!("../examples/python-quality-policy.json")).unwrap();
    let workspace = Workspace::new(
        "patterns.py",
        concat!(
            "def _unused[T]():\n",
            "    return 'T if elif for while case catch _unused'\n",
            "# _unused and T are prose, not references\n\n",
            "def values(items: list[int]):\n    return items\n\n",
            "class Derived(Base[int]):\n    pass\n\n",
            "class FalseTemporary:\n",
            "    def one(self): return 'self.a = None'\n",
            "    def two(self): return 'self.b = None'\n",
            "    def three(self): return 'self.c = None'\n",
            "    def four(self): return 'idle'\n",
        ),
        &policy,
    );
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let data = report(&output);
    let crap = measurements(&data)
        .find(|finding| {
            finding["rule_id"] == "python.function_crap"
                && finding["symbol"]
                    .as_str()
                    .is_some_and(|symbol| symbol.ends_with("::_unused"))
        })
        .expect("Python function CRAP observation");
    assert_eq!(
        crap["evaluation"]["observed"],
        json!({"numerator": 2, "denominator": 1})
    );
    assert!(data["findings"].as_array().unwrap().iter().all(|finding| {
        finding["rule_id"] != "python.temporary_fields" || finding["evaluation"]["matched"] == false
    }));
    assert!(data["findings"].as_array().unwrap().iter().all(|finding| {
        finding["rule_id"] != "python.unused_type_parameters"
            || !finding["symbol"].as_str().is_some_and(|symbol| {
                symbol.ends_with("patterns.py::values") || symbol.ends_with("patterns.py::Derived")
            })
    }));
    matched_rule(&data, "python.unused_code");
    matched_rule(&data, "python.unused_type_parameters");
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
        assert!(finding.get("source_excerpt").is_none());
        assert_eq!(
            finding_measurement(&data, finding)["location"],
            finding["location"]
        );
    }
}

#[test]
fn source_suppressions_are_rule_specific_audited_and_consistent_across_languages() {
    for (language, file, source) in [
        (
            "rust",
            "src/lib.rs",
            "// smells: ignore[rust.function_arguments] -- stable external API\n\n#[inline]\nfn publish(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32) {}\n",
        ),
        (
            "python",
            "app.py",
            "# smells: ignore[python.function_arguments] -- stable external API\n\n@command\ndef publish(a, b, c, d):\n    pass\n",
        ),
        (
            "typescript",
            "app.ts",
            "class Publisher {\n  // smells: ignore[typescript.function_arguments] -- stable external API\n\n  @command\n  publish(a: number, b: number, c: number, d: number): void {}\n}\n",
        ),
    ] {
        let mut policy = if language == "rust" {
            serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap()
        } else {
            portable_policy(language)
        };
        if language == "rust" {
            policy["rules"]["rust.primitive_slots"]["mode"] = json!("off");
        } else if language == "typescript" {
            policy["rules"]["typescript.lazy_class"]["mode"] = json!("off");
        }
        let workspace = Workspace::new(file, source, &policy);
        let output = workspace.check_path_source_only();
        assert_eq!(output.status.code(), Some(0), "{language}: {output:?}");
        let data = report(&output);
        assert_eq!(
            data["summary"]["verdict"], "passed_with_ignored_findings",
            "{language}: {}",
            data["summary"]
        );
        assert_eq!(data["summary"]["ignored_findings"], 1);
        let rule_id = format!("{language}.function_arguments");
        let finding = data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["rule_id"] == rule_id)
            .expect("ignored match remains visible");
        assert_eq!(finding["status"], "ignored_match");
        assert_eq!(finding["blocking"], false);
        assert_eq!(finding["suppression"]["reason"], "stable external API");
        assert_eq!(finding["guidance_ref"], "long-parameter-list@1");
        assert_eq!(data["suppressions"][0]["state"], "used");

        let table = workspace.check_path_source_table();
        assert_eq!(table.status.code(), Some(0), "{language}: {table:?}");
        let stdout = String::from_utf8(table.stdout).unwrap();
        assert!(
            stdout.contains("ignored findings: 1"),
            "{language}: {stdout}"
        );
        assert!(
            stdout.contains("Matches | Blocking | Ignored | Review | Coverage"),
            "{language}: {stdout}"
        );
    }
}

#[test]
fn suppression_belongs_to_the_target_declaration_not_nested_declarations() {
    for (language, file, source) in [
        (
            "rust",
            "src/lib.rs",
            "// smells: ignore[rust.function_arguments] -- outer API is stable\nfn outer(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32) {\n    fn inner(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32) {}\n}\n",
        ),
        (
            "python",
            "app.py",
            "# smells: ignore[python.function_arguments] -- outer API is stable\ndef outer(a, b, c, d):\n    def inner(a, b, c, d): pass\n",
        ),
        (
            "typescript",
            "app.ts",
            "// smells: ignore[typescript.function_arguments] -- outer API is stable\nfunction outer(a: number, b: number, c: number, d: number): void {\n  function inner(a: number, b: number, c: number, d: number): void {}\n}\n",
        ),
    ] {
        let mut policy = if language == "rust" {
            serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap()
        } else {
            portable_policy(language)
        };
        if language == "rust" {
            policy["rules"]["rust.primitive_slots"]["mode"] = json!("off");
        }
        let workspace = Workspace::new(file, source, &policy);
        let output = workspace.check_path_source_only();
        assert_eq!(output.status.code(), Some(1), "{language}: {output:?}");
        let data = report(&output);
        let rule_id = format!("{language}.function_arguments");
        let argument_findings: Vec<_> = data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["rule_id"] == rule_id)
            .collect();
        assert_eq!(
            argument_findings.len(),
            2,
            "{language}: {argument_findings:?}"
        );
        assert_eq!(
            argument_findings[0]["status"], "ignored_match",
            "{language}"
        );
        assert_eq!(argument_findings[1]["status"], "violation", "{language}");
        assert_eq!(
            argument_findings[1]["suppression"],
            Value::Null,
            "{language}"
        );
        assert_eq!(data["summary"]["ignored_findings"], 1, "{language}");
        assert_eq!(data["summary"]["blocking_findings"], 1, "{language}");
    }
}

#[test]
fn directive_text_inside_strings_never_creates_a_suppression() {
    for (language, file, source) in [
        (
            "rust",
            "src/lib.rs",
            "const NOTE: &str = r#\"// smells: ignore[rust.function_arguments] -- not a comment\"#;\nfn publish(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32) {}\n",
        ),
        (
            "python",
            "app.py",
            "NOTE = '''# smells: ignore[python.function_arguments] -- not a comment'''\ndef publish(a, b, c, d):\n    pass\n",
        ),
        (
            "typescript",
            "app.ts",
            "const note = `// smells: ignore[typescript.function_arguments] -- not a comment`;\nfunction publish(a: number, b: number, c: number, d: number): void {}\n",
        ),
    ] {
        let mut policy = if language == "rust" {
            serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap()
        } else {
            portable_policy(language)
        };
        if language == "rust" {
            policy["rules"]["rust.primitive_slots"]["mode"] = json!("off");
        }
        let workspace = Workspace::new(file, source, &policy);
        let output = workspace.check_path_source_only();
        assert_eq!(output.status.code(), Some(1), "{language}: {output:?}");
        let data = report(&output);
        assert_eq!(data["suppressions"], json!([]), "{language}");
        let rule_id = format!("{language}.function_arguments");
        let finding = data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["rule_id"] == rule_id)
            .expect("unsuppressed argument finding");
        assert_eq!(finding["status"], "violation", "{language}");
        assert_eq!(finding["suppression"], Value::Null, "{language}");
    }
}

#[test]
fn malformed_unknown_misplaced_and_unused_suppressions_invalidate_the_scan() {
    for (expected, source) in [
        (
            "invalid suppression",
            "// smells: ignore[rust.function_arguments] --\nfn publish(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32) {}\n",
        ),
        (
            "unknown suppression rule",
            "// smells: ignore[rust.not_a_rule] -- legacy API\nfn publish(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32) {}\n",
        ),
        (
            "misplaced suppression",
            "fn outer() {\n    // smells: ignore[rust.function_arguments] -- not a declaration\n    let value = 1;\n    let _ = value;\n}\n",
        ),
        (
            "unused suppression",
            "// smells: ignore[rust.function_arguments] -- short API\nfn publish(a:i32) {}\n",
        ),
    ] {
        let policy: Value =
            serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap();
        let workspace = Workspace::new("src/lib.rs", source, &policy);
        let output = workspace.check_path_source_only();
        assert_eq!(output.status.code(), Some(2), "{expected}: {output:?}");
        let data = report(&output);
        assert_eq!(data["summary"]["verdict"], "incomplete_due_to_errors");
        assert!(
            data["errors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|error| error.as_str().unwrap().contains(expected)),
            "missing {expected}: {}",
            data["errors"]
        );
    }
}

#[test]
fn a_related_location_cannot_suppress_a_multi_location_finding() {
    let source = "fn first(a:i32,b:i32,c:i32)->i32 { let one=a+1; let two=b+one; let three=c+two; three*2 }\n\
// smells: ignore[rust.duplicate_functions] -- second copy is intentional\n\
fn second(a:i32,b:i32,c:i32)->i32 { let one=a+1; let two=b+one; let three=c+two; three*2 }\n";
    let mut policy: Value =
        serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap();
    policy["rules"]["rust.duplicate_functions"]["mode"] = json!("required");
    let workspace = Workspace::new("src/lib.rs", source, &policy);
    let output = workspace.check_path_source_only();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let data = report(&output);
    assert!(
        data["errors"][0]
            .as_str()
            .unwrap()
            .contains("unused suppression")
    );
    let duplicate = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "rust.duplicate_functions")
        .expect("duplicate finding");
    assert_eq!(duplicate["location"]["line"], 1);
    assert_eq!(duplicate["related_locations"][0]["line"], 3);
    assert_eq!(duplicate["status"], "violation");
    assert_eq!(duplicate["suppression"], Value::Null);
}

#[test]
fn alternative_interface_budget_charges_only_surviving_exact_comparisons() {
    for (language, file, source) in [
        (
            "python",
            "app.py",
            (0..300)
                .map(|index| {
                    format!(
                        "class C{index}:\n    def method_{index}(self):\n        return {index}\n"
                    )
                })
                .collect::<String>(),
        ),
        (
            "typescript",
            "app.ts",
            (0..300)
                .map(|index| {
                    format!("class C{index} {{ method_{index}(): number {{ return {index}; }} }}\n")
                })
                .collect::<String>(),
        ),
    ] {
        let mut policy = portable_policy(language);
        policy["rules"][format!("{language}.alternative_interfaces")]["mode"] = json!("report");
        policy["limits"]["maximum_pairs"] = json!(5);
        let workspace = Workspace::new(file, &source, &policy);
        let output = workspace.check_path();
        assert_ne!(output.status.code(), Some(2), "{language}: {output:?}");
        let data = report(&output);
        assert!(data["errors"].as_array().unwrap().is_empty(), "{language}");
        assert!(
            data["measurements"]
                .as_array()
                .unwrap()
                .iter()
                .all(|measurement| {
                    measurement["rule_id"] != format!("{language}.alternative_interfaces")
                })
        );
    }
}

#[test]
fn runtime_manifests_partition_monorepo_results_by_repository_implementation() {
    let workspace = Workspace::new(
        "scripts/root_task.py",
        "def root_task():\n    return 1\n",
        &portable_policy("python"),
    );
    workspace.source("backend/pyproject.toml", "[project]\nname='backend'\n");
    workspace.source("backend/app.py", "def backend():\n    return 1\n");
    workspace.source("frontend/package.json", r#"{"name":"frontend"}"#);
    workspace.source(
        "frontend/generate_fixture.py",
        "def generate_fixture():\n    return 1\n",
    );
    workspace.source("services/voice/pyproject.toml", "[project]\nname='voice'\n");
    workspace.source("services/voice/main.py", "def voice():\n    return 1\n");

    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    assert_eq!(data["report_schema_version"], 7);
    assert_eq!(
        data["implementation_results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|result| result["implementation_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["backend", "frontend", "services/voice", "__unowned__"]
    );

    let backend = implementation_result(&data, "backend");
    assert_eq!(backend["implementation_root"], "backend");
    assert_eq!(backend["ownership"], "runtime_manifest");
    assert_eq!(backend["implementation_types"], json!(["python_project"]));
    assert_eq!(backend["runtime_types"], json!(["python"]));
    assert_eq!(
        backend["runtime_manifests"],
        json!(["backend/pyproject.toml"])
    );
    assert_eq!(backend["scanned_files"], 1);
    assert_eq!(backend["smell_results"].as_array().unwrap().len(), 23);

    let frontend = implementation_result(&data, "frontend");
    assert_eq!(
        frontend["implementation_types"],
        json!(["javascript_typescript_package"])
    );
    assert_eq!(frontend["runtime_types"], json!(["javascript_typescript"]));
    assert_eq!(frontend["scanned_files"], 1);

    let unowned = implementation_result(&data, "__unowned__");
    assert_eq!(unowned["implementation_root"], Value::Null);
    assert_eq!(unowned["ownership"], "unowned_source");
    assert_eq!(unowned["implementation_types"], json!(["unowned_source"]));
    assert_eq!(unowned["runtime_types"], json!([]));
    assert_eq!(unowned["runtime_manifests"], json!([]));
    assert_eq!(unowned["scanned_files"], 1);

    let table = workspace.check_path_table();
    assert_eq!(table.status.code(), Some(0));
    let table = String::from_utf8(table.stdout).unwrap();
    for implementation in ["backend", "frontend", "services/voice", "__unowned__"] {
        assert!(
            table.contains(&format!("Implementation: {implementation}")),
            "{implementation}: {table}"
        );
    }
}

#[test]
fn nearest_runtime_manifest_owns_sources_in_path_and_staged_scans() {
    let workspace = Workspace::new(
        "root.py",
        "def root():\n    return 1\n",
        &portable_policy("python"),
    );
    workspace.source("package.json", r#"{"name":"repository"}"#);
    workspace.source("backend/pyproject.toml", "[project]\nname='backend'\n");
    workspace.source("backend/app.py", "def backend():\n    return 1\n");

    for output in [workspace.check_path(), {
        workspace.stage();
        workspace.check_staged()
    }] {
        assert_eq!(output.status.code(), Some(0));
        let data = report(&output);
        assert_eq!(
            data["implementation_results"]
                .as_array()
                .unwrap()
                .iter()
                .map(|result| result["implementation_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [".", "backend"]
        );
        assert_eq!(implementation_result(&data, ".")["scanned_files"], 1);
        assert_eq!(implementation_result(&data, "backend")["scanned_files"], 1);
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
        "stderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
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
        assert!(finding.get("source_excerpt").is_none());
        assert_eq!(
            finding_measurement(&data, finding)["location"],
            finding["location"]
        );
    }
    let result = smell_result(&data, "large-class");
    assert_eq!(result["state"], "blocking_match");
    assert_eq!(result["matched_findings"], 3);
    assert_eq!(
        result["matched_rule_ids"],
        json!([
            "python.class_fields",
            "python.class_method_lines",
            "python.class_methods"
        ])
    );
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
    let result = smell_result(&data, "large-class");
    assert_eq!(result["state"], "blocking_match");
    assert_eq!(result["matched_findings"], 3);
    assert_eq!(
        result["matched_rule_ids"],
        json!([
            "typescript.class_fields",
            "typescript.class_method_lines",
            "typescript.class_methods"
        ])
    );
}

#[test]
fn typescript_abstract_method_signatures_count_as_class_operations() {
    let source = r#"abstract class Service {
  abstract run(value: number): number;
}
"#;
    let mut policy = portable_policy("typescript");
    require_maximum(&mut policy, "typescript.class_methods", 0);
    require(
        &mut policy,
        "typescript.data_class",
        json!({"minimum_fields":1,"maximum_operations":0}),
    );
    let workspace = Workspace::new("service.ts", source, &policy);
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let data = report(&output);
    let methods = measurements(&data)
        .find(|finding| finding["rule_id"] == "typescript.class_methods")
        .unwrap();
    assert_eq!(methods["evaluation"]["observed"], 1);
    let data_class = measurements(&data)
        .find(|finding| finding["rule_id"] == "typescript.data_class")
        .unwrap();
    assert_eq!(data_class["evaluation"]["observed"]["operations"], 1);
    assert_eq!(data_class["evaluation"]["matched"], false);
}

#[test]
fn typescript_function_rules_include_class_methods_but_not_object_literal_methods() {
    let source = r#"const helpers = {
  objectMethod(a: number, b: number, c: number) {
    const sum = a + b;
    return sum + c;
  }
};

class Service {
  classMethod(a: number, b: number, c: number) {
    const sum = a + b;
    return sum + c;
  }
}
"#;
    let mut policy = portable_policy("typescript");
    require_maximum(&mut policy, "typescript.function_arguments", 2);
    require_maximum(&mut policy, "typescript.function_lines", 1);
    require(
        &mut policy,
        "typescript.comment_share",
        json!({"minimum_code_lines":2,"minimum_share_percent":1}),
    );
    require(
        &mut policy,
        "typescript.duplicate_functions",
        json!({"minimum_similarity_basis_points":10000,"minimum_tokens":4}),
    );
    require(
        &mut policy,
        "typescript.data_clumps",
        json!({"minimum_group_size":3,"minimum_declarations":2}),
    );

    let workspace = Workspace::new("service.ts", source, &policy);
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    let findings = data["findings"].as_array().unwrap();

    assert!(findings.iter().any(|finding| {
        finding["rule_id"] == "typescript.function_arguments"
            && finding["symbol"] == "classMethod"
            && finding["evaluation"]["observed"] == 3
    }));
    assert!(findings.iter().all(|finding| {
        let measurement = finding_measurement(&data, finding);
        finding["symbol"] != "objectMethod"
            && finding["related_symbol"] != "objectMethod"
            && measurement["evidence"]["supporting_symbols"]
                .as_array()
                .is_none_or(|symbols| symbols.iter().all(|symbol| symbol != "objectMethod"))
    }));
    assert!(findings.iter().all(|finding| {
        !matches!(
            finding["rule_id"].as_str(),
            Some("typescript.duplicate_functions" | "typescript.data_clumps")
        )
    }));
}

#[test]
fn typescript_empty_method_bodies_do_not_panic() {
    let source = r#"class Empty {
  value(): void {}
}
"#;
    let workspace = Workspace::new("empty.ts", source, &portable_policy("typescript"));
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(report(&output)["errors"].as_array().unwrap().is_empty());
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
        assert!(finding.get("source_excerpt").is_none());
        assert_eq!(
            finding_measurement(&data, finding)["location"],
            finding["location"]
        );
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
fn typescript_portable_source_patterns_emit_deterministic_evidence() {
    let source = r#"class Record {
  name: string;
  age: number;
}

class Tiny {}

function first(x: number, y: number, z: number) {
  // explain the calculation
  // preserve the invariant
  const value = x + y;
  return value + z;
}

function second(x: number, y: number, z: number) {
  const value = x + y;
  return value + z;
}

function third(x: number, y: number, z: number) {
  return x + y + z;
}
"#;
    let mut policy = portable_policy("typescript");
    require(
        &mut policy,
        "typescript.comment_share",
        json!({"minimum_code_lines":2,"minimum_share_percent":40}),
    );
    require(
        &mut policy,
        "typescript.duplicate_functions",
        json!({"minimum_similarity_basis_points":10000,"minimum_tokens":4}),
    );
    require(
        &mut policy,
        "typescript.data_clumps",
        json!({"minimum_group_size":3,"minimum_declarations":3}),
    );
    require(
        &mut policy,
        "typescript.data_class",
        json!({"minimum_fields":2,"maximum_operations":0}),
    );
    require(
        &mut policy,
        "typescript.lazy_class",
        json!({"maximum_fields":0,"maximum_functions":0,"maximum_lines":0}),
    );
    let workspace = Workspace::new("patterns.ts", source, &policy);
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    for rule in [
        "typescript.comment_share",
        "typescript.duplicate_functions",
        "typescript.data_clumps",
        "typescript.data_class",
        "typescript.lazy_class",
    ] {
        let finding = data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["rule_id"] == rule && finding["evaluation"]["matched"] == true)
            .unwrap_or_else(|| panic!("missing matched {rule}"));
        assert_eq!(finding["blocking"], true);
        assert!(finding.get("source_excerpt").is_none());
        assert_eq!(
            finding_measurement(&data, finding)["location"],
            finding["location"]
        );
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
fn every_portable_authored_source_rule_executes_on_a_matched_fixture() {
    let cases = [
        (
            "python",
            "coverage.py",
            r#"class Record:
    first: int
    second: int

    def read(self):
        return self.first

def first(alpha: int, beta: int, gamma: int):
    # explain
    value = alpha + beta
    return value + gamma

def second(alpha: int, beta: int, gamma: int):
    value = alpha + beta
    return value + gamma
"#,
        ),
        (
            "typescript",
            "coverage.ts",
            r#"class Record {
  first: number;
  second: number;
  read() { return this.first; }
}

function first(alpha: number, beta: number, gamma: number) {
  // explain
  const value = alpha + beta;
  return value + gamma;
}

function second(alpha: number, beta: number, gamma: number) {
  const value = alpha + beta;
  return value + gamma;
}
"#,
        ),
    ];
    for (language, file, source) in cases {
        let mut policy = portable_policy(language);
        require(
            &mut policy,
            &format!("{language}.duplicate_functions"),
            json!({"minimum_similarity_basis_points":10000,"minimum_tokens":4}),
        );
        require(
            &mut policy,
            &format!("{language}.data_clumps"),
            json!({"minimum_group_size":3,"minimum_declarations":2}),
        );
        let workspace = Workspace::new(file, source, &policy);
        let output = workspace.check_path();
        assert!(matches!(output.status.code(), Some(0 | 1)), "{language}");
        let data = report(&output);
        let emitted = measurements(&data)
            .map(|finding| finding["rule_id"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        for suffix in [
            "function_lines",
            "function_arguments",
            "class_fields",
            "class_methods",
            "class_method_lines",
            "comment_share",
            "duplicate_functions",
            "data_clumps",
            "data_class",
            "lazy_class",
        ] {
            let id = format!("{language}.{suffix}");
            assert!(emitted.contains(id.as_str()), "missing execution: {id}");
        }
    }
}

#[test]
fn mixed_signature_comments_are_not_comment_only_lines() {
    for (language, file, source) in [
        (
            "python",
            "mixed.py",
            "def sample(a,  # mixed with signature code\n    b):\n    # comment only\n    return a + b\n",
        ),
        (
            "typescript",
            "mixed.ts",
            "function sample(a: number, // mixed with signature code\n  b: number) {\n  // comment only\n  return a + b;\n}\n",
        ),
    ] {
        let mut policy = portable_policy(language);
        require(
            &mut policy,
            &format!("{language}.comment_share"),
            json!({"minimum_code_lines":1,"minimum_share_percent":60}),
        );
        let workspace = Workspace::new(file, source, &policy);
        let output = workspace.check_path();
        assert_eq!(output.status.code(), Some(0));
        let data = report(&output);
        let finding = measurements(&data)
            .find(|finding| finding["rule_id"] == format!("{language}.comment_share"))
            .expect("comment measurement");
        assert_eq!(finding["evaluation"]["observed"]["comment_lines"], 1);
        assert_eq!(finding["evaluation"]["observed"]["code_lines"], 1);
        assert_eq!(finding["evaluation"]["matched"], false);
    }
}

#[test]
fn duplicate_budget_counts_only_exactly_pruned_candidates() {
    let mut source = String::new();
    for index in 0..450 {
        source.push_str(&format!("def tiny_{index}():\n    return {index}\n\n"));
    }
    source.push_str(
        r#"def duplicate_a(value):
    first = value + 1
    second = first * 2
    third = second - 3
    fourth = third / 4
    return fourth

def duplicate_b(item):
    alpha = item + 8
    beta = alpha * 9
    gamma = beta - 10
    delta = gamma / 11
    return delta
"#,
    );
    let mut policy = portable_policy("python");
    policy["limits"]["maximum_pairs"] = json!(1);
    require_maximum(&mut policy, "python.function_lines", 100);
    require(
        &mut policy,
        "python.duplicate_functions",
        json!({"minimum_similarity_basis_points":10000,"minimum_tokens":20}),
    );
    let workspace = Workspace::new("large.py", &source, &policy);
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    assert!(data["errors"].as_array().unwrap().is_empty());
    let duplicate = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["rule_id"] == "python.duplicate_functions"
                && finding["evaluation"]["matched"] == true
        })
        .expect("exact duplicate candidate");
    assert_eq!(duplicate["symbol"], "duplicate_a");
    assert_eq!(duplicate["related_symbols"], json!(["duplicate_b"]));
    let result = smell_result(&data, "duplicate-code");
    assert_eq!(result["matched_findings"], 1);
    assert_eq!(result["affected_symbols"], 2);
}

#[test]
fn duplicate_budget_still_fails_closed_for_too_many_exact_candidates() {
    let body = r#"    first = value + 1
    second = first * 2
    third = second - 3
    fourth = third / 4
    return fourth
"#;
    let source =
        format!("def first(value):\n{body}\ndef second(value):\n{body}\ndef third(value):\n{body}");
    let mut policy = portable_policy("python");
    policy["limits"]["maximum_pairs"] = json!(1);
    require_maximum(&mut policy, "python.function_lines", 100);
    require(
        &mut policy,
        "python.duplicate_functions",
        json!({"minimum_similarity_basis_points":10000,"minimum_tokens":20}),
    );
    let workspace = Workspace::new("duplicates.py", &source, &policy);
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        report(&output)["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error == "maximum_pairs budget exceeded")
    );
}

fn duplicate_only_policy(language: &str, threshold: u64, maximum_pairs: u64) -> Value {
    let mut policy = if language == "rust" {
        serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap()
    } else {
        portable_policy(language)
    };
    for selection in policy["rules"].as_object_mut().unwrap().values_mut() {
        selection["mode"] = json!("off");
    }
    policy["limits"]["maximum_pairs"] = json!(maximum_pairs);
    policy["rules"][format!("{language}.duplicate_functions")] = selection(
        "required",
        json!({"minimum_similarity_basis_points":threshold,"minimum_tokens":4}),
    );
    policy
}

fn cross_language_duplicate_source(language: &str, functions: usize) -> (&'static str, String) {
    let expressions = [
        "value + value + value + value + value + value",
        "value + value + value + value + value - value",
        "value + value + value + value + value + value",
    ];
    match language {
        "rust" => (
            "lib.rs",
            expressions[..functions]
                .iter()
                .enumerate()
                .map(|(index, body)| format!("fn function_{index}(value:i32)->i32{{{body}}}\n"))
                .collect(),
        ),
        "python" => (
            "module.py",
            expressions[..functions]
                .iter()
                .enumerate()
                .map(|(index, body)| format!("function_{index} = lambda value: {body}\n"))
                .collect(),
        ),
        "typescript" => (
            "module.ts",
            expressions[..functions]
                .iter()
                .enumerate()
                .map(|(index, body)| {
                    format!("const function_{index} = (value: number) => {body};\n")
                })
                .collect(),
        ),
        _ => panic!("unsupported fixture language"),
    }
}

#[test]
fn duplicate_boundaries_and_evidence_are_equal_across_all_language_packs() {
    for threshold in [5_999, 6_000, 6_001] {
        let mut observations = Vec::new();
        for language in ["rust", "python", "typescript"] {
            let (file, source) = cross_language_duplicate_source(language, 2);
            let workspace = Workspace::new(
                file,
                &source,
                &duplicate_only_policy(language, threshold, 10),
            );
            let output = workspace.check_path();
            let data = report(&output);
            let finding =
                data["findings"].as_array().unwrap().iter().find(|finding| {
                    finding["rule_id"] == format!("{language}.duplicate_functions")
                });
            if threshold <= 6_000 {
                let finding = finding.unwrap_or_else(|| {
                    panic!("missing {language} duplicate at threshold {threshold}")
                });
                let measurement = finding_measurement(&data, finding);
                observations.push((
                    finding["evaluation"]["observed"].clone(),
                    measurement["evidence"]["token_counts"].clone(),
                ));
            } else {
                assert!(
                    finding.is_none(),
                    "{language} matched above the exact boundary"
                );
            }
        }
        if threshold <= 6_000 {
            assert_eq!(
                observations,
                vec![(json!({"intersection":6,"union":10}), json!([11, 11])); 3]
            );
        }
    }
}

#[test]
fn duplicate_pair_budget_fails_closed_identically_across_all_language_packs() {
    for language in ["rust", "python", "typescript"] {
        let (file, source) = cross_language_duplicate_source(language, 3);
        let workspace = Workspace::new(file, &source, &duplicate_only_policy(language, 5_000, 1));
        let output = workspace.check_path();
        assert_eq!(output.status.code(), Some(2), "{language}");
        assert!(
            report(&output)["errors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|error| error == "maximum_pairs budget exceeded"),
            "{language} did not fail on the same exact-comparison budget"
        );
    }
}

#[test]
fn portable_metrics_distinguish_tree_nodes_from_rust_lexer_tokens() {
    let mut policy = portable_policy("python");
    policy["rules"]["python.duplicate_functions"]["mode"] = json!("off");
    let workspace = Workspace::new(
        "module.py",
        "def measured(value):\n    return value + 1\n",
        &policy,
    );
    let metrics_path = workspace.path.join("metrics.json");
    let cache_path = workspace.path.join("cache");
    let output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--path",
            ".",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .env("SMELLS_METRICS_FILE", &metrics_path)
        .env("SMELLS_CACHE_DIR", &cache_path)
        .current_dir(&workspace.path)
        .output()
        .expect("run scanner with metrics");
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metrics: Value = serde_json::from_slice(&fs::read(metrics_path).unwrap()).unwrap();
    assert_eq!(metrics["schema_version"], 2);
    assert!(metrics["syntax_nodes"].as_u64().unwrap() > 0);
    assert_eq!(metrics["syntax_tokens"], 0);
    assert_eq!(metrics["fact_cache_misses"], 1);

    let warm_metrics_path = workspace.path.join("warm-metrics.json");
    let warm_output = Command::new(env!("CARGO_BIN_EXE_smells"))
        .args([
            "check",
            "--path",
            ".",
            "--policy",
            "quality-policy.json",
            "--format",
            "json",
        ])
        .env("SMELLS_METRICS_FILE", &warm_metrics_path)
        .env("SMELLS_CACHE_DIR", &cache_path)
        .current_dir(&workspace.path)
        .output()
        .expect("run warm scanner with metrics");
    assert!(matches!(warm_output.status.code(), Some(0 | 1)));
    let warm: Value = serde_json::from_slice(&fs::read(warm_metrics_path).unwrap()).unwrap();
    assert_eq!(warm["syntax_nodes"], 0);
    assert_eq!(warm["syntax_tokens"], 0);
    assert_eq!(warm["fact_cache_hits"], 1);
    assert_eq!(warm["fact_cache_misses"], 0);
    assert!(warm["functions"].as_u64().unwrap() > 0);
}

#[test]
fn typescript_import_type_generic_calls_parse_without_hiding_other_errors() {
    let valid = r#"async function emptyCall(importOriginal: any) {
  return importOriginal<typeof import("reactflow")>()
}
"#;
    let policy: Value =
        serde_json::from_str(include_str!("../examples/typescript-quality-policy.json")).unwrap();
    let workspace = Workspace::new("valid.tsx", valid, &policy);
    fs::write(
        workspace.path.join("trailing.ts"),
        r#"async function trailingCall(importOriginal: any) {
  return importOriginal<typeof import("module")>("module",)
}
"#,
    )
    .unwrap();
    workspace.initialize_git();
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(report(&output)["errors"].as_array().unwrap().is_empty());

    fs::write(
        workspace.path.join("broken.ts"),
        r#"async function compatible(importOriginal: any) {
  await importOriginal<typeof import("module")>()
}
function broken( {
"#,
    )
    .unwrap();
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        report(&output)["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error == "parse error in broken.ts")
    );
}

#[test]
fn typescript_built_in_class_model_honors_direct_member_contract() {
    let mut policy = portable_policy("typescript");
    require_only(
        &mut policy,
        &[
            "typescript.primitive_slots",
            "typescript.alternative_interfaces",
        ],
    );
    require(
        &mut policy,
        "typescript.primitive_slots",
        json!({"minimum_raw_slots": 2, "minimum_share_percent": 100}),
    );
    require(
        &mut policy,
        "typescript.alternative_interfaces",
        json!({"minimum_similarity_basis_points": 1, "minimum_tokens": 4}),
    );
    let workspace = Workspace::new(
        "members.ts",
        concat!(
            "class First {\n",
            "  constructor(private count: number, readonly label: string) {}\n",
            "  execute = (value: number) => value + value + value + 1;\n",
            "}\n",
            "class Second { transform = (value: number) => value + value + value + 1; }\n",
        ),
        &policy,
    );
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    let primitive = matched_rule(&data, "typescript.primitive_slots");
    assert_eq!(primitive["evaluation"]["observed"]["raw_slots"], 2);
    assert!(
        !data["findings"].as_array().unwrap().iter().any(|finding| {
            finding["rule_id"] == "typescript.alternative_interfaces"
                && finding["evaluation"]["matched"] == true
        }),
        "arrow-valued fields are callables, not direct class methods"
    );
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
        assert!(
            registry["smells"]
                .as_array()
                .unwrap()
                .iter()
                .all(|smell| !smell["rules"].as_array().unwrap().is_empty())
        );
        assert_eq!(
            registry["rules"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|rule| rule["implementation"] == "implemented")
                .count(),
            28
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
        assert_eq!(status["implemented_rules"], 28);

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
fn typescript_collects_class_and_callable_expressions_and_generators() {
    let mut policy = portable_policy("typescript");
    require_only(
        &mut policy,
        &[
            "typescript.function_arguments",
            "typescript.primitive_slots",
        ],
    );
    require(
        &mut policy,
        "typescript.primitive_slots",
        json!({"minimum_raw_slots": 2, "minimum_share_percent": 100}),
    );
    let workspace = Workspace::new(
        "expressions.ts",
        concat!(
            "const RecordType = class { first: string; second: number; };\n",
            "const expressed = function(a: number, b: number, c: number, d: number) { return a + b + c + d; };\n",
            "const arrowed = (a: number, b: number, c: number, d: number) => a + b + c + d;\n",
            "function* declared(a: number, b: number, c: number, d: number) { yield a + b + c + d; }\n",
            "const generated = function*(a: number, b: number, c: number, d: number) { yield a + b + c + d; };\n",
        ),
        &policy,
    );
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    let overloaded_callables = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| {
            finding["rule_id"] == "typescript.function_arguments"
                && finding["evaluation"]["matched"] == true
        })
        .count();
    assert_eq!(overloaded_callables, 4);
    let primitive = matched_rule(&data, "typescript.primitive_slots");
    assert_eq!(primitive["evaluation"]["observed"]["raw_slots"], 2);
}

#[test]
fn rust_built_in_model_keeps_same_named_types_in_separate_sources() {
    let mut policy: Value =
        serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap();
    require_only(&mut policy, &["rust.primitive_slots"]);
    let workspace = Workspace::new(
        "lib.rs",
        "struct Config { one: i32, two: i32, three: i32, four: i32, five: i32 }\n",
        &policy,
    );
    workspace.source("other.rs", "struct Config;\n");
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    let findings = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| {
            finding["rule_id"] == "rust.primitive_slots" && finding["evaluation"]["matched"] == true
        })
        .collect::<Vec<_>>();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["symbol"], "lib.rs::Config");
}

#[test]
fn rust_relation_collectors_resolve_qualified_same_named_traits() {
    let mut policy: Value =
        serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap();
    require_only(&mut policy, &["rust.port_conformance"]);
    let workspace = Workspace::new(
        "lib.rs",
        concat!(
            "mod a { pub trait Port { fn save(&self); } }\n",
            "mod b { pub trait Port { fn remove(&self); } }\n",
            "struct Service;\n",
            "impl a::Port for Service { fn save(&self) {} }\n",
        ),
        &policy,
    );
    let output = workspace.check_path();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let data = report(&output);
    let finding = measurements(&data)
        .find(|finding| finding["rule_id"] == "rust.port_conformance")
        .expect("port conformance measurement");
    assert_eq!(finding["evaluation"]["observed"], 0);
    assert_eq!(finding["evaluation"]["matched"], false);
    assert_eq!(finding["related_symbols"], json!(["lib.rs::a::Port"]));
}

#[test]
fn built_in_text_collectors_ignore_comments_strings_and_imports() {
    for (language, file, source) in [
        (
            "python",
            "masked.py",
            concat!(
                "import fake.deep.navigation as imported\n",
                "# order.customer.address.city customer._secret\n",
                "TEXT = 'order.customer.address.city customer._secret External.prototype.patch = replacement'\n",
                "def clean(value):\n    return value\n",
            ),
        ),
        (
            "typescript",
            "masked.ts",
            concat!(
                "import { value } from 'fake.deep.navigation';\n",
                "// order.customer.address.city customer._secret\n",
                "const text = 'order.customer.address.city customer._secret External.prototype.patch = replacement';\n",
                "function clean(value: number) { return value; }\n",
            ),
        ),
    ] {
        let mut policy = portable_policy(language);
        let rules = [
            format!("{language}.navigation_chains"),
            format!("{language}.foreign_accesses"),
            format!("{language}.dependency_contract"),
            format!("{language}.library_capabilities"),
        ];
        require_only(
            &mut policy,
            &rules.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let workspace = Workspace::new(file, source, &policy);
        let output = workspace.check_path();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{language}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let data = report(&output);
        assert!(
            data["findings"].as_array().unwrap().iter().all(|finding| {
                finding["evaluation"]["matched"] == false
                    || !rules
                        .iter()
                        .any(|rule_id| finding["rule_id"] == rule_id.as_str())
            }),
            "{language}: {data}"
        );
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
    let finding = measurements(&data)
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
fn required_portable_rule_uses_the_built_in_collector_without_evidence() {
    let mut policy = portable_policy("python");
    require(
        &mut policy,
        "python.refused_bequest",
        json!({"minimum_inherited_members":3,"minimum_unused_percent":80}),
    );
    let workspace = Workspace::new("model.py", "class Child:\n    pass\n", &policy);
    let output = workspace.check_path();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    assert_ne!(data["summary"]["verdict"], "incomplete_due_to_errors");
    assert_eq!(data["errors"], json!([]));
    assert!(
        smell_result(&data, "refused-bequest")["measured_rule_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule == "python.refused_bequest")
    );
}

#[test]
fn complete_provider_evidence_is_pinned_and_evaluated_by_the_scanner() {
    let mut policy = portable_policy("python");
    require(
        &mut policy,
        "python.refused_bequest",
        json!({"minimum_inherited_members":3,"minimum_unused_percent":80}),
    );
    let workspace = Workspace::new("model.py", "class Child:\n    pass\n", &policy);
    let missing = workspace.check_path();
    assert_eq!(missing.status.code(), Some(0));
    let input_sha256 = report(&missing)["input_sha256"]
        .as_str()
        .unwrap()
        .to_string();
    let evidence = json!({
        "schema_version": 1,
        "scanner_version": "0.5.0",
        "rule_pack": "python-v1",
        "input_sha256": input_sha256,
        "providers": [{
            "rule_id": "python.refused_bequest",
            "provider": {
                "name": "fixture-type-graph",
                "version": "1",
                "configuration_sha256": "0".repeat(64)
            },
            "complete": true,
            "observations": [{
                "symbol": "Child",
                "location": {"path":"model.py","line":1,"column":1},
                "related_symbols": [],
                "related_locations": [],
                "measurements": {"inherited_members":5,"unused_members":4},
                "evidence": {"inheritance_graph":"fixture"}
            }]
        }]
    });
    fs::write(
        workspace.path.join("evidence.json"),
        serde_json::to_vec(&evidence).unwrap(),
    )
    .unwrap();
    let output = workspace.check_path_with_evidence("evidence.json");
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    assert!(data["provider_evidence_sha256"].as_str().unwrap().len() == 64);
    let finding = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["rule_id"] == "python.refused_bequest")
        .expect("provider finding");
    assert_eq!(finding["evaluation"]["matched"], true);
    assert_eq!(finding["evaluation"]["observed"]["inherited_members"], 5);
    assert_eq!(finding["blocking"], true);
    assert!(finding.get("source_excerpt").is_none());
    assert_eq!(finding_measurement(&data, finding)["location"]["line"], 1);
}

#[test]
fn staged_provider_evidence_cannot_be_replaced_by_unstaged_bytes() {
    let mut policy = portable_policy("typescript");
    require(
        &mut policy,
        "typescript.navigation_chains",
        json!({"minimum_transitions":3}),
    );
    let workspace = Workspace::new(
        "service.ts",
        "export const service = { client: { account: { name: 'ok' } } };\n",
        &policy,
    );
    workspace.stage();
    let missing = workspace.check_staged();
    assert_eq!(missing.status.code(), Some(0));
    let input_sha256 = report(&missing)["input_sha256"]
        .as_str()
        .unwrap()
        .to_string();
    let evidence = json!({
        "schema_version": 1,
        "scanner_version": "0.5.0",
        "rule_pack": "typescript-v1",
        "input_sha256": input_sha256,
        "providers": [{
            "rule_id": "typescript.navigation_chains",
            "provider": {
                "name": "fixture-type-resolver",
                "version": "1",
                "configuration_sha256": "a".repeat(64)
            },
            "complete": true,
            "observations": [{
                "symbol": "service",
                "location": {"path":"service.ts","line":1,"column":14},
                "related_symbols": [],
                "related_locations": [],
                "measurements": {"transitions":3},
                "evidence": {"owners":["Service","Client","Account","Name"]}
            }]
        }]
    });
    fs::write(
        workspace.path.join("evidence.json"),
        serde_json::to_vec(&evidence).unwrap(),
    )
    .unwrap();
    let add = Command::new("git")
        .args(["add", "evidence.json"])
        .current_dir(&workspace.path)
        .output()
        .unwrap();
    assert!(add.status.success());
    fs::write(workspace.path.join("evidence.json"), b"not staged evidence").unwrap();

    let output = workspace.check_staged_with_evidence("evidence.json");
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = report(&output);
    assert_eq!(
        data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["rule_id"] == "typescript.navigation_chains")
            .unwrap()["evaluation"]["matched"],
        true
    );
}
