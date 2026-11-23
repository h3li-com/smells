use std::path::Path;

use crate::policy::{Policy, SelectionOptions};

use super::CapturedInput;

/// Repository state selected for one scan.
#[derive(Clone, Copy)]
pub(crate) enum ScanTarget<'a> {
    WorkingTree(&'a Path),
    Staged,
}

/// Request accepted by the scanner's inbound port.
pub(crate) struct ScanRequest<'a> {
    pub(crate) target: ScanTarget<'a>,
    pub(crate) policy_path: &'a Path,
    pub(crate) evidence_path: Option<&'a Path>,
    pub(crate) base_ref: Option<&'a str>,
    pub(crate) selection: &'a SelectionOptions,
}

/// Reference Git snapshot captured when running a Debt Envelope Change Check.
pub(crate) struct ReferenceSnapshot {
    pub(crate) reference_ref: String,
    pub(crate) reference_commit: String,
    pub(crate) captured: Option<CapturedInput>,
    pub(crate) reference_policy: Option<Policy>,
    pub(crate) reference_policy_sha256: Option<String>,
}

/// Complete repository input returned through the repository outbound port.
pub(crate) struct CapturedRepository {
    pub(crate) captured: CapturedInput,
    pub(crate) evidence_bytes: Option<Vec<u8>>,
    pub(crate) reference: Option<ReferenceSnapshot>,
}
