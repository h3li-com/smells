use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema_version: u32,
    pub rule_pack: String,
    pub language: String,
    pub catalog: Catalog,
    pub smells: Vec<Smell>,
    pub rules: Vec<Rule>,
    #[serde(skip)]
    pub guidance_catalog: Vec<WhenToIgnore>,
    #[serde(skip)]
    pub(crate) catalog_versions: crate::catalog::CatalogVersions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub source: String,
    pub checked_on: String,
    pub item_count: usize,
    pub guidance_pack: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Smell {
    pub id: String,
    pub name: String,
    pub category: String,
    pub applicability: String,
    pub rules: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WhenToIgnore {
    pub guidance_ref: String,
    pub smell_id: String,
    pub version: u32,
    pub provenance: String,
    pub source_url: String,
    pub checked_on: String,
    pub items: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuidanceCatalog {
    schema_version: u32,
    catalog_id: String,
    records: Vec<WhenToIgnore>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub version: u32,
    pub smell: String,
    pub kind: String,
    pub implementation: String,
    pub inputs: Vec<String>,
    pub parameters: BTreeMap<String, u64>,
    pub contract: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Required,
    Report,
    Off,
}

#[derive(Clone, Debug, Default)]
pub struct SelectionOptions {
    pub included_groups: Vec<String>,
    pub only_groups: Vec<String>,
    pub excluded_groups: Vec<String>,
    pub all_groups: bool,
    pub no_default_groups: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ResolvedSelection {
    pub default_groups: Vec<String>,
    pub included_groups: Vec<String>,
    pub only_groups: Vec<String>,
    pub excluded_groups: Vec<String>,
    pub all_groups: bool,
    pub no_default_groups: bool,
    pub available_groups: BTreeMap<String, Vec<String>>,
    pub selected_rule_ids: Vec<String>,
    pub active_rule_ids: Vec<String>,
    pub excluded_rule_ids: Vec<String>,
    pub rule_groups: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub version: u32,
    pub mode: Mode,
    #[serde(deserialize_with = "unique_map")]
    pub parameters: BTreeMap<String, u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub maximum_files: usize,
    pub maximum_pairs: usize,
    pub maximum_group_combinations: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRoleDeclaration {
    pub role: String,
    pub framework: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub configuration: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema_version: u32,
    pub rule_pack: String,
    pub scanner_version: String,
    pub scope: String,
    #[serde(default = "all_default_groups")]
    pub default_groups: Vec<String>,
    pub exclude_directories: Vec<String>,
    #[serde(default)]
    pub source_roles: Vec<SourceRoleDeclaration>,
    pub limits: Limits,
    #[serde(deserialize_with = "unique_map")]
    pub rules: BTreeMap<String, Selection>,
    pub exceptions: Vec<serde_json::Value>,
    #[serde(skip)]
    pub resolved: ResolvedSelection,
}

fn all_default_groups() -> Vec<String> {
    vec!["all".into()]
}

struct UniqueMapVisitor<T>(std::marker::PhantomData<T>);
impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for UniqueMapVisitor<T> {
    type Value = BTreeMap<String, T>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("an object with unique keys")
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        mut input: A,
    ) -> Result<Self::Value, A::Error> {
        let mut values = BTreeMap::new();
        while let Some((key, value)) = input.next_entry::<String, T>()? {
            if values.insert(key.clone(), value).is_some() {
                return Err(serde::de::Error::custom(format!("duplicate key: {key}")));
            }
        }
        Ok(values)
    }
}

// serde rejects duplicate struct fields; maps need the same protection explicitly.
fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    deserializer.deserialize_map(UniqueMapVisitor(std::marker::PhantomData))
}

pub fn registry_json(rule_pack: &str) -> Result<&'static str, String> {
    Ok(match rule_pack {
        "rust-v1" => include_str!("../rules/rust-v1.json"),
        "python-v1" => include_str!("../rules/python-v1.json"),
        "typescript-v1" => include_str!("../rules/typescript-v1.json"),
        _ => return Err(format!("unsupported rule pack: {rule_pack}")),
    })
}

fn validate_catalog(registry: &Registry, rule_pack: &str) -> Result<(), String> {
    if registry.schema_version != 1
        || registry.rule_pack != rule_pack
        || !matches!(registry.language.as_str(), "rust" | "python" | "typescript")
        || registry.catalog.source != "https://refactoring.guru/refactoring/smells"
        || registry.catalog.checked_on != "2026-09-19"
        || registry.catalog.item_count != 23
        || registry.catalog.guidance_pack != "when-to-ignore-v1"
        || registry.smells.len() != registry.catalog.item_count
    {
        return Err(format!("invalid embedded {rule_pack} catalog"));
    }
    Ok(())
}

fn canonical_smells() -> BTreeSet<(&'static str, &'static str, &'static str)> {
    [
        ("long-method", "Long Method", "bloaters"),
        ("large-class", "Large Class", "bloaters"),
        ("primitive-obsession", "Primitive Obsession", "bloaters"),
        ("long-parameter-list", "Long Parameter List", "bloaters"),
        ("data-clumps", "Data Clumps", "bloaters"),
        (
            "alternative-classes-with-different-interfaces",
            "Alternative Classes with Different Interfaces",
            "object-orientation-abusers",
        ),
        (
            "refused-bequest",
            "Refused Bequest",
            "object-orientation-abusers",
        ),
        (
            "switch-statements",
            "Switch Statements",
            "object-orientation-abusers",
        ),
        (
            "temporary-field",
            "Temporary Field",
            "object-orientation-abusers",
        ),
        ("divergent-change", "Divergent Change", "change-preventers"),
        (
            "parallel-inheritance-hierarchies",
            "Parallel Inheritance Hierarchies",
            "change-preventers",
        ),
        ("shotgun-surgery", "Shotgun Surgery", "change-preventers"),
        ("comments", "Comments", "dispensables"),
        ("duplicate-code", "Duplicate Code", "dispensables"),
        ("data-class", "Data Class", "dispensables"),
        ("dead-code", "Dead Code", "dispensables"),
        ("lazy-class", "Lazy Class", "dispensables"),
        (
            "speculative-generality",
            "Speculative Generality",
            "dispensables",
        ),
        ("feature-envy", "Feature Envy", "couplers"),
        (
            "inappropriate-intimacy",
            "Inappropriate Intimacy",
            "couplers",
        ),
        (
            "incomplete-library-class",
            "Incomplete Library Class",
            "couplers",
        ),
        ("message-chains", "Message Chains", "couplers"),
        ("middle-man", "Middle Man", "couplers"),
    ]
    .into_iter()
    .collect()
}

fn validate_smell_catalog(registry: &Registry) -> Result<(), String> {
    let smells: BTreeSet<_> = registry
        .smells
        .iter()
        .map(|s| (s.id.as_str(), s.name.as_str(), s.category.as_str()))
        .collect();
    let rules: BTreeSet<_> = registry.rules.iter().map(|r| &r.id).collect();
    if smells != canonical_smells() || rules.len() != registry.rules.len() {
        return Err("registry does not exactly match the pinned canonical smell catalog".into());
    }
    Ok(())
}

fn validate_smell_mappings(registry: &Registry) -> Result<(), String> {
    for smell in &registry.smells {
        if !matches!(
            smell.applicability.as_str(),
            "applicable" | "not_applicable_native_rust"
        ) || (smell.applicability == "applicable") == smell.rules.is_empty()
        {
            return Err(format!("invalid applicability: {}", smell.id));
        }
        for id in &smell.rules {
            if !registry
                .rules
                .iter()
                .any(|r| r.id == *id && r.smell == smell.id)
            {
                return Err(format!("invalid smell rule mapping: {id}"));
            }
        }
    }
    Ok(())
}

fn validate_applicability(registry: &Registry) -> Result<(), String> {
    if registry.language == "rust" {
        let inapplicable: BTreeSet<_> = registry
            .smells
            .iter()
            .filter(|smell| smell.applicability == "not_applicable_native_rust")
            .map(|smell| smell.id.as_str())
            .collect();
        if inapplicable != BTreeSet::from(["parallel-inheritance-hierarchies", "refused-bequest"]) {
            return Err("invalid native Rust applicability exclusions".into());
        }
    } else if registry
        .smells
        .iter()
        .any(|smell| smell.applicability != "applicable")
    {
        return Err(format!(
            "{} catalog must mark every canonical smell applicable",
            registry.language
        ));
    }
    Ok(())
}

fn valid_rule(rule: &Rule, registry: &Registry) -> bool {
    rule.version > 0
        && !rule.inputs.is_empty()
        && !rule.contract.is_empty()
        && matches!(
            rule.implementation.as_str(),
            "implemented" | "not_implemented"
        )
        && matches!(
            rule.kind.as_str(),
            "metric" | "indicator" | "project_contract" | "history"
        )
        && registry
            .smells
            .iter()
            .any(|smell| smell.rules.contains(&rule.id))
}

fn validate_rules(registry: &Registry) -> Result<(), String> {
    for rule in &registry.rules {
        if !valid_rule(rule, registry) {
            return Err(format!("invalid rule contract: {}", rule.id));
        }
    }
    Ok(())
}

fn validate_guidance_coverage(
    registry: &Registry,
    catalog: &GuidanceCatalog,
) -> Result<(), String> {
    let smell_ids: BTreeSet<_> = registry
        .smells
        .iter()
        .map(|smell| smell.id.as_str())
        .collect();
    let guidance_ids: BTreeSet<_> = catalog
        .records
        .iter()
        .map(|guidance| guidance.smell_id.as_str())
        .collect();
    if catalog.records.len() != smell_ids.len() || guidance_ids != smell_ids {
        return Err("when-to-ignore guidance must cover every smell exactly once".into());
    }
    Ok(())
}

fn valid_guidance_identity(guidance: &WhenToIgnore) -> bool {
    guidance.version == 1
        && guidance.guidance_ref == format!("{}@1", guidance.smell_id)
        && guidance.source_url == format!("https://refactoring.guru/smells/{}", guidance.smell_id)
}

fn valid_guidance_content(guidance: &WhenToIgnore) -> bool {
    matches!(
        guidance.provenance.as_str(),
        "refactoring_guru_verbatim" | "smells_authored"
    ) && guidance.checked_on == "2026-09-25"
        && !guidance.items.is_empty()
        && guidance.items.iter().all(|item| !item.trim().is_empty())
}

fn validate_guidance(guidance: &WhenToIgnore) -> Result<(), String> {
    if valid_guidance_identity(guidance) && valid_guidance_content(guidance) {
        return Ok(());
    }
    Err(format!(
        "invalid when-to-ignore guidance: {}",
        guidance.smell_id
    ))
}

fn valid_guidance_catalog_header(catalog: &GuidanceCatalog) -> bool {
    catalog.schema_version == 1 && catalog.catalog_id == "when-to-ignore-v1"
}

fn embedded_guidance_catalog(registry: &Registry) -> Result<Vec<WhenToIgnore>, String> {
    let catalog: GuidanceCatalog =
        serde_json::from_str(include_str!("../rules/when-to-ignore-v1.json"))
            .map_err(|error| format!("invalid embedded when-to-ignore catalog: {error}"))?;
    if !valid_guidance_catalog_header(&catalog) {
        return Err("invalid embedded when-to-ignore catalog header".into());
    }
    validate_guidance_coverage(registry, &catalog)?;

    for guidance in &catalog.records {
        validate_guidance(guidance)?;
    }

    Ok(catalog.records)
}

pub fn registry(rule_pack: &str) -> Result<Registry, String> {
    let source = registry_json(rule_pack)?;
    let mut registry: Registry =
        serde_json::from_str(source).map_err(|e| format!("invalid embedded registry: {e}"))?;
    validate_catalog(&registry, rule_pack)?;
    validate_smell_catalog(&registry)?;
    validate_smell_mappings(&registry)?;
    validate_applicability(&registry)?;
    validate_rules(&registry)?;
    registry.catalog_versions = crate::catalog::validate_embedded(&registry)?;
    registry.guidance_catalog = embedded_guidance_catalog(&registry)?;
    Ok(registry)
}

pub fn registry_for_policy(bytes: &str) -> Result<Registry, String> {
    let value: serde_json::Value =
        serde_json::from_str(bytes).map_err(|e| format!("invalid policy: {e}"))?;
    let rule_pack = value
        .get("rule_pack")
        .and_then(serde_json::Value::as_str)
        .ok_or("policy rule_pack must be a string")?;
    registry(rule_pack)
}

fn validate_policy_header(policy: &Policy, registry: &Registry) -> Result<(), String> {
    if policy.schema_version != 2 || policy.rule_pack != registry.rule_pack {
        return Err("policy schema/rule-pack version mismatch".into());
    }
    if policy.scanner_version != env!("CARGO_PKG_VERSION") {
        return Err("scanner version does not match policy pin".into());
    }
    Ok(())
}

fn validate_scope(policy: &Policy, registry: &Registry) -> Result<(), String> {
    let expected_scope = if registry.language == "rust" {
        "authored_all_cfg"
    } else {
        "authored_source"
    };
    if policy.scope != expected_scope {
        return Err(format!(
            "only {expected_scope} scope is implemented for {}",
            registry.language
        ));
    }
    Ok(())
}

fn validate_budgets_and_exceptions(policy: &Policy) -> Result<(), String> {
    if policy.limits.maximum_files == 0
        || policy.limits.maximum_pairs == 0
        || policy.limits.maximum_group_combinations == 0
    {
        return Err("analysis budgets must be positive".into());
    }
    if !policy.exceptions.is_empty() {
        return Err(
            "exact exceptions are not implemented; cannot silently apply exceptions".into(),
        );
    }
    Ok(())
}

fn validate_exclusions(policy: &Policy) -> Result<(), String> {
    let exclusions: BTreeSet<_> = policy
        .exclude_directories
        .iter()
        .map(String::as_str)
        .collect();
    if exclusions.len() != policy.exclude_directories.len()
        || exclusions.iter().any(|s| {
            s.is_empty() || s.contains('/') || s.contains('\\') || matches!(*s, "." | "..")
        })
        || !exclusions.contains(".git")
    {
        return Err("exclude_directories must be unique directory names and include .git".into());
    }
    Ok(())
}

fn valid_anchored_role_path(path: &str, allow_glob: bool) -> bool {
    if !path.starts_with('/')
        || path.len() == 1
        || path.contains('\\')
        || path.contains("//")
        || path
            .split('/')
            .any(|component| matches!(component, "." | ".."))
    {
        return false;
    }
    if allow_glob {
        !path.contains(['?', '[', ']', '{', '}'])
    } else {
        !path.contains('*') && !path.contains(['?', '[', ']', '{', '}'])
    }
}

fn framework_supported(language: &str, framework: &str) -> bool {
    match language {
        "python" => matches!(framework, "alembic" | "django"),
        "typescript" => matches!(
            framework,
            "typeorm" | "knex" | "mikro_orm" | "kysely" | "sequelize_umzug" | "prisma_8"
        ),
        "rust" => matches!(framework, "sea_orm" | "refinery" | "rusqlite_migration"),
        _ => false,
    }
}

fn valid_source_role_declaration(declaration: &SourceRoleDeclaration, registry: &Registry) -> bool {
    let unique_paths = declaration.paths.iter().collect::<BTreeSet<_>>();
    let unique_configuration = declaration.configuration.iter().collect::<BTreeSet<_>>();
    declaration.role == "database_migration"
        && framework_supported(&registry.language, &declaration.framework)
        && !declaration.paths.is_empty()
        && unique_paths.len() == declaration.paths.len()
        && unique_configuration.len() == declaration.configuration.len()
        && declaration
            .paths
            .iter()
            .all(|path| valid_anchored_role_path(path, true))
        && declaration
            .configuration
            .iter()
            .all(|path| valid_anchored_role_path(path, false))
}

fn validate_source_role_directories(
    declaration: &SourceRoleDeclaration,
    excluded_directories: &[String],
) -> Result<(), String> {
    for path in declaration.paths.iter().chain(&declaration.configuration) {
        if path.split('/').any(|component| {
            excluded_directories
                .iter()
                .any(|excluded| excluded == component)
        }) {
            return Err(format!(
                "source role path traverses an excluded directory: {path}"
            ));
        }
    }
    Ok(())
}

fn record_source_role_identity<'a>(
    declaration: &'a SourceRoleDeclaration,
    identities: &mut BTreeSet<(&'a str, &'a str, &'a Vec<String>, &'a Vec<String>)>,
    scopes: &mut BTreeMap<&'a str, &'a str>,
) -> Result<(), String> {
    let identity = (
        declaration.role.as_str(),
        declaration.framework.as_str(),
        &declaration.paths,
        &declaration.configuration,
    );
    if !identities.insert(identity) {
        return Err("duplicate source role declaration".into());
    }
    for path in &declaration.paths {
        if let Some(previous) = scopes.insert(path, &declaration.framework) {
            return Err(format!(
                "duplicate or ambiguous source role path scope: {path} ({previous}, {})",
                declaration.framework
            ));
        }
    }
    Ok(())
}

