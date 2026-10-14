use crate::core::model::{CapturedRepository, ScanRequest};

/// Secondary port through which the scanner obtains repository snapshots.
pub(crate) trait Repository {
    fn capture(&self, request: &ScanRequest<'_>) -> Result<CapturedRepository, String>;
}
