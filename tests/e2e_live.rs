use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct StagedRepository {
    path: PathBuf,
}

impl StagedRepository {
    fn with_required_data_class_finding() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("smells-live-e2e-{}-{nonce}", std::process::id()));
        fs::create_dir_all(path.join("src")).expect("create live E2E repository");
        fs::write(
            path.join("src/lib.rs"),
            include_str!("fixtures/catalog/lib.rs"),
        )
        .expect("write source fixture");

        let mut policy: Value =
            serde_json::from_str(include_str!("../examples/quality-policy.json"))
                .expect("example policy parses");
        *policy
            .pointer_mut("/rules/rust.data_class/mode")
            .expect("data-class policy entry") = json!("required");
        fs::write(
            path.join("quality-policy.json"),
            serde_json::to_vec(&policy).expect("serialize policy"),
        )
        .expect("write policy");

        let repository = Self { path };
        repository.git(&["init", "--quiet"]);
        repository.git(&["add", "src/lib.rs", "quality-policy.json"]);
        repository
    }

    fn git(&self, arguments: &[&str]) {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(&self.path)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn run_hook(&self) -> Output {
        let scanner_directory = Path::new(env!("CARGO_BIN_EXE_smells"))
            .parent()
            .expect("scanner binary directory");
        let mut search_path = vec![scanner_directory.to_path_buf()];
        if let Some(existing) = env::var_os("PATH") {
            search_path.extend(env::split_paths(&existing));
        }
        let search_path = env::join_paths(search_path).expect("join PATH");
        Command::new("sh")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("hooks/pre-commit.example"))
            .current_dir(&self.path)
            .env("PATH", search_path)
            .output()
            .expect("run example hook")
    }
}

impl Drop for StagedRepository {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("remove live E2E repository");
    }
}

#[test]
#[ignore = "requires live network; run with: cargo test --locked --test e2e_live -- --ignored --nocapture"]
fn failing_hook_exposes_a_reference_that_can_be_researched_live() {
    let repository = StagedRepository::with_required_data_class_finding();
    let hook = repository.run_hook();
    assert_eq!(
        hook.status.code(),
        Some(1),
        "hook must block the required finding: {}",
        String::from_utf8_lossy(&hook.stderr)
    );

    let summary = String::from_utf8(hook.stdout).expect("hook summary is UTF-8");
    assert!(summary.contains("Finding Log: smells-findings.log"));
    assert!(summary.contains("Start at the Issue Index"));
    let report: Value = serde_json::from_slice(
        &fs::read(repository.path.join("smells-report.json")).expect("saved Evidence Report"),
    )
    .expect("saved report is JSON");
    assert_eq!(report["summary"]["verdict"], "blocked_by_required_patterns");
    let finding = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .find(|finding| {
            finding["rule_id"] == "rust.data_class"
                && finding["status"] == "violation"
                && finding["evaluation"]["matched"] == true
        })
        .expect("hook reports a blocking Data Class finding");

    let diagnostic = &finding["diagnostic"];
    let reference_url = diagnostic["reference_url"]
        .as_str()
        .expect("finding has a reference URL");
    assert_eq!(reference_url, "https://refactoring.guru/smells/data-class");
    assert_eq!(diagnostic["reference_check"]["required"], true);
    assert_eq!(diagnostic["reference_check"]["non_negotiable"], true);
    assert_eq!(
        diagnostic["reference_check"]["action"],
        "perform_external_research_call_to_reference_url"
    );
    assert_eq!(
        diagnostic["reference_check"]["required_before"],
        "review_remediation_or_suppression"
    );
    assert!(
        diagnostic["review"]
            .as_str()
            .expect("review guidance")
            .contains(reference_url),
        "review must direct the research call to the emitted URL"
    );

    println!("researching {reference_url}");
    let research = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--connect-timeout",
            "10",
            "--max-time",
            "30",
            "--user-agent",
            "smells-live-e2e/0.1",
            reference_url,
        ])
        .output()
        .expect("curl is required for the opt-in live E2E test");
    assert!(
        research.status.success(),
        "live reference research failed: {}",
        String::from_utf8_lossy(&research.stderr)
    );

    let page = String::from_utf8(research.stdout).expect("reference page is UTF-8 HTML");
    assert!(page.contains("Data Class"), "wrong smell page returned");
    assert!(
        page.contains("Signs and Symptoms"),
        "reference page does not contain its smell guidance"
    );
}

fn visible_page_text(html: &str) -> String {
    let mut visible = String::with_capacity(html.len());
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => visible.push(character),
            _ => {}
        }
    }
    for (entity, replacement) in [
        ("&rsquo;", "’"),
        ("&#8217;", "’"),
        ("&#x27;", "'"),
        ("&#39;", "'"),
        ("&quot;", "\""),
        ("&amp;", "&"),
        ("&nbsp;", " "),
    ] {
        visible = visible.replace(entity, replacement);
    }
    visible.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
#[ignore = "requires live network; run with: cargo test --locked --test e2e_live -- --ignored --nocapture"]
fn embedded_refactoring_guru_when_to_ignore_quotes_are_fresh() {
    let catalog: Value = serde_json::from_str(include_str!("../rules/when-to-ignore-v1.json"))
        .expect("embedded guidance catalog parses");
    let records = catalog["records"].as_array().unwrap();
    let verbatim = records
        .iter()
        .filter(|record| record["provenance"] == "refactoring_guru_verbatim")
        .collect::<Vec<_>>();
    assert_eq!(verbatim.len(), 13);
    for record in verbatim {
        let url = record["source_url"].as_str().unwrap();
        let response = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--connect-timeout",
                "10",
                "--max-time",
                "30",
                "--user-agent",
                "smells-guidance-freshness/0.5.0",
                url,
            ])
            .output()
            .expect("curl is required for the opt-in freshness audit");
        assert!(
            response.status.success(),
            "failed to fetch {url}: {}",
            String::from_utf8_lossy(&response.stderr)
        );
        let page = visible_page_text(&String::from_utf8(response.stdout).unwrap());
        for item in record["items"].as_array().unwrap() {
            let item = item
                .as_str()
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                page.contains(&item),
                "embedded quote changed at {url}: {item:?}"
            );
        }
    }
}
