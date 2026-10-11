use super::{
    common::{location_at, observation, primitive_type},
    model::{ClassFact, SourceModel},
    syntax::code_only,
};
use crate::{core::model::Input, evidence::Observation, policy::Rule};
use regex::Regex;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
    sync::OnceLock,
};

fn semantically_nominal_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "id", "uuid", "email", "phone", "amount", "price", "currency", "date", "time", "address",
        "country", "postcode", "zip",
    ]
    .iter()
    .any(|role| name == *role || name.ends_with(&format!("_{role}")))
}

pub(super) fn nominal_slot_contract(rule: &Rule, model: &SourceModel) -> Vec<Observation> {
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    model
        .classes
        .iter()
        .filter_map(|class| {
            let mismatches = class
                .fields
                .iter()
                .filter(|field| {
                    semantically_nominal_name(&field.name)
                        && primitive_type(language, &field.authored_type)
                })
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>();
            (!mismatches.is_empty()).then(|| {
                observation(
                    class.symbol.clone(),
                    class.location.clone(),
                    [("mismatches", mismatches.len() as u64)],
                    json!({
                        "collector": "authored_semantic_slot_name_v1",
                        "primitive_semantic_slots": mismatches,
                        "scope": "well-known domain-role suffixes; optional project contracts may strengthen this check",
                    }),
                )
            })
        })
        .collect()
}

fn base_leaf(base: &str) -> &str {
    base.rsplit([':', '.'])
        .find(|part| !part.is_empty())
        .unwrap_or(base)
}

fn logical_symbol(class: &ClassFact) -> String {
    let (source_path, authored_owner) = class
        .symbol
        .split_once("::")
        .unwrap_or(("", class.symbol.as_str()));
    let without_extension = Path::new(source_path).with_extension("");
    let mut modules = without_extension
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str().map(str::to_string),
            _ => None,
        })
        .collect::<Vec<_>>();
    if modules.first().is_some_and(|part| part == "src") {
        modules.remove(0);
    }
    if modules
        .last()
        .is_some_and(|part| matches!(part.as_str(), "lib" | "main" | "mod"))
    {
        modules.pop();
    }
    modules.extend(authored_owner.split("::").map(str::to_string));
    modules.join("::")
}

fn qualified_base_target(derived: &ClassFact, base: &str) -> Option<String> {
    let normalized = base.replace('.', "::");
    let mut parts = normalized
        .split("::")
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let mut owner = logical_symbol(derived)
        .split("::")
        .map(str::to_string)
        .collect::<Vec<_>>();
    owner.pop();
    match parts.first().copied() {
        Some("crate") => {
            parts.remove(0);
            owner.clear();
        }
        Some("self") => {
            parts.remove(0);
        }
        Some("super") => {
            while parts.first().copied() == Some("super") {
                parts.remove(0);
                owner.pop();
            }
        }
        _ => owner.clear(),
    }
    owner.extend(parts.into_iter().map(str::to_string));
    Some(owner.join("::"))
}

fn resolve_declared_base<'a>(
    model: &'a SourceModel,
    derived: &ClassFact,
    base: &str,
    interfaces_only: bool,
) -> Option<&'a ClassFact> {
    let target = qualified_base_target(derived, base);
    let mut candidates = model
        .classes
        .iter()
        .filter(|class| !interfaces_only || class.interface)
        .filter(|class| class.name == base_leaf(base))
        .filter(|class| {
            target
                .as_ref()
                .is_none_or(|target| logical_symbol(class) == *target)
        });
    let resolved = candidates.next()?;
    candidates.next().is_none().then_some(resolved)
}

pub(super) fn port_conformance(model: &SourceModel) -> Vec<Observation> {
    let mut observations = Vec::new();
    for implementation in model.classes.iter().filter(|class| !class.interface) {
        let implemented = implementation
            .methods
            .iter()
            .map(|method| method.name.as_str())
            .collect::<BTreeSet<_>>();
        for port in implementation
            .bases
            .iter()
            .filter_map(|base| resolve_declared_base(model, implementation, base, true))
        {
            let required = port
                .methods
                .iter()
                .map(|method| method.name.as_str())
                .collect::<BTreeSet<_>>();
            let missing = required
                .difference(&implemented)
                .copied()
                .collect::<Vec<_>>();
            let mut candidate = observation(
                implementation.symbol.clone(),
                implementation.location.clone(),
                [("failures", missing.len() as u64)],
                json!({
                    "collector": "authored_port_shape_v1",
                    "port": port.symbol,
                    "missing_methods": missing,
                    "scope": "declared interface/Protocol/ABC method names",
                }),
            );
            candidate.related_symbols.push(port.symbol.clone());
            candidate.related_locations.push(port.location.clone());
            observations.push(candidate);
        }
    }
    observations
}

