use super::{
    common::simple_names,
    syntax::{code_only, code_only_from_tree},
};
use crate::{core::model::Input, policy::Rule, report::Location};
use quote::ToTokens;
use rayon::prelude::*;
use regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, OnceLock},
};
use syn::{ImplItem, TraitItem, spanned::Spanned, visit::Visit};
use tree_sitter::{Language, Node, Parser, Tree};

/// Whether a declaration is production code or test code: a conventional test path or
/// a `#[cfg(test)]` module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum CodeScope {
    #[default]
    Production,
    Test,
}

impl CodeScope {
    pub(super) fn of(test: bool) -> Self {
        if test { Self::Test } else { Self::Production }
    }
}

#[derive(Clone)]
pub(super) struct FunctionFact {
    pub(super) name: String,
    pub(super) symbol: String,
    pub(super) location: Location,
    /// First line holding only body code: line coverage starts here, so a declaration
    /// line that runs at import or definition time never credits an uncalled body.
    pub(super) body_line: usize,
    /// Last source line of the declaration, used to map line coverage.
    pub(super) end_line: usize,
    pub(super) type_parameters: Vec<String>,
    pub(super) body: Option<Arc<str>>,
    /// Cyclomatic complexity of the body, counted on the syntax tree.
    pub(super) complexity: Option<u64>,
    pub(super) source: Arc<str>,
    pub(super) private: bool,
    /// Called on an owned receiver (`self`, `cls`, or an instance `this`).
    pub(super) receiver: bool,
    /// Invoked by a harness or runtime rather than by authored code.
    pub(super) entry_point: bool,
    pub(super) scope: CodeScope,
}

#[derive(Clone)]
pub(super) struct FieldFact {
    pub(super) name: String,
    pub(super) authored_type: String,
    pub(super) temporary: bool,
    pub(super) private: bool,
}

#[derive(Clone)]
pub(super) struct ClassFact {
    pub(super) name: String,
    pub(super) symbol: String,
    pub(super) location: Location,
    pub(super) type_parameters: Vec<String>,
    pub(super) source: Arc<str>,
    pub(super) bases: Vec<String>,
    pub(super) interface: bool,
    pub(super) private: bool,
    pub(super) fields: Vec<FieldFact>,
    pub(super) methods: Vec<FunctionFact>,
    pub(super) range: (usize, usize),
    pub(super) scope: CodeScope,
}

#[derive(Default)]
pub(super) struct SourceModel {
    pub(super) functions: Vec<FunctionFact>,
    pub(super) classes: Vec<ClassFact>,
    pub(super) code_by_path: BTreeMap<String, Arc<str>>,
    /// Names referenced only through attribute string paths, such as
    /// `#[serde(default = "defaults")]`, which code masking hides.
    pub(super) attribute_references: BTreeSet<String>,
}

fn annotated_slot() -> &'static Regex {
    static SLOT: OnceLock<Regex> = OnceLock::new();
    SLOT.get_or_init(|| {
        Regex::new(
            r"(?m)^\s*(?:(?:public|private|protected|readonly|static|declare)\s+)*[A-Za-z_][A-Za-z0-9_]*[?!]?\s*:\s*([^=;\n]+)",
        )
        .expect("valid built-in annotated-slot expression")
    })
}

fn typescript_slot() -> &'static Regex {
    static SLOT: OnceLock<Regex> = OnceLock::new();
    SLOT.get_or_init(|| {
        Regex::new(
            r"(?m)^\s*(?:(?:public|private|protected|readonly|static|declare)\s+)*([A-Za-z_][A-Za-z0-9_]*)[?!]?\s*:\s*([^=;{}\n]+)",
        )
        .expect("valid TypeScript class-slot expression")
    })
}

fn parser_language(language: &str, path: &str) -> Result<Language, String> {
    match language {
        "python" => Ok(tree_sitter_python::LANGUAGE.into()),
        "typescript"
            if Path::new(path)
                .extension()
                .is_some_and(|extension| extension == "tsx") =>
        {
            Ok(tree_sitter_typescript::LANGUAGE_TSX.into())
        }
        "typescript" => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        _ => Err(format!("no portable source model for {language}")),
    }
}

pub(super) fn class_node(kind: &str, language: &str) -> bool {
    match language {
        "python" => kind == "class_definition",
        "typescript" => matches!(
            kind,
            "class" | "class_declaration" | "abstract_class_declaration" | "interface_declaration"
        ),
        _ => false,
    }
}

pub(super) fn function_node(kind: &str, language: &str) -> bool {
    match language {
        "python" => matches!(kind, "function_definition" | "lambda"),
        "typescript" => matches!(
            kind,
            "function_declaration"
                | "function_expression"
                | "generator_function_declaration"
                | "generator_function"
                | "arrow_function"
                | "method_definition"
                | "method_signature"
                | "abstract_method_signature"
        ),
        _ => false,
    }
}

fn direct_class_owner<'tree>(node: Node<'tree>, language: &str) -> Option<Node<'tree>> {
    let parent = node.parent()?;
    match language {
        "python" => {
            let body = if parent.kind() == "decorated_definition" {
                parent.parent()?
            } else {
                parent
            };
            (body.kind() == "block")
                .then(|| body.parent())
                .flatten()
                .filter(|owner| class_node(owner.kind(), language))
        }
        "typescript" => parent
            .parent()
            .filter(|owner| class_node(owner.kind(), language)),
        _ => None,
    }
}

