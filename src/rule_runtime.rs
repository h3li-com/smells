/// Closed identity for every rule backed by the built-in evidence runtime.
///
/// Rule IDs are language-qualified, but the collector/evaluator contract is
/// shared by suffix. Parsing that suffix in one catalog prevents the collector
/// and evaluator dispatch tables from silently drifting apart.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum RuleKind {
    AlternativeInterfaces,
    DependencyContract,
    DivergentChange,
    ForeignAccesses,
    ForwardingShare,
    FunctionCrap,
    LibraryCapabilities,
    NavigationChains,
    NominalSlotContract,
    ParallelInheritance,
    PortConformance,
    PrimitiveSlots,
    RefusedBequest,
    RepeatedDispatch,
    ShotgunSurgery,
    TemporaryFields,
    UnusedCode,
    UnusedTypeParameters,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CollectorDomain {
    Contracts(ContractCollector),
    History,
    Source,
    Structural(StructuralCollector),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContractCollector {
    Capability,
    Inheritance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StructuralCollector {
    ClassShape,
    FunctionRisk,
    Repetition,
}

struct RuntimeDefinition {
    suffix: &'static str,
    kind: RuleKind,
    collector: CollectorDomain,
}

const CATALOG: &[RuntimeDefinition] = &[
    RuntimeDefinition {
        suffix: "alternative_interfaces",
        kind: RuleKind::AlternativeInterfaces,
        collector: CollectorDomain::Structural(StructuralCollector::ClassShape),
    },
    RuntimeDefinition {
        suffix: "dependency_contract",
        kind: RuleKind::DependencyContract,
        collector: CollectorDomain::Source,
    },
    RuntimeDefinition {
        suffix: "divergent_change",
        kind: RuleKind::DivergentChange,
        collector: CollectorDomain::History,
    },
    RuntimeDefinition {
        suffix: "foreign_accesses",
        kind: RuleKind::ForeignAccesses,
        collector: CollectorDomain::Source,
    },
    RuntimeDefinition {
        suffix: "forwarding_share",
        kind: RuleKind::ForwardingShare,
        collector: CollectorDomain::Structural(StructuralCollector::ClassShape),
    },
    RuntimeDefinition {
        suffix: "function_crap",
        kind: RuleKind::FunctionCrap,
        collector: CollectorDomain::Structural(StructuralCollector::FunctionRisk),
    },
    RuntimeDefinition {
        suffix: "library_capabilities",
        kind: RuleKind::LibraryCapabilities,
        collector: CollectorDomain::Contracts(ContractCollector::Capability),
    },
    RuntimeDefinition {
        suffix: "navigation_chains",
        kind: RuleKind::NavigationChains,
        collector: CollectorDomain::Source,
    },
    RuntimeDefinition {
        suffix: "nominal_slot_contract",
        kind: RuleKind::NominalSlotContract,
        collector: CollectorDomain::Contracts(ContractCollector::Capability),
    },
    RuntimeDefinition {
        suffix: "parallel_inheritance",
        kind: RuleKind::ParallelInheritance,
        collector: CollectorDomain::Contracts(ContractCollector::Inheritance),
    },
    RuntimeDefinition {
        suffix: "port_conformance",
        kind: RuleKind::PortConformance,
        collector: CollectorDomain::Contracts(ContractCollector::Inheritance),
    },
    RuntimeDefinition {
        suffix: "primitive_slots",
        kind: RuleKind::PrimitiveSlots,
        collector: CollectorDomain::Source,
    },
    RuntimeDefinition {
        suffix: "refused_bequest",
        kind: RuleKind::RefusedBequest,
        collector: CollectorDomain::Contracts(ContractCollector::Inheritance),
    },
    RuntimeDefinition {
        suffix: "repeated_dispatch",
        kind: RuleKind::RepeatedDispatch,
        collector: CollectorDomain::Structural(StructuralCollector::Repetition),
    },
    RuntimeDefinition {
        suffix: "shotgun_surgery",
        kind: RuleKind::ShotgunSurgery,
        collector: CollectorDomain::History,
    },
    RuntimeDefinition {
        suffix: "temporary_fields",
        kind: RuleKind::TemporaryFields,
        collector: CollectorDomain::Structural(StructuralCollector::ClassShape),
    },
    RuntimeDefinition {
        suffix: "unused_code",
        kind: RuleKind::UnusedCode,
        collector: CollectorDomain::Structural(StructuralCollector::FunctionRisk),
    },
    RuntimeDefinition {
        suffix: "unused_type_parameters",
        kind: RuleKind::UnusedTypeParameters,
        collector: CollectorDomain::Structural(StructuralCollector::FunctionRisk),
    },
];

impl RuleKind {
    #[cfg(test)]
    pub(crate) fn all() -> impl ExactSizeIterator<Item = Self> {
        CATALOG.iter().map(|definition| definition.kind)
    }

    pub(crate) fn from_id(id: &str) -> Result<Self, String> {
        let suffix = id.split_once('.').map_or(id, |(_, suffix)| suffix);
        CATALOG
            .iter()
            .find_map(|definition| (definition.suffix == suffix).then_some(definition.kind))
            .ok_or_else(|| format!("no built-in rule runtime for {id}"))
    }

    pub(crate) fn collector_domain(self) -> CollectorDomain {
        CATALOG
            .iter()
            .find_map(|definition| (definition.kind == self).then_some(definition.collector))
            .expect("every rule kind has a collector domain")
    }

    pub(crate) fn needs_history(self) -> bool {
        matches!(self, Self::DivergentChange | Self::ShotgunSurgery)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_has_unique_suffixes_and_kinds() {
        assert_eq!(CATALOG.len(), 18);
        assert_eq!(
            CATALOG
                .iter()
                .map(|definition| definition.suffix)
                .collect::<BTreeSet<_>>()
                .len(),
            CATALOG.len()
        );
        assert_eq!(
            CATALOG
                .iter()
                .map(|definition| definition.kind)
                .collect::<BTreeSet<_>>()
                .len(),
            CATALOG.len()
        );
        assert!(
            CATALOG
                .iter()
                .all(|definition| { definition.kind.collector_domain() == definition.collector })
        );
    }
}