fn rejection_stub(body: &str) -> bool {
    let compact = body
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim()
        .trim_end_matches(';')
        .trim();
    let compact = compact.split_whitespace().collect::<Vec<_>>().join(" ");
    compact == "pass"
        || compact == "return NotImplemented"
        || compact.starts_with("raise NotImplemented")
        || compact.starts_with("throw new Error")
}

pub(super) fn refused_bequest(model: &SourceModel) -> Vec<Observation> {
    let mut observations = Vec::new();
    for derived in &model.classes {
        for base in derived
            .bases
            .iter()
            .filter_map(|base| resolve_declared_base(model, derived, base, false))
        {
            let inherited = base
                .methods
                .iter()
                .filter(|method| !matches!(method.name.as_str(), "__init__" | "constructor"))
                .collect::<Vec<_>>();
            if inherited.is_empty() {
                continue;
            }
            let rejected = inherited
                .iter()
                .filter(|method| {
                    derived
                        .methods
                        .iter()
                        .find(|candidate| candidate.name == method.name)
                        .and_then(|candidate| candidate.body.as_deref())
                        .is_some_and(rejection_stub)
                })
                .map(|method| method.name.as_str())
                .collect::<Vec<_>>();
            let mut candidate = observation(
                derived.symbol.clone(),
                derived.location.clone(),
                [
                    ("inherited_members", inherited.len() as u64),
                    ("unused_members", rejected.len() as u64),
                ],
                json!({
                    "collector": "authored_inherited_member_rejection_v1",
                    "base": base.symbol,
                    "rejected_members": rejected,
                }),
            );
            candidate.related_symbols.push(base.symbol.clone());
            candidate.related_locations.push(base.location.clone());
            observations.push(candidate);
        }
    }
    observations
}

fn hierarchy_role(child: &str, base: &str) -> Option<String> {
    let base = base.strip_suffix("Base").unwrap_or(base);
    let role = child
        .strip_suffix(base)
        .or_else(|| child.strip_prefix(base))?
        .trim_matches('_');
    (!role.is_empty()).then(|| role.to_ascii_lowercase())
}

pub(super) fn parallel_inheritance(model: &SourceModel) -> Vec<Observation> {
    let mut by_base: BTreeMap<&str, BTreeMap<String, &ClassFact>> = BTreeMap::new();
    for child in &model.classes {
        for base in &child.bases {
            let resolved = resolve_declared_base(model, child, base, false);
            let same_named_declarations = model
                .classes
                .iter()
                .filter(|class| class.name == base_leaf(base))
                .count();
            let (base_key, base_name) = if let Some(resolved) = resolved {
                (resolved.symbol.as_str(), resolved.name.as_str())
            } else if same_named_declarations == 0 {
                (base.as_str(), base_leaf(base))
            } else {
                continue;
            };
            if let Some(role) = hierarchy_role(&child.name, base_name) {
                by_base.entry(base_key).or_default().insert(role, child);
            }
        }
    }
    let families = by_base.into_iter().collect::<Vec<_>>();
    let mut observations = Vec::new();
    for (index, (first_base, first_roles)) in families.iter().enumerate() {
        for (second_base, second_roles) in families.iter().skip(index + 1) {
            let shared = first_roles
                .keys()
                .filter(|role| second_roles.contains_key(*role))
                .collect::<Vec<_>>();
            let Some(first) = shared.first().and_then(|role| first_roles.get(*role)) else {
                continue;
            };
            let related = shared
                .iter()
                .filter_map(|role| second_roles.get(*role))
                .collect::<Vec<_>>();
            let mut candidate = observation(
                first.symbol.clone(),
                first.location.clone(),
                [("parallel_pairs", shared.len() as u64)],
                json!({
                    "collector": "authored_parallel_hierarchy_roles_v1",
                    "base_pair": [first_base, second_base],
                    "shared_roles": shared,
                }),
            );
            for class in related {
                candidate.related_symbols.push(class.symbol.clone());
                candidate.related_locations.push(class.location.clone());
            }
            observations.push(candidate);
        }
    }
    observations
}

