use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedSourceRole {
    pub role: String,
    pub framework: String,
    pub implementation_id: String,
    pub path: String,
    pub path_scope: String,
    pub source: String,
    pub confidence: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RoleContext {
    pub source_role: String,
    pub declaration_role: String,
    pub framework: String,
    pub implementation_id: String,
    pub source: String,
    pub confidence: String,
    pub path_scope: String,
    pub evidence: Vec<String>,
    pub disposition: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MigrationArtifact {
    pub role: String,
    pub framework: String,
    pub implementation_id: String,
    pub path: String,
    pub status: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceRoleCandidate {
    pub role: String,
    pub path: String,
    pub confidence: String,
    pub evidence: Vec<String>,
}
