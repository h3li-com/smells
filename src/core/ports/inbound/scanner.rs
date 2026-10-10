use crate::{input::CapturedInput, report::Report};

/// Primary port for deterministic smell analysis.
pub(crate) trait Scanner {
    fn check(
        &self,
        captured: &CapturedInput,
        evidence_bytes: Option<&[u8]>,
    ) -> Result<Report, String>;
}