pub(super) fn library_capabilities(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    if rule.id.starts_with("rust.") {
        return Ok(model
            .classes
            .iter()
            .filter(|class| {
                class.interface
                    && !class.methods.is_empty()
                    && (class.name.ends_with("Ext") || class.name.ends_with("Extension"))
            })
            .map(|class| {
                observation(
                    class.symbol.clone(),
                    class.location.clone(),
                    [("failures", 1)],
                    json!({
                        "collector": "rust_extension_trait_workaround_v1",
                        "scope": "authored non-empty traits named *Ext or *Extension",
                    }),
                )
            })
            .collect());
    }
    static WORKAROUND: OnceLock<Regex> = OnceLock::new();
    let workaround = WORKAROUND.get_or_init(|| {
        Regex::new(
            r#"(?m)(?:\b[A-Za-z_$][A-Za-z0-9_$]*\.prototype\.[A-Za-z_$][A-Za-z0-9_$]*\s*=|\bsetattr\s*\(\s*[A-Za-z_][A-Za-z0-9_.]*\s*,|\b[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\s*=)"#,
        )
        .expect("valid library workaround expression")
    });
    let mut observations = Vec::new();
    for (path, source) in &input.files {
        let code = code_only(path, source)?;
        let failures = workaround.find_iter(&code).collect::<Vec<_>>();
        if let Some(first) = failures.first() {
            observations.push(observation(
                format!("{path}::library-extension-workarounds"),
                location_at(path, source, first.start()),
                [("failures", failures.len() as u64)],
                json!({
                    "collector": "authored_library_extension_workaround_v1",
                    "scope": "prototype or foreign attribute mutation; optional capability tests may replace this evidence",
                }),
            ));
        }
    }
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        collectors::model::{FieldFact, FunctionFact},
        core::model::{History, Input},
        policy::{Limits, Policy, ResolvedSelection},
        report::Location,
    };
    use std::{collections::BTreeMap, sync::Arc};

    fn location(path: &str) -> Location {
        Location {
            path: path.into(),
            line: 1,
            column: 1,
        }
    }

    fn function(name: &str, body: Option<&str>) -> FunctionFact {
        FunctionFact {
            name: name.into(),
            symbol: format!("domain::{name}"),
            location: location("src/domain.rs"),
            type_parameters: Vec::new(),
            body: body.map(Arc::from),
            source: Arc::from(body.unwrap_or_default()),
            private: false,
        }
    }

    fn class(path: &str, owner: &str, name: &str) -> ClassFact {
        ClassFact {
            name: name.into(),
            symbol: format!("{path}::{owner}"),
            location: location(path),
            type_parameters: Vec::new(),
            source: Arc::from(""),
            bases: Vec::new(),
            interface: false,
            private: false,
            fields: Vec::new(),
            methods: Vec::new(),
            range: (0, 0),
        }
    }

    fn rule(id: &str) -> Rule {
        Rule {
            id: id.into(),
            version: 1,
            smell: "test".into(),
            kind: "threshold".into(),
            implementation: "test".into(),
            inputs: Vec::new(),
            parameters: BTreeMap::new(),
            contract: "test".into(),
        }
    }

    fn input(files: BTreeMap<String, String>) -> Input {
        Input {
            files,
            manifests: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            artifact_candidates: BTreeMap::new(),
            implementations: Vec::new(),
            source_roles: Vec::new(),
            history: History::default(),
            policy: Policy {
                schema_version: 1,
                rule_pack: "rust-v1".into(),
                scanner_version: env!("CARGO_PKG_VERSION").into(),
                scope: "repository".into(),
                default_groups: vec!["all".into()],
                exclude_directories: Vec::new(),
                source_roles: Vec::new(),
                limits: Limits {
                    maximum_files: 10,
                    maximum_pairs: 10,
                    maximum_group_combinations: 10,
                },
                rules: BTreeMap::new(),
                exceptions: Vec::new(),
                resolved: ResolvedSelection::default(),
            },
            digest: "0".repeat(64),
            mode: "test",
        }
    }

    #[test]
    fn nominal_slot_names_require_an_exact_domain_suffix_and_a_primitive_type() {
        for accepted in ["id", "account_id", "billing_currency", "postal_address"] {
            assert!(semantically_nominal_name(accepted));
        }
        for rejected in ["identity", "candidate", "currency_code", "addressable"] {
            assert!(!semantically_nominal_name(rejected));
        }

        let mut account = class("src/domain.rs", "Account", "Account");
        account.fields = vec![
            FieldFact {
                name: "account_id".into(),
                authored_type: "String".into(),
                temporary: false,
                private: false,
            },
            FieldFact {
                name: "country".into(),
                authored_type: "Country".into(),
                temporary: false,
                private: false,
            },
            FieldFact {
                name: "identity".into(),
                authored_type: "String".into(),
                temporary: false,
                private: false,
            },
        ];
        let observations = nominal_slot_contract(
            &rule("rust.nominal_slot_contract"),
            &SourceModel {
                classes: vec![account],
                ..SourceModel::default()
            },
        );
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["mismatches"], 1);
        assert_eq!(
            observations[0].evidence["primitive_semantic_slots"][0],
            "account_id"
        );
    }

    #[test]
    fn logical_symbols_resolve_crate_self_super_and_external_bases() {
        let derived = class("src/domain/orders/handler.rs", "nested::Handler", "Handler");
        assert_eq!(
            logical_symbol(&derived),
            "domain::orders::handler::nested::Handler"
        );
        assert_eq!(
            qualified_base_target(&derived, "crate::ports::Reader").as_deref(),
            Some("ports::Reader")
        );
        assert_eq!(
            qualified_base_target(&derived, "self::Reader").as_deref(),
            Some("domain::orders::handler::nested::Reader")
        );
        assert_eq!(
            qualified_base_target(&derived, "super::super::Reader").as_deref(),
            Some("domain::orders::Reader")
        );
        assert_eq!(
            qualified_base_target(&derived, "external::Reader").as_deref(),
            Some("external::Reader")
        );
        assert_eq!(qualified_base_target(&derived, "Reader"), None);
    }

    #[test]
    fn rejection_stubs_are_narrow_and_drive_refused_bequest_counts() {
        for accepted in [
            "pass",
            "return NotImplemented",
            "raise NotImplementedError()",
            "{ throw new Error('unsupported'); }",
        ] {
            assert!(rejection_stub(accepted));
        }
        for rejected in [
            "return value",
            "raise ValueError()",
            "throw error",
            "pass; audit()",
        ] {
            assert!(!rejection_stub(rejected));
        }

        let mut base = class("src/base.py", "Port", "Port");
        base.methods = vec![
            function("__init__", Some("pass")),
            function("load", None),
            function("save", None),
        ];
        let mut derived = class("src/adapter.py", "Adapter", "Adapter");
        derived.bases = vec!["Port".into()];
        derived.methods = vec![
            function("load", Some("raise NotImplementedError()")),
            function("save", Some("return value")),
        ];
        let observations = refused_bequest(&SourceModel {
            classes: vec![base, derived],
            ..SourceModel::default()
        });
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["inherited_members"], 2);
        assert_eq!(observations[0].measurements["unused_members"], 1);
        assert_eq!(observations[0].evidence["rejected_members"][0], "load");
    }

    #[test]
    fn parallel_hierarchies_report_each_shared_role_once() {
        let mut cat = class("src/animals.rs", "CatAnimal", "CatAnimal");
        cat.bases = vec!["Animal".into()];
        let mut dog = class("src/animals.rs", "DogAnimal", "DogAnimal");
        dog.bases = vec!["Animal".into()];
        let mut cat_view = class("src/views.rs", "CatView", "CatView");
        cat_view.bases = vec!["View".into()];
        let mut dog_view = class("src/views.rs", "DogView", "DogView");
        dog_view.bases = vec!["View".into()];
        let observations = parallel_inheritance(&SourceModel {
            classes: vec![cat, dog, cat_view, dog_view],
            ..SourceModel::default()
        });
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["parallel_pairs"], 2);
        assert_eq!(observations[0].related_symbols.len(), 2);
    }

    #[test]
    fn rust_library_capabilities_require_a_nonempty_extension_trait() {
        let mut extension = class("src/ext.rs", "ReadExt", "ReadExt");
        extension.interface = true;
        extension.methods = vec![function("read_more", None)];
        let mut empty = class("src/empty.rs", "EmptyExt", "EmptyExt");
        empty.interface = true;
        let mut ordinary = class("src/port.rs", "ReadPort", "ReadPort");
        ordinary.interface = true;
        ordinary.methods = vec![function("read", None)];

        let observations = library_capabilities(
            &rule("rust.library_capabilities"),
            &SourceModel {
                classes: vec![extension, empty, ordinary],
                ..SourceModel::default()
            },
            &input(BTreeMap::new()),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].symbol, "src/ext.rs::ReadExt");
    }
}
