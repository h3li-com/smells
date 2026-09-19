use crate::{
    policy::Policy,
    report::Report,
    scan::{Facts, Function, syntax},
};
use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::result::Result;
use syn::{
    visit::{self, Visit},
    *,
};

fn parameter(policy: &Policy, id: &str, name: &str) -> u64 {
    policy.parameter(id, name)
}

fn raw(ty: &Type) -> bool {
    if let Type::Reference(ty) = ty {
        return raw(&ty.elem);
    }
    let Type::Path(ty) = ty else {
        return false;
    };
    if ty.qself.is_some()
        || ty
            .path
            .segments
            .iter()
            .any(|s| !matches!(s.arguments, PathArguments::None))
    {
        return false;
    }
    let name = ty
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::");
    matches!(
        name.as_str(),
        "bool"
            | "char"
            | "str"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "String"
            | "std::string::String"
            | "alloc::string::String"
    )
}

fn optional(ty: &Type) -> bool {
    let Type::Path(ty) = ty else {
        return false;
    };
    let name = ty
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::");
    ty.qself.is_none()
        && matches!(
            name.as_str(),
            "Option" | "std::option::Option" | "core::option::Option"
        )
        && matches!(ty.path.segments.last().map(|s|&s.arguments),Some(PathArguments::AngleBracketed(args)) if args.args.len()==1)
}

fn primitives(facts: &Facts, policy: &Policy, report: &mut Report) {
    let id = "rust.primitive_slots";
    if !policy.enabled(id) {
        return;
    }
    for slots in &facts.slots {
        let total = slots.types.len();
        if total == 0 {
            continue;
        }
        let raw = slots.types.iter().filter(|ty| raw(ty)).count();
        let count = parameter(policy, id, "minimum_raw_slots");
        let percent = parameter(policy, id, "minimum_share_percent");
        let matched =
            raw as u64 >= count && 100_u128 * raw as u128 >= percent as u128 * total as u128;
        report.finding(
            policy,
            id,
            &slots.symbol,
            &slots.location,
            "raw syntax slots and share",
            json!({"raw":raw,"total":total}),
            "count and share >=",
            json!({"minimum_raw_slots":count,"minimum_share_percent":percent}),
            matched,
            json!({"type_interpretation":"spelled_syntax_not_alias_or_type_resolution"}),
        );
    }
}

fn expression(body: &Block) -> Option<&Expr> {
    if body.stmts.len() != 1 {
        return None;
    }
    let Stmt::Expr(expr, _) = &body.stmts[0] else {
        return None;
    };
    if let Expr::Return(expr) = expr {
        expr.expr.as_deref()
    } else {
        Some(expr)
    }
}

fn identifier(expr: &Expr) -> Option<String> {
    if let Expr::Path(path) = expr {
        if path.qself.is_none() {
            return path.path.get_ident().map(ToString::to_string);
        }
    }
    None
}

fn self_field(expr: &Expr) -> Option<String> {
    let Expr::Field(field) = expr else {
        return None;
    };
    if identifier(&field.base).as_deref() != Some("self") {
        return None;
    }
    Some(match &field.member {
        Member::Named(name) => name.to_string(),
        Member::Unnamed(index) => index.index.to_string(),
    })
}

fn arguments(function: &Function) -> Option<Vec<String>> {
    function
        .signature
        .inputs
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(arg) = arg {
                Some(arg)
            } else {
                None
            }
        })
        .map(|arg| {
            if let Pat::Ident(name) = arg.pat.as_ref() {
                if name.subpat.is_none() {
                    return Some(name.ident.to_string());
                }
            }
            None
        })
        .collect()
}

fn accessor(function: &Function) -> bool {
    if function.signature.receiver().is_none() {
        return false;
    }
    let Some(expr) = function.body.as_ref().and_then(expression) else {
        return false;
    };
    if self_field(expr).is_some() {
        return true;
    }
    if let Expr::Reference(expr) = expr {
        return self_field(&expr.expr).is_some();
    }
    if let Expr::Assign(assign) = expr {
        return self_field(&assign.left).is_some()
            && arguments(function).is_some_and(|args| {
                args.len() == 1 && identifier(&assign.right) == Some(args[0].clone())
            });
    }
    false
}