fn validate_source_roles(policy: &Policy, registry: &Registry) -> Result<(), String> {
    let mut identities = BTreeSet::new();
    let mut scopes = BTreeMap::<&str, &str>::new();
    for declaration in &policy.source_roles {
        if !valid_source_role_declaration(declaration, registry) {
            return Err(format!(
                "invalid source role declaration for {}",
                declaration.framework
            ));
        }
        validate_source_role_directories(declaration, &policy.exclude_directories)?;
        record_source_role_identity(declaration, &mut identities, &mut scopes)?;
    }
    Ok(())
}

fn validate_selected_rules(policy: &Policy, registry: &Registry) -> Result<(), String> {
    let expected: BTreeSet<_> = registry.rules.iter().map(|r| r.id.as_str()).collect();
    let actual: BTreeSet<_> = policy.rules.keys().map(String::as_str).collect();
    if expected != actual {
        return Err(
            "policy must explicitly select every registered rule, with no unknown rules".into(),
        );
    }
    Ok(())
}

fn group_definitions(registry: &Registry) -> BTreeMap<String, BTreeSet<String>> {
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
        ("all".into(), BTreeSet::new()),
        ("source".into(), BTreeSet::new()),
        ("evidence".into(), BTreeSet::new()),
    ]);
    for smell in &registry.smells {
        groups.entry(smell.category.clone()).or_default();
    }
    for rule in &registry.rules {
        groups.get_mut("all").unwrap().insert(rule.id.clone());
        let evaluator_group = if rule.inputs.iter().any(|input| input == "authored_source") {
            "source"
        } else {
            "evidence"
        };
        groups
            .get_mut(evaluator_group)
            .unwrap()
            .insert(rule.id.clone());
        let category = &registry
            .smells
            .iter()
            .find(|smell| smell.id == rule.smell)
            .expect("validated smell mapping")
            .category;
        groups
            .get_mut(category)
            .expect("registered category group")
            .insert(rule.id.clone());
    }
    groups
}

