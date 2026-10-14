use crate::{core::model::ScanRequest, report::Report};

/// Primary port for deterministic smell analysis.
pub(crate) trait Scanner {
    fn check(&self, request: &ScanRequest<'_>) -> Result<Report, String>;
}