fn forwarding(function: &Function) -> bool {
    if function.signature.receiver().is_none() {
        return false;
    }
    let Some(expr) = function.body.as_ref().and_then(expression) else {
        return false;
    };
    let (args, skip) = match expr {
        Expr::MethodCall(call) if self_field(&call.receiver).is_some() => (&call.args, 0),
        Expr::Call(call)
            if matches!(call.func.as_ref(), Expr::Path(_))
                && call.args.first().is_some_and(|argument| {
                    self_field(argument).is_some()
                        || matches!(argument, Expr::Reference(reference) if self_field(&reference.expr).is_some())
                }) =>
        {
            (&call.args, 1)
        }
        _ => return false,
    };
    let Some(parameters) = arguments(function) else {
        return false;
    };
    let passed: Option<Vec<_>> = args.iter().skip(skip).map(identifier).collect();
    passed == Some(parameters)
}

#[derive(Default)]
struct FieldUses {
    fields: BTreeSet<String>,
}
impl<'ast> Visit<'ast> for FieldUses {
    fn visit_expr_field(&mut self, expr: &'ast ExprField) {
        if let Some(field) = self_field(&Expr::Field(expr.clone())) {
            self.fields.insert(field);
        }
        visit::visit_expr_field(self, expr);
    }
    fn visit_item_fn(&mut self, _: &'ast ItemFn) {}
}

fn types(facts: &Facts, policy: &Policy, report: &mut Report) {
    for (symbol, ty) in &facts.types {
        let functions: Vec<_> = ty.functions.iter().map(|i| &facts.functions[*i]).collect();
        let receivers: Vec<_> = functions
            .iter()
            .copied()
            .filter(|f| f.signature.receiver().is_some())
            .collect();
        let operations = functions.iter().filter(|f| !accessor(f)).count();
        let lines: usize = functions.iter().map(|f| f.lines).sum();
        let id = "rust.data_class";
        if policy.enabled(id) {
            let minimum = parameter(policy, id, "minimum_fields");
            let maximum = parameter(policy, id, "maximum_operations");
            report.finding(policy,id,symbol,&ty.location,"fields and non-accessor operations",json!({"fields":ty.field_count,"operations":operations}),"fields >= and operations <=",
                json!({"minimum_fields":minimum,"maximum_operations":maximum}),ty.field_count as u64>=minimum && operations as u64<=maximum,
                json!({"accessor_definition":"strict_source_shape","enum_fields":"maximum_per_variant"}));
        }
        let id = "rust.lazy_class";
        if policy.enabled(id) {
            let fields = parameter(policy, id, "maximum_fields");
            let count = parameter(policy, id, "maximum_functions");
            let code = parameter(policy, id, "maximum_lines");
            report.finding(
                policy,
                id,
                symbol,
                &ty.location,
                "fields/functions/code lines",
                json!({"fields":ty.field_count,"functions":functions.len(),"lines":lines}),
                "all <=",
                json!({"maximum_fields":fields,"maximum_functions":count,"maximum_lines":code}),
                ty.field_count as u64 <= fields
                    && functions.len() as u64 <= count
                    && lines as u64 <= code,
                json!({"scope":"source_authored_members"}),
            );
        }
        let id = "rust.forwarding_share";
        if policy.enabled(id) && !receivers.is_empty() {
            let forward = receivers.iter().filter(|f| forwarding(f)).count();
            let minimum = parameter(policy, id, "minimum_methods");
            let share = parameter(policy, id, "minimum_share_percent");
            report.finding(
                policy,
                id,
                symbol,
                &ty.location,
                "syntactic forwarding share",
                json!({"forwarding":forward,"methods":receivers.len()}),
                "methods and share >=",
                json!({"minimum_methods":minimum,"minimum_share_percent":share}),
                receivers.len() as u64 >= minimum
                    && 100_u128 * forward as u128 >= share as u128 * receivers.len() as u128,
                json!({"resolved_type_equivalence":"not_checked"}),
            );
        }
        let id = "rust.temporary_fields";
        if policy.enabled(id) && !ty.is_enum && !receivers.is_empty() {
            let uses: Vec<_> = receivers
                .iter()
                .map(|f| {
                    let mut used = FieldUses::default();
                    if let Some(body) = &f.body {
                        used.visit_block(body);
                    }
                    used.fields
                })
                .collect();
            let maximum = parameter(policy, id, "maximum_use_percent");
            let min_methods = parameter(policy, id, "minimum_methods");
            let min_fields = parameter(policy, id, "minimum_fields");
            let fields: Vec<_> = ty
                .fields
                .iter()
                .filter(|(_, ty)| optional(ty))
                .map(|(name, _)| {
                    let count = uses.iter().filter(|used| used.contains(name)).count();
                    (name, count)
                })
                .filter(|(_, count)| {
                    100_u128 * (*count as u128) <= maximum as u128 * receivers.len() as u128
                })
                .collect();
            report.finding(policy,id,symbol,&ty.location,"low-use optional syntax fields",json!({"fields":fields.len(),"methods":receivers.len()}),"field/method minima and use <=",
                json!({"minimum_fields":min_fields,"minimum_methods":min_methods,"maximum_use_percent":maximum}),fields.len() as u64>=min_fields && receivers.len() as u64>=min_methods,
                json!({"field_use_counts":fields,"option_interpretation":"spelled_syntax","access_scope":"direct_receiver_projections"}));
        }
    }
}

fn comments(facts: &Facts, policy: &Policy, report: &mut Report) {
    let id = "rust.comment_share";
    if !policy.enabled(id) {
        return;
    }
    for function in &facts.functions {
        if function.body.is_none() || function.lines + function.comments == 0 {
            continue;
        }
        let code = parameter(policy, id, "minimum_code_lines");
        let share = parameter(policy, id, "minimum_share_percent");
        let total = function.lines + function.comments;
        report.finding(
            policy,
            id,
            &function.symbol,
            &function.location,
            "ordinary comment-only body-line share",
            json!({"comment_lines":function.comments,"code_lines":function.lines}),
            "code minimum and share >=",
            json!({"minimum_code_lines":code,"minimum_share_percent":share}),
            function.lines as u64 >= code
                && 100_u128 * function.comments as u128 >= share as u128 * total as u128,
            json!({"denominator":total,"doc_comments":"excluded"}),
        );
    }
}

fn combinations(
    slots: &[(String, String)],
    k: usize,
    start: usize,
    chosen: &mut Vec<(String, String)>,
    groups: &mut Vec<Vec<(String, String)>>,
    remaining: &mut usize,
) -> Result<(), String> {
    if chosen.len() == k {
        if *remaining == 0 {
            return Err("maximum_group_combinations budget exceeded".into());
        }
        *remaining -= 1;
        groups.push(chosen.clone());
        return Ok(());
    }
    let needed = k - chosen.len();
    if needed > slots.len().saturating_sub(start) {
        return Ok(());
    }
    for index in start..=slots.len() - needed {
        chosen.push(slots[index].clone());
        combinations(slots, k, index + 1, chosen, groups, remaining)?;
        chosen.pop();
    }
    Ok(())
}

fn clumps(facts: &Facts, policy: &Policy, report: &mut Report) {
    let id = "rust.data_clumps";
    if !policy.enabled(id) {
        return;
    }
    let size = parameter(policy, id, "minimum_group_size");
    let minimum = parameter(policy, id, "minimum_declarations");
    let mut remaining = policy.limits.maximum_group_combinations;
    let mut groups: BTreeMap<Vec<(String, String)>, BTreeSet<usize>> = BTreeMap::new();
    let mut declarations = BTreeSet::new();
    for (index, slots) in facts.slots.iter().enumerate() {
        if !declarations.insert((
            &slots.location.path,
            slots.location.line,
            slots.location.column,
        )) {
            continue;
        }
        let named: Vec<_> = slots
            .named
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if (named.len() as u64) < size {
            continue;
        }
        let mut subsets = vec![];
        if let Err(error) = combinations(
            &named,
            size as usize,
            0,
            &mut vec![],
            &mut subsets,
            &mut remaining,
        ) {
            report.errors.push(error);
            return;
        }
        for group in subsets {
            groups.entry(group).or_default().insert(index);
        }
    }
    for (group, support) in groups {
        if (support.len() as u64) < minimum {
            continue;
        }
        let populations: Vec<_> = support.into_iter().map(|i| &facts.slots[i]).collect();
        let first = populations[0];
        report.finding(policy,id,&first.symbol,&first.location,"repeated named syntax group",json!({"slots":group.len(),"declarations":populations.len()}),"both >=",
            json!({"minimum_group_size":size,"minimum_declarations":minimum}),true,json!({"slots":group,"supporting_symbols":populations.iter().map(|s|&s.symbol).collect::<Vec<_>>(),"type_interpretation":"exact_syntax"}));
        report.findings.last_mut().unwrap().related_symbols = populations
            .iter()
            .skip(1)
            .map(|s| s.symbol.clone())
            .collect();
        report.findings.last_mut().unwrap().related_locations = populations
            .iter()
            .skip(1)
            .map(|slots| slots.location.clone())
            .collect();
    }
}

fn normalized(stream: TokenStream, result: &mut Vec<String>) {
    for token in stream {
        match token {
            TokenTree::Group(group) => {
                result.push(format!("open:{:?}", group.delimiter()));
                normalized(group.stream(), result);
                result.push(format!("close:{:?}", group.delimiter()));
            }
            TokenTree::Punct(punct) => result.push(format!("punct:{}", punct.as_char())),
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                let keyword = matches!(
                    name.as_str(),
                    "as" | "async"
                        | "await"
                        | "break"
                        | "const"
                        | "continue"
                        | "else"
                        | "enum"
                        | "false"
                        | "fn"
                        | "for"
                        | "if"
                        | "impl"
                        | "in"
                        | "let"
                        | "loop"
                        | "match"
                        | "mod"
                        | "move"
                        | "mut"
                        | "pub"
                        | "ref"
                        | "return"
                        | "self"
                        | "Self"
                        | "static"
                        | "struct"
                        | "super"
                        | "trait"
                        | "true"
                        | "type"
                        | "unsafe"
                        | "use"
                        | "where"
                        | "while"
                        | "yield"
                );
                result.push(if keyword {
                    format!("keyword:{name}")
                } else {
                    "identifier".into()
                });
            }
            TokenTree::Literal(literal) => {
                let tag = match syn::parse_str::<Lit>(&literal.to_string()) {
                    Ok(Lit::Str(_)) => "str".into(),
                    Ok(Lit::ByteStr(_)) => "bytes".into(),
                    Ok(Lit::CStr(_)) => "cstr".into(),
                    Ok(Lit::Char(_)) => "char".into(),
                    Ok(Lit::Byte(_)) => "byte".into(),
                    Ok(Lit::Int(value)) => format!("int:{}", value.suffix()),
                    Ok(Lit::Float(value)) => format!("float:{}", value.suffix()),
                    _ => "other".into(),
                };
                result.push(format!("literal:{tag}"));
            }
        }
    }
}

