use crate::{
    core::model::Input,
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
    Attribute, Block, Expr, ExprLit, Fields, FnArg, ImplItem, Item, ItemEnum, ItemFn, ItemImpl,
    ItemMod, ItemStruct, ItemTrait, ItemType, Lit, Meta, Pat, Signature, TraitItem, Type, UseGroup,
    UseName, UseRename, UseTree,
    visit::{self, Visit},
};

mod extract;
mod graph;

#[derive(Clone)]
pub(crate) struct Module {
    pub key: String,
    pub root: String,
    pub path: String,
    pub items: Vec<Item>,
    pub imports: BTreeMap<String, Vec<String>>,
    /// Compiled only for tests: a test target root or a `#[cfg(test)]` module chain.
    pub test: bool,
    /// This module's key at the first load of its file, when the file or an
    /// enclosing file was already loaded at another module path, as when two
    /// crates share a source file. The module still resolves names, but only
    /// the first load contributes declarations and functions.
    pub repeated: Option<String>,
}

pub(crate) struct TypeFacts {
    pub location: Location,
    pub fields: Vec<(String, Type)>,
    pub field_count: usize,
    pub is_enum: bool,
    pub variants: BTreeSet<String>,
    pub functions: Vec<usize>,
}

pub(crate) struct Function {
    pub symbol: String,
    pub module: String,
    pub location: Location,
    pub signature: Signature,
    pub body: Option<Block>,
    pub body_bytes: Option<(usize, usize)>,
    pub owner: Option<String>,
    pub lines: usize,
    pub comments: usize,
    /// Declared in a test module.
    pub test: bool,
}

pub(crate) struct Slots {
    pub symbol: String,
    pub location: Location,
    pub types: Vec<Type>,
    pub named: Vec<(String, String)>,
    /// The struct whose fields these are.
    pub declares: Option<String>,
    /// The type a function's slots build, from its return type: `Self` names the
    /// owner, and `Option` or `Result` names the type they wrap.
    pub returns: Option<String>,
}

pub(crate) struct Facts {
    pub modules: BTreeMap<String, Module>,
    pub types: BTreeMap<String, TypeFacts>,
    pub functions: Vec<Function>,
    pub slots: Vec<Slots>,
    pub aliases: BTreeMap<String, (String, Type)>,
    /// Each module path that conditional compilation declares more than once,
    /// mapped to the keys of its later configuration variants in source order.
    pub variants: BTreeMap<String, Vec<String>>,
}

/// Marks the key of a module declared again under another `cfg`, such as
/// `imp [cfg variant 2]` for the Windows twin of a Unix `imp` module.
const VARIANT_MARKER: &str = " [cfg variant ";

/// The key of the `ordinal`-th configuration variant of module `key`.
fn variant_key(key: &str, ordinal: usize) -> String {
    format!("{key}{VARIANT_MARKER}{ordinal}]")
}

/// `symbol` with every configuration variant marker removed, so the same item
/// declared in two variants compares equal.
pub(crate) fn without_variants(symbol: &str) -> String {
    let mut result = String::with_capacity(symbol.len());
    let mut rest = symbol;
    while let Some(start) = rest.find(VARIANT_MARKER) {
        result.push_str(&rest[..start]);
        rest = rest[start..].split_once(']').map_or("", |(_, tail)| tail);
    }
    result.push_str(rest);
    result
}

pub(crate) fn location(path: &str, span: Span) -> Location {
    Location {
        path: path.into(),
        line: span.start().line,
        column: span.start().column + 1,
    }
}

/// `parse error in <path>:<line>:<column>: <reason>`, shared by every Rust
/// scanner seam so one broken file reports one message.
pub(crate) fn parse_error_message(path: &str, error: &syn::Error) -> String {
    let at = location(path, error.span());
    format!("parse error in {path}:{}:{}: {error}", at.line, at.column)
}

pub(crate) fn syntax<T: ToTokens>(value: &T) -> String {
    value.to_token_stream().to_string()
}

pub(crate) fn check(input: &Input, registry: &Registry) -> Report {
    let mut report = Report::new(
        registry,
        &input.policy,
        input.mode,
        &input.implementations,
        &input.history,
    );
    report.begin_scan(input);
    for rule_id in crate::policy::required_unimplemented_rule_ids(registry, &input.policy) {
        report.error(format!("required detector not implemented: {rule_id}"));
    }
    let parsed = graph::parse_sources(input, &mut report);
    if parsed.len() != input.files.len() {
        return report;
    }
    let sources = graph::Sources {
        parsed: &parsed,
        directory_links: &input.directory_links,
    };
    let referenced = graph::referenced_sources(sources, &mut report);
    if report.has_errors() {
        return report;
    }
    let roots = graph::root_sources(&parsed, &referenced);
    if roots.is_empty() {
        report.error("cyclic source module graph has no root");
        return report;
    }
    let mut facts = Facts {
        variants: BTreeMap::new(),
        modules: BTreeMap::new(),
        types: BTreeMap::new(),
        functions: vec![],
        slots: vec![],
        aliases: BTreeMap::new(),
    };
    (facts.modules, facts.variants) = graph::collect_modules(roots, sources, &mut report);
    if let Err(error) = graph::validate_module_coverage(&facts.modules, &parsed) {
        report.error(error);
    }
    if report.has_errors() {
        return report;
    }
    if let Err(error) = extract::declarations(&mut facts, &input.policy, &mut report) {
        report.error(error);
        return report;
    }
    extract::functions(&mut facts, input, &mut report);
    metrics::add(Counter::Functions, facts.functions.len());
    extract::record_type_metrics(&facts, input, &mut report);
    crate::patterns::check(&facts, &input.policy, &mut report);
    report
}

#[cfg(test)]
mod tests {
    use super::{variant_key, without_variants};

    #[test]
    fn variant_markers_strip_back_to_the_canonical_symbol() {
        let module = variant_key("crate::imp", 2);
        assert_eq!(module, "crate::imp [cfg variant 2]");
        let nested = format!("{}::Driver::run", variant_key(&format!("{module}::io"), 13));
        assert_eq!(without_variants(&nested), "crate::imp::io::Driver::run");
        assert_eq!(without_variants("crate::plain"), "crate::plain");
        assert_eq!(without_variants("crate [cfg variant 2"), "crate");
    }
}
