use std::path::Path;

use crate::{input::CapturedInput, policy::SelectionOptions};

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
    pub(crate) selection: &'a SelectionOptions,
}

/// Complete repository input returned through the repository outbound port.
pub(crate) struct CapturedRepository {
    pub(crate) captured: CapturedInput,
    pub(crate) evidence_bytes: Option<Vec<u8>>,
}
