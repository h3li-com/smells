//! Naming conventions that both source analysis and the portable extractor read,
//! so they live apart from either.

/// Whether a name is a dunder such as `__init__` or `__html__`: wrapped in double
/// underscores around a non-empty name.
pub(crate) fn dunder(name: &str) -> bool {
    name.len() > 4 && name.starts_with("__") && name.ends_with("__")
}
