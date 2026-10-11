use crate::policy::Registry;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const SOURCE_MANIFEST_JSON: &str = include_str!("../rules/sources-v1.json");
const CONCEPT_CATALOG_JSON: &str = include_str!("../rules/concept-catalog-v1.json");

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceKind {
    BookEdition,
    ResearchPaper,
    LiveWebCatalog,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Monitoring {
    StableIdentifier,
    WebContent,
    Manual,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reuse {
    CiteAndParaphrase,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceManifest {
    schema_version: u32,
    manifest_id: String,
    manifest_version: String,
    reviewed_on: String,
    sources: Vec<Source>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    source_id: String,
    kind: SourceKind,
    title: String,
    authors: Vec<String>,
    #[serde(default)]
    contributors: Vec<String>,
    publisher: String,
    publication_year: Option<u32>,
    edition: Option<String>,
    isbn: Option<String>,
    doi: Option<String>,
    canonical_url: String,
    full_text_url: Option<String>,
    edition_url: Option<String>,
    revision: String,
    reviewed_on: String,
    catalog_fingerprint: String,
    monitoring: Monitoring,
    reuse: Reuse,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum CatalogState {
    Candidate,
    Accepted,
    Deprecated,
    Retired,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceRelationship {
    Defines,
    Names,
    LegacyAlias,
    Refines,
    Equates,
    Classifies,
    Component,
    Motivates,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AccessScope {
    FullText,
    TableOfContents,
    PublisherRecord,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReferenceRole {
    CatalogInventory,
    ConceptGuidance,
    Taxonomy,
    MetricDetectionPattern,
    DetectionMethod,
}

impl SourceRelationship {
    fn as_str(self) -> &'static str {
        match self {
            Self::Defines => "defines",
            Self::Names => "names",
            Self::LegacyAlias => "legacy_alias",
            Self::Refines => "refines",
            Self::Equates => "equates",
            Self::Classifies => "classifies",
            Self::Component => "component",
            Self::Motivates => "motivates",
        }
    }
}

impl AccessScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::FullText => "full_text",
            Self::TableOfContents => "table_of_contents",
            Self::PublisherRecord => "publisher_record",
        }
    }
}

impl ReferenceRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::CatalogInventory => "catalog_inventory",
            Self::ConceptGuidance => "concept_guidance",
            Self::Taxonomy => "taxonomy",
            Self::MetricDetectionPattern => "metric_detection_pattern",
            Self::DetectionMethod => "detection_method",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Applicability {
    Applicable,
    Contextual,
    NotApplicable,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum DetectorSupport {
    None,
    Research,
    Preview,
    Stable,
    Deprecated,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportState {
    applicability: Applicability,
    detector_support: DetectorSupport,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LanguageSupport {
    rust: SupportState,
    python: SupportState,
    typescript: SupportState,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConceptCatalog {
    schema_version: u32,
    catalog_id: String,
    catalog_version: String,
    source_manifest: String,
    item_count: usize,
    concepts: Vec<Concept>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Concept {
    id: String,
    name: Option<String>,
    catalog_state: CatalogState,
    summary: String,
    aliases: Vec<Alias>,
    references: Vec<ConceptReference>,
    language_support: LanguageSupport,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Alias {
    name: String,
    source_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConceptReference {
    source_id: String,
    source_term: String,
    relationship: SourceRelationship,
    reference_url: String,
    source_locator: String,
    source_category: Option<String>,
    ordinal: usize,
    access_scope: AccessScope,
    reference_role: ReferenceRole,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct CatalogVersions {
    #[serde(rename = "catalog_version")]
    pub catalog: String,
    #[serde(rename = "source_manifest_version")]
    pub source_manifest: String,
}

fn parse() -> Result<(SourceManifest, ConceptCatalog), String> {
    let sources = serde_json::from_str(SOURCE_MANIFEST_JSON)
        .map_err(|error| format!("invalid embedded source manifest: {error}"))?;
    let catalog = serde_json::from_str(CONCEPT_CATALOG_JSON)
        .map_err(|error| format!("invalid embedded concept catalog: {error}"))?;
    Ok((sources, catalog))
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn fixed_ascii_digits(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse_date(value: &str) -> Option<(u32, u32, u32)> {
    let (year, remainder) = value.split_once('-')?;
    let (month, day) = remainder.split_once('-')?;
    if !fixed_ascii_digits(year, 4) || !fixed_ascii_digits(month, 2) || !fixed_ascii_digits(day, 2)
    {
        return None;
    }
    Some((year.parse().ok()?, month.parse().ok()?, day.parse().ok()?))
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_in_month(year: u32, month: u32) -> Option<u32> {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Some(31),
        4 | 6 | 9 | 11 => Some(30),
        2 if is_leap_year(year) => Some(29),
        2 => Some(28),
        _ => None,
    }
}

fn valid_date(value: &str) -> bool {
    let Some((year, month, day)) = parse_date(value) else {
        return false;
    };
    let Some(max_day) = days_in_month(year, month) else {
        return false;
    };
    year > 0 && day > 0 && day <= max_day
}

fn valid_https_url(value: &str) -> bool {
    let Some(remainder) = value.strip_prefix("https://") else {
        return false;
    };
    if remainder.is_empty()
        || remainder
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control() || byte == b'\\')
    {
        return false;
    }
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    !authority.is_empty()
        && authority.bytes().any(|byte| byte.is_ascii_alphanumeric())
        && authority.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':' | b'[' | b']')
        })
}

fn valid_fingerprint_field(value: &str) -> bool {
    !value.trim().is_empty() && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_source_manifest_header(manifest: &SourceManifest) -> bool {
    manifest.schema_version == 1
        && manifest.manifest_id == "smells-sources"
        && valid_version(&manifest.manifest_version)
        && valid_date(&manifest.reviewed_on)
        && !manifest.sources.is_empty()
}

fn valid_names(values: &[String], required: bool) -> bool {
    if required && values.is_empty() {
        return false;
    }
    let unique = values.iter().map(String::as_str).collect::<BTreeSet<_>>();
    unique.len() == values.len() && unique.iter().all(|value| !value.trim().is_empty())
}

fn valid_book_metadata(source: &Source) -> bool {
    source.publication_year.is_some()
        && source
            .edition
            .as_ref()
            .is_some_and(|value| !value.is_empty())
        && source.isbn.as_ref().is_some_and(|value| {
            value.len() == 13 && value.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn valid_research_paper_metadata(source: &Source) -> bool {
    source.publication_year.is_some()
        && source.doi.as_ref().is_some_and(|value| !value.is_empty())
        && source.edition.is_none()
        && source.isbn.is_none()
}

fn valid_live_web_metadata(source: &Source) -> bool {
    source.publication_year.is_none()
        && source.edition.is_none()
        && source.isbn.is_none()
        && source.doi.is_none()
}

fn valid_publication_metadata(source: &Source) -> bool {
    match source.kind {
        SourceKind::BookEdition => valid_book_metadata(source),
        SourceKind::ResearchPaper => valid_research_paper_metadata(source),
        SourceKind::LiveWebCatalog => valid_live_web_metadata(source),
    }
}

fn valid_source_identity(source: &Source) -> bool {
    valid_identifier(&source.source_id)
        && !source.title.trim().is_empty()
        && valid_names(&source.authors, true)
        && valid_names(&source.contributors, false)
        && !source.publisher.trim().is_empty()
        && !source.revision.trim().is_empty()
}

fn valid_source_location_and_review(source: &Source, manifest_reviewed_on: &str) -> bool {
    valid_https_url(&source.canonical_url)
        && source
            .full_text_url
            .as_ref()
            .is_none_or(|url| valid_https_url(url))
        && source
            .edition_url
            .as_ref()
            .is_none_or(|url| valid_https_url(url))
        && valid_date(&source.reviewed_on)
        && source.reviewed_on == manifest_reviewed_on
}

fn valid_source_year(source: &Source) -> bool {
    source
        .publication_year
        .is_none_or(|year| (1900..=2100).contains(&year))
}

fn valid_source(source: &Source, manifest_reviewed_on: &str) -> bool {
    let _typed_metadata = (&source.monitoring, &source.reuse);
    valid_source_identity(source)
        && valid_publication_metadata(source)
        && valid_source_location_and_review(source, manifest_reviewed_on)
        && valid_source_year(source)
        && valid_sha256(&source.catalog_fingerprint)
}

fn validate_sources(manifest: &SourceManifest) -> Result<BTreeSet<&str>, String> {
    if !valid_source_manifest_header(manifest) {
        return Err("invalid embedded source manifest header".into());
    }
    let mut ids = BTreeSet::new();
    let mut fingerprints = BTreeSet::new();
    for source in &manifest.sources {
        if !valid_source(source, &manifest.reviewed_on)
            || !ids.insert(source.source_id.as_str())
            || !fingerprints.insert(source.catalog_fingerprint.as_str())
        {
            return Err(format!("invalid embedded source: {}", source.source_id));
        }
    }
    Ok(ids)
}

fn register_label(
    labels: &mut BTreeMap<String, String>,
    label: &str,
    concept_id: &str,
) -> Result<(), String> {
    let normalized = label.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(format!("empty concept label: {concept_id}"));
    }
    if let Some(existing) = labels.insert(normalized, concept_id.to_string())
        && existing != concept_id
    {
        return Err(format!(
            "concept label resolves to more than one concept: {label}"
        ));
    }
    Ok(())
}

fn validate_reference(
    reference: &ConceptReference,
    source_ids: &BTreeSet<&str>,
) -> Result<(), String> {
    let typed_metadata = (
        reference.relationship,
        reference.access_scope,
        reference.reference_role,
    );
    if !source_ids.contains(reference.source_id.as_str())
        || !valid_fingerprint_field(&reference.source_term)
        || !valid_https_url(&reference.reference_url)
        || !valid_fingerprint_field(&reference.source_locator)
        || reference.ordinal == 0
        || reference
            .source_category
            .as_ref()
            .is_some_and(|category| !valid_fingerprint_field(category))
    {
        return Err(format!(
            "invalid concept reference: {}",
            reference.source_id
        ));
    }
    let _ = typed_metadata;
    Ok(())
}

fn valid_concept_catalog_header(catalog: &ConceptCatalog) -> bool {
    catalog.schema_version == 1
        && catalog.catalog_id == "smells-core"
        && valid_version(&catalog.catalog_version)
        && catalog.item_count == 36
        && catalog.concepts.len() == catalog.item_count
}

fn valid_concept_metadata(concept: &Concept, canonical_name: Option<&str>) -> bool {
    valid_identifier(&concept.id)
        && canonical_name.is_some_and(|name| !name.trim().is_empty())
        && !concept.summary.trim().is_empty()
        && !concept.references.is_empty()
        && (concept.catalog_state == CatalogState::Accepted) != concept.name.is_some()
}

fn validate_concept_references<'a>(
    concept: &'a Concept,
    source_ids: &BTreeSet<&str>,
    represented_sources: &mut BTreeSet<&'a str>,
) -> Result<(), String> {
    let mut reference_sources = BTreeSet::new();
    for reference in &concept.references {
        validate_reference(reference, source_ids)?;
        if !reference_sources.insert(reference.source_id.as_str()) {
            return Err(format!(
                "duplicate source reference for concept: {}",
                concept.id
            ));
        }
        represented_sources.insert(reference.source_id.as_str());
    }
    Ok(())
}

fn validate_aliases(
    concept: &Concept,
    canonical_name: &str,
    labels: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let mut aliases = BTreeSet::new();
    for alias in &concept.aliases {
        if alias.name.trim().is_empty()
            || alias.name.eq_ignore_ascii_case(canonical_name)
            || !aliases.insert(alias.name.to_lowercase())
            || alias.source_ids.is_empty()
        {
            return Err(format!("invalid alias for concept: {}", concept.id));
        }
        let alias_sources = alias.source_ids.iter().collect::<BTreeSet<_>>();
        let evidenced_sources = concept
            .references
            .iter()
            .filter(|reference| reference.source_term.eq_ignore_ascii_case(&alias.name))
            .map(|reference| &reference.source_id)
            .collect::<BTreeSet<_>>();
        if alias_sources.len() != alias.source_ids.len() || alias_sources != evidenced_sources {
            return Err(format!("invalid alias sources for concept: {}", concept.id));
        }
        register_label(labels, &alias.name, &concept.id)?;
    }
    Ok(())
}

fn valid_lifecycle_counts(states: &BTreeMap<CatalogState, usize>) -> bool {
    states.get(&CatalogState::Accepted) == Some(&23)
        && states.get(&CatalogState::Candidate) == Some(&13)
        && states
            .get(&CatalogState::Deprecated)
            .copied()
            .unwrap_or_default()
            == 0
        && states
            .get(&CatalogState::Retired)
            .copied()
            .unwrap_or_default()
            == 0
}

fn validate_concept<'a>(
    concept: &'a Concept,
    source_ids: &BTreeSet<&str>,
    registry: &Registry,
    concept_ids: &mut BTreeSet<&'a str>,
    labels: &mut BTreeMap<String, String>,
    states: &mut BTreeMap<CatalogState, usize>,
    represented_sources: &mut BTreeSet<&'a str>,
) -> Result<(), String> {
    let policy_smell = registry.smells.iter().find(|smell| smell.id == concept.id);
    let canonical_name = if concept.catalog_state == CatalogState::Accepted {
        policy_smell.map(|smell| smell.name.as_str())
    } else {
        concept.name.as_deref()
    };
    if !concept_ids.insert(concept.id.as_str()) || !valid_concept_metadata(concept, canonical_name)
    {
        return Err(format!("invalid embedded concept: {}", concept.id));
    }
    let canonical_name = canonical_name.expect("validated canonical name");
    register_label(labels, canonical_name, &concept.id)?;
    *states.entry(concept.catalog_state).or_default() += 1;
    validate_concept_references(concept, source_ids, represented_sources)?;
    validate_aliases(concept, canonical_name, labels)?;
    validate_language_support(concept, policy_smell, &registry.language)
}

fn validate_concepts<'a>(
    catalog: &'a ConceptCatalog,
    source_ids: &BTreeSet<&str>,
    registry: &Registry,
) -> Result<BTreeSet<&'a str>, String> {
    if !valid_concept_catalog_header(catalog) {
        return Err("invalid embedded concept catalog header".into());
    }
    let mut concept_ids = BTreeSet::new();
    let mut labels = BTreeMap::new();
    let mut states = BTreeMap::<CatalogState, usize>::new();
    let mut represented_sources = BTreeSet::new();
    for concept in &catalog.concepts {
        validate_concept(
            concept,
            source_ids,
            registry,
            &mut concept_ids,
            &mut labels,
            &mut states,
            &mut represented_sources,
        )?;
    }
    if !valid_lifecycle_counts(&states) {
        return Err("invalid embedded concept lifecycle counts".into());
    }
    if represented_sources != *source_ids {
        return Err("every source must ground at least one concept".into());
    }
    Ok(concept_ids)
}

fn language_support<'a>(support: &'a LanguageSupport, language: &str) -> Option<&'a SupportState> {
    match language {
        "rust" => Some(&support.rust),
        "python" => Some(&support.python),
        "typescript" => Some(&support.typescript),
        _ => None,
    }
}

fn validate_candidate_support(concept: &Concept) -> Result<(), String> {
    let supports = [
        &concept.language_support.rust,
        &concept.language_support.python,
        &concept.language_support.typescript,
    ];
    if supports
        .iter()
        .any(|support| support.detector_support != DetectorSupport::None)
    {
        return Err(format!(
            "candidate concept unexpectedly has detector support: {}",
            concept.id
        ));
    }
    Ok(())
}

fn expected_policy_support(smell: &crate::policy::Smell) -> (Applicability, DetectorSupport) {
    let applicability = if smell.applicability == "applicable" {
        Applicability::Applicable
    } else {
        Applicability::NotApplicable
    };
    let detector = if smell.rules.is_empty() {
        DetectorSupport::None
    } else {
        DetectorSupport::Stable
    };
    (applicability, detector)
}

fn validate_language_support(
    concept: &Concept,
    policy_smell: Option<&crate::policy::Smell>,
    language: &str,
) -> Result<(), String> {
    let support = language_support(&concept.language_support, language)
        .ok_or_else(|| format!("unsupported catalog language: {language}"))?;
    if concept.catalog_state == CatalogState::Candidate {
        return validate_candidate_support(concept);
    }
    let smell = policy_smell.ok_or_else(|| {
        format!(
            "accepted concept is absent from existing {language} policy: {}",
            concept.id
        )
    })?;
    let (expected_applicability, expected_detector) = expected_policy_support(smell);
    if support.applicability != expected_applicability
        || support.detector_support != expected_detector
    {
        return Err(format!(
            "catalog support does not match existing {language} policy: {}",
            concept.id
        ));
    }
    Ok(())
}

fn validate_reference_inventories(
    catalog: &ConceptCatalog,
    manifest: &SourceManifest,
) -> Result<(), String> {
    let mut references = BTreeMap::<&str, Vec<&ConceptReference>>::new();
    for reference in catalog
        .concepts
        .iter()
        .flat_map(|concept| &concept.references)
    {
        references
            .entry(reference.source_id.as_str())
            .or_default()
            .push(reference);
    }
    let expected = BTreeMap::from([
        ("fowler-beck-refactoring-1e", 22),
        ("fowler-refactoring-2e", 24),
        ("refactoring-guru-smells-web", 23),
        ("mantyla-lassenius-2006", 23),
        ("lanza-marinescu-2006", 11),
        ("decor-2010", 4),
    ]);
    let counts = references
        .iter()
        .map(|(source_id, entries)| (*source_id, entries.len()))
        .collect::<BTreeMap<_, _>>();
    if counts != expected {
        return Err("embedded source reference inventory drifted".into());
    }

    for source in &manifest.sources {
        let entries = references
            .get_mut(source.source_id.as_str())
            .expect("source representation checked");
        entries.sort_by_key(|reference| reference.ordinal);
        if entries
            .iter()
            .enumerate()
            .any(|(index, reference)| reference.ordinal != index + 1)
        {
            return Err(format!(
                "embedded source reference ordinals drifted: {}",
                source.source_id
            ));
        }
        let normalized = entries
            .iter()
            .map(|reference| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    reference.ordinal,
                    reference.source_term,
                    reference.source_category.as_deref().unwrap_or_default(),
                    reference.relationship.as_str(),
                    reference.reference_url,
                    reference.source_locator,
                    reference.access_scope.as_str(),
                    reference.reference_role.as_str()
                )
            })
            .collect::<String>();
        let fingerprint = format!("sha256:{:x}", Sha256::digest(normalized.as_bytes()));
        if fingerprint != source.catalog_fingerprint {
            return Err(format!(
                "embedded source fingerprint drifted: {}",
                source.source_id
            ));
        }
    }
    Ok(())
}

fn validate_registry_alignment(
    catalog: &ConceptCatalog,
    registry: &Registry,
) -> Result<(), String> {
    let accepted = catalog
        .concepts
        .iter()
        .filter(|concept| concept.catalog_state == CatalogState::Accepted)
        .map(|concept| concept.id.as_str())
        .collect::<BTreeSet<_>>();
    let policy = registry
        .smells
        .iter()
        .map(|smell| smell.id.as_str())
        .collect::<BTreeSet<_>>();
    if accepted != policy {
        return Err(format!(
            "concept references do not align with existing {} smell policy",
            registry.rule_pack
        ));
    }
    Ok(())
}

fn embedded_registries() -> Result<Vec<Registry>, String> {
    ["rust-v1", "python-v1", "typescript-v1"]
        .into_iter()
        .map(|rule_pack| {
            let json = crate::policy::registry_json(rule_pack)?;
            serde_json::from_str(json)
                .map_err(|error| format!("invalid embedded registry {rule_pack}: {error}"))
        })
        .collect()
}

fn validate_all_registry_support(
    catalog: &ConceptCatalog,
    registries: &[Registry],
) -> Result<(), String> {
    for registry in registries {
        validate_registry_alignment(catalog, registry)?;
        for concept in &catalog.concepts {
            let policy_smell = registry.smells.iter().find(|smell| smell.id == concept.id);
            validate_language_support(concept, policy_smell, &registry.language)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_embedded(registry: &Registry) -> Result<CatalogVersions, String> {
    let (sources, catalog) = parse()?;
    let source_ids = validate_sources(&sources)?;
    let expected_manifest = format!("{}@{}", sources.manifest_id, sources.manifest_version);
    if catalog.source_manifest != expected_manifest {
        return Err("concept catalog references a different source manifest".into());
    }
    validate_concepts(&catalog, &source_ids, registry)?;
    validate_reference_inventories(&catalog, &sources)?;
    validate_all_registry_support(&catalog, &embedded_registries()?)?;
    Ok(CatalogVersions {
        catalog: catalog.catalog_version,
        source_manifest: sources.manifest_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(rule_pack: &str) -> Registry {
        serde_json::from_str(crate::policy::registry_json(rule_pack).unwrap()).unwrap()
    }

    #[test]
    fn embedded_references_augment_every_existing_policy() {
        for rule_pack in ["rust-v1", "python-v1", "typescript-v1"] {
            validate_embedded(&registry(rule_pack)).unwrap();
        }
    }

    #[test]
    fn every_source_claim_has_an_explicit_https_url_and_locator() {
        let (_, catalog) = parse().unwrap();
        for reference in catalog
            .concepts
            .iter()
            .flat_map(|concept| &concept.references)
        {
            assert!(reference.reference_url.starts_with("https://"));
            assert!(!reference.source_locator.trim().is_empty());
        }
    }

    #[test]
    fn source_validation_rejects_impossible_dates_and_empty_https_hosts() {
        let (mut sources, _) = parse().unwrap();
        sources.sources[0].reviewed_on = "2026-99-99".into();
        assert!(validate_sources(&sources).is_err());

        let (mut sources, _) = parse().unwrap();
        sources.sources[0].canonical_url = "https://".into();
        assert!(validate_sources(&sources).is_err());

        let (mut sources, _) = parse().unwrap();
        sources.sources[1].edition_url = Some("https://".into());
        assert!(validate_sources(&sources).is_err());

        let (mut sources, _) = parse().unwrap();
        sources.sources[3].full_text_url = Some("https://".into());
        assert!(validate_sources(&sources).is_err());
    }

    #[test]
    fn date_validation_handles_format_and_leap_year_boundaries() {
        for valid in [
            "2000-02-29",
            "2024-02-29",
            "2026-01-31",
            "2026-02-28",
            "2026-09-27",
        ] {
            assert!(valid_date(valid), "expected valid date: {valid}");
        }
        for invalid in [
            "0000-01-01",
            "1900-02-29",
            "2023-02-29",
            "2026-00-01",
            "2026-13-01",
            "2026-04-31",
            "2026-01-00",
            "2026-1-01",
            "2026/01/01",
            "+026-01-01",
            "2026-+1-+1",
            "date-value",
        ] {
            assert!(!valid_date(invalid), "expected invalid date: {invalid}");
        }
    }

    #[test]
    fn catalog_identifiers_and_versions_enforce_the_published_format() {
        for valid in ["rust", "rust-v1", "rule-23"] {
            assert!(
                valid_identifier(valid),
                "expected valid identifier: {valid}"
            );
        }
        for invalid in ["", "-rust", "rust-", "rust--v1", "Rust", "rust_v1"] {
            assert!(
                !valid_identifier(invalid),
                "expected invalid identifier: {invalid}"
            );
        }

        for valid in ["0.0.0", "1.2.3", "2026.10.11"] {
            assert!(valid_version(valid), "expected valid version: {valid}");
        }
        for invalid in ["", "1", "1.2", "1.2.3.4", "1..3", "1.two.3"] {
            assert!(
                !valid_version(invalid),
                "expected invalid version: {invalid}"
            );
        }
    }

    #[test]
    fn catalog_urls_require_a_nonempty_safe_https_authority() {
        for valid in [
            "https://example.com",
            "https://example.com/path?q=1#fragment",
            "https://[::1]:443/source",
        ] {
            assert!(valid_https_url(valid), "expected valid URL: {valid}");
        }
        for invalid in [
            "",
            "http://example.com",
            "https://",
            "https:///path",
            "https://---/path",
            "https://example.com/has space",
            "https://example.com\\path",
            "https://example_com/path",
            "https://example.com/\u{7f}",
        ] {
            assert!(
                !valid_https_url(invalid),
                "expected invalid URL: {invalid:?}"
            );
        }
    }

    #[test]
    fn catalog_fingerprints_and_names_reject_ambiguous_values() {
        let digest = format!("sha256:{}", "a".repeat(64));
        assert!(valid_sha256(&digest));
        for invalid in [
            "",
            "sha256:abc",
            &format!("sha256:{}", "A".repeat(64)),
            &format!("sha257:{}", "a".repeat(64)),
            &format!("sha256:{}g", "a".repeat(63)),
        ] {
            assert!(!valid_sha256(invalid), "expected invalid digest: {invalid}");
        }

        assert!(valid_names(&["Alice".into(), "Bob".into()], true));
        assert!(valid_names(&[], false));
        assert!(!valid_names(&[], true));
        assert!(!valid_names(&["Alice".into(), "Alice".into()], false));
        assert!(!valid_names(&["   ".into()], false));
    }

    #[test]
    fn source_manifest_header_requires_every_identity_field() {
        let (manifest, _) = parse().unwrap();
        assert!(valid_source_manifest_header(&manifest));

        let (mut invalid, _) = parse().unwrap();
        invalid.schema_version = 2;
        assert!(!valid_source_manifest_header(&invalid));

        let (mut invalid, _) = parse().unwrap();
        invalid.manifest_id = "other-sources".into();
        assert!(!valid_source_manifest_header(&invalid));

        let (mut invalid, _) = parse().unwrap();
        invalid.manifest_version = "1.0".into();
        assert!(!valid_source_manifest_header(&invalid));

        let (mut invalid, _) = parse().unwrap();
        invalid.reviewed_on = "2026-02-30".into();
        assert!(!valid_source_manifest_header(&invalid));

        let (mut invalid, _) = parse().unwrap();
        invalid.sources.clear();
        assert!(!valid_source_manifest_header(&invalid));
    }

    #[test]
    fn publication_metadata_matches_each_source_kind() {
        let (manifest, _) = parse().unwrap();
        let book = manifest
            .sources
            .iter()
            .find(|source| matches!(source.kind, SourceKind::BookEdition))
            .unwrap();
        let paper = manifest
            .sources
            .iter()
            .find(|source| matches!(source.kind, SourceKind::ResearchPaper))
            .unwrap();
        let web = manifest
            .sources
            .iter()
            .find(|source| matches!(source.kind, SourceKind::LiveWebCatalog))
            .unwrap();

        assert!(valid_book_metadata(book));
        assert!(valid_research_paper_metadata(paper));
        assert!(valid_live_web_metadata(web));
        assert!(valid_publication_metadata(book));
        assert!(valid_publication_metadata(paper));
        assert!(valid_publication_metadata(web));
    }

    #[test]
    fn publication_metadata_rejects_each_forbidden_or_missing_field() {
        let source = |kind| {
            let (manifest, _) = parse().unwrap();
            manifest
                .sources
                .into_iter()
                .find(|source| std::mem::discriminant(&source.kind) == kind)
                .unwrap()
        };

        let book_kind = std::mem::discriminant(&SourceKind::BookEdition);
        let mut invalid = source(book_kind);
        invalid.publication_year = None;
        assert!(!valid_book_metadata(&invalid));
        let mut invalid = source(book_kind);
        invalid.edition = Some(String::new());
        assert!(!valid_book_metadata(&invalid));
        let mut invalid = source(book_kind);
        invalid.isbn = Some("123".into());
        assert!(!valid_book_metadata(&invalid));
        let mut invalid = source(book_kind);
        invalid.isbn = Some("978000000000x".into());
        assert!(!valid_book_metadata(&invalid));

        let paper_kind = std::mem::discriminant(&SourceKind::ResearchPaper);
        let mut invalid = source(paper_kind);
        invalid.publication_year = None;
        assert!(!valid_research_paper_metadata(&invalid));
        let mut invalid = source(paper_kind);
        invalid.doi = Some(String::new());
        assert!(!valid_research_paper_metadata(&invalid));
        let mut invalid = source(paper_kind);
        invalid.edition = Some("first".into());
        assert!(!valid_research_paper_metadata(&invalid));
        let mut invalid = source(paper_kind);
        invalid.isbn = Some("9780000000000".into());
        assert!(!valid_research_paper_metadata(&invalid));

        let web_kind = std::mem::discriminant(&SourceKind::LiveWebCatalog);
        let mut invalid = source(web_kind);
        invalid.publication_year = Some(2026);
        assert!(!valid_live_web_metadata(&invalid));
        let mut invalid = source(web_kind);
        invalid.edition = Some("live".into());
        assert!(!valid_live_web_metadata(&invalid));
        let mut invalid = source(web_kind);
        invalid.isbn = Some("9780000000000".into());
        assert!(!valid_live_web_metadata(&invalid));
        let mut invalid = source(web_kind);
        invalid.doi = Some("10.1/live".into());
        assert!(!valid_live_web_metadata(&invalid));

        let mut invalid_book = source(book_kind);
        invalid_book.publication_year = None;
        assert!(!valid_publication_metadata(&invalid_book));
        let mut invalid_paper = source(paper_kind);
        invalid_paper.doi = Some(String::new());
        assert!(!valid_publication_metadata(&invalid_paper));
        let mut invalid_web = source(web_kind);
        invalid_web.publication_year = Some(2026);
        assert!(!valid_publication_metadata(&invalid_web));
    }

    #[test]
    fn source_identity_requires_distinct_nonblank_attribution() {
        let source = || parse().unwrap().0.sources.remove(0);
        assert!(valid_source_identity(&source()));

        let mut invalid = source();
        invalid.source_id = "Invalid_ID".into();
        assert!(!valid_source_identity(&invalid));
        let mut invalid = source();
        invalid.title = "  ".into();
        assert!(!valid_source_identity(&invalid));
        let mut invalid = source();
        invalid.authors.clear();
        assert!(!valid_source_identity(&invalid));
        let mut invalid = source();
        invalid.contributors = vec!["same".into(), "same".into()];
        assert!(!valid_source_identity(&invalid));
        let mut invalid = source();
        invalid.publisher = "\t".into();
        assert!(!valid_source_identity(&invalid));
        let mut invalid = source();
        invalid.revision = String::new();
        assert!(!valid_source_identity(&invalid));
    }

    #[test]
    fn source_location_review_and_year_are_bounded() {
        let (manifest, _) = parse().unwrap();
        let source = &manifest.sources[0];
        assert!(valid_source_location_and_review(
            source,
            &manifest.reviewed_on
        ));
        assert!(valid_source_year(source));

        let fresh = || parse().unwrap().0.sources.remove(0);
        let mut invalid = fresh();
        invalid.canonical_url = "http://example.com".into();
        assert!(!valid_source_location_and_review(
            &invalid,
            &manifest.reviewed_on
        ));
        let mut invalid = fresh();
        invalid.full_text_url = Some("http://example.com".into());
        assert!(!valid_source_location_and_review(
            &invalid,
            &manifest.reviewed_on
        ));
        let mut invalid = fresh();
        invalid.edition_url = Some("http://example.com".into());
        assert!(!valid_source_location_and_review(
            &invalid,
            &manifest.reviewed_on
        ));
        let mut invalid = fresh();
        invalid.reviewed_on = "2026-02-30".into();
        assert!(!valid_source_location_and_review(
            &invalid,
            &manifest.reviewed_on
        ));
        let invalid = fresh();
        assert!(!valid_source_location_and_review(&invalid, "2026-10-10"));

        let mut invalid = fresh();
        invalid.publication_year = Some(1899);
        assert!(!valid_source_year(&invalid));
        invalid.publication_year = Some(1900);
        assert!(valid_source_year(&invalid));
        invalid.publication_year = Some(2100);
        assert!(valid_source_year(&invalid));
        invalid.publication_year = Some(2101);
        assert!(!valid_source_year(&invalid));
        invalid.publication_year = None;
        assert!(valid_source_year(&invalid));
    }

    #[test]
    fn source_validation_rejects_duplicate_identities_and_fingerprints() {
        let (mut manifest, _) = parse().unwrap();
        manifest.sources[1].source_id = manifest.sources[0].source_id.clone();
        assert!(validate_sources(&manifest).is_err());

        let (mut manifest, _) = parse().unwrap();
        manifest.sources[1].catalog_fingerprint = manifest.sources[0].catalog_fingerprint.clone();
        assert!(validate_sources(&manifest).is_err());
    }

    #[test]
    fn concept_catalog_header_requires_the_complete_published_inventory() {
        let (_, catalog) = parse().unwrap();
        assert!(valid_concept_catalog_header(&catalog));

        let (_, mut invalid) = parse().unwrap();
        invalid.schema_version = 2;
        assert!(!valid_concept_catalog_header(&invalid));
        let (_, mut invalid) = parse().unwrap();
        invalid.catalog_id = "another-catalog".into();
        assert!(!valid_concept_catalog_header(&invalid));
        let (_, mut invalid) = parse().unwrap();
        invalid.catalog_version = "1.0".into();
        assert!(!valid_concept_catalog_header(&invalid));
        let (_, mut invalid) = parse().unwrap();
        invalid.item_count -= 1;
        assert!(!valid_concept_catalog_header(&invalid));
        let (_, mut invalid) = parse().unwrap();
        invalid.concepts.pop();
        assert!(!valid_concept_catalog_header(&invalid));
    }

    #[test]
    fn concept_metadata_requires_a_canonical_name_summary_and_evidence() {
        let (_, catalog) = parse().unwrap();
        let accepted = catalog
            .concepts
            .iter()
            .find(|concept| concept.catalog_state == CatalogState::Accepted)
            .unwrap();
        assert!(valid_concept_metadata(accepted, Some("Long Method")));
        assert!(!valid_concept_metadata(accepted, None));
        assert!(!valid_concept_metadata(accepted, Some("  ")));

        let (_, mut invalid) = parse().unwrap();
        invalid.concepts[0].id = "Invalid_ID".into();
        assert!(!valid_concept_metadata(
            &invalid.concepts[0],
            Some("Long Method")
        ));
        let (_, mut invalid) = parse().unwrap();
        invalid.concepts[0].summary = "\t".into();
        assert!(!valid_concept_metadata(
            &invalid.concepts[0],
            Some("Long Method")
        ));
        let (_, mut invalid) = parse().unwrap();
        invalid.concepts[0].references.clear();
        assert!(!valid_concept_metadata(
            &invalid.concepts[0],
            Some("Long Method")
        ));
        let (_, mut invalid) = parse().unwrap();
        invalid.concepts[0].name = Some("Duplicate Name".into());
        assert!(!valid_concept_metadata(
            &invalid.concepts[0],
            Some("Long Method")
        ));

        let (_, mut invalid) = parse().unwrap();
        let candidate = invalid
            .concepts
            .iter_mut()
            .find(|concept| concept.catalog_state == CatalogState::Candidate)
            .unwrap();
        assert!(valid_concept_metadata(candidate, candidate.name.as_deref()));
        candidate.name = None;
        assert!(!valid_concept_metadata(candidate, None));
    }

    #[test]
    fn duplicate_concept_identity_is_rejected_before_other_catalog_checks() {
        let (sources, mut catalog) = parse().unwrap();
        let source_ids = validate_sources(&sources).unwrap();
        let policy = registry("rust-v1");
        catalog.concepts[1].id = catalog.concepts[0].id.clone();

        assert_eq!(
            validate_concepts(&catalog, &source_ids, &policy).unwrap_err(),
            "invalid embedded concept: long-method"
        );
    }

    #[test]
    fn catalog_lifecycle_and_registry_alignment_fail_closed() {
        let valid = BTreeMap::from([(CatalogState::Accepted, 23), (CatalogState::Candidate, 13)]);
        assert!(valid_lifecycle_counts(&valid));
        for invalid in [
            BTreeMap::from([(CatalogState::Accepted, 22), (CatalogState::Candidate, 13)]),
            BTreeMap::from([(CatalogState::Accepted, 23), (CatalogState::Candidate, 12)]),
            BTreeMap::from([
                (CatalogState::Accepted, 23),
                (CatalogState::Candidate, 13),
                (CatalogState::Deprecated, 1),
            ]),
            BTreeMap::from([
                (CatalogState::Accepted, 23),
                (CatalogState::Candidate, 13),
                (CatalogState::Retired, 1),
            ]),
        ] {
            assert!(!valid_lifecycle_counts(&invalid));
        }

        let (_, catalog) = parse().unwrap();
        let mut policy = registry("rust-v1");
        policy.smells.pop();
        assert!(validate_registry_alignment(&catalog, &policy).is_err());
    }

    #[test]
    fn candidate_concepts_cannot_claim_detector_support() {
        let (_, mut catalog) = parse().unwrap();
        let candidate = catalog
            .concepts
            .iter_mut()
            .find(|concept| concept.catalog_state == CatalogState::Candidate)
            .unwrap();
        assert!(validate_candidate_support(candidate).is_ok());
        candidate.language_support.rust.detector_support = DetectorSupport::Stable;
        assert!(validate_candidate_support(candidate).is_err());
    }

    #[test]
    fn source_relationships_have_stable_fingerprint_labels() {
        let labels = [
            (SourceRelationship::Defines, "defines"),
            (SourceRelationship::Names, "names"),
            (SourceRelationship::LegacyAlias, "legacy_alias"),
            (SourceRelationship::Refines, "refines"),
            (SourceRelationship::Equates, "equates"),
            (SourceRelationship::Classifies, "classifies"),
            (SourceRelationship::Component, "component"),
            (SourceRelationship::Motivates, "motivates"),
        ];
        for (relationship, expected) in labels {
            assert_eq!(relationship.as_str(), expected);
        }
    }

    #[test]
    fn candidate_applicability_is_independent_of_detector_support() {
        let (_, catalog) = parse().unwrap();
        let global_data = catalog
            .concepts
            .iter()
            .find(|concept| concept.id == "global-data")
            .unwrap();
        assert_eq!(
            global_data.language_support.python.applicability,
            Applicability::Applicable
        );
        assert_eq!(
            global_data.language_support.python.detector_support,
            DetectorSupport::None
        );
    }

    #[test]
    fn every_language_support_entry_is_checked_for_every_scan() {
        let (_, mut catalog) = parse().unwrap();
        catalog.concepts[0].language_support.python.applicability = Applicability::NotApplicable;
        assert_eq!(
            validate_all_registry_support(&catalog, &embedded_registries().unwrap()).unwrap_err(),
            "catalog support does not match existing python policy: long-method"
        );
    }

    #[test]
    fn reference_fields_reject_fingerprint_delimiters() {
        let (sources, mut catalog) = parse().unwrap();
        let source_ids = validate_sources(&sources).unwrap();
        catalog.concepts[0].references[0].source_locator.push('\t');
        assert!(validate_reference(&catalog.concepts[0].references[0], &source_ids).is_err());
    }

    #[test]
    fn concept_references_require_every_identity_and_location_field() {
        let invalid_reference = |mutate: fn(&mut ConceptReference)| {
            let (sources, mut catalog) = parse().unwrap();
            let source_ids = validate_sources(&sources).unwrap();
            let reference = &mut catalog.concepts[0].references[0];
            mutate(reference);
            validate_reference(reference, &source_ids)
        };

        assert!(
            invalid_reference(|reference| reference.source_id = "missing-source".into()).is_err()
        );
        assert!(invalid_reference(|reference| reference.source_term = "\n".into()).is_err());
        assert!(
            invalid_reference(|reference| reference.reference_url = "http://example.com".into())
                .is_err()
        );
        assert!(invalid_reference(|reference| reference.source_locator = "\0".into()).is_err());
        assert!(invalid_reference(|reference| reference.ordinal = 0).is_err());
        assert!(
            invalid_reference(|reference| reference.source_category = Some("\r".into())).is_err()
        );
    }

    #[test]
    fn source_reference_counts_preserve_the_published_inventories() {
        let (sources, catalog) = parse().unwrap();
        validate_reference_inventories(&catalog, &sources).unwrap();
    }

    #[test]
    fn mantyla_2006_is_the_only_mantyla_source_and_keeps_all_23_smells() {
        let (sources, catalog) = parse().unwrap();
        let mantyla_sources = sources
            .sources
            .iter()
            .filter(|source| {
                source
                    .authors
                    .iter()
                    .any(|author| author.contains("Mäntylä"))
            })
            .collect::<Vec<_>>();
        assert_eq!(mantyla_sources.len(), 1);
        assert_eq!(mantyla_sources[0].source_id, "mantyla-lassenius-2006");
        assert_eq!(
            mantyla_sources[0].full_text_url.as_deref(),
            Some("https://mmantyla.github.io/ESE_2006.pdf")
        );

        let references = catalog
            .concepts
            .iter()
            .flat_map(|concept| &concept.references)
            .filter(|reference| reference.source_id == "mantyla-lassenius-2006")
            .collect::<Vec<_>>();
        assert_eq!(references.len(), 23);
        assert!(references.iter().all(|reference| {
            reference
                .reference_url
                .starts_with("https://mmantyla.github.io/ESE_2006.pdf#page=")
        }));

        for (term, category, ordinal) in [
            ("Parallel Inheritance Hierarchies", "Change Preventers", 12),
            ("Message Chains", "Couplers", 20),
            ("Middle Man", "Couplers", 21),
            ("Comments", "Other smells (ungrouped)", 22),
            ("Incomplete Library Class", "Other smells (ungrouped)", 23),
        ] {
            let reference = references
                .iter()
                .find(|reference| reference.source_term == term)
                .unwrap();
            assert_eq!(reference.source_category.as_deref(), Some(category));
            assert_eq!(reference.ordinal, ordinal);
        }
    }

    #[test]
    fn decor_uses_hal_for_open_full_text_and_claim_evidence() {
        let (sources, catalog) = parse().unwrap();
        let source = sources
            .sources
            .iter()
            .find(|source| source.source_id == "decor-2010")
            .unwrap();
        assert_eq!(
            source.full_text_url.as_deref(),
            Some("https://inria.hal.science/inria-00538476/document")
        );

        let references = catalog
            .concepts
            .iter()
            .flat_map(|concept| &concept.references)
            .filter(|reference| reference.source_id == "decor-2010")
            .collect::<Vec<_>>();
        assert_eq!(references.len(), 4);
        assert!(references.iter().all(|reference| {
            reference.reference_url == "https://inria.hal.science/inria-00538476/document#page=6"
                && reference.source_locator.contains("§4.1.2, Table 1")
        }));
    }

    #[test]
    fn aliases_cannot_resolve_to_different_concepts() {
        let (sources, mut catalog) = parse().unwrap();
        let source_ids = validate_sources(&sources).unwrap();
        let policy = registry("python-v1");
        catalog.concepts[1]
            .references
            .iter_mut()
            .find(|reference| reference.source_id == "fowler-refactoring-2e")
            .unwrap()
            .source_term = "Long Function".into();
        catalog.concepts[1].aliases.push(Alias {
            name: "Long Function".into(),
            source_ids: vec!["fowler-refactoring-2e".into()],
        });
        assert_eq!(
            validate_concepts(&catalog, &source_ids, &policy).unwrap_err(),
            "concept label resolves to more than one concept: Long Function"
        );
    }

    #[test]
    fn alias_sources_must_also_reference_the_concept() {
        let (sources, mut catalog) = parse().unwrap();
        let source_ids = validate_sources(&sources).unwrap();
        let policy = registry("python-v1");
        catalog.concepts[0].aliases.push(Alias {
            name: "Unproven Alias".into(),
            source_ids: vec!["decor-2010".into()],
        });
        assert_eq!(
            validate_concepts(&catalog, &source_ids, &policy).unwrap_err(),
            "invalid alias sources for concept: long-method"
        );
    }

    #[test]
    fn aliases_require_distinct_evidenced_names_and_sources() {
        let invalid_alias = |mutate: fn(&mut Concept)| {
            let (sources, mut catalog) = parse().unwrap();
            let source_ids = validate_sources(&sources).unwrap();
            let policy = registry("rust-v1");
            let concept = catalog
                .concepts
                .iter_mut()
                .find(|concept| !concept.aliases.is_empty())
                .unwrap();
            mutate(concept);
            validate_concepts(&catalog, &source_ids, &policy).unwrap_err()
        };

        for error in [
            invalid_alias(|concept| concept.aliases[0].name = "  ".into()),
            invalid_alias(|concept| {
                let policy = registry("rust-v1");
                let canonical_name = policy
                    .smells
                    .iter()
                    .find(|smell| smell.id == concept.id)
                    .unwrap()
                    .name
                    .clone();
                concept.aliases[0].name = canonical_name;
            }),
            invalid_alias(|concept| {
                let name = concept.aliases[0].name.to_uppercase();
                let source_ids = concept.aliases[0].source_ids.clone();
                concept.aliases.push(Alias { name, source_ids });
            }),
            invalid_alias(|concept| concept.aliases[0].source_ids.clear()),
        ] {
            assert_eq!(error, "invalid alias for concept: long-method");
        }
    }

    #[test]
    fn source_ordinals_and_fingerprints_are_fail_closed() {
        let (sources, mut catalog) = parse().unwrap();
        catalog.concepts[0].references[0].ordinal = 99;
        assert_eq!(
            validate_reference_inventories(&catalog, &sources).unwrap_err(),
            "embedded source reference ordinals drifted: fowler-beck-refactoring-1e"
        );

        let (sources, mut catalog) = parse().unwrap();
        catalog.concepts[0].references[0].source_locator = "wrong page".into();
        assert_eq!(
            validate_reference_inventories(&catalog, &sources).unwrap_err(),
            "embedded source fingerprint drifted: fowler-beck-refactoring-1e"
        );
    }

    #[test]
    fn schema_documents_validate_the_embedded_catalogs() {
        for (schema_json, instance_json) in [
            (
                include_str!("../schemas/source-manifest.schema.json"),
                SOURCE_MANIFEST_JSON,
            ),
            (
                include_str!("../schemas/concept-catalog.schema.json"),
                CONCEPT_CATALOG_JSON,
            ),
        ] {
            let schema = serde_json::from_str::<serde_json::Value>(schema_json).unwrap();
            let instance = serde_json::from_str::<serde_json::Value>(instance_json).unwrap();
            jsonschema::draft202012::meta::validate(&schema).unwrap();
            let validator = jsonschema::draft202012::options()
                .should_validate_formats(true)
                .build(&schema)
                .unwrap();
            validator.validate(&instance).unwrap();
        }

        let source_schema = serde_json::from_str::<serde_json::Value>(include_str!(
            "../schemas/source-manifest.schema.json"
        ))
        .unwrap();
        let validator = jsonschema::draft202012::options()
            .should_validate_formats(true)
            .build(&source_schema)
            .unwrap();
        let mut invalid = serde_json::from_str::<serde_json::Value>(SOURCE_MANIFEST_JSON).unwrap();
        invalid["reviewed_on"] = "2026-99-99".into();
        assert!(!validator.is_valid(&invalid));

        let mut invalid = serde_json::from_str::<serde_json::Value>(SOURCE_MANIFEST_JSON).unwrap();
        invalid["sources"][3]["full_text_url"] = "http://example.com/paper.pdf".into();
        assert!(!validator.is_valid(&invalid));

        let mut incomplete =
            serde_json::from_str::<serde_json::Value>(SOURCE_MANIFEST_JSON).unwrap();
        incomplete["sources"][0]
            .as_object_mut()
            .unwrap()
            .remove("isbn");
        assert!(!validator.is_valid(&incomplete));

        let catalog_schema = serde_json::from_str::<serde_json::Value>(include_str!(
            "../schemas/concept-catalog.schema.json"
        ))
        .unwrap();
        let validator = jsonschema::draft202012::options()
            .should_validate_formats(true)
            .build(&catalog_schema)
            .unwrap();
        let mut invalid = serde_json::from_str::<serde_json::Value>(CONCEPT_CATALOG_JSON).unwrap();
        invalid["concepts"][0]["references"][0]["source_locator"] = "wrong\tpage".into();
        assert!(!validator.is_valid(&invalid));
    }
}