fn collect_model_nodes<'tree>(
    node: Node<'tree>,
    language: &str,
    classes: &mut Vec<Node<'tree>>,
    functions: &mut Vec<Node<'tree>>,
) {
    if class_node(node.kind(), language) {
        classes.push(node);
    }
    if function_node(node.kind(), language) {
        functions.push(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_model_nodes(child, language, classes, functions);
    }
}

fn text_of(node: Node<'_>, source: &str) -> String {
    node.utf8_text(source.as_bytes()).unwrap_or_default().into()
}

fn node_location(path: &str, node: Node<'_>) -> Location {
    Location {
        path: path.into(),
        line: node.start_position().row + 1,
        column: node.start_position().column + 1,
    }
}

fn node_name(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("name").map_or_else(
        || format!("anonymous@{}", node.start_position().row + 1),
        |name| text_of(name, source),
    )
}

fn header_before_body(node: Node<'_>, source: &str) -> String {
    let end = node
        .child_by_field_name("body")
        .map_or(node.end_byte(), |body| body.start_byte());
    source[node.start_byte()..end].trim().into()
}

fn class_bases(header: &str, language: &str, name: &str) -> Vec<String> {
    let tail = header
        .find(name)
        .map_or(header, |offset| &header[offset + name.len()..]);
    if language == "python" {
        return tail
            .strip_prefix('(')
            .and_then(|value| value.split_once(')'))
            .map_or_else(Vec::new, |(bases, _)| {
                bases
                    .split(',')
                    .filter_map(|base| simple_names(base).last().cloned())
                    .collect()
            });
    }
    let mut bases = Vec::new();
    for marker in ["extends", "implements"] {
        if let Some((_, remainder)) = tail.split_once(marker) {
            let clause = remainder
                .split(['{', '\n'])
                .next()
                .unwrap_or_default()
                .split("implements")
                .next()
                .unwrap_or_default();
            bases.extend(
                clause
                    .split(',')
                    .filter_map(|base| simple_names(base).last().cloned()),
            );
        }
    }
    bases.sort();
    bases.dedup();
    bases
}

fn authored_slot_temporary(whole: &str, authored_type: &str) -> bool {
    whole.contains('?')
        || authored_type.contains("None")
        || authored_type.contains("Optional")
        || authored_type.contains("null")
        || authored_type.contains("undefined")
}

fn authored_slot_private(name: &str, whole: &str) -> bool {
    name.starts_with('_')
        || whole.split_whitespace().any(|part| part == "private")
        || whole.contains('#')
}

fn class_field_declarations(body: Node<'_>, source: &str, language: &str) -> String {
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .filter(|child| match language {
            "python" => child.kind() == "expression_statement",
            "typescript" => matches!(
                child.kind(),
                "public_field_definition" | "property_signature" | "abstract_property_signature"
            ),
            _ => false,
        })
        .map(|child| text_of(child, source))
        .collect::<Vec<_>>()
        .join("\n")
}

fn authored_field(slot: &regex::Captures<'_>, language: &str) -> Option<FieldFact> {
    let whole = slot.get(0).expect("annotated slot match").as_str();
    let (name, authored_type) = if language == "typescript" {
        (slot[1].to_string(), slot[2].trim().to_string())
    } else {
        let name = simple_names(whole.split(':').next().unwrap_or_default())
            .last()
            .cloned()?;
        (name, slot[1].trim().to_string())
    };
    Some(FieldFact {
        temporary: authored_slot_temporary(whole, &authored_type),
        private: authored_slot_private(&name, whole),
        name,
        authored_type,
    })
}

fn declared_fields(body: Node<'_>, source: &str, language: &str) -> BTreeMap<String, FieldFact> {
    let declarations = class_field_declarations(body, source, language);
    let slot = if language == "typescript" {
        typescript_slot()
    } else {
        annotated_slot()
    };
    slot.captures_iter(&declarations)
        .filter_map(|capture| authored_field(&capture, language))
        .map(|field| (field.name.clone(), field))
        .collect()
}

fn typescript_parameter_property(parameter: Node<'_>, source: &str) -> Option<FieldFact> {
    let mut child_cursor = parameter.walk();
    let property = parameter.children(&mut child_cursor).any(|child| {
        matches!(
            child.kind(),
            "accessibility_modifier" | "readonly" | "override_modifier"
        )
    });
    if !property {
        return None;
    }
    let name_node = parameter
        .child_by_field_name("name")
        .or_else(|| parameter.child_by_field_name("pattern"))?;
    let name = text_of(name_node, source);
    let authored_type = parameter
        .child_by_field_name("type")
        .map(|ty| text_of(ty, source))
        .unwrap_or_default()
        .trim_start_matches(':')
        .trim()
        .to_string();
    let parameter_source = text_of(parameter, source);
    Some(FieldFact {
        temporary: parameter_source.contains('?')
            || authored_type.contains("null")
            || authored_type.contains("undefined"),
        private: name.starts_with('_')
            || parameter_source
                .split_whitespace()
                .any(|part| part == "private"),
        name,
        authored_type,
    })
}

fn typescript_parameter_fields(body: Node<'_>, source: &str) -> Vec<FieldFact> {
    let mut fields = Vec::new();
    let mut cursor = body.walk();
    for method in body.named_children(&mut cursor).filter(|child| {
        child.kind() == "method_definition" && node_name(*child, source) == "constructor"
    }) {
        let Some(parameters) = method.child_by_field_name("parameters") else {
            continue;
        };
        let mut parameter_cursor = parameters.walk();
        fields.extend(
            parameters
                .named_children(&mut parameter_cursor)
                .filter_map(|parameter| typescript_parameter_property(parameter, source)),
        );
    }
    fields
}

fn nullable_assignment_fields(
    node: Node<'_>,
    authored_code: &str,
    language: &str,
) -> Vec<FieldFact> {
    let receiver = if language == "python" { "self" } else { "this" };
    let assignment = Regex::new(&format!(
        r"\b{receiver}\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(None|null|undefined)\b"
    ))
    .expect("valid receiver assignment expression");
    let class_source = text_of(node, authored_code);
    assignment
        .captures_iter(&class_source)
        .map(|field| {
            let name = field[1].to_string();
            FieldFact {
                private: name.starts_with('_'),
                name,
                authored_type: field[2].to_string(),
                temporary: true,
            }
        })
        .collect()
}

fn class_fields(
    node: Node<'_>,
    source: &str,
    authored_code: &str,
    language: &str,
) -> Vec<FieldFact> {
    let body = node.child_by_field_name("body").unwrap_or(node);
    let mut fields = declared_fields(body, source, language);
    if language == "typescript" {
        fields.extend(
            typescript_parameter_fields(body, source)
                .into_iter()
                .map(|field| (field.name.clone(), field)),
        );
    }
    for field in nullable_assignment_fields(node, authored_code, language) {
        fields.entry(field.name.clone()).or_insert(field);
    }
    fields.into_values().collect()
}

fn rust_location(path: &str, span: proc_macro2::Span) -> Location {
    Location {
        path: path.into(),
        line: span.start().line,
        column: span.start().column + 1,
    }
}

fn rust_type_name(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn rust_owner(module_path: &str, name: &str) -> String {
    if module_path.is_empty() {
        name.to_string()
    } else {
        format!("{module_path}::{name}")
    }
}

fn rust_class_key(path: &str, module_path: &str, name: &str) -> String {
    format!("{path}::{}", rust_owner(module_path, name))
}

/// The line after a multi-line brace body's `{`, so the header row (`fn f() {`) never
/// counts as body coverage; a one-line body keeps its only line. Code written on the
/// `{` line itself is therefore not measured: that blind spot is cheaper than letting
/// an executed declaration line credit an uncalled body.
fn brace_body_line(open: usize, close: usize) -> usize {
    if close > open { open + 1 } else { open }
}

fn rust_function(
    path: &str,
    owner: Option<&str>,
    signature: &syn::Signature,
    body: Option<&syn::Block>,
    private: bool,
) -> FunctionFact {
    let name = signature.ident.to_string();
    let signature_source = signature.to_token_stream().to_string();
    let body_source = body
        .map(ToTokens::to_token_stream)
        .map(|tokens| tokens.to_string());
    let raw_source = format!(
        "{signature_source} {}",
        body_source.as_deref().unwrap_or_default()
    );
    FunctionFact {
        symbol: owner.map_or_else(
            || format!("{path}::{name}"),
            |owner| format!("{path}::{owner}.{name}"),
        ),
        location: rust_location(path, signature.ident.span()),
        body_line: body.map_or_else(
            || signature.ident.span().start().line,
            |body| {
                let span = body.brace_token.span;
                brace_body_line(span.open().start().line, span.close().end().line)
            },
        ),
        end_line: body.map_or_else(
            || signature.ident.span().end().line,
            |body| body.brace_token.span.close().end().line,
        ),
        source: code_only(path, &raw_source)
            .expect("Rust token masking cannot fail")
            .into(),
        type_parameters: rust_type_parameters(&signature.generics),
        complexity: body.map(super::complexity::rust),
        body: body_source.map(|body| {
            Arc::from(code_only(path, &body).expect("Rust function-body token masking cannot fail"))
        }),
        private,
        receiver: signature.receiver().is_some(),
        entry_point: false,
        scope: CodeScope::Production,
        name,
    }
}

/// Whether `#[allow]` or `#[expect]` names `dead_code` or `unused`: the author has
/// already declared the item intentionally unreferenced.
fn rust_allows_unused(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        let syn::Meta::List(list) = &attribute.meta else {
            return false;
        };
        (list.path.is_ident("allow") || list.path.is_ident("expect"))
            && list.tokens.clone().into_iter().any(|token| {
                matches!(token, proc_macro2::TokenTree::Ident(ident) if ident == "dead_code" || ident == "unused")
            })
    })
}