fn unique_groups(values: &[String], label: &str) -> Result<BTreeSet<String>, String> {
    let set: BTreeSet<_> = values.iter().cloned().collect();
    if set.len() != values.len() {
        return Err(format!("duplicate {label} selector"));
    }
    Ok(set)
}

fn validate_group_names(
    names: &BTreeSet<String>,
    groups: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    for name in names {
        if !groups.contains_key(name) {
            return Err(format!("unknown policy group: {name}"));
        }
    }
    Ok(())
}

fn add_group_rules(
    selected: &mut BTreeSet<String>,
    names: &BTreeSet<String>,
    groups: &BTreeMap<String, BTreeSet<String>>,
) {
    for name in names {
        selected.extend(groups[name].iter().cloned());
    }
}

struct NamedGroupSelection {
    defaults: BTreeSet<String>,
    included: BTreeSet<String>,
    only: BTreeSet<String>,
    excluded: BTreeSet<String>,
}

fn validate_selection_options(options: &SelectionOptions) -> Result<(), String> {
    if !options.only_groups.is_empty()
        && (!options.included_groups.is_empty() || options.all_groups || options.no_default_groups)
    {
        return Err(
            "--only-group conflicts with --group, --all-groups, and --no-default-groups".into(),
        );
    }
    Ok(())
}

