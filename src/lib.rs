mod adapters;
mod catalog;
mod collectors;
mod core;
mod evidence;
mod evidence_evaluators;
mod input;
mod metrics;
mod migration;
mod patterns;
mod policy;
mod portable;
mod report;
mod rule_runtime;
mod scan;
mod similarity;
mod suppressions;
mod typescript_compat;

/// Runs the command-line adapter with already-collected process arguments.
///
/// The binary owns process concerns such as reading the environment and
/// selecting an exit code. The adapter owns command parsing and presentation.
pub fn run(args: Vec<String>) -> Result<u8, String> {
    adapters::inbound::cli::run(args)
}
