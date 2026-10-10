use crate::{
    core::ports::inbound::Scanner, evidence, input::CapturedInput, portable, report::Report, scan,
    suppressions,
};

pub(crate) struct ScanEngine;

impl Scanner for ScanEngine {
    fn check(
        &self,
        captured: &CapturedInput,
        evidence_bytes: Option<&[u8]>,
    ) -> Result<Report, String> {
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
