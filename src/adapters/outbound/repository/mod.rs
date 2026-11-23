use std::fs;

use crate::{
    core::{
        model::{CapturedRepository, ScanRequest, ScanTarget},
        ports::outbound::Repository,
    },
    input,
};

/// Filesystem and Git implementation of the repository outbound port.
pub(crate) struct LocalRepository;

impl Repository for LocalRepository {
    fn capture(&self, request: &ScanRequest<'_>) -> Result<CapturedRepository, String> {
        if request.base_ref.is_some() && request.evidence_path.is_some() {
            return Err("--evidence cannot be used with --base".into());
        }
        match request.target {
            ScanTarget::WorkingTree(source) => capture_working_tree(source, request),
            ScanTarget::Staged => capture_staged(request),
        }
    }
}

fn capture_working_tree(
    source: &std::path::Path,
    request: &ScanRequest<'_>,
) -> Result<CapturedRepository, String> {
    let captured = input::working_tree(source, request.policy_path, request.selection)?;
    let evidence_bytes = request
        .evidence_path
        .map(|path| {
            fs::read(path).map_err(|error| format!("cannot read provider evidence: {error}"))
        })
        .transpose()?;
    let reference = request
        .base_ref
        .map(|base_ref| {
            input::git_ref_for_working_tree(
                source,
                base_ref,
                request.policy_path,
                request.selection,
            )
        })
        .transpose()?;
    Ok(CapturedRepository {
        captured,
        evidence_bytes,
        reference,
    })
}

fn capture_staged(request: &ScanRequest<'_>) -> Result<CapturedRepository, String> {
    let (captured, evidence_bytes) = input::staged(
        request.policy_path,
        request.evidence_path,
        request.selection,
    )?;
    let reference = request
        .base_ref
        .map(|base_ref| input::git_ref_for_staged(base_ref, request.policy_path, request.selection))
        .transpose()?;
    Ok(CapturedRepository {
        captured,
        evidence_bytes,
        reference,
    })
}
