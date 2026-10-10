use std::collections::BTreeMap;

use crate::policy::{Policy, Registry};

use super::ResolvedSourceRole;

pub struct Input {
    pub files: BTreeMap<String, String>,
    pub artifacts: BTreeMap<String, Vec<u8>>,
    pub implementations: Vec<Implementation>,
    pub source_roles: Vec<ResolvedSourceRole>,
    pub history: History,
    pub policy: Policy,
    pub digest: String,
    pub mode: &'static str,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub available: bool,
    pub commits: Vec<HistoryCommit>,
}

#[derive(Clone, Debug)]
pub struct HistoryCommit {
    pub id: String,
    pub files: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Implementation {
    pub id: String,
    pub root: Option<String>,
    pub ownership: &'static str,
    pub implementation_types: Vec<String>,
    pub runtime_types: Vec<String>,
    pub runtime_manifests: Vec<String>,
    pub source_files: Vec<String>,
}

pub struct CapturedInput {
    pub input: Input,
    pub registry: Registry,
}