fn validate_named_group_selection(
    selection: &NamedGroupSelection,
    groups: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    if selection.defaults.is_empty() {
        return Err("default_groups must select at least one policy group".into());
    }
    for names in [
        &selection.defaults,
        &selection.included,
        &selection.only,
        &selection.excluded,
    ] {
        validate_group_names(names, groups)?;
    }
    Ok(())
}

fn normalized_group_selection(
    policy: &Policy,
    options: &SelectionOptions,
    groups: &BTreeMap<String, BTreeSet<String>>,
) -> Result<NamedGroupSelection, String> {
    validate_selection_options(options)?;
    let selection = NamedGroupSelection {
        defaults: unique_groups(&policy.default_groups, "default group")?,
        included: unique_groups(&options.included_groups, "--group")?,
        only: unique_groups(&options.only_groups, "--only-group")?,
        excluded: unique_groups(&options.excluded_groups, "--no-group")?,
    };
    validate_named_group_selection(&selection, groups)?;
    Ok(selection)
}

fn selected_rules(
    selection: &NamedGroupSelection,
    options: &SelectionOptions,
    groups: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let mut selected = BTreeSet::new();
    if !selection.only.is_empty() {
        add_group_rules(&mut selected, &selection.only, groups);
    } else {
        if !options.no_default_groups {
            add_group_rules(&mut selected, &selection.defaults, groups);
        }
        if options.all_groups {
            selected.extend(groups["all"].iter().cloned());
        }
        add_group_rules(&mut selected, &selection.included, groups);
    }
    let mut removed = BTreeSet::new();
    add_group_rules(&mut removed, &selection.excluded, groups);
    selected.retain(|rule| !removed.contains(rule));
    selected
}