fn fingerprint(function: &Function) -> (usize, BTreeMap<Vec<String>, usize>) {
    let mut tokens = vec![];
    if let Some(body) = &function.body {
        for stmt in &body.stmts {
            normalized(stmt.to_token_stream(), &mut tokens);
        }
    }
    let mut counts = BTreeMap::new();
    for window in tokens.windows(4) {
        *counts.entry(window.to_vec()).or_insert(0) += 1;
    }
    (tokens.len(), counts)
}

fn interface(function: &Function) -> String {
    let signature = &function.signature;
    let types: Vec<_> = signature
        .inputs
        .iter()
        .map(|input| match input {
            FnArg::Receiver(receiver) => syntax(receiver),
            FnArg::Typed(input) => syntax(&input.ty),
        })
        .collect();
    format!(
        "{}|{:?}|{}|{}|{}|{}|{}|{}|{}|{}",
        signature.ident,
        types,
        syntax(&signature.output),
        syntax(&signature.generics),
        syntax(&signature.generics.where_clause),
        syntax(&signature.asyncness),
        syntax(&signature.constness),
        syntax(&signature.unsafety),
        syntax(&signature.abi),
        syntax(&signature.variadic)
    )
}

fn duplicates(facts: &Facts, policy: &Policy, report: &mut Report) {
    let duplicate = "rust.duplicate_functions";
    let alternative = "rust.alternative_interfaces";
    if !policy.enabled(duplicate) && !policy.enabled(alternative) {
        return;
    }
    let mut declarations = BTreeSet::new();
    let functions: Vec<_> = facts
        .functions
        .iter()
        .filter(|f| f.body.is_some())
        .filter(|f| declarations.insert((&f.location.path, f.location.line, f.location.column)))
        .collect();
    let fingerprints: Vec<_> = functions.iter().map(|f| fingerprint(f)).collect();
    let mut comparisons = 0;
    for a in 0..functions.len() {
        for b in a + 1..functions.len() {
            if functions[a].location.path == functions[b].location.path
                && matches!((functions[a].body_bytes, functions[b].body_bytes),
                    (Some((start_a,end_a)),Some((start_b,end_b))) if start_a < end_b && start_b < end_a)
            {
                continue;
            }
            comparisons += 1;
            if comparisons > policy.limits.maximum_pairs {
                report.errors.push("maximum_pairs budget exceeded".into());
                return;
            }
            let (size_a, counts_a) = &fingerprints[a];
            let (size_b, counts_b) = &fingerprints[b];
            let keys: BTreeSet<_> = counts_a.keys().chain(counts_b.keys()).collect();
            let mut intersection = 0;
            let mut union = 0;
            for key in keys {
                let a = *counts_a.get(key).unwrap_or(&0);
                let b = *counts_b.get(key).unwrap_or(&0);
                intersection += a.min(b);
                union += a.max(b);
            }
            if union == 0 {
                continue;
            }
            let (first, second, ordered_sizes) = if functions[a].symbol <= functions[b].symbol {
                (functions[a], functions[b], [size_a, size_b])
            } else {
                (functions[b], functions[a], [size_b, size_a])
            };
            for id in [duplicate, alternative] {
                if !policy.enabled(id) {
                    continue;
                }
                if id == alternative
                    && (first.owner.is_none()
                        || second.owner.is_none()
                        || first.owner == second.owner
                        || interface(first) == interface(second))
                {
                    continue;
                }
                let minimum = parameter(policy, id, "minimum_tokens");
                let similarity = parameter(policy, id, "minimum_similarity_basis_points");
                if (*size_a as u64) < minimum
                    || (*size_b as u64) < minimum
                    || intersection as u128 * 10_000 < similarity as u128 * union as u128
                {
                    continue;
                }
                report.finding(policy,id,&first.symbol,&first.location,"normalized 4-token multiset Jaccard",json!({"intersection":intersection,"union":union}),">=",
                    json!({"minimum_similarity_basis_points":similarity,"minimum_tokens":minimum}),true,
                    json!({"token_counts":ordered_sizes,"other_location":second.location,"normalization":"syntax_tokens_v1_not_semantic_equivalence"}));
                report
                    .findings
                    .last_mut()
                    .unwrap()
                    .related_symbols
                    .push(second.symbol.clone());
                report
                    .findings
                    .last_mut()
                    .unwrap()
                    .related_locations
                    .push(second.location.clone());
            }
        }
    }
}

