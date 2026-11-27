use super::{
    BTreeSet, Block, Expr, Facts, Fields, FnArg, Function, ImplItem, Input, Item, ItemEnum, ItemFn,
    ItemImpl, ItemStruct, ItemTrait, ItemType, Module, Pat, Policy, Report, Result, Signature,
    Slots, Span, TraitItem, Type, TypeFacts, Visit, location, syntax, visit,
};

fn body_counts(source: &str, body: &Block) -> Result<(usize, usize), String> {
    let start = body.brace_token.span.open().end();
    let begin_byte = body.brace_token.span.open().byte_range().end;
    let end_byte = body.brace_token.span.close().byte_range().start;
    let text = source
        .get(begin_byte..end_byte)
        .ok_or("invalid source span")?;
    let mut code = BTreeSet::new();
    let mut comments = BTreeSet::new();
    let mut offset = 0;
    let mut line = start.line;
    for token in rustc_lexer::tokenize(text, rustc_lexer::FrontmatterAllowed::No) {
        let length = token.len as usize;
        let part = &text[offset..offset + length];
        let ordinary = match token.kind {
            rustc_lexer::TokenKind::LineComment { doc_style }
            | rustc_lexer::TokenKind::BlockComment { doc_style, .. } => doc_style.is_none(),
            _ => false,
        };
        let is_code = !matches!(
            token.kind,
            rustc_lexer::TokenKind::Whitespace
                | rustc_lexer::TokenKind::LineComment { .. }
                | rustc_lexer::TokenKind::BlockComment { .. }
        );
        for (index, chunk) in part.split('\n').enumerate() {
            if is_code && !chunk.is_empty() {
                code.insert(line + index);
            }
            if ordinary && !chunk.trim().is_empty() {
                comments.insert(line + index);
            }
        }
        line += part.bytes().filter(|b| *b == b'\n').count();
        offset += length;
    }
    comments.retain(|line| !code.contains(line));
    Ok((code.len(), comments.len()))
}

fn field_slots(
    symbol: &str,
    path: &str,
    span: Span,
    fields: &Fields,
    declares: Option<String>,
) -> Slots {
    Slots {
        declares,
        returns: None,
        symbol: symbol.into(),
        location: location(path, span),
        types: fields.iter().map(|f| f.ty.clone()).collect(),
        named: fields
            .iter()
            .filter_map(|f| f.ident.as_ref().map(|n| (n.to_string(), syntax(&f.ty))))
            .collect(),
    }
}

/// The key for a type declared in `module`. A second declaration of the same name,
/// such as an `enum` under one `cfg` and a `struct` under its negation, becomes a
/// configuration variant; `impl` blocks bind to the first declaration.
fn type_key(facts: &Facts, module: &Module, name: &syn::Ident) -> String {
    let key = format!("{}::{name}", module.key);
    if !facts.types.contains_key(&key) {
        return key;
    }
    // `key` is one of the taken types, so one of `types.len()` distinct
    // variant candidates is always free.
    (2..)
        .take(facts.types.len())
        .map(|ordinal| super::variant_key(&key, ordinal))
        .find(|candidate| !facts.types.contains_key(candidate))
        .unwrap_or(key)
}

/// The location of a declaration in `module`, recorded as test code when the module is.
fn declared_at(
    module: &Module,
    span: proc_macro2::Span,
    report: &mut Report,
) -> crate::report::Location {
    let declared = location(&module.path, span);
    if module.test {
        report.test_code.insert(declared.clone());
    }
    declared
}

fn struct_declaration(
    facts: &mut Facts,
    module: &Module,
    item: &ItemStruct,
    policy: &Policy,
    report: &mut Report,
) {
    let key = type_key(facts, module, &item.ident);
    let loc = declared_at(module, item.ident.span(), report);
    report.maximum(
        policy,
        "rust.type_fields",
        &key,
        &loc,
        item.fields.len(),
        "declared fields",
    );
    facts.slots.push(field_slots(
        &key,
        &module.path,
        item.ident.span(),
        &item.fields,
        Some(item.ident.to_string()),
    ));
    let fields = item
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            (
                field
                    .ident
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or(index.to_string()),
                field.ty.clone(),
            )
        })
        .collect();
    facts.types.insert(
        key,
        TypeFacts {
            location: loc,
            fields,
            field_count: item.fields.len(),
            is_enum: false,
            variants: BTreeSet::new(),
            functions: vec![],
        },
    );
}

