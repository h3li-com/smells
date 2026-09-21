use super::*;

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

fn field_slots(symbol: &str, path: &str, span: Span, fields: &Fields) -> Slots {
    Slots {
        symbol: symbol.into(),
        location: location(path, span),
        types: fields.iter().map(|f| f.ty.clone()).collect(),
        named: fields
            .iter()
            .filter_map(|f| f.ident.as_ref().map(|n| (n.to_string(), syntax(&f.ty))))
            .collect(),
    }
}

fn insert_type(
    facts: &mut Facts,
    key: String,
    ty: TypeFacts,
    message: String,
) -> Result<(), String> {
    if facts.types.insert(key, ty).is_some() {
        Err(message)
    } else {
        Ok(())
    }
}

fn struct_declaration(
    facts: &mut Facts,
    module: &Module,
    item: &ItemStruct,
    policy: &Policy,
    report: &mut Report,
) -> Result<(), String> {
    let key = format!("{}::{}", module.key, item.ident);
    let loc = location(&module.path, item.ident.span());
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
    insert_type(
        facts,
        key.clone(),
        TypeFacts {
            location: loc,
            fields,
            field_count: item.fields.len(),
            is_enum: false,
            variants: BTreeSet::new(),
            functions: vec![],
        },
        format!("ambiguous authored type: {key}; active-cfg analysis is pending"),
    )
}

fn enum_declaration(
    facts: &mut Facts,
    module: &Module,
    item: &ItemEnum,
    policy: &Policy,
    report: &mut Report,
) -> Result<(), String> {
    let key = format!("{}::{}", module.key, item.ident);
    let loc = location(&module.path, item.ident.span());
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
        let variant_location = location(&module.path, variant.ident.span());
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
        ));
    }
    insert_type(
        facts,
        key.clone(),
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
        format!("ambiguous authored type: {key}"),
    )
}

fn type_alias(facts: &mut Facts, module: &Module, item: &ItemType) -> Result<(), String> {
    let key = format!("{}::{}", module.key, item.ident);
    if facts
        .aliases
        .insert(key.clone(), (module.key.clone(), (*item.ty).clone()))
        .is_some()
    {
        Err(format!("ambiguous type alias: {key}"))
    } else {
        Ok(())
    }
}

fn trait_declaration(module: &Module, item: &ItemTrait, policy: &Policy, report: &mut Report) {
    let key = format!("{}::{}", module.key, item.ident);
    let loc = location(&module.path, item.ident.span());
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
        Item::Trait(item) => {
            trait_declaration(module, item, policy, report);
            Ok(())
        }
        Item::Verbatim(_) => Err(format!("unsupported item syntax in {}", module.path)),
        _ => Ok(()),
    }
}

pub(super) fn declarations(
    facts: &mut Facts,
    policy: &Policy,
    report: &mut Report,
) -> Result<(), String> {
    let modules: Vec<_> = facts.modules.values().cloned().collect();
    for module in &modules {
        for item in &module.items {
            declaration(facts, module, item, policy, report)?;
        }
    }
    Ok(())
}

struct Nested<'a> {
    items: Vec<&'a ItemFn>,
    unsupported: bool,
}
impl<'ast> Visit<'ast> for Nested<'ast> {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        self.items.push(node);
    }
    fn visit_item(&mut self, node: &'ast Item) {
        if matches!(
            node,
            Item::Struct(_)
                | Item::Enum(_)
                | Item::Impl(_)
                | Item::Type(_)
                | Item::Mod(_)
                | Item::Trait(_)
        ) {
            self.unsupported = true;
        }
        visit::visit_item(self, node);
    }
    fn visit_expr(&mut self, node: &'ast Expr) {
        if matches!(node, Expr::Verbatim(_)) {
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

#[allow(clippy::too_many_arguments)]
fn function(
    facts: &mut Facts,
    module: &Module,
    symbol: String,
    signature: &Signature,
    body: Option<&Block>,
    owner: Option<String>,
    input: &Input,
    report: &mut Report,
) {
    let loc = location(&module.path, signature.ident.span());
    let Some((lines, comments)) = function_counts(module, body, input, report) else {
        return;
    };
    report.maximum(
        &input.policy,
        "rust.function_arguments",
        &symbol,
        &loc,
        signature.inputs.len(),
        "signature inputs including receiver",
    );
    if body.is_some() {
        report.maximum(
            &input.policy,
            "rust.function_lines",
            &symbol,
            &loc,
            lines,
            "body code lines",
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
        symbol: symbol.clone(),
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
        symbol: symbol.clone(),
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
    });
    if let Some(owner) = owner {
        facts.types.get_mut(&owner).unwrap().functions.push(index);
    }
    if let Some(block) = body {
        let mut nested = Nested {
            items: vec![],
            unsupported: false,
        };
        nested.visit_block(block);
        if nested.unsupported {
            report.error(format!(
                "unsupported local declaration/verbatim syntax in {symbol}"
            ));
        }
        // Recursing one named function at a time avoids counting descendants twice.
        for nested in nested.items {
            let nested_symbol = format!(
                "{symbol}::{}@{}:{}",
                nested.sig.ident,
                nested.sig.ident.span().start().line,
                nested.sig.ident.span().start().column
            );
            function(
                facts,
                module,
                nested_symbol,
                &nested.sig,
                Some(&nested.block),
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
    function(
        facts,
        module,
        format!("{}::{}", module.key, item.sig.ident),
        &item.sig,
        Some(&item.block),
        None,
        input,
        report,
    );
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
                format!("{}::{}::{}", module.key, item.ident, member.sig.ident),
                &member.sig,
                member.default.as_ref(),
                None,
                input,
                report,
            );
        }
    }
}

fn impl_functions(
    facts: &mut Facts,
    module: &Module,
    item: &ItemImpl,
    input: &Input,
    report: &mut Report,
) {
    let owner = match facts.owner(&item.self_ty, &module.key) {
        Ok(owner) => owner,
        Err(error) => {
            report.error(error);
            None
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
                format!("{target}{trait_name}::{}", member.sig.ident),
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
    let modules: Vec<_> = facts.modules.values().cloned().collect();
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

pub(super) fn record_type_metrics(facts: &Facts, policy: &Policy, report: &mut Report) {
    for (symbol, ty) in &facts.types {
        report.maximum(
            policy,
            "rust.type_functions",
            symbol,
            &ty.location,
            ty.functions.len(),
            "owned associated functions",
        );
        report.maximum(
            policy,
            "rust.type_function_lines",
            symbol,
            &ty.location,
            ty.functions
                .iter()
                .map(|index| facts.functions[*index].lines)
                .sum(),
            "summed associated body code lines",
        );
    }
}