fn variant_paths(pattern: &Pat, paths: &mut Vec<(Vec<String>, String)>) {
    let path = match pattern {
        Pat::Path(p) => Some(&p.path),
        Pat::Struct(p) => Some(&p.path),
        Pat::TupleStruct(p) => Some(&p.path),
        Pat::Or(p) => {
            for case in &p.cases {
                variant_paths(case, paths);
            }
            None
        }
        _ => None,
    };
    if let Some(path) = path {
        let mut segments: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        if segments.len() > 1 {
            let variant = segments.pop().unwrap();
            paths.push((segments, variant));
        }
    }
}

struct Matches<'a> {
    facts: &'a Facts,
    module: &'a str,
    function_owner: Option<&'a str>,
    minimum: u64,
    owners: BTreeSet<String>,
}
impl<'ast> Visit<'ast> for Matches<'_> {
    fn visit_expr_match(&mut self, expr: &'ast ExprMatch) {
        if expr.arms.len() as u64 >= self.minimum {
            let mut paths = vec![];
            for arm in &expr.arms {
                variant_paths(&arm.pat, &mut paths);
            }
            let owners: Option<Vec<_>> = paths
                .iter()
                .map(|(path, variant)| {
                    let owner = if path.len() == 1 && path[0] == "Self" {
                        self.function_owner.map(str::to_owned)
                    } else {
                        self.facts.resolve(path, self.module, 0).ok().flatten()
                    }?;
                    self.facts
                        .types
                        .get(&owner)
                        .filter(|ty| ty.is_enum && ty.variants.contains(variant))
                        .map(|_| owner)
                })
                .collect();
            if let Some(owners) = owners {
                let owners: BTreeSet<_> = owners.into_iter().collect();
                if owners.len() == 1 {
                    let owner = owners.into_iter().next().unwrap();
                    if self.facts.types[&owner].is_enum {
                        self.owners.insert(owner);
                    }
                }
            }
        }
        visit::visit_expr_match(self, expr);
    }
    fn visit_item_fn(&mut self, _: &'ast ItemFn) {}
}

