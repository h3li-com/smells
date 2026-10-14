use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceSpan {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedSourceRole {
    pub role: String,
    pub framework: String,
    pub implementation_id: String,
    pub path: String,
    pub path_scope: String,
    pub source: String,
    pub confidence: String,
    pub contract_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration_span: Option<SourceSpan>,
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
    pub contract_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration_span: Option<SourceSpan>,
    pub path_scope: String,
    pub evidence: Vec<String>,
    pub disposition: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MigrationIntegrity {
    pub mechanism: String,
    pub content_normalization: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MigrationArtifact {
    pub role: String,
    pub framework: String,
    pub implementation_id: String,
    pub path: String,
    pub status: String,
    pub contract_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration_span: Option<SourceSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<MigrationIntegrity>,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MigrationArtifactCandidate {
    pub role: String,
    pub path: String,
    pub matched_convention: String,
    pub evidence_sought: Vec<String>,
    pub unresolved_reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceRoleCandidate {
    pub role: String,
    pub path: String,
    pub confidence: String,
    pub evidence: Vec<String>,
}