fn enum_declaration(
    facts: &mut Facts,
    module: &Module,
    item: &ItemEnum,
    policy: &Policy,
    report: &mut Report,
) {
    let key = type_key(facts, module, &item.ident);
    let loc = declared_at(module, item.ident.span(), report);
    report.maximum(
        policy,
        "rust.enum_variants",
        &key,
        &loc,
        item.variants.len(),
        "declared variants",
    );
    for variant in &item.variants {
        let name = format!("{key}::{}", variant.ident);
        let variant_location = declared_at(module, variant.ident.span(), report);
        report.maximum(
            policy,
            "rust.type_fields",
            &name,
            &variant_location,
            variant.fields.len(),
            "declared variant fields",
        );
        facts.slots.push(field_slots(
            &name,
            &module.path,
            variant.ident.span(),
            &variant.fields,
            None,
        ));
    }
    facts.types.insert(
        key,
        TypeFacts {
            location: loc,
            fields: vec![],
            field_count: item
                .variants
                .iter()
                .map(|variant| variant.fields.len())
                .max()
                .unwrap_or(0),
            is_enum: true,
            variants: item
                .variants
                .iter()
                .map(|variant| variant.ident.to_string())
                .collect(),
            functions: vec![],
        },
    );
}

/// The first alias of a name wins; a later `cfg` twin aliases the same name.
fn type_alias(facts: &mut Facts, module: &Module, item: &ItemType) {
    facts
        .aliases
        .entry(format!("{}::{}", module.key, item.ident))
        .or_insert_with(|| (module.key.clone(), (*item.ty).clone()));
}

fn trait_declaration(module: &Module, item: &ItemTrait, policy: &Policy, report: &mut Report) {
    let key = format!("{}::{}", module.key, item.ident);
    let loc = declared_at(module, item.ident.span(), report);
    report.maximum(
        policy,
        "rust.trait_functions",
        &key,
        &loc,
        item.items
            .iter()
            .filter(|member| matches!(member, TraitItem::Fn(_)))
            .count(),
        "trait associated functions",
    );
}

fn declaration(
    facts: &mut Facts,
    module: &Module,
    item: &Item,
    policy: &Policy,
    report: &mut Report,
) -> Result<(), String> {
    match item {
        Item::Struct(item) => struct_declaration(facts, module, item, policy, report),
        Item::Enum(item) => enum_declaration(facts, module, item, policy, report),
        Item::Type(item) => type_alias(facts, module, item),
        Item::Trait(item) => trait_declaration(module, item, policy, report),
        Item::Verbatim(_) => return Err(format!("unsupported item syntax in {}", module.path)),
        _ => {}
    }
    Ok(())
}

pub(super) fn declarations(
    facts: &mut Facts,
    policy: &Policy,
    report: &mut Report,
) -> Result<(), String> {
    let modules: Vec<_> = facts
        .modules
        .values()
        .filter(|module| module.repeated.is_none())
        .cloned()
        .collect();
    for module in &modules {
        for item in &module.items {
            declaration(facts, module, item, policy, report)?;
        }
    }
    Ok(())
}

/// Named functions declared inside a body, including methods of local `impl` and
/// `trait` items. Local types own no module-level metrics, so their methods are
/// measured as functions without an owner.
struct Nested<'a> {
    items: Vec<(&'a Signature, &'a Block)>,
    unsupported: bool,
}
impl<'ast> Visit<'ast> for Nested<'ast> {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        self.items.push((&node.sig, &node.block));
    }
    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.items.push((&node.sig, &node.block));
    }
    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        if let Some(block) = &node.default {
            self.items.push((&node.sig, block));
        }
    }
    fn visit_expr(&mut self, node: &'ast Expr) {
        // syn spells a stray `;` statement as an empty verbatim expression.
        if matches!(node, Expr::Verbatim(tokens) if !tokens.is_empty()) {
            self.unsupported = true;
        }
        visit::visit_expr(self, node);
    }
}

