use serde_json::{Value, json};
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
        fs::write(self.path.join(path), source).unwrap();
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
        .find(|f| f["rule_id"] == rule && f["symbol"].as_str().unwrap().ends_with(suffix))
        .unwrap_or_else(|| panic!("missing {rule} {suffix}"))
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
        17
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
}

#[test]
fn shared_modules_do_not_invent_distinct_clumps_or_duplicate_bodies() {
    let workspace = Workspace::new("mod shared;");
    workspace.source("src/main.rs", "mod shared;");
    workspace.source("src/shared.rs", "fn a(x:i32,y:i32,z:i32)->i32{let p=x+1;let q=y+p;let r=z*q;r}\nfn b(x:i32,y:i32,z:i32)->i32{let p=x+2;let q=y+p;let r=z*q;r}");
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);
    assert!(!matched(&data, "rust.data_clumps"));
    let pairs: Vec<_> = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule_id"] == "rust.duplicate_functions")
        .collect();
    assert_eq!(pairs.len(), 1);
    assert_ne!(pairs[0]["location"], pairs[0]["evidence"]["other_location"]);
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
fn catalog_fixture_exercises_every_implemented_rule() {
    let workspace = Workspace::new(include_str!("fixtures/catalog/lib.rs"));
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0));
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
            data["findings"]
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
                "review_or_remediation"
            );
            assert_eq!(
                rule["reference_check"]["unavailable_action"],
                "report_reference_research_incomplete_and_do_not_review_or_remediate"
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
    assert_eq!(data["report_schema_version"], 3);
    let results = data["smell_results"].as_array().unwrap();
    assert_eq!(results.len(), 23);
    assert_eq!(results[0]["smell_id"], "long-method");
    assert_eq!(results[1]["smell_id"], "large-class");

    let large_class = smell_result(&data, "large-class");
    assert_eq!(large_class["smell"], "Large Class");
    assert_eq!(large_class["category"], "bloaters");
    assert_eq!(large_class["state"], "blocking_match");
    assert_eq!(
        large_class["coverage_status"],
        "measured_defined_source_scope"
    );
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
fn smell_results_distinguish_checked_pending_and_inapplicable_patterns() {
    let workspace = Workspace::new("fn concise() {}");
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(0));
    let data = report(&output);

    let parameters = smell_result(&data, "long-parameter-list");
    assert_eq!(parameters["state"], "checked_no_match_in_measured_scope");
    assert_eq!(
        parameters["coverage_status"],
        "measured_defined_source_scope"
    );
    assert_eq!(parameters["matched_findings"], 0);

    let long_method = smell_result(&data, "long-method");
    assert_eq!(long_method["state"], "checked_no_match_in_measured_scope");
    assert_eq!(
        long_method["coverage_status"],
        "measured_with_pending_rules"
    );
    assert_eq!(
        long_method["pending_rule_ids"],
        json!(["rust.function_crap"])
    );

    let dead_code = smell_result(&data, "dead-code");
    assert_eq!(dead_code["state"], "pending");
    assert_eq!(dead_code["coverage_status"], "pending");
    assert_eq!(dead_code["pending_rule_ids"], json!(["rust.unused_code"]));

    let refused_bequest = smell_result(&data, "refused-bequest");
    assert_eq!(refused_bequest["state"], "not_applicable");
    assert_eq!(refused_bequest["coverage_status"], "not_applicable");
}

#[test]
fn matched_findings_are_self_contained_llm_diagnostics() {
    let workspace = Workspace::new(&format!(
        "{}fn overloaded(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32){{}}",
        " ".repeat(500)
    ));
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(1));
    let data = report(&output);
    assert_eq!(data["summary"]["verdict"], "blocked_by_required_patterns");
    let finding = finding(&data, "rust.function_arguments", "::overloaded");
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
        "review_or_remediation"
    );
    assert_eq!(
        finding["diagnostic"]["reference_check"]["unavailable_action"],
        "report_reference_research_incomplete_and_do_not_review_or_remediate"
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
    let excerpt = &finding["source_excerpt"];
    assert_eq!(excerpt["focus_line"], 1);
    assert!(excerpt["focus_column"].as_u64().unwrap() > 500);
    assert!(
        excerpt["lines"][0]["text"]
            .as_str()
            .unwrap()
            .contains("fn overloaded")
    );
    assert_eq!(excerpt["lines"][0]["truncated_before"], true);
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
        assert_eq!(output.status.code(), Some(if lines > 100 { 1 } else { 0 }));
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
        assert_eq!(output.status.code(), Some(if count > 7 { 1 } else { 0 }));
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
        assert_eq!(output.status.code(), Some(if count > 15 { 1 } else { 0 }));
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
        assert_eq!(output.status.code(), Some(if count > 20 { 1 } else { 0 }));
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
        assert_eq!(output.status.code(), Some(if count > 20 { 1 } else { 0 }));
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
        assert_eq!(output.status.code(), Some(if lines > 500 { 1 } else { 0 }));
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
        "--policy",
        "quality-policy.json",
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
}

