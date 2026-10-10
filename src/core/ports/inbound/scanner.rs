use crate::{
    core::{model::ScanRequest, ports::outbound::Repository},
    report::Report,
};

/// Primary port for deterministic smell analysis.
pub(crate) trait Scanner {
    fn check(
        &self,
        repository: &dyn Repository,
        request: &ScanRequest<'_>,
    ) -> Result<Report, String>;
}