/// Private to the unused-code check: an inherited visibility, no `_` prefix (which
/// rustc treats as intentionally unused), and no `dead_code` allowance.
fn rust_private(
    visibility: &syn::Visibility,
    name: &syn::Ident,
    attributes: &[syn::Attribute],
) -> bool {
    matches!(visibility, syn::Visibility::Inherited)
        && !name.to_string().starts_with('_')
        && !rust_allows_unused(attributes)
}

/// Test and benchmark functions run under the harness, and a crate-level
/// `main` runs as the binary entry point.
fn rust_entry_point(function: &syn::ItemFn, module_path: &[String]) -> bool {
    (module_path.is_empty() && function.sig.ident == "main")
        || function.attrs.iter().any(|attribute| {
            attribute.path().segments.last().is_some_and(|segment| {
                matches!(segment.ident.to_string().as_str(), "test" | "bench")
            })
        })
}

#[derive(Default)]
struct AttributeReferences(BTreeSet<String>);

impl AttributeReferences {
    fn collect(&mut self, tokens: proc_macro2::TokenStream) {
        for token in tokens {
            match token {
                proc_macro2::TokenTree::Group(group) => self.collect(group.stream()),
                proc_macro2::TokenTree::Literal(literal) => {
                    let Ok(literal) = syn::parse_str::<syn::LitStr>(&literal.to_string()) else {
                        continue;
                    };
                    if let Ok(path) = syn::parse_str::<syn::Path>(&literal.value())
                        && let Some(segment) = path.segments.last()
                    {
                        self.0.insert(segment.ident.to_string());
                    }
                }
                proc_macro2::TokenTree::Ident(_) | proc_macro2::TokenTree::Punct(_) => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for AttributeReferences {
    fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
        if !attribute.path().is_ident("doc") {
            self.collect(attribute.meta.to_token_stream());
        }
    }
}

fn ensure_rust_class<'a>(
    classes: &'a mut BTreeMap<String, ClassFact>,
    path: &str,
    module_path: &str,
    name: &str,
    location: Location,
) -> &'a mut ClassFact {
    let key = rust_class_key(path, module_path, name);
    classes.entry(key.clone()).or_insert_with(|| ClassFact {
        name: name.into(),
        symbol: key,
        location,
        type_parameters: Vec::new(),
        source: Arc::from(""),
        bases: Vec::new(),
        interface: false,
        private: false,
        fields: Vec::new(),
        methods: Vec::new(),
        range: (0, 0),
        scope: CodeScope::Production,
    })
}

fn rust_field(index: usize, field: &syn::Field) -> FieldFact {
    let authored_type = field.ty.to_token_stream().to_string();
    FieldFact {
        name: field
            .ident
            .as_ref()
            .map_or_else(|| index.to_string(), ToString::to_string),
        authored_type: authored_type.replace(' ', ""),
        temporary: authored_type.split_whitespace().next() == Some("Option"),
        private: matches!(field.vis, syn::Visibility::Inherited),
    }
}

fn rust_type_parameters(generics: &syn::Generics) -> Vec<String> {
    generics
        .params
        .iter()
        .filter_map(|parameter| match parameter {
            syn::GenericParam::Type(parameter) => Some(parameter.ident.to_string()),
            syn::GenericParam::Lifetime(_) | syn::GenericParam::Const(_) => None,
        })
        .collect()
}

struct RustModelVisitor<'model> {
    path: String,
    module_path: Vec<String>,
    /// Inside a conventional test path or a `#[cfg(test)]` module.
    test: bool,
    model: &'model mut SourceModel,
    classes: &'model mut BTreeMap<String, ClassFact>,
}

