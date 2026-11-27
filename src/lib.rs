//! Command-line code-smell scanner.
//!
//! The binary is the product; this library exposes only [`run`], so `main.rs`
//! stays a thin wrapper that owns process concerns such as the exit code.

mod adapters;
mod catalog;
mod collectors;
mod core;
mod coverage;
mod doctor;
mod evidence;
mod evidence_evaluators;
mod input;
mod metrics;
mod migration;
mod names;
mod patterns;
mod policy;
mod portable;
mod report;
mod rule_runtime;
mod scan;
mod similarity;
mod suppression_markers;
mod suppressions;
mod typescript_compat;

use adapters::outbound::repository::LocalRepository;
use core::ScanEngine;
use std::fmt;

/// A failure that stops a run, such as a bad argument or an unreadable policy.
///
/// The message is already written for the user, so [`Display`](fmt::Display)
/// prints it unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// Runs the command-line adapter with already-collected process arguments.
///
/// The binary owns process concerns such as reading the environment and
/// selecting an exit code. The adapter owns command parsing and presentation.
///
/// # Errors
///
/// Returns an [`Error`] when the arguments are invalid or the scan cannot
/// complete, for example because a policy or source file cannot be read.
pub fn run(args: &[String]) -> Result<u8, Error> {
    let repository = LocalRepository;
    let scanner = ScanEngine::new(&repository);
    adapters::inbound::cli::run(args, &scanner).map_err(|message| Error { message })
}