#[test]
fn required_missing_detectors_error_instead_of_passing() {
    let catalog: Value = serde_json::from_str(include_str!("../rules/rust-v1.json")).unwrap();
    let pending: Vec<_> = catalog["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rule| rule["implementation"] == "not_implemented")
        .map(|rule| rule["id"].as_str().unwrap())
        .collect();
    assert_eq!(pending.len(), 11);
    for rule in pending {
        let workspace = Workspace::new("fn f(){}");
        workspace.modify(&format!("/rules/{rule}/mode"), json!("required"));
        let output = workspace.check();
        assert_eq!(output.status.code(), Some(2), "{rule}");
        assert!(
            report(&output)["errors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|error| error.as_str().unwrap().contains(rule)),
            "{rule}"
        );
        let data = report(&output);
        let smell_id = catalog["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|definition| definition["id"] == rule)
            .unwrap()["smell"]
            .as_str()
            .unwrap();
        assert_eq!(smell_result(&data, smell_id)["state"], "error", "{rule}");
    }
}

#[test]
fn malformed_unknown_duplicate_and_out_of_range_policy_values_error() {
    let workspace = Workspace::new("fn f(){}");
    for (pointer, value) in [
        ("/schema_version", json!(1)),
        ("/scanner_version", json!("latest")),
        ("/rules/rust.function_lines/version", json!(2)),
        (
            "/rules/rust.primitive_slots/parameters/minimum_share_percent",
            json!(101),
        ),
        ("/exceptions", json!([{"reason":"ignore everything"}])),
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
        "fn f(){struct Local;}",
        "#[cfg(test)]struct A;#[cfg(not(test))]struct A;",
        "#[cfg(test)]mod a{}#[cfg(not(test))]mod a{}",
        "#[cfg(test)]use external::A;#[cfg(not(test))]use another::A;",
    ] {
        let workspace = Workspace::new(source);
        assert_eq!(workspace.check().status.code(), Some(2), "{source}");
    }
}

#[test]
fn conditional_module_paths_do_not_silently_pick_the_default_file() {
    let workspace =
        Workspace::new("#[cfg_attr(feature=\"alternate\",path=\"other.rs\")]mod child;");
    workspace.source("src/child.rs", "struct A;");
    workspace.source("src/other.rs", "struct B;");
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        report(&output)["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error.as_str().unwrap().contains("conditional module path"))
    );
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
        assert_eq!(workspace.check().status.code(), Some(2), "{pointer}");
    }
}

#[test]
fn all_authored_cfg_branches_are_explicitly_the_scan_scope() {
    let workspace = Workspace::new(
        "#[cfg(test)] fn test_only(a:i32,b:i32,c:i32,d:i32,e:i32,f:i32,g:i32,h:i32){}",
    );
    let output = workspace.check();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(report(&output)["scope"], "authored_all_cfg");
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
    assert_eq!(report(&output)["status"], "valid_contracts");
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
fn duplicate_similarity_compares_the_fraction_without_rounding() {
    let source =
        "fn a()->i32{let x=1;let y=x+2;let z=y*3;z} fn b(){let x=9;let y=x+8;let z=y*7;z;}";
    for (threshold, expected) in [(9444, true), (9445, false)] {
        let workspace = Workspace::new(source);
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
            assert_eq!(
                finding["related_source_excerpts"].as_array().unwrap().len(),
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
        data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["rule_id"] == "rust.function_lines")
            .count(),
        3
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
