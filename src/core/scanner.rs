use crate::{
    core::{
        model::{CapturedInput, ScanRequest},
        ports::{inbound::Scanner, outbound::Repository},
    },
    evidence, metrics, portable,
    report::Report,
    scan, suppressions,
};

pub(crate) struct ScanEngine<'a> {
    repository: &'a dyn Repository,
}

impl<'a> ScanEngine<'a> {
    pub(crate) fn new(repository: &'a dyn Repository) -> Self {
        Self { repository }
    }
}

impl Scanner for ScanEngine<'_> {
    fn check(&self, request: &ScanRequest<'_>) -> Result<Report, String> {
        let captured_repository = self.repository.capture(request)?;
        metrics::add(
            metrics::Counter::Files,
            captured_repository.captured.input.files.len(),
        );
        build_report(
            &captured_repository.captured,
            captured_repository.evidence_bytes.as_deref(),
        )
    }
}

fn build_report(captured: &CapturedInput, evidence_bytes: Option<&[u8]>) -> Result<Report, String> {
    let (evidence, evidence_sha256) = load_evidence(evidence_bytes, captured)?;
    let mut report = analyze(captured)?;
    report.set_provider_evidence_digest(evidence_sha256);
    evidence::apply(
        evidence.as_ref(),
        &captured.input,
        &captured.registry,
        &mut report,
    );
    report.apply_suppressions(suppressions::parse(
        &captured.input.files,
        &captured.registry,
    ));
    report.attach_sources(&captured.input.files);
    report.finish();
    Ok(report)
}

fn analyze(captured: &CapturedInput) -> Result<Report, String> {
    Ok(match captured.registry.language.as_str() {
        "rust" => scan::check(&captured.input, &captured.registry),
        "python" | "typescript" => portable::check(&captured.input, &captured.registry),
        language => return Err(format!("unsupported registry language: {language}")),
    })
}

fn load_evidence(
    bytes: Option<&[u8]>,
    captured: &CapturedInput,
) -> Result<(Option<evidence::EvidenceBundle>, String), String> {
    let Some(bytes) = bytes else {
        return Ok((None, String::new()));
    };
    let bundle = evidence::parse(bytes)?;
    evidence::validate(&bundle, &captured.input, &captured.registry)?;
    Ok((Some(bundle), evidence::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, collections::BTreeMap, path::Path};

    use serde_json::json;

    use super::*;
    use crate::{
        core::model::{CapturedRepository, ScanTarget},
        core::model::{History, Implementation, Input},
        policy::SelectionOptions,
    };

    struct InMemoryRepository {
        captures: Cell<usize>,
        failure: Option<&'static str>,
        evidence_bytes: Option<Vec<u8>>,
    }

    impl InMemoryRepository {
        fn snapshot() -> Self {
            Self {
                captures: Cell::new(0),
                failure: None,
                evidence_bytes: None,
            }
        }

        fn failing(message: &'static str) -> Self {
            Self {
                captures: Cell::new(0),
                failure: Some(message),
                evidence_bytes: None,
            }
        }

        fn with_evidence(bytes: &[u8]) -> Self {
            Self {
                captures: Cell::new(0),
                failure: None,
                evidence_bytes: Some(bytes.to_vec()),
            }
        }
    }

    impl Repository for InMemoryRepository {
        fn capture(&self, _request: &ScanRequest<'_>) -> Result<CapturedRepository, String> {
            self.captures.set(self.captures.get() + 1);
            if let Some(message) = self.failure {
                return Err(message.into());
            }
            Ok(CapturedRepository {
                captured: captured_input(),
                evidence_bytes: self.evidence_bytes.clone(),
            })
        }
    }

    fn captured_input() -> CapturedInput {
        let registry = crate::policy::registry("rust-v1").unwrap();
        let policy = crate::policy::parse(
            include_str!("../../examples/quality-policy.json"),
            &registry,
        )
        .unwrap();
        let files = BTreeMap::from([(
            "src/lib.rs".into(),
            "pub fn answer() -> usize { 42 }\n".into(),
        )]);
        CapturedInput {
            input: Input {
                files,
                manifests: BTreeMap::new(),
                artifacts: BTreeMap::new(),
                artifact_candidates: BTreeMap::new(),
                implementations: vec![Implementation {
                    id: ".".into(),
                    root: Some(".".into()),
                    ownership: "runtime_manifest",
                    implementation_types: vec!["rust_cargo_project".into()],
                    runtime_types: vec!["rust".into()],
                    runtime_manifests: vec!["Cargo.toml".into()],
                    source_files: vec!["src/lib.rs".into()],
                }],
                source_roles: Vec::new(),
                history: History {
                    available: true,
                    commits: Vec::new(),
                },
                policy,
                digest: "in-memory-snapshot".into(),
                mode: "working_tree",
            },
            registry,
        }
    }

    fn request(selection: &SelectionOptions) -> ScanRequest<'_> {
        ScanRequest {
            target: ScanTarget::WorkingTree(Path::new("unused-by-in-memory-repository")),
            policy_path: Path::new("quality-policy.json"),
            evidence_path: None,
            selection,
        }
    }

    #[test]
    fn scanner_completes_a_report_from_the_repository_snapshot() {
        let repository = InMemoryRepository::snapshot();
        let selection = SelectionOptions::default();

        let scanner = ScanEngine::new(&repository);
        let report = scanner.check(&request(&selection)).unwrap();
        let report = serde_json::to_value(report).unwrap();

        assert_eq!(repository.captures.get(), 1);
        assert_eq!(report["source_mode"], json!("working_tree"));
        assert_eq!(report["scanned_files"], json!(["src/lib.rs"]));
        assert_eq!(report["summary"]["implementations"], json!(1));
        assert_eq!(report["summary"]["error_count"], json!(0));
    }

    #[test]
    fn scanner_propagates_repository_failures() {
        let repository = InMemoryRepository::failing("snapshot unavailable");
        let selection = SelectionOptions::default();

        let scanner = ScanEngine::new(&repository);
        let error = match scanner.check(&request(&selection)) {
            Ok(_) => panic!("repository failure must stop the scan"),
            Err(error) => error,
        };

        assert_eq!(repository.captures.get(), 1);
        assert_eq!(error, "snapshot unavailable");
    }

    #[test]
    fn scanner_rejects_invalid_provider_evidence_from_the_repository() {
        let repository = InMemoryRepository::with_evidence(b"not JSON");
        let selection = SelectionOptions::default();

        let scanner = ScanEngine::new(&repository);
        let error = match scanner.check(&request(&selection)) {
            Ok(_) => panic!("invalid evidence must stop the scan"),
            Err(error) => error,
        };

        assert_eq!(repository.captures.get(), 1);
        assert!(error.starts_with("invalid provider evidence:"));
    }
}
