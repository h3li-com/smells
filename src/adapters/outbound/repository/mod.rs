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
        match request.target {
            ScanTarget::WorkingTree(source) => {
                let captured = input::working_tree(source, request.policy_path, request.selection)?;
                let evidence_bytes = request
                    .evidence_path
                    .map(|path| {
                        fs::read(path)
                            .map_err(|error| format!("cannot read provider evidence: {error}"))
                    })
                    .transpose()?;
                Ok(CapturedRepository {
                    captured,
                    evidence_bytes,
                })
            }
            ScanTarget::Staged => {
                let (captured, evidence_bytes) = input::staged(
                    request.policy_path,
                    request.evidence_path,
                    request.selection,
                )?;
                Ok(CapturedRepository {
                    captured,
                    evidence_bytes,
                })
            }
        }
    }
}
