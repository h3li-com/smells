use crate::{
    input::Input,
    metrics::{self, Counter},
    policy::{Policy, Registry},
    report::{Location, Report},
};
use proc_macro2::Span;
use quote::ToTokens;
use std::result::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};
use syn::{
    visit::{self, Visit},
    *,
};

mod extract;
mod graph;

#[derive(Clone)]
pub struct Module {
    pub key: String,
    pub root: String,
    pub path: String,
    pub items: Vec<Item>,
    pub imports: BTreeMap<String, Vec<String>>,
}

pub struct TypeFacts {
    pub location: Location,
    pub fields: Vec<(String, Type)>,
    pub field_count: usize,
    pub is_enum: bool,
    pub variants: BTreeSet<String>,
    pub functions: Vec<usize>,
}

pub struct Function {
    pub symbol: String,
    pub module: String,
    pub location: Location,
    pub signature: Signature,
    pub body: Option<Block>,
    pub body_bytes: Option<(usize, usize)>,
    pub owner: Option<String>,
    pub lines: usize,
    pub comments: usize,
}

pub struct Slots {
    pub symbol: String,
    pub location: Location,
    pub types: Vec<Type>,
    pub named: Vec<(String, String)>,
}

pub struct Facts {
    pub modules: BTreeMap<String, Module>,
    pub types: BTreeMap<String, TypeFacts>,
    pub functions: Vec<Function>,
    pub slots: Vec<Slots>,
    pub aliases: BTreeMap<String, (String, Type)>,
}

pub fn location(path: &str, span: Span) -> Location {
    Location {
        path: path.into(),
        line: span.start().line,
        column: span.start().column + 1,
    }
}

pub fn syntax<T: ToTokens>(value: &T) -> String {
    value.to_token_stream().to_string()
}

fn validate_required_rules(registry: &Registry, input: &Input, report: &mut Report) {
    for rule in &registry.rules {
        if rule.implementation == "not_implemented" && input.policy.required(&rule.id) {
            report.error(format!("required detector not implemented: {}", rule.id));
        }
    }
}

pub fn check(input: &Input, registry: &Registry) -> Report {
    let mut report = Report::new(
        registry,
        &input.policy,
        input.mode,
        &input.implementations,
        &input.history,
    );
    report.begin_scan(input);
    validate_required_rules(registry, input, &mut report);
    let parsed = graph::parse_sources(input, &mut report);
    if parsed.len() != input.files.len() {
        return report;
    }
    let referenced = graph::referenced_sources(&parsed, &mut report);
    if report.has_errors() {
        return report;
    }
    let roots = graph::root_sources(&parsed, &referenced);
    if roots.is_empty() {
        report.error("cyclic source module graph has no root");
        return report;
    }
    let mut facts = Facts {
        modules: BTreeMap::new(),
        types: BTreeMap::new(),
        functions: vec![],
        slots: vec![],
        aliases: BTreeMap::new(),
    };
    facts.modules = graph::collect_modules(roots, &parsed, &mut report);
    graph::validate_module_coverage(&facts.modules, &parsed, &mut report);
    if report.has_errors() {
        return report;
    }
    if let Err(error) = extract::declarations(&mut facts, &input.policy, &mut report) {
        report.error(error);
        return report;
    }
    extract::functions(&mut facts, input, &mut report);
    metrics::add(Counter::Functions, facts.functions.len());
    extract::record_type_metrics(&facts, &input.policy, &mut report);
    crate::patterns::check(&facts, &input.policy, &mut report);
    report
}
