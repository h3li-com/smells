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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub source: String,
    pub checked_on: String,
    pub item_count: usize,
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
    pub exclude_directories: Vec<String>,
    pub limits: Limits,
    #[serde(deserialize_with = "unique_map")]
    pub rules: BTreeMap<String, Selection>,
    pub exceptions: Vec<serde_json::Value>,
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

pub fn registry(rule_pack: &str) -> Result<Registry, String> {
    let source = registry_json(rule_pack)?;
    let registry: Registry =
        serde_json::from_str(source).map_err(|e| format!("invalid embedded registry: {e}"))?;
    if registry.schema_version != 1
        || registry.rule_pack != rule_pack
        || !matches!(registry.language.as_str(), "rust" | "python" | "typescript")
        || registry.catalog.source != "https://refactoring.guru/refactoring/smells"
        || registry.catalog.checked_on != "2026-09-19"
        || registry.catalog.item_count != 23
        || registry.smells.len() != registry.catalog.item_count
    {
        return Err(format!("invalid embedded {rule_pack} catalog"));
    }
    let canonical: BTreeSet<_> = [
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
    .collect();
    let smells: BTreeSet<_> = registry
        .smells
        .iter()
        .map(|s| (s.id.as_str(), s.name.as_str(), s.category.as_str()))
        .collect();
    let rules: BTreeSet<_> = registry.rules.iter().map(|r| &r.id).collect();
    if smells != canonical || rules.len() != registry.rules.len() {
        return Err("registry does not exactly match the pinned canonical smell catalog".into());
    }
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
    for rule in &registry.rules {
        if rule.version != 1
            || rule.inputs.is_empty()
            || rule.contract.is_empty()
            || !matches!(
                rule.implementation.as_str(),
                "implemented" | "not_implemented"
            )
            || !matches!(
                rule.kind.as_str(),
                "metric" | "indicator" | "project_contract" | "history"
            )
            || !registry.smells.iter().any(|s| s.rules.contains(&rule.id))
        {
            return Err(format!("invalid rule contract: {}", rule.id));
        }
    }
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

pub fn parse(bytes: &str, registry: &Registry) -> Result<Policy, String> {
    let policy: Policy = serde_json::from_str(bytes).map_err(|e| format!("invalid policy: {e}"))?;
    if policy.schema_version != 2 || policy.rule_pack != registry.rule_pack {
        return Err("policy schema/rule-pack version mismatch".into());
    }
    if policy.scanner_version != env!("CARGO_PKG_VERSION") {
        return Err("scanner version does not match policy pin".into());
    }
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
    let exclusions: BTreeSet<_> = policy.exclude_directories.iter().collect();
    if exclusions.len() != policy.exclude_directories.len()
        || exclusions.iter().any(|s| {
            s.is_empty() || s.contains('/') || s.contains('\\') || matches!(s.as_str(), "." | "..")
        })
        || !exclusions.contains(&".git".to_string())
    {
        return Err("exclude_directories must be unique directory names and include .git".into());
    }
    let expected: BTreeSet<_> = registry.rules.iter().map(|r| r.id.as_str()).collect();
    let actual: BTreeSet<_> = policy.rules.keys().map(String::as_str).collect();
    if expected != actual {
        return Err(
            "policy must explicitly select every registered rule, with no unknown rules".into(),
        );
    }
    for rule in &registry.rules {
        let selected = &policy.rules[&rule.id];
        if selected.version != rule.version || selected.parameters.keys().ne(rule.parameters.keys())
        {
            return Err(format!("rule version/parameter keys mismatch: {}", rule.id));
        }
        for (name, value) in &selected.parameters {
            let invalid = (name.contains("percent") && *value > 100)
                || (name.contains("basis_points") && *value > 10_000)
                || (name.starts_with("minimum_") && *value == 0)
                || (name == "minimum_group_size" && *value < 2)
                || (name == "minimum_tokens" && *value < 4);
            if invalid {
                return Err(format!("invalid parameter {}.{name}", rule.id));
            }
        }
    }
    Ok(policy)
}

impl Policy {
    pub fn enabled(&self, id: &str) -> bool {
        self.rules[id].mode != Mode::Off
    }
    pub fn required(&self, id: &str) -> bool {
        self.rules[id].mode == Mode::Required
    }
    pub fn parameter(&self, id: &str, name: &str) -> u64 {
        self.rules[id].parameters[name]
    }
}