fn function_counts(
    module: &Module,
    body: Option<&Block>,
    input: &Input,
    report: &mut Report,
) -> Option<(usize, usize)> {
    let Some(block) = body else {
        return Some((0, 0));
    };
    match body_counts(&input.files[&module.path], block) {
        Ok(counts) => Some(counts),
        Err(error) => {
            report.error(format!("{}: {error}", module.path));
            None
        }
    }
}

/// The type a function builds, by name: its return type's last path segment, with
/// `Self` resolved to the owner's name and `Option<T>` or `Result<T, E>` to `T`.
fn built_type(signature: &Signature, owner: Option<&str>) -> Option<String> {
    let syn::ReturnType::Type(_, ty) = &signature.output else {
        return None;
    };
    let mut ty: &Type = ty;
    loop {
        let Type::Path(path) = ty else {
            return None;
        };
        let segment = path.path.segments.last()?;
        let name = segment.ident.to_string();
        if matches!(name.as_str(), "Option" | "Result")
            && let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments
            && let Some(syn::GenericArgument::Type(inner)) = arguments.args.first()
        {
            ty = inner;
            continue;
        }
        if name == "Self" {
            return owner.map(|owner| owner.rsplit("::").next().unwrap_or(owner).to_string());
        }
        return Some(name);
    }
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the syn visitor hands over each piece of function context separately"
)]
fn function(
    facts: &mut Facts,
    module: &Module,
    symbol: &str,
    signature: &Signature,
    body: Option<&Block>,
    owner: Option<String>,
    input: &Input,
    report: &mut Report,
) {
    let loc = declared_at(module, signature.ident.span(), report);
    let Some((lines, comments)) = function_counts(module, body, input, report) else {
        return;
    };
    report.maximum(
        &input.policy,
        "rust.function_arguments",
        symbol,
        &loc,
        signature.inputs.len(),
        "signature inputs including receiver",
    );
    if body.is_some() {
        report.maximum_with_role(
            &input.policy,
            "rust.function_lines",
            symbol,
            &loc,
            crate::report::RoleAdjustedMaximum::new(
                lines,
                "body code lines",
                crate::migration::function_role(
                    input,
                    &module.path,
                    symbol,
                    owner.as_deref(),
                    &loc,
                ),
            ),
        );
    }
    let parameters: Vec<_> = signature
        .inputs
        .iter()
        .filter_map(|p| {
            if let FnArg::Typed(p) = p {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    facts.slots.push(Slots {
        declares: None,
        returns: built_type(signature, owner.as_deref()),
        symbol: symbol.to_string(),
        location: loc.clone(),
        types: parameters.iter().map(|p| (*p.ty).clone()).collect(),
        named: parameters
            .iter()
            .filter_map(|p| {
                if let Pat::Ident(name) = p.pat.as_ref() {
                    Some((name.ident.to_string(), syntax(&p.ty)))
                } else {
                    None
                }
            })
            .collect(),
    });
    let index = facts.functions.len();
    facts.functions.push(Function {
        symbol: symbol.to_string(),
        module: module.key.clone(),
        location: loc,
        signature: signature.clone(),
        body: body.cloned(),
        body_bytes: body.map(|body| {
            (
                body.brace_token.span.open().byte_range().end,
                body.brace_token.span.close().byte_range().start,
            )
        }),
        owner: owner.clone(),
        lines,
        comments,
        test: module.test,
    });
    if let Some(owner) = owner {
        facts
            .types
            .get_mut(&owner)
            .expect("method owners are registered before their functions")
            .functions
            .push(index);
    }
    if let Some(block) = body {
        let mut nested = Nested {
            items: vec![],
            unsupported: false,
        };
        nested.visit_block(block);
        if nested.unsupported {
            report.error(format!("unsupported verbatim syntax in {symbol}"));
        }
        // Recursing one named function at a time avoids counting descendants twice.
        for (sig, block) in nested.items {
            let nested_symbol = format!(
                "{symbol}::{}@{}:{}",
                sig.ident,
                sig.ident.span().start().line,
                sig.ident.span().start().column
            );
            function(
                facts,
                module,
                &nested_symbol,
                sig,
                Some(block),
                None,
                input,
                report,
            );
        }
    }
}

fn free_function(
    facts: &mut Facts,
    module: &Module,
    item: &ItemFn,
    input: &Input,
    report: &mut Report,
) {
    let test = crate::core::test_suite::rust_test_item(&item.attrs);
    if test {
        // Recorded first, so the function's own findings already see it as test code.
        report
            .test_code
            .insert(location(&module.path, item.sig.ident.span()));
    }
    let recorded = facts.functions.len();
    function(
        facts,
        module,
        &format!("{}::{}", module.key, item.sig.ident),
        &item.sig,
        Some(&item.block),
        None,
        input,
        report,
    );
    // `function` records nothing when the body cannot be counted.
    if let Some(function) = facts.functions.get_mut(recorded) {
        function.test |= test;
    }
}

fn trait_functions(
    facts: &mut Facts,
    module: &Module,
    item: &ItemTrait,
    input: &Input,
    report: &mut Report,
) {
    for member in &item.items {
        if let TraitItem::Fn(member) = member {
            function(
                facts,
                module,
                &format!("{}::{}::{}", module.key, item.ident, member.sig.ident),
                &member.sig,
                member.default.as_ref(),
                None,
                input,
                report,
            );
        }
    }
}

/// `impl<T> Trait for T` implements for every type, so it has no local owner.
fn generic_parameter(item: &ItemImpl) -> bool {
    let Type::Path(path) = &*item.self_ty else {
        return false;
    };
    path.qself.is_none()
        && path.path.segments.len() == 1
        && item
            .generics
            .type_params()
            .any(|param| param.ident == path.path.segments[0].ident)
}

fn impl_functions(
    facts: &mut Facts,
    module: &Module,
    item: &ItemImpl,
    input: &Input,
    report: &mut Report,
) {
    let owner = if generic_parameter(item) {
        None
    } else {
        match facts.owner(&item.self_ty, &module.key) {
            Ok(owner) => owner,
            Err(error) => {
                report.error(error);
                None
            }
        }
    };
    let target = owner
        .clone()
        .unwrap_or_else(|| format!("{}::impl({})", module.key, syntax(&item.self_ty)));
    let trait_name = item
        .trait_
        .as_ref()
        .map(|(_, path, _)| format!("[{}]", syntax(path)))
        .unwrap_or_default();
    for member in &item.items {
        if let ImplItem::Fn(member) = member {
            function(
                facts,
                module,
                &format!("{target}{trait_name}::{}", member.sig.ident),
                &member.sig,
                Some(&member.block),
                owner.clone(),
                input,
                report,
            );
        }
    }
}

pub(super) fn functions(facts: &mut Facts, input: &Input, report: &mut Report) {
    let modules: Vec<_> = facts
        .modules
        .values()
        .filter(|module| module.repeated.is_none())
        .cloned()
        .collect();
    for module in modules {
        for item in &module.items {
            match item {
                Item::Fn(item) => free_function(facts, &module, item, input, report),
                Item::Trait(item) => trait_functions(facts, &module, item, input, report),
                Item::Impl(item) => impl_functions(facts, &module, item, input, report),
                _ => {}
            }
        }
    }
}

pub(super) fn record_type_metrics(facts: &Facts, input: &Input, report: &mut Report) {
    for (symbol, ty) in &facts.types {
        report.maximum(
            &input.policy,
            "rust.type_functions",
            symbol,
            &ty.location,
            ty.functions.len(),
            "owned associated functions",
        );
        report.maximum_with_role(
            &input.policy,
            "rust.type_function_lines",
            symbol,
            &ty.location,
            crate::report::RoleAdjustedMaximum::new(
                ty.functions
                    .iter()
                    .map(|index| facts.functions[*index].lines)
                    .sum(),
                "summed associated body code lines",
                crate::migration::type_role(input, &ty.location.path, symbol, &ty.location),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Facts, Module, Nested, Report, body_counts, declaration, type_key};
    use crate::core::model::History;
    use std::collections::BTreeMap;
    use syn::visit::Visit;

    #[test]
    fn rust_body_counts_code_and_ordinary_comment_only_lines_exactly() {
        let source = concat!(
            "fn sample() {\n",
            "    let first = 1; // trailing comment is a code line\n",
            "    // ordinary comment\n",
            "    /* block\n",
            "       comment */\n",
            "    /// documentation comment is not ordinary commentary\n",
            "    let second = 2;\n",
            "}\n",
        );
        let file = syn::parse_file(source).unwrap();
        let syn::Item::Fn(function) = &file.items[0] else {
            panic!("function fixture");
        };
        assert_eq!(body_counts(source, &function.block).unwrap(), (2, 3));

        let compact = "fn compact() { let value = 1; }";
        let file = syn::parse_file(compact).unwrap();
        let syn::Item::Fn(function) = &file.items[0] else {
            panic!("function fixture");
        };
        assert_eq!(body_counts(compact, &function.block).unwrap(), (1, 0));
    }

    #[test]
    fn unsupported_verbatim_items_fail_closed() {
        let registry = crate::policy::registry("rust-v1").unwrap();
        let policy = crate::policy::parse(
            include_str!("../../examples/quality-policy.json"),
            &registry,
        )
        .unwrap();
        let mut report = Report::new(&registry, &policy, "test", &[], &History::default());
        let mut facts = Facts {
            variants: BTreeMap::new(),
            modules: BTreeMap::new(),
            types: BTreeMap::new(),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };
        let module = Module {
            key: "crate".into(),
            root: "crate".into(),
            path: "src/lib.rs".into(),
            items: Vec::new(),
            imports: BTreeMap::new(),
            test: false,
            repeated: None,
        };
        let unsupported = syn::Item::Verbatim("unsupported syntax".parse().unwrap());

        assert_eq!(
            declaration(&mut facts, &module, &unsupported, &policy, &mut report).unwrap_err(),
            "unsupported item syntax in src/lib.rs"
        );
    }

    #[test]
    fn repeated_type_names_become_numbered_configuration_variants() {
        let mut facts = Facts {
            variants: BTreeMap::new(),
            modules: BTreeMap::new(),
            types: BTreeMap::new(),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };
        let module = Module {
            key: "crate::io".into(),
            root: "crate".into(),
            path: "src/io.rs".into(),
            items: Vec::new(),
            imports: BTreeMap::new(),
            test: false,
            repeated: None,
        };
        let name = syn::Ident::new("Stack", proc_macro2::Span::call_site());
        let mut keys = Vec::new();
        for _ in 0..3 {
            let key = type_key(&facts, &module, &name);
            facts.types.insert(
                key.clone(),
                crate::scan::TypeFacts {
                    location: crate::report::Location {
                        path: "src/io.rs".into(),
                        line: 1,
                        column: 1,
                    },
                    fields: Vec::new(),
                    field_count: 0,
                    is_enum: false,
                    variants: std::collections::BTreeSet::new(),
                    functions: Vec::new(),
                },
            );
            keys.push(key);
        }

        assert_eq!(
            keys,
            [
                "crate::io::Stack",
                "crate::io::Stack [cfg variant 2]",
                "crate::io::Stack [cfg variant 3]"
            ]
        );
    }

    #[test]
    fn nested_visitor_marks_verbatim_expressions_and_measures_local_methods() {
        let expression = syn::Expr::Verbatim("unsupported expression".parse().unwrap());
        let mut nested = Nested {
            items: Vec::new(),
            unsupported: false,
        };
        nested.visit_expr(&expression);
        assert!(nested.unsupported);

        let empty_statement: syn::Block = syn::parse_quote!({
            return 1;
        });
        let mut nested = Nested {
            items: Vec::new(),
            unsupported: false,
        };
        nested.visit_block(&empty_statement);
        assert!(!nested.unsupported);

        let local_types: syn::Block = syn::parse_quote!({
            struct Local;
            impl Local {
                fn method(&self) {}
            }
            trait Shape {
                fn area(&self) -> u8 {
                    0
                }
                fn sides(&self) -> u8;
            }
        });
        let mut nested = Nested {
            items: Vec::new(),
            unsupported: false,
        };
        nested.visit_block(&local_types);
        assert!(!nested.unsupported);
        let names: Vec<_> = nested
            .items
            .iter()
            .map(|(sig, _)| sig.ident.to_string())
            .collect();
        assert_eq!(names, ["method", "area"]);

        let local_function: syn::Item = syn::parse_quote!(
            fn local() {}
        );
        let mut nested = Nested {
            items: Vec::new(),
            unsupported: false,
        };
        nested.visit_item(&local_function);
        assert!(!nested.unsupported);
        assert_eq!(nested.items.len(), 1);
    }
}