impl RustModelVisitor<'_> {
    fn module(&self) -> String {
        self.module_path.join("::")
    }

    fn record_nested_functions(&mut self, parent: &str, block: &syn::Block) {
        let mut nested = NestedRustFunctions::default();
        nested.visit_block(block);
        for function in nested.items {
            let symbol = format!(
                "{parent}::{}@{}:{}",
                function.sig.ident,
                function.sig.ident.span().start().line,
                function.sig.ident.span().start().column
            );
            let mut fact =
                rust_function(&self.path, None, &function.sig, Some(&function.block), true);
            fact.symbol.clone_from(&symbol);
            fact.scope = CodeScope::of(self.test);
            self.model.functions.push(fact);
            self.record_nested_functions(&symbol, &function.block);
        }
    }
}

#[derive(Default)]
struct NestedRustFunctions<'ast> {
    items: Vec<&'ast syn::ItemFn>,
}

impl<'ast> Visit<'ast> for NestedRustFunctions<'ast> {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        self.items.push(function);
    }

    fn visit_item_const(&mut self, _constant: &'ast syn::ItemConst) {}

    fn visit_item_static(&mut self, _static_item: &'ast syn::ItemStatic) {}

    fn visit_item_struct(&mut self, _structure: &'ast syn::ItemStruct) {}

    fn visit_item_enum(&mut self, _enumeration: &'ast syn::ItemEnum) {}

    fn visit_item_trait(&mut self, _trait_item: &'ast syn::ItemTrait) {}

    fn visit_item_impl(&mut self, _implementation: &'ast syn::ItemImpl) {}

    fn visit_item_mod(&mut self, _module: &'ast syn::ItemMod) {}
}

impl<'ast> Visit<'ast> for RustModelVisitor<'_> {
    fn visit_item_const(&mut self, _constant: &'ast syn::ItemConst) {}

    fn visit_item_static(&mut self, _static_item: &'ast syn::ItemStatic) {}

    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        let mut fact = rust_function(
            &self.path,
            None,
            &function.sig,
            Some(&function.block),
            rust_private(&function.vis, &function.sig.ident, &function.attrs),
        );
        fact.entry_point = rust_entry_point(function, &self.module_path);
        fact.scope =
            CodeScope::of(self.test || crate::core::test_suite::rust_cfg_test(&function.attrs));
        let module = self.module();
        if !module.is_empty() {
            fact.symbol = format!("{}::{module}::{}", self.path, function.sig.ident);
        }
        let symbol = fact.symbol.clone();
        self.model.functions.push(fact);
        self.record_nested_functions(&symbol, &function.block);
    }

    fn visit_item_struct(&mut self, structure: &'ast syn::ItemStruct) {
        let name = structure.ident.to_string();
        let module = self.module();
        let class = ensure_rust_class(
            self.classes,
            &self.path,
            &module,
            &name,
            rust_location(&self.path, structure.ident.span()),
        );
        class.source = code_only(&self.path, &structure.to_token_stream().to_string())
            .expect("Rust type token masking cannot fail")
            .into();
        class.type_parameters = rust_type_parameters(&structure.generics);
        class.private = rust_private(&structure.vis, &structure.ident, &structure.attrs);
        class.scope = CodeScope::of(self.test);
        class.fields = structure
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| rust_field(index, field))
            .collect();
    }

    fn visit_item_enum(&mut self, enumeration: &'ast syn::ItemEnum) {
        let name = enumeration.ident.to_string();
        let module = self.module();
        let class = ensure_rust_class(
            self.classes,
            &self.path,
            &module,
            &name,
            rust_location(&self.path, enumeration.ident.span()),
        );
        class.source = code_only(&self.path, &enumeration.to_token_stream().to_string())
            .expect("Rust type token masking cannot fail")
            .into();
        class.type_parameters = rust_type_parameters(&enumeration.generics);
        class.private = rust_private(&enumeration.vis, &enumeration.ident, &enumeration.attrs);
        class.scope = CodeScope::of(self.test);
    }

    fn visit_item_trait(&mut self, trait_item: &'ast syn::ItemTrait) {
        let name = trait_item.ident.to_string();
        let module = self.module();
        let owner = rust_owner(&module, &name);
        let functions = trait_item
            .items
            .iter()
            .filter_map(|method| match method {
                TraitItem::Fn(method) => {
                    let mut fact = rust_function(
                        &self.path,
                        Some(&owner),
                        &method.sig,
                        method.default.as_ref(),
                        false,
                    );
                    fact.scope = CodeScope::of(self.test);
                    Some(fact)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let class = ensure_rust_class(
            self.classes,
            &self.path,
            &module,
            &name,
            rust_location(&self.path, trait_item.ident.span()),
        );
        class.interface = true;
        class.source = code_only(&self.path, &trait_item.to_token_stream().to_string())
            .expect("Rust trait token masking cannot fail")
            .into();
        class.type_parameters = rust_type_parameters(&trait_item.generics);
        class.methods.extend(functions.iter().cloned());
        self.model.functions.extend(functions);
        for method in &trait_item.items {
            if let TraitItem::Fn(method) = method
                && let Some(block) = &method.default
            {
                let parent =
                    rust_function(&self.path, Some(&owner), &method.sig, Some(block), false).symbol;
                self.record_nested_functions(&parent, block);
            }
        }
    }

    fn visit_item_impl(&mut self, implementation: &'ast syn::ItemImpl) {
        let Some(name) = rust_type_name(&implementation.self_ty) else {
            return;
        };
        let module = self.module();
        let owner = rust_owner(&module, &name);
        let inherent =
            implementation.trait_.is_none() && !rust_allows_unused(&implementation.attrs);
        let test = self.test || crate::core::test_suite::rust_cfg_test(&implementation.attrs);
        let functions = implementation
            .items
            .iter()
            .filter_map(|method| match method {
                ImplItem::Fn(method) => {
                    let mut fact = rust_function(
                        &self.path,
                        Some(&owner),
                        &method.sig,
                        Some(&method.block),
                        inherent && rust_private(&method.vis, &method.sig.ident, &method.attrs),
                    );
                    fact.scope = CodeScope::of(test);
                    Some(fact)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let base = implementation.trait_.as_ref().map(|(_, trait_path, _)| {
            trait_path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>()
                .join("::")
        });
        let class = ensure_rust_class(
            self.classes,
            &self.path,
            &module,
            &name,
            rust_location(&self.path, implementation.self_ty.span()),
        );
        class.bases.extend(base);
        class.methods.extend(functions.iter().cloned());
        self.model.functions.extend(functions);
        for method in &implementation.items {
            if let ImplItem::Fn(method) = method {
                let parent = format!("{}::{}.{}", self.path, owner, method.sig.ident);
                self.record_nested_functions(&parent, &method.block);
            }
        }
    }

    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let Some((_, items)) = &module.content else {
            return;
        };
        let enclosing_test = self.test;
        self.test |= crate::core::test_suite::rust_cfg_test(&module.attrs);
        self.module_path.push(module.ident.to_string());
        for item in items {
            self.visit_item(item);
        }
        self.module_path.pop();
        self.test = enclosing_test;
    }
}

fn rust_source_model(input: &Input) -> Result<SourceModel, String> {
    let mut model = SourceModel::default();
    let mut classes = BTreeMap::new();
    for (path, source) in &input.files {
        model
            .code_by_path
            .insert(path.clone(), code_only(path, source)?.into());
        let file = syn::parse_file(source)
            .map_err(|error| crate::scan::parse_error_message(path, &error))?;
        let mut references = AttributeReferences::default();
        references.visit_file(&file);
        model.attribute_references.extend(references.0);
        RustModelVisitor {
            path: path.clone(),
            module_path: Vec::new(),
            test: crate::core::test_suite::conventional_test_path(path),
            model: &mut model,
            classes: &mut classes,
        }
        .visit_file(&file);
    }
    for class in classes.values_mut() {
        class.bases.sort();
        class.bases.dedup();
    }
    model.classes = classes.into_values().collect();
    Ok(model)
}

pub(super) fn parse_portable_tree(
    language: &str,
    path: &str,
    source: &str,
) -> Result<Tree, String> {
    let grammar = parser_language(language, path)?;
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|error| format!("cannot load collector grammar for {path}: {error}"))?;
    let tree = if language == "typescript" {
        crate::typescript_compat::parse(&mut parser, source, path)?
    } else {
        parser
            .parse(source, None)
            .ok_or_else(|| format!("collector parser cancelled for {path}"))?
    };
    if crate::typescript_compat::has_syntax_error(&tree) {
        return Err(crate::typescript_compat::parse_error_message(path, &tree));
    }
    Ok(tree)
}

fn portable_type_parameters(node: Node<'_>, source: &str, language: &str) -> Vec<String> {
    let Some(parameters) = node.child_by_field_name("type_parameters") else {
        return Vec::new();
    };
    let mut cursor = parameters.walk();
    parameters
        .named_children(&mut cursor)
        .filter_map(|parameter| {
            if language == "typescript" {
                parameter
                    .child_by_field_name("name")
                    .map(|name| text_of(name, source))
            } else {
                simple_names(&text_of(parameter, source)).first().cloned()
            }
        })
        .collect()
}

fn portable_class(
    node: Node<'_>,
    source: &str,
    authored_code: &str,
    path: &str,
    language: &str,
) -> ClassFact {
    let name = node_name(node, source);
    let header = header_before_body(node, source);
    ClassFact {
        symbol: format!("{path}::{name}"),
        location: node_location(path, node),
        type_parameters: portable_type_parameters(node, source, language),
        source: text_of(node, authored_code).into(),
        bases: class_bases(&header, language, &name),
        interface: node.kind() == "interface_declaration"
            || header.contains("Protocol")
            || header.contains("ABC"),
        private: portable_private(node, &name, ""),
        fields: class_fields(node, source, authored_code, language),
        methods: Vec::new(),
        scope: CodeScope::of(crate::core::test_suite::conventional_test_path(path)),
        name,
        range: (node.start_byte(), node.end_byte()),
    }
}

fn portable_receiver(signature: &str, language: &str) -> bool {
    static PYTHON_RECEIVER: OnceLock<Regex> = OnceLock::new();
    if language == "python" {
        PYTHON_RECEIVER
            .get_or_init(|| {
                Regex::new(r"\bdef\s+[A-Za-z_][A-Za-z0-9_]*\s*\(\s*(?:self|cls)\b")
                    .expect("valid Python receiver expression")
            })
            .is_match(signature)
    } else {
        !signature.split_whitespace().any(|word| word == "static")
    }
}

/// Private to the unused-code check: a `_` name or `private` modifier, unless the
/// declaration is exported (`export function _union`) or is a single-underscore
/// protocol hook such as `_repr_markdown_` or `_missing_`, which a framework calls.
fn portable_private(node: Node<'_>, name: &str, signature: &str) -> bool {
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor
        .filter(|parent| matches!(parent.kind(), "variable_declarator" | "lexical_declaration"))
    {
        ancestor = parent.parent();
    }
    let exported = ancestor.is_some_and(|parent| parent.kind() == "export_statement");
    let hook =
        name != "_" && name.starts_with('_') && name.ends_with('_') && !name.starts_with("__");
    !exported && !hook && (name.starts_with('_') || signature.contains("private "))
}

fn portable_function(
    node: Node<'_>,
    source: &str,
    authored_code: &str,
    path: &str,
    language: &str,
    classes: &[ClassFact],
) -> (FunctionFact, Option<usize>) {
    let name = node_name(node, source);
    let owner_index = direct_class_owner(node, language).and_then(|owner| {
        let range = (owner.start_byte(), owner.end_byte());
        classes.iter().position(|class| class.range == range)
    });
    let owner = owner_index.map(|index| classes[index].name.clone());
    let body_node = node.child_by_field_name("body");
    let body = body_node.map(|body| Arc::from(text_of(body, authored_code)));
    let signature = header_before_body(node, source);
    let function = FunctionFact {
        symbol: owner.as_ref().map_or_else(
            || format!("{path}::{name}"),
            |owner| format!("{path}::{owner}.{name}"),
        ),
        location: node_location(path, node),
        body_line: body_node.map_or(node.start_position().row + 1, |body| {
            let (open, close) = (body.start_position().row + 1, body.end_position().row + 1);
            // A Python block starts at its first statement; a brace block at its `{`.
            if source.as_bytes().get(body.start_byte()) == Some(&b'{') {
                brace_body_line(open, close)
            } else {
                open
            }
        }),
        end_line: node.end_position().row + 1,
        type_parameters: portable_type_parameters(node, source, language),
        source: text_of(node, authored_code).into(),
        body,
        complexity: body_node.map(|body| super::complexity::portable(node, body, language)),
        private: portable_private(node, &name, &signature),
        receiver: owner.is_some() && portable_receiver(&signature, language),
        entry_point: false,
        scope: CodeScope::of(crate::core::test_suite::conventional_test_path(path)),
        name,
    };
    (function, owner_index)
}

fn append_portable_file(
    model: &mut SourceModel,
    language: &str,
    path: &str,
    source: &str,
) -> Result<(), String> {
    let tree = parse_portable_tree(language, path, source)?;
    let authored_code = code_only_from_tree(path, source, tree.root_node());
    let mut class_nodes = Vec::new();
    let mut function_nodes = Vec::new();
    collect_model_nodes(
        tree.root_node(),
        language,
        &mut class_nodes,
        &mut function_nodes,
    );
    let mut classes = class_nodes
        .into_iter()
        .map(|node| portable_class(node, source, &authored_code, path, language))
        .collect::<Vec<_>>();
    for node in function_nodes {
        let (function, owner_index) =
            portable_function(node, source, &authored_code, path, language, &classes);
        if let Some(index) = owner_index {
            classes[index].methods.push(function.clone());
        }
        model.functions.push(function);
    }
    model.classes.append(&mut classes);
    model.code_by_path.insert(path.into(), authored_code.into());
    Ok(())
}

fn portable_source_model(language: &str, input: &Input) -> Result<SourceModel, String> {
    let sources = input.files.iter().collect::<Vec<_>>();
    let partials = sources
        .par_iter()
        .map(|(path, source)| {
            let mut model = SourceModel::default();
            append_portable_file(&mut model, language, path, source)?;
            Ok(model)
        })
        .collect::<Vec<Result<SourceModel, String>>>();
    let mut model = SourceModel::default();
    for partial in partials {
        let mut partial = partial?;
        model.functions.append(&mut partial.functions);
        model.classes.append(&mut partial.classes);
        model.code_by_path.append(&mut partial.code_by_path);
    }
    Ok(model)
}

pub(super) fn source_model(rule: &Rule, input: &Input) -> Result<SourceModel, String> {
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    if language == "rust" {
        rust_source_model(input)
    } else {
        portable_source_model(language, input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn input(rule_pack: &str, files: &[(&str, &str)]) -> Input {
        let files = files
            .iter()
            .map(|(path, source)| ((*path).into(), (*source).into()))
            .collect();
        Input::fixture(rule_pack, files)
    }

    #[test]
    fn authored_field_flags_have_independent_temporary_and_private_boundaries() {
        for (whole, authored_type, expected) in [
            ("value?: string", "string", true),
            ("value: Optional[str]", "Optional[str]", true),
            ("value: str | None", "str | None", true),
            ("value: string | null", "string | null", true),
            ("value: string | undefined", "string | undefined", true),
            ("value: string", "string", false),
        ] {
            assert_eq!(authored_slot_temporary(whole, authored_type), expected);
        }
        for (name, whole, expected) in [
            ("_value", "_value: str", true),
            ("value", "private value: string", true),
            ("value", "#value: string", true),
            ("value", "public value: string", false),
        ] {
            assert_eq!(authored_slot_private(name, whole), expected);
        }
    }

    #[test]
    fn python_source_model_preserves_classes_fields_methods_and_locations() {
        let source = concat!(
            "class _Account(Protocol):\n",
            "    pending: Optional[str]\n",
            "    count: int\n",
            "    def __init__(self):\n        self.runtime = None\n",
            "    def load(self):\n        return self.count\n",
            "\n",
            "def top_level():\n    return 1\n",
        );
        let model = source_model(
            &rule("python.temporary_fields"),
            &input("python-v1", &[("account.py", source)]),
        )
        .unwrap();

        assert_eq!(model.classes.len(), 1);
        let account = &model.classes[0];
        assert_eq!(account.name, "_Account");
        assert_eq!(account.location.line, 1);
        assert_eq!(account.location.column, 1);
        assert!(account.interface);
        assert!(account.private);
        assert_eq!(account.methods.len(), 2);
        assert_eq!(
            account
                .fields
                .iter()
                .map(|field| (field.name.as_str(), field.temporary))
                .collect::<BTreeMap<_, _>>(),
            BTreeMap::from([("count", false), ("pending", true), ("runtime", true)])
        );
        assert_eq!(
            model
                .functions
                .iter()
                .map(|function| function.name.as_str())
                .collect::<Vec<_>>(),
            ["__init__", "load", "top_level"]
        );
        assert_eq!(
            model
                .functions
                .iter()
                .map(|function| (
                    function.location.line,
                    function.body_line,
                    function.end_line
                ))
                .collect::<Vec<_>>(),
            [(4, 5, 5), (6, 7, 7), (9, 10, 10)]
        );
    }

    #[test]
    fn coverage_starts_below_the_declaration_line_in_every_language() {
        let lines = |rule_name: &str, pack: &str, path: &str, source: &str| {
            source_model(&rule(rule_name), &input(pack, &[(path, source)]))
                .unwrap()
                .functions
                .iter()
                .map(|function| (function.name.clone(), function.body_line, function.end_line))
                .collect::<Vec<_>>()
        };
        let owned = |rows: &[(&str, usize, usize)]| {
            rows.iter()
                .map(|(name, body, end)| ((*name).to_string(), *body, *end))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            lines(
                "python.temporary_fields",
                "python-v1",
                "a.py",
                "def wide(\n    a,\n):\n    return a\n\ndef one(): return 1\n",
            ),
            owned(&[("wide", 4, 4), ("one", 6, 6)])
        );
        assert_eq!(
            lines(
                "typescript.temporary_fields",
                "typescript-v1",
                "a.ts",
                "function open(a: number) {\n  return a;\n}\nfunction own(a: number)\n{\n  return a;\n}\nfunction one() { return 1; }\nfunction empty() {\n}\nabstract class K {\n  abstract m(): void;\n}\n",
            ),
            owned(&[
                ("open", 2, 3),
                ("own", 6, 7),
                ("one", 8, 8),
                ("empty", 10, 10),
                ("m", 12, 12)
            ])
        );
        assert_eq!(
            lines(
                "rust.temporary_fields",
                "rust-v1",
                "src/lib.rs",
                "fn open(a: u8) -> u8 {\n    a\n}\nfn one() -> u8 { 1 }\n",
            ),
            owned(&[("open", 2, 3), ("one", 4, 4)])
        );
    }

    #[test]
    fn typescript_source_model_uses_tsx_grammar_and_constructor_parameter_properties() {
        let source = concat!(
            "export class ViewModel<T> {\n",
            "  constructor(private id: string, readonly pending?: T) {}\n",
            "  render(): JSX.Element { return <section>{this.id}</section>; }\n",
            "}\n",
            "export const Standalone = () => <aside />;\n",
        );
        let model = source_model(
            &rule("typescript.temporary_fields"),
            &input("typescript-v1", &[("view.tsx", source)]),
        )
        .unwrap();

        let view = &model.classes[0];
        assert_eq!(view.type_parameters, ["T"]);
        assert_eq!(view.methods.len(), 2);
        assert_eq!(
            view.fields
                .iter()
                .map(|field| {
                    (
                        field.name.as_str(),
                        field.authored_type.as_str(),
                        field.temporary,
                        field.private,
                    )
                })
                .collect::<Vec<_>>(),
            [("id", "string", false, true), ("pending", "T", true, false),]
        );
        assert!(
            model
                .functions
                .iter()
                .any(|function| function.name == "anonymous@5")
        );
        assert!(
            model
                .functions
                .iter()
                .all(|function| function.location.line > 0)
        );
        assert!(
            model
                .functions
                .iter()
                .all(|function| function.location.column > 0)
        );
    }

    #[test]
    fn typescript_parser_uses_ts_grammar_for_angle_bracket_assertions() {
        let source = "const value = <string>input;\n";
        assert!(parse_portable_tree("typescript", "value.ts", source).is_ok());
        assert!(parse_portable_tree("typescript", "value.tsx", source).is_err());
        assert!(parse_portable_tree("python", "value.py", "value: int = 1\n").is_ok());
        assert!(parser_language("unknown", "value.ts").is_err());
    }

    #[test]
    fn typescript_parameter_properties_keep_temporary_type_signals_independent() {
        let source = concat!(
            "class Model {\n",
            "  constructor(\n",
            "    readonly nullable: string | null,\n",
            "    readonly undefinable: string | undefined,\n",
            "    readonly optional?: string,\n",
            "    readonly stable: string,\n",
            "    readonly _internal: string,\n",
            "    private visible: string,\n",
            "    plain: string,\n",
            "  ) {}\n",
            "  build(readonly leaked: string) {}\n",
            "}\n",
        );
        let model = source_model(
            &rule("typescript.temporary_fields"),
            &input("typescript-v1", &[("model.ts", source)]),
        )
        .unwrap();

        assert_eq!(
            model.classes[0]
                .fields
                .iter()
                .map(|field| (field.name.as_str(), field.temporary))
                .collect::<BTreeMap<_, _>>(),
            BTreeMap::from([
                ("_internal", false),
                ("nullable", true),
                ("optional", true),
                ("stable", false),
                ("undefinable", true),
                ("visible", false),
            ])
        );
        assert_eq!(
            model.classes[0]
                .fields
                .iter()
                .map(|field| (field.name.as_str(), field.private))
                .collect::<BTreeMap<_, _>>(),
            BTreeMap::from([
                ("_internal", true),
                ("nullable", false),
                ("optional", false),
                ("stable", false),
                ("undefinable", false),
                ("visible", true),
            ])
        );
    }

    #[test]
    fn rust_source_model_keeps_struct_enum_trait_and_impl_contracts_distinct() {
        let source = concat!(
            "pub trait Port<T> { fn load(&self) -> T; }\n",
            "pub struct Account<T> { pub id: String, pending: Option<T> }\n",
            "pub enum State<T> { Ready(T), Empty }\n",
            "impl<T: Clone> Port<T> for Account<T> {\n",
            " fn load(&self) -> T { fn nested() {} nested(); self.pending.clone().unwrap() }\n",
            "}\n",
            "impl<T> Account<T> { pub fn id(&self) -> &str { &self.id } }\n",
        );
        let model = source_model(
            &rule("rust.temporary_fields"),
            &input("rust-v1", &[("src/domain.rs", source)]),
        )
        .unwrap();

        assert_eq!(
            model
                .classes
                .iter()
                .map(|class| class.name.as_str())
                .collect::<Vec<_>>(),
            ["Account", "Port", "State"]
        );
        let account = model
            .classes
            .iter()
            .find(|class| class.name == "Account")
            .unwrap();
        assert_eq!(account.type_parameters, ["T"]);
        assert_eq!(account.bases, ["Port"]);
        assert_eq!(account.methods.len(), 2);
        assert_eq!(
            account
                .fields
                .iter()
                .map(|field| {
                    (
                        field.name.as_str(),
                        field.authored_type.as_str(),
                        field.temporary,
                        field.private,
                    )
                })
                .collect::<Vec<_>>(),
            [
                ("id", "String", false, false),
                ("pending", "Option<T>", true, true),
            ]
        );
        let port = model
            .classes
            .iter()
            .find(|class| class.name == "Port")
            .unwrap();
        assert!(port.interface);
        assert_eq!(port.methods.len(), 1);
        let state = model
            .classes
            .iter()
            .find(|class| class.name == "State")
            .unwrap();
        assert_eq!(state.type_parameters, ["T"]);
        assert!(
            model
                .functions
                .iter()
                .any(|function| function.symbol.contains("nested@"))
        );
    }

    #[test]
    fn rust_impl_method_privacy_distinguishes_visibility_and_trait_ownership() {
        let source = concat!(
            "pub trait Port { fn trait_method(&self); }\n",
            "pub struct Service;\n",
            "impl Service {\n",
            "    pub fn public_method(&self) {}\n",
            "    fn private_method(&self) {}\n",
            "}\n",
            "impl Port for Service {\n",
            "    fn trait_method(&self) {}\n",
            "}\n",
        );
        let model = source_model(
            &rule("rust.temporary_fields"),
            &input("rust-v1", &[("src/lib.rs", source)]),
        )
        .unwrap();
        let private = |suffix: &str| {
            model
                .functions
                .iter()
                .find(|function| function.symbol.ends_with(suffix))
                .unwrap_or_else(|| panic!("missing Rust function ending in {suffix}"))
                .private
        };

        assert!(!private("::Service.public_method"));
        assert!(private("::Service.private_method"));
        assert!(!private("::Service.trait_method"));
    }

    #[test]
    fn rust_model_marks_harness_entry_points_and_attribute_path_references() {
        let source = concat!(
            "fn main() {}\n",
            "#[derive(Deserialize)]\n",
            "struct Config { #[serde(default = \"defaults::port\", with = \"codec\")] port: u16 }\n",
            "/// Mentions \"helper\" only in prose.\n",
            "fn helper() {}\n",
            "mod nested { fn main() {} #[test] fn runs() {} #[tokio::test] async fn runs_async() {} }\n",
        );
        let model = source_model(
            &rule("rust.unused_code"),
            &input("rust-v1", &[("src/main.rs", source)]),
        )
        .unwrap();
        let entry_points = model
            .functions
            .iter()
            .filter(|function| function.entry_point)
            .map(|function| function.symbol.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            entry_points,
            [
                "src/main.rs::main",
                "src/main.rs::nested::runs",
                "src/main.rs::nested::runs_async"
            ]
        );
        assert_eq!(
            model.attribute_references,
            BTreeSet::from(["codec".into(), "port".into()])
        );
    }

    #[test]
    fn rust_model_exempts_declared_unused_items_and_marks_test_code() {
        let source = concat!(
            "fn plain() {}\n",
            "fn _ignored() {}\n",
            "#[allow(dead_code)] fn allowed() {}\n",
            "#[allow(clippy::too_many_lines)] fn linted() {}\n",
            "#[expect(unused, reason = \"fixture\")] struct Expected;\n",
            "#[cfg(not(test))] mod live { fn kept() {} }\n",
            "#[cfg(test)] mod tests { fn helper() {} struct Fixture; impl Fixture { fn build() {} } }\n",
            "#[cfg(test)] fn gated() {}\n",
            "#[allow(dead_code)] impl Holder { fn method() {} }\n",
        );
        let model = source_model(
            &rule("rust.unused_code"),
            &input(
                "rust-v1",
                &[("src/lib.rs", source), ("tests/cli.rs", "fn shared() {}\n")],
            ),
        )
        .unwrap();
        let function = |symbol: &str| {
            model
                .functions
                .iter()
                .find(|function| function.symbol == symbol)
                .unwrap_or_else(|| panic!("missing {symbol}"))
        };
        let flags = |symbol: &str| {
            (
                function(symbol).private,
                function(symbol).scope == CodeScope::Test,
            )
        };
        assert_eq!(flags("src/lib.rs::plain"), (true, false));
        assert_eq!(flags("src/lib.rs::_ignored"), (false, false));
        assert_eq!(flags("src/lib.rs::allowed"), (false, false));
        assert_eq!(flags("src/lib.rs::linted"), (true, false));
        assert_eq!(flags("src/lib.rs::live::kept"), (true, false));
        assert_eq!(flags("src/lib.rs::tests::helper"), (true, true));
        assert_eq!(flags("src/lib.rs::tests::Fixture.build"), (true, true));
        assert_eq!(flags("src/lib.rs::gated"), (true, true));
        assert_eq!(flags("src/lib.rs::Holder.method"), (false, false));
        assert_eq!(flags("tests/cli.rs::shared"), (true, true));
        let class = |name: &str| {
            model
                .classes
                .iter()
                .find(|class| class.name == name)
                .map(|class| (class.private, class.scope == CodeScope::Test))
        };
        assert_eq!(class("Expected"), Some((false, false)));
        assert_eq!(class("Fixture"), Some((true, true)));
    }

    #[test]
    fn portable_privacy_exempts_exports_and_protocol_hooks() {
        let typescript = source_model(
            &rule("typescript.unused_code"),
            &input(
                "typescript-v1",
                &[(
                    "src/a.spec.ts",
                    "export function _a() {}\nexport const _b = () => 1;\nfunction _c() {}\nclass K { private d() {} }\n",
                )],
            ),
        )
        .unwrap();
        let private = typescript
            .functions
            .iter()
            .filter(|function| function.private)
            .map(|function| (function.name.as_str(), function.scope == CodeScope::Test))
            .collect::<Vec<_>>();
        assert_eq!(private, [("_c", true), ("d", true)]);
        let python = source_model(
            &rule("python.unused_code"),
            &input(
                "python-v1",
                &[(
                    "app.py",
                    "class Thing:\n    def _repr_markdown_(self): pass\n    def __eq__(self, o): pass\n    def _helper(self): pass\n",
                )],
            ),
        )
        .unwrap();
        let private = python
            .functions
            .iter()
            .map(|function| {
                (
                    function.name.as_str(),
                    function.private,
                    function.scope == CodeScope::Test,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            private,
            [
                ("_repr_markdown_", false, false),
                ("__eq__", true, false),
                ("_helper", true, false)
            ]
        );
    }
}
