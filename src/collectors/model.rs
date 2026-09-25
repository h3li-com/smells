use super::{
    common::simple_names,
    syntax::{code_only, code_only_from_tree},
};
use crate::{input::Input, policy::Rule, report::Location};
use quote::ToTokens;
use rayon::prelude::*;
use regex::Regex;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, OnceLock},
};
use syn::{ImplItem, TraitItem, spanned::Spanned, visit::Visit};
use tree_sitter::{Language, Node, Parser, Tree};

#[derive(Clone)]
pub(super) struct FunctionFact {
    pub(super) name: String,
    pub(super) symbol: String,
    pub(super) location: Location,
    pub(super) type_parameters: Vec<String>,
    pub(super) body: Option<Arc<str>>,
    pub(super) source: Arc<str>,
    pub(super) private: bool,
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
}

#[derive(Default)]
pub(super) struct SourceModel {
    pub(super) functions: Vec<FunctionFact>,
    pub(super) classes: Vec<ClassFact>,
    pub(super) code_by_path: BTreeMap<String, Arc<str>>,
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

fn class_node(kind: &str, language: &str) -> bool {
    match language {
        "python" => kind == "class_definition",
        "typescript" => matches!(
            kind,
            "class" | "class_declaration" | "abstract_class_declaration" | "interface_declaration"
        ),
        _ => false,
    }
}

fn function_node(kind: &str, language: &str) -> bool {
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
    node.child_by_field_name("name")
        .map(|name| text_of(name, source))
        .unwrap_or_else(|| format!("anonymous@{}", node.start_position().row + 1))
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

fn authored_field(slot: regex::Captures<'_>, language: &str) -> Option<FieldFact> {
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
        .filter_map(|capture| authored_field(capture, language))
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
        source: code_only(path, &raw_source)
            .expect("Rust token masking cannot fail")
            .into(),
        type_parameters: rust_type_parameters(&signature.generics),
        body: body_source.map(|body| {
            Arc::from(code_only(path, &body).expect("Rust function-body token masking cannot fail"))
        }),
        private,
        name,
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
            matches!(function.vis, syn::Visibility::Inherited),
        );
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
        class.private = matches!(structure.vis, syn::Visibility::Inherited);
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
        class.private = matches!(enumeration.vis, syn::Visibility::Inherited);
    }

    fn visit_item_trait(&mut self, trait_item: &'ast syn::ItemTrait) {
        let name = trait_item.ident.to_string();
        let module = self.module();
        let owner = rust_owner(&module, &name);
        let functions = trait_item
            .items
            .iter()
            .filter_map(|method| match method {
                TraitItem::Fn(method) => Some(rust_function(
                    &self.path,
                    Some(&owner),
                    &method.sig,
                    method.default.as_ref(),
                    false,
                )),
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
        let private = implementation.trait_.is_none();
        let functions = implementation
            .items
            .iter()
            .filter_map(|method| match method {
                ImplItem::Fn(method) => Some(rust_function(
                    &self.path,
                    Some(&owner),
                    &method.sig,
                    Some(&method.block),
                    private && matches!(method.vis, syn::Visibility::Inherited),
                )),
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
                let parent = rust_function(
                    &self.path,
                    Some(&owner),
                    &method.sig,
                    Some(&method.block),
                    private && matches!(method.vis, syn::Visibility::Inherited),
                )
                .symbol;
                self.record_nested_functions(&parent, &method.block);
            }
        }
    }

    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let Some((_, items)) = &module.content else {
            return;
        };
        self.module_path.push(module.ident.to_string());
        for item in items {
            self.visit_item(item);
        }
        self.module_path.pop();
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
            .map_err(|error| format!("collector parse error in {path}: {error}"))?;
        RustModelVisitor {
            path: path.clone(),
            module_path: Vec::new(),
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

fn parse_portable_tree(language: &str, path: &str, source: &str) -> Result<Tree, String> {
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
    if tree.root_node().has_error() {
        return Err(format!("collector parse error in {path}"));
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
        private: name.starts_with('_'),
        fields: class_fields(node, source, authored_code, language),
        methods: Vec::new(),
        name,
        range: (node.start_byte(), node.end_byte()),
    }
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
    let body = node
        .child_by_field_name("body")
        .map(|body| Arc::from(text_of(body, authored_code)));
    let signature = header_before_body(node, source);
    let function = FunctionFact {
        symbol: owner.as_ref().map_or_else(
            || format!("{path}::{name}"),
            |owner| format!("{path}::{owner}.{name}"),
        ),
        location: node_location(path, node),
        type_parameters: portable_type_parameters(node, source, language),
        source: text_of(node, authored_code).into(),
        body,
        private: name.starts_with('_') || signature.contains("private "),
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
