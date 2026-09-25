mod common;
mod contracts;
mod history;
mod model;
mod source;
mod structural;
mod syntax;

use crate::{
    evidence::Observation,
    input::Input,
    policy::Rule,
    rule_runtime::{CollectorDomain, ContractCollector, RuleKind, StructuralCollector},
};
use model::SourceModel;
use std::sync::OnceLock;

/// One scan-scoped built-in collector session. The parsed source model is built
/// lazily and reused by every type/coverage/contract rule in the language pack.
pub(crate) struct BuiltInCollectors<'a> {
    input: &'a Input,
    model: OnceLock<Result<SourceModel, String>>,
}

impl<'a> BuiltInCollectors<'a> {
    pub(crate) fn new(input: &'a Input) -> Self {
        Self {
            input,
            model: OnceLock::new(),
        }
    }

    fn model(&self, rule: &Rule) -> Result<&SourceModel, String> {
        match self
            .model
            .get_or_init(|| model::source_model(rule, self.input))
        {
            Ok(model) => Ok(model),
            Err(error) => Err(error.clone()),
        }
    }

    /// Collect deterministic built-in observations for a rule that also accepts
    /// optional external evidence. An empty result means the collector completed
    /// and found no candidate in its defined scope.
    pub(crate) fn collect(&self, rule: &Rule) -> Result<Vec<Observation>, String> {
        let kind = RuleKind::from_id(&rule.id)?;
        if kind.needs_history() && !self.input.history.available {
            return Err(format!(
                "Git history is unavailable for built-in collector: {}",
                rule.id
            ));
        }
        match kind.collector_domain() {
            CollectorDomain::Source => self.collect_source(kind, rule),
            CollectorDomain::Structural(group) => self.collect_structural(group, kind, rule),
            CollectorDomain::Contracts(group) => self.collect_contracts(group, kind, rule),
            CollectorDomain::History => Ok(self.collect_history(kind)),
        }
    }

    pub(crate) fn collect_with(
        &self,
        rule: &Rule,
        mut emit: impl FnMut(Observation),
    ) -> Result<(), String> {
        let kind = RuleKind::from_id(&rule.id)?;
        if matches!(kind, RuleKind::NavigationChains) {
            return source::visit_navigation_chains(self.model(rule)?, self.input, emit);
        }
        for observation in self.collect(rule)? {
            emit(observation);
        }
        Ok(())
    }

    fn collect_source(&self, kind: RuleKind, rule: &Rule) -> Result<Vec<Observation>, String> {
        if matches!(kind, RuleKind::NavigationChains) {
            unreachable!("navigation chains stream observations");
        }
        let model = self.model(rule)?;
        match kind {
            RuleKind::ForeignAccesses => source::foreign_accesses(model, self.input),
            RuleKind::DependencyContract => source::dependency_contract(rule, model, self.input),
            RuleKind::PrimitiveSlots => Ok(source::primitive_slots(rule, model)),
            _ => unreachable!("collector domain and source dispatcher must agree"),
        }
    }

    fn collect_structural(
        &self,
        group: StructuralCollector,
        kind: RuleKind,
        rule: &Rule,
    ) -> Result<Vec<Observation>, String> {
        match group {
            StructuralCollector::ClassShape => self.collect_class_shape(kind, rule),
            StructuralCollector::FunctionRisk => self.collect_function_risk(kind, rule),
            StructuralCollector::Repetition => Ok(structural::repeated_dispatch(self.model(rule)?)),
        }
    }

    fn collect_class_shape(&self, kind: RuleKind, rule: &Rule) -> Result<Vec<Observation>, String> {
        match kind {
            RuleKind::AlternativeInterfaces => {
                structural::alternative_interfaces(rule, self.model(rule)?, self.input)
            }
            RuleKind::ForwardingShare => Ok(structural::forwarding_share(rule, self.model(rule)?)),
            RuleKind::TemporaryFields => Ok(structural::temporary_fields(rule, self.model(rule)?)),
            _ => unreachable!("collector group and class-shape dispatcher must agree"),
        }
    }

    fn collect_function_risk(
        &self,
        kind: RuleKind,
        rule: &Rule,
    ) -> Result<Vec<Observation>, String> {
        match kind {
            RuleKind::FunctionCrap => Ok(structural::function_crap(rule, self.model(rule)?)),
            RuleKind::UnusedCode => Ok(structural::unused_code(self.model(rule)?)),
            RuleKind::UnusedTypeParameters => {
                Ok(structural::unused_type_parameters(self.model(rule)?))
            }
            _ => unreachable!("collector group and function-risk dispatcher must agree"),
        }
    }

    fn collect_contracts(
        &self,
        group: ContractCollector,
        kind: RuleKind,
        rule: &Rule,
    ) -> Result<Vec<Observation>, String> {
        match group {
            ContractCollector::Capability => self.collect_capabilities(kind, rule),
            ContractCollector::Inheritance => self.collect_inheritance(kind, rule),
        }
    }

    fn collect_capabilities(
        &self,
        kind: RuleKind,
        rule: &Rule,
    ) -> Result<Vec<Observation>, String> {
        match kind {
            RuleKind::LibraryCapabilities => {
                contracts::library_capabilities(rule, self.model(rule)?, self.input)
            }
            RuleKind::NominalSlotContract => {
                Ok(contracts::nominal_slot_contract(rule, self.model(rule)?))
            }
            _ => unreachable!("collector group and capability dispatcher must agree"),
        }
    }

    fn collect_inheritance(&self, kind: RuleKind, rule: &Rule) -> Result<Vec<Observation>, String> {
        match kind {
            RuleKind::ParallelInheritance => Ok(contracts::parallel_inheritance(self.model(rule)?)),
            RuleKind::PortConformance => Ok(contracts::port_conformance(self.model(rule)?)),
            RuleKind::RefusedBequest => Ok(contracts::refused_bequest(self.model(rule)?)),
            _ => unreachable!("collector group and inheritance dispatcher must agree"),
        }
    }

    fn collect_history(&self, kind: RuleKind) -> Vec<Observation> {
        match kind {
            RuleKind::DivergentChange => history::divergent_change(self.input),
            RuleKind::ShotgunSurgery => history::shotgun_surgery(self.input),
            _ => unreachable!("collector domain and history dispatcher must agree"),
        }
    }
}