fn dispatch(facts: &Facts, policy: &Policy, report: &mut Report) {
    let id = "rust.repeated_dispatch";
    if !policy.enabled(id) {
        return;
    }
    let mut groups: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for (index, function) in facts.functions.iter().enumerate() {
        let mut matches = Matches {
            facts,
            module: &function.module,
            function_owner: function.owner.as_deref(),
            minimum: parameter(policy, id, "minimum_arms"),
            owners: BTreeSet::new(),
        };
        if let Some(body) = &function.body {
            matches.visit_block(body);
        }
        for owner in matches.owners {
            groups.entry(owner).or_default().insert(index);
        }
    }
    let sites = parameter(policy, id, "minimum_sites");
    let arms = parameter(policy, id, "minimum_arms");
    for (owner, support) in groups {
        let callables: Vec<_> = support.iter().map(|i| &facts.functions[*i]).collect();
        let first = callables[0];
        report.finding(policy,id,&owner,&first.location,"distinct named callable dispatch sites",json!(support.len()),">=",json!(sites),support.len() as u64>=sites,
            json!({"minimum_arms":arms,"callables":callables.iter().map(|function|&function.symbol).collect::<Vec<_>>(),"scope":"resolved_local_enum_variant_patterns_not_scrutinee_type_inference"}));
        let finding = report.findings.last_mut().unwrap();
        finding.related_symbols = callables
            .iter()
            .skip(1)
            .map(|function| function.symbol.clone())
            .collect();
        finding.related_locations = callables
            .iter()
            .skip(1)
            .map(|function| function.location.clone())
            .collect();
    }
}

pub fn check(facts: &Facts, policy: &Policy, report: &mut Report) {
    primitives(facts, policy, report);
    types(facts, policy, report);
    comments(facts, policy, report);
    clumps(facts, policy, report);
    duplicates(facts, policy, report);
    dispatch(facts, policy, report);
}