fn rule_group_memberships(
    all: &BTreeSet<String>,
    groups: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, Vec<String>> {
    all.iter()
        .map(|rule| {
            let memberships = groups
                .iter()
                .filter(|(_, rules)| rules.contains(rule))
                .map(|(name, _)| name.clone())
                .collect();
            (rule.clone(), memberships)
        })
        .collect()
}

fn resolve_selection(
    policy: &Policy,
    registry: &Registry,
    options: &SelectionOptions,
) -> Result<ResolvedSelection, String> {
    let groups = group_definitions(registry);
    let names = normalized_group_selection(policy, options, &groups)?;
    let selected = selected_rules(&names, options, &groups);
    let all = &groups["all"];
    let active: Vec<_> = selected
        .iter()
        .filter(|rule| policy.rules[*rule].mode != Mode::Off)
        .cloned()
        .collect();
    Ok(ResolvedSelection {
        default_groups: names.defaults.into_iter().collect(),
        included_groups: names.included.into_iter().collect(),
        only_groups: names.only.into_iter().collect(),
        excluded_groups: names.excluded.into_iter().collect(),
        all_groups: options.all_groups,
        no_default_groups: options.no_default_groups,
        available_groups: groups
            .iter()
            .map(|(name, rules)| (name.clone(), rules.iter().cloned().collect()))
            .collect(),
        selected_rule_ids: selected.iter().cloned().collect(),
        active_rule_ids: active,
        excluded_rule_ids: all.difference(&selected).cloned().collect(),
        rule_groups: rule_group_memberships(all, &groups),
    })
}

fn invalid_parameter(name: &str, value: u64) -> bool {
    (name.contains("percent") && value > 100)
        || (name.contains("basis_points") && value > 10_000)
        || (name.starts_with("minimum_") && value == 0)
        || (name == "minimum_group_size" && value < 2)
        || (name == "minimum_tokens" && value < 4)
}

fn validate_rule_parameters(policy: &Policy, registry: &Registry) -> Result<(), String> {
    for rule in &registry.rules {
        let selected = &policy.rules[&rule.id];
        if selected.version != rule.version || selected.parameters.keys().ne(rule.parameters.keys())
        {
            return Err(format!("rule version/parameter keys mismatch: {}", rule.id));
        }
        for (name, value) in &selected.parameters {
            if invalid_parameter(name, *value) {
                return Err(format!("invalid parameter {}.{name}", rule.id));
            }
        }
    }
    Ok(())
}

pub fn parse_with_selection(
    bytes: &str,
    registry: &Registry,
    options: &SelectionOptions,
) -> Result<Policy, String> {
    let mut policy: Policy =
        serde_json::from_str(bytes).map_err(|e| format!("invalid policy: {e}"))?;
    validate_policy_header(&policy, registry)?;
    validate_scope(&policy, registry)?;
    validate_budgets_and_exceptions(&policy)?;
    validate_exclusions(&policy)?;
    validate_source_roles(&policy, registry)?;
    validate_selected_rules(&policy, registry)?;
    validate_rule_parameters(&policy, registry)?;
    policy.resolved = resolve_selection(&policy, registry, options)?;
    Ok(policy)
}

pub fn parse(bytes: &str, registry: &Registry) -> Result<Policy, String> {
    parse_with_selection(bytes, registry, &SelectionOptions::default())
}

impl Policy {
    pub fn selected(&self, id: &str) -> bool {
        self.resolved
            .selected_rule_ids
            .binary_search_by(|rule| rule.as_str().cmp(id))
            .is_ok()
    }
    pub fn enabled(&self, id: &str) -> bool {
        self.resolved
            .active_rule_ids
            .binary_search_by(|rule| rule.as_str().cmp(id))
            .is_ok()
    }
    pub fn required(&self, id: &str) -> bool {
        self.rules[id].mode == Mode::Required
    }
    pub fn parameter(&self, id: &str, name: &str) -> u64 {
        self.rules[id].parameters[name]
    }
}

pub(crate) fn required_unimplemented_rule_ids<'a>(
    registry: &'a Registry,
    policy: &Policy,
) -> Vec<&'a str> {
    registry
        .rules
        .iter()
        .filter(|rule| rule.implementation == "not_implemented" && policy.required(&rule.id))
        .map(|rule| rule.id.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn raw_registry(rule_pack: &str) -> Registry {
        serde_json::from_str(registry_json(rule_pack).unwrap()).unwrap()
    }

    fn rust_policy_value() -> Value {
        serde_json::from_str(include_str!("../examples/quality-policy.json")).unwrap()
    }

    fn policy_result(value: &Value) -> Result<Policy, String> {
        parse(
            &serde_json::to_string(value).unwrap(),
            &registry("rust-v1").unwrap(),
        )
    }

    #[test]
    fn registry_catalog_header_rejects_drift_in_every_pinned_field() {
        let invalid = |mutate: fn(&mut Registry)| {
            let mut registry = raw_registry("rust-v1");
            mutate(&mut registry);
            validate_catalog(&registry, "rust-v1").is_err()
        };

        assert!(invalid(|registry| registry.schema_version = 2));
        assert!(invalid(|registry| registry.rule_pack = "python-v1".into()));
        assert!(invalid(|registry| registry.language = "java".into()));
        assert!(invalid(
            |registry| registry.catalog.source = "https://example.com".into()
        ));
        assert!(invalid(
            |registry| registry.catalog.checked_on = "2026-10-11".into()
        ));
        assert!(invalid(|registry| registry.catalog.item_count = 22));
        assert!(invalid(
            |registry| registry.catalog.guidance_pack = "other-guidance".into()
        ));
        assert!(invalid(|registry| {
            registry.smells.pop();
        }));
    }

    #[test]
    fn registry_smells_and_rules_must_form_one_complete_bijection() {
        let mut registry = raw_registry("rust-v1");
        registry.smells.pop();
        assert!(validate_smell_catalog(&registry).is_err());

        let mut registry = raw_registry("rust-v1");
        registry.rules[1].id = registry.rules[0].id.clone();
        assert!(validate_smell_catalog(&registry).is_err());

        let mut registry = raw_registry("rust-v1");
        registry.smells[0].applicability = "unknown".into();
        assert!(validate_smell_mappings(&registry).is_err());

        let mut registry = raw_registry("rust-v1");
        let applicable = registry
            .smells
            .iter_mut()
            .find(|smell| smell.applicability == "applicable")
            .unwrap();
        applicable.rules.clear();
        assert!(validate_smell_mappings(&registry).is_err());

        let mut registry = raw_registry("rust-v1");
        registry.smells[0].rules[0] = "rust.missing-rule".into();
        assert!(validate_smell_mappings(&registry).is_err());
    }

    #[test]
    fn language_applicability_is_exact_for_rust_and_portable_packs() {
        let mut rust = raw_registry("rust-v1");
        rust.smells
            .iter_mut()
            .find(|smell| smell.id == "refused-bequest")
            .unwrap()
            .applicability = "applicable".into();
        assert!(validate_applicability(&rust).is_err());

        let mut python = raw_registry("python-v1");
        python.smells[0].applicability = "not_applicable_native_rust".into();
        assert!(validate_applicability(&python).is_err());
    }

    #[test]
    fn rule_contract_requires_version_inputs_kind_implementation_and_owner() {
        let valid = raw_registry("rust-v1");
        assert!(valid.rules.iter().all(|rule| valid_rule(rule, &valid)));

        let invalid = |mutate: fn(&mut Rule)| {
            let mut registry = raw_registry("rust-v1");
            let mut rule = registry.rules.remove(0);
            mutate(&mut rule);
            !valid_rule(&rule, &registry)
        };
        assert!(invalid(|rule| rule.version = 0));
        assert!(invalid(|rule| rule.inputs.clear()));
        assert!(invalid(|rule| rule.contract.clear()));
        assert!(invalid(|rule| rule.implementation = "planned".into()));
        assert!(invalid(|rule| rule.kind = "heuristic".into()));
        assert!(invalid(|rule| rule.id = "rust.unowned-rule".into()));

        let mut registry = raw_registry("rust-v1");
        registry.rules[0].version = 0;
        assert!(validate_rules(&registry).is_err());
    }

    #[test]
    fn guidance_identity_content_and_coverage_fail_closed() {
        let registry = registry("rust-v1").unwrap();
        let guidance = registry.guidance_catalog[0].clone();
        assert!(valid_guidance_identity(&guidance));
        assert!(valid_guidance_content(&guidance));
        assert!(validate_guidance(&guidance).is_ok());

        let invalid_identity = |mutate: fn(&mut WhenToIgnore)| {
            let mut guidance = registry.guidance_catalog[0].clone();
            mutate(&mut guidance);
            !valid_guidance_identity(&guidance) && validate_guidance(&guidance).is_err()
        };
        assert!(invalid_identity(|guidance| guidance.version = 2));
        assert!(invalid_identity(
            |guidance| guidance.guidance_ref = "wrong@1".into()
        ));
        assert!(invalid_identity(
            |guidance| guidance.source_url = "https://example.com".into()
        ));

        let invalid_content = |mutate: fn(&mut WhenToIgnore)| {
            let mut guidance = registry.guidance_catalog[0].clone();
            mutate(&mut guidance);
            !valid_guidance_content(&guidance) && validate_guidance(&guidance).is_err()
        };
        assert!(invalid_content(
            |guidance| guidance.provenance = "unknown".into()
        ));
        assert!(invalid_content(
            |guidance| guidance.checked_on = "2026-10-11".into()
        ));
        assert!(invalid_content(|guidance| guidance.items.clear()));
        assert!(invalid_content(|guidance| guidance.items[0] = "  ".into()));

        let mut catalog: GuidanceCatalog =
            serde_json::from_str(include_str!("../rules/when-to-ignore-v1.json")).unwrap();
        assert!(valid_guidance_catalog_header(&catalog));
        catalog.schema_version = 2;
        assert!(!valid_guidance_catalog_header(&catalog));
        let mut catalog: GuidanceCatalog =
            serde_json::from_str(include_str!("../rules/when-to-ignore-v1.json")).unwrap();
        catalog.catalog_id = "other-guidance".into();
        assert!(!valid_guidance_catalog_header(&catalog));

        let mut catalog: GuidanceCatalog =
            serde_json::from_str(include_str!("../rules/when-to-ignore-v1.json")).unwrap();
        catalog.records.pop();
        assert!(validate_guidance_coverage(&registry, &catalog).is_err());
        let mut catalog: GuidanceCatalog =
            serde_json::from_str(include_str!("../rules/when-to-ignore-v1.json")).unwrap();
        catalog.records[1].smell_id = catalog.records[0].smell_id.clone();
        assert!(validate_guidance_coverage(&registry, &catalog).is_err());
    }

    #[test]
    fn policy_header_scope_budgets_and_exclusions_reject_each_invalid_shape() {
        let invalid = |pointer: &str, replacement: Value| {
            let mut value = rust_policy_value();
            *value.pointer_mut(pointer).unwrap() = replacement;
            policy_result(&value).is_err()
        };
        assert!(invalid("/schema_version", json!(3)));
        assert!(invalid("/rule_pack", json!("python-v1")));
        assert!(invalid("/scanner_version", json!("0.0.0")));
        assert!(invalid("/scope", json!("authored_source")));
        assert!(invalid("/limits/maximum_files", json!(0)));
        assert!(invalid("/limits/maximum_pairs", json!(0)));
        assert!(invalid("/limits/maximum_group_combinations", json!(0)));
        assert!(invalid(
            "/exceptions",
            json!([{"rule": "rust.function_lines"}])
        ));
        assert!(invalid("/exclude_directories", json!([".git", ".git"])));
        assert!(invalid("/exclude_directories", json!([".git", "a/b"])));
        assert!(invalid("/exclude_directories", json!([".git", "a\\b"])));
        assert!(invalid("/exclude_directories", json!([".git", "."])));
        assert!(invalid("/exclude_directories", json!([".git", ".."])));
        assert!(invalid("/exclude_directories", json!(["target"])));
    }

    #[test]
    fn anchored_role_paths_and_frameworks_are_language_specific() {
        for path in ["/migrations", "/db/*/migrations"] {
            assert!(valid_anchored_role_path(path, true), "valid path: {path}");
        }
        for path in [
            "",
            "/",
            "migrations",
            "/db\\migrations",
            "/db//migrations",
            "/db/./migrations",
            "/db/../migrations",
            "/db/?/migrations",
            "/db/[m]/migrations",
            "/db/{m}/migrations",
        ] {
            assert!(
                !valid_anchored_role_path(path, true),
                "invalid path: {path}"
            );
        }
        assert!(!valid_anchored_role_path("/db/*/config", false));

        for (language, framework) in [
            ("python", "alembic"),
            ("python", "django"),
            ("typescript", "typeorm"),
            ("typescript", "drizzle"),
            ("rust", "sea_orm"),
            ("rust", "refinery"),
            ("rust", "rusqlite_migration"),
        ] {
            let expected = framework != "drizzle";
            assert_eq!(framework_supported(language, framework), expected);
        }
        assert!(!framework_supported("java", "flyway"));
    }

    #[test]
    fn source_role_policy_rejects_excluded_duplicate_and_ambiguous_scopes() {
        let declaration = json!({
            "role": "database_migration",
            "framework": "refinery",
            "paths": ["/migrations/*.rs"],
            "configuration": ["/src/lib.rs"]
        });
        let mut value = rust_policy_value();
        value["source_roles"] = json!([declaration]);
        assert!(policy_result(&value).is_ok());

        let mut excluded = value.clone();
        excluded["source_roles"][0]["paths"] = json!(["/target/migrations/*.rs"]);
        assert!(policy_result(&excluded).is_err());

        let mut duplicate = value.clone();
        duplicate["source_roles"] = json!([
            duplicate["source_roles"][0].clone(),
            duplicate["source_roles"][0].clone()
        ]);
        assert!(policy_result(&duplicate).is_err());

        let mut ambiguous = value;
        let mut second = ambiguous["source_roles"][0].clone();
        second["framework"] = json!("sea_orm");
        ambiguous["source_roles"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(policy_result(&ambiguous).is_err());
    }

    #[test]
    fn policy_requires_every_registered_rule_and_parameter_boundary() {
        let mut value = rust_policy_value();
        let first_rule = value["rules"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        value["rules"].as_object_mut().unwrap().remove(&first_rule);
        assert!(policy_result(&value).is_err());

        for (name, value, expected) in [
            ("minimum_items", 0, true),
            ("minimum_items", 1, false),
            ("maximum_percent", 100, false),
            ("maximum_percent", 101, true),
            ("minimum_basis_points", 10_000, false),
            ("minimum_basis_points", 10_001, true),
            ("minimum_group_size", 1, true),
            ("minimum_group_size", 2, false),
            ("minimum_tokens", 3, true),
            ("minimum_tokens", 4, false),
        ] {
            assert_eq!(invalid_parameter(name, value), expected, "{name}={value}");
        }
    }

    #[test]
    fn only_required_unimplemented_rules_are_reported() {
        let mut first_registry = registry("rust-v1").unwrap();
        let policy = parse(
            include_str!("../examples/quality-policy.json"),
            &first_registry,
        )
        .unwrap();
        let required = first_registry
            .rules
            .iter_mut()
            .find(|rule| policy.required(&rule.id))
            .unwrap();
        let required_id = required.id.clone();
        required.implementation = "not_implemented".into();
        assert_eq!(
            required_unimplemented_rule_ids(&first_registry, &policy),
            [required_id.as_str()]
        );

        let mut registry = registry("rust-v1").unwrap();
        let nonrequired = registry
            .rules
            .iter_mut()
            .find(|rule| !policy.required(&rule.id))
            .unwrap();
        nonrequired.implementation = "not_implemented".into();
        assert!(required_unimplemented_rule_ids(&registry, &policy).is_empty());
    }

    #[test]
    fn generated_groups_assign_every_rule_to_its_input_kind_and_smell_category() {
        let registry = registry("rust-v1").unwrap();
        let groups = group_definitions(&registry);
        for rule in &registry.rules {
            assert!(groups["all"].contains(&rule.id));
            let source_rule = rule.inputs.iter().any(|input| input == "authored_source");
            assert_eq!(groups["source"].contains(&rule.id), source_rule);
            assert_eq!(groups["evidence"].contains(&rule.id), !source_rule);

            let category = &registry
                .smells
                .iter()
                .find(|smell| smell.id == rule.smell)
                .unwrap()
                .category;
            assert!(groups[category].contains(&rule.id));
            for other in registry
                .smells
                .iter()
                .map(|smell| &smell.category)
                .filter(|other| *other != category)
            {
                assert!(!groups[other].contains(&rule.id));
            }
        }
    }

    #[test]
    fn duplicate_map_error_preserves_the_unique_key_contract() {
        let registry = registry("rust-v1").unwrap();
        let mut value = rust_policy_value();
        value["rules"] = json!([]);
        let error = parse(&serde_json::to_string(&value).unwrap(), &registry).unwrap_err();
        assert!(error.contains("an object with unique keys"), "{error}");
    }
}
