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
    rule.version == 1
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

fn embedded_guidance_catalog(registry: &Registry) -> Result<Vec<WhenToIgnore>, String> {
    let catalog: GuidanceCatalog =
        serde_json::from_str(include_str!("../rules/when-to-ignore-v1.json"))
            .map_err(|error| format!("invalid embedded when-to-ignore catalog: {error}"))?;
    if catalog.schema_version != 1 || catalog.catalog_id != "when-to-ignore-v1" {
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
