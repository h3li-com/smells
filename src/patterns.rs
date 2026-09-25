use crate::{
    metrics::{self, Counter},
    policy::Policy,
    report::{FindingRelations, Report},
    scan::{Facts, Function, syntax},
    similarity::exact_jaccard_pairs,
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
    if let Expr::Path(path) = expr
        && path.qself.is_none()
    {
        return path.path.get_ident().map(ToString::to_string);
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
            if let Pat::Ident(name) = arg.pat.as_ref()
                && name.subpat.is_none()
            {
                return Some(name.ident.to_string());
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

struct TypeMetrics<'a> {
    functions: Vec<&'a Function>,
    receivers: Vec<&'a Function>,
    operations: usize,
    lines: usize,
}

fn type_metrics<'a>(ty: &crate::scan::TypeFacts, facts: &'a Facts) -> TypeMetrics<'a> {
    let functions: Vec<_> = ty
        .functions
        .iter()
        .map(|index| &facts.functions[*index])
        .collect();
    let receivers = functions
        .iter()
        .copied()
        .filter(|function| function.signature.receiver().is_some())
        .collect();
    TypeMetrics {
        operations: functions
            .iter()
            .filter(|function| !accessor(function))
            .count(),
        lines: functions.iter().map(|function| function.lines).sum(),
        functions,
        receivers,
    }
}

fn data_class(
    symbol: &str,
    ty: &crate::scan::TypeFacts,
    metrics: &TypeMetrics<'_>,
    policy: &Policy,
    report: &mut Report,
) {
    let id = "rust.data_class";
    if !policy.enabled(id) {
        return;
    }
    let minimum = parameter(policy, id, "minimum_fields");
    let maximum = parameter(policy, id, "maximum_operations");
    report.finding(
        policy,
        id,
        symbol,
        &ty.location,
        "fields and non-accessor operations",
        json!({"fields":ty.field_count,"operations":metrics.operations}),
        "fields >= and operations <=",
        json!({"minimum_fields":minimum,"maximum_operations":maximum}),
        ty.field_count as u64 >= minimum && metrics.operations as u64 <= maximum,
        json!({"accessor_definition":"strict_source_shape","enum_fields":"maximum_per_variant"}),
    );
}

fn lazy_class(
    symbol: &str,
    ty: &crate::scan::TypeFacts,
    metrics: &TypeMetrics<'_>,
    policy: &Policy,
    report: &mut Report,
) {
    let id = "rust.lazy_class";
    if !policy.enabled(id) {
        return;
    }
    let fields = parameter(policy, id, "maximum_fields");
    let count = parameter(policy, id, "maximum_functions");
    let code = parameter(policy, id, "maximum_lines");
    report.finding(
        policy,
        id,
        symbol,
        &ty.location,
        "fields/functions/code lines",
        json!({"fields":ty.field_count,"functions":metrics.functions.len(),"lines":metrics.lines}),
        "all <=",
        json!({"maximum_fields":fields,"maximum_functions":count,"maximum_lines":code}),
        ty.field_count as u64 <= fields
            && metrics.functions.len() as u64 <= count
            && metrics.lines as u64 <= code,
        json!({"scope":"source_authored_members"}),
    );
}

fn forwarding_share(
    symbol: &str,
    ty: &crate::scan::TypeFacts,
    metrics: &TypeMetrics<'_>,
    policy: &Policy,
    report: &mut Report,
) {
    let id = "rust.forwarding_share";
    if !policy.enabled(id) || metrics.receivers.is_empty() {
        return;
    }
    let forward = metrics
        .receivers
        .iter()
        .filter(|function| forwarding(function))
        .count();
    let minimum = parameter(policy, id, "minimum_methods");
    let share = parameter(policy, id, "minimum_share_percent");
    report.finding(
        policy,
        id,
        symbol,
        &ty.location,
        "syntactic forwarding share",
        json!({"forwarding":forward,"methods":metrics.receivers.len()}),
        "methods and share >=",
        json!({"minimum_methods":minimum,"minimum_share_percent":share}),
        metrics.receivers.len() as u64 >= minimum
            && 100_u128 * forward as u128 >= share as u128 * metrics.receivers.len() as u128,
        json!({"resolved_type_equivalence":"not_checked"}),
    );
}

fn field_uses(function: &Function) -> BTreeSet<String> {
    let mut used = FieldUses::default();
    if let Some(body) = &function.body {
        used.visit_block(body);
    }
    used.fields
}

fn temporary_fields(
    symbol: &str,
    ty: &crate::scan::TypeFacts,
    metrics: &TypeMetrics<'_>,
    policy: &Policy,
    report: &mut Report,
) {
    let id = "rust.temporary_fields";
    if !policy.enabled(id) || ty.is_enum || metrics.receivers.is_empty() {
        return;
    }
    let uses: Vec<_> = metrics.receivers.iter().map(|f| field_uses(f)).collect();
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
            100_u128 * (*count as u128) <= maximum as u128 * metrics.receivers.len() as u128
        })
        .collect();
    report.finding(policy,id,symbol,&ty.location,"low-use optional syntax fields",json!({"fields":fields.len(),"methods":metrics.receivers.len()}),"field/method minima and use <=",
        json!({"minimum_fields":min_fields,"minimum_methods":min_methods,"maximum_use_percent":maximum}),fields.len() as u64>=min_fields && metrics.receivers.len() as u64>=min_methods,
        json!({"field_use_counts":fields,"option_interpretation":"spelled_syntax","access_scope":"direct_receiver_projections"}));
}

fn types(facts: &Facts, policy: &Policy, report: &mut Report) {
    for (symbol, ty) in &facts.types {
        let metrics = type_metrics(ty, facts);
        data_class(symbol, ty, &metrics, policy, report);
        lazy_class(symbol, ty, &metrics, policy, report);
        forwarding_share(symbol, ty, &metrics, policy, report);
        temporary_fields(symbol, ty, &metrics, policy, report);
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
            report.error(error);
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
        report.finding_with_relations(policy,id,&first.symbol,&first.location,"repeated named syntax group",json!({"slots":group.len(),"declarations":populations.len()}),"both >=",
            json!({"minimum_group_size":size,"minimum_declarations":minimum}),true,
            FindingRelations {
                evidence: json!({"slots":group,"supporting_symbols":populations.iter().map(|s|&s.symbol).collect::<Vec<_>>(),"type_interpretation":"exact_syntax"}),
                symbols: populations
                    .iter()
                    .skip(1)
                    .map(|slots| slots.symbol.clone())
                    .collect(),
                locations: populations
                    .iter()
                    .skip(1)
                    .map(|slots| slots.location.clone())
                    .collect(),
            },
        );
    }
}

fn rust_keyword(name: &str) -> bool {
    matches!(
        name,
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
    )
}

fn simple_literal_tag(literal: &Lit) -> Option<&'static str> {
    match literal {
        Lit::Str(_) => Some("str"),
        Lit::ByteStr(_) => Some("bytes"),
        Lit::CStr(_) => Some("cstr"),
        Lit::Char(_) => Some("char"),
        Lit::Byte(_) => Some("byte"),
        _ => None,
    }
}

fn numeric_literal_tag(literal: &Lit) -> Option<String> {
    match literal {
        Lit::Int(value) => Some(format!("int:{}", value.suffix())),
        Lit::Float(value) => Some(format!("float:{}", value.suffix())),
        _ => None,
    }
}

fn literal_tag(literal: &proc_macro2::Literal) -> String {
    let Ok(literal) = syn::parse_str::<Lit>(&literal.to_string()) else {
        return "other".into();
    };
    simple_literal_tag(&literal)
        .map(str::to_string)
        .or_else(|| numeric_literal_tag(&literal))
        .unwrap_or_else(|| "other".into())
}

fn normalize_token(token: TokenTree, result: &mut Vec<String>) {
    match token {
        TokenTree::Group(group) => {
            result.push(format!("open:{:?}", group.delimiter()));
            normalized(group.stream(), result);
            result.push(format!("close:{:?}", group.delimiter()));
        }
        TokenTree::Punct(punct) => result.push(format!("punct:{}", punct.as_char())),
        TokenTree::Ident(ident) => {
            let name = ident.to_string();
            result.push(if rust_keyword(&name) {
                format!("keyword:{name}")
            } else {
                "identifier".into()
            });
        }
        TokenTree::Literal(literal) => result.push(format!("literal:{}", literal_tag(&literal))),
    }
}

fn normalized(stream: TokenStream, result: &mut Vec<String>) {
    for token in stream {
        normalize_token(token, result);
    }
}

fn normalized_tokens(function: &Function) -> Vec<String> {
    let mut tokens = vec![];
    if let Some(body) = &function.body {
        for stmt in &body.stmts {
            normalized(stmt.to_token_stream(), &mut tokens);
        }
    }
    tokens
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

fn duplicate_functions(facts: &Facts) -> Vec<&Function> {
    let mut declarations = BTreeSet::new();
    facts
        .functions
        .iter()
        .filter(|function| function.body.is_some())
        .filter(|function| {
            declarations.insert((
                &function.location.path,
                function.location.line,
                function.location.column,
            ))
        })
        .collect()
}

fn overlapping_bodies(first: &Function, second: &Function) -> bool {
    first.location.path == second.location.path
        && matches!(
            (first.body_bytes, second.body_bytes),
            (Some((first_start, first_end)), Some((second_start, second_end)))
                if first_start < second_end && second_start < first_end
        )
}

fn duplicate_thresholds(policy: &Policy) -> Option<(u64, u64)> {
    ["rust.duplicate_functions", "rust.alternative_interfaces"]
        .into_iter()
        .filter(|id| policy.enabled(id))
        .map(|id| {
            (
                parameter(policy, id, "minimum_tokens"),
                parameter(policy, id, "minimum_similarity_basis_points"),
            )
        })
        .reduce(|left, right| (left.0.min(right.0), left.1.min(right.1)))
}

fn ordered_pair<'a>(
    first: &'a Function,
    second: &'a Function,
    first_size: usize,
    second_size: usize,
) -> (&'a Function, &'a Function, [usize; 2]) {
    if first.symbol <= second.symbol {
        (first, second, [first_size, second_size])
    } else {
        (second, first, [second_size, first_size])
    }
}

fn duplicate_rule_applies(
    id: &str,
    alternative: &str,
    first: &Function,
    second: &Function,
) -> bool {
    id != alternative
        || (first.owner.is_some()
            && second.owner.is_some()
            && first.owner != second.owner
            && interface(first) != interface(second))
}

struct DuplicateEvidence {
    token_counts: [usize; 2],
    intersection: usize,
    union: usize,
}

fn report_duplicate(
    policy: &Policy,
    id: &str,
    first: &Function,
    second: &Function,
    evidence: &DuplicateEvidence,
    report: &mut Report,
) {
    let minimum = parameter(policy, id, "minimum_tokens");
    let similarity = parameter(policy, id, "minimum_similarity_basis_points");
    if evidence.token_counts[0] as u64 >= minimum
        && evidence.token_counts[1] as u64 >= minimum
        && evidence.intersection as u128 * 10_000 >= similarity as u128 * evidence.union as u128
    {
        report.finding_with_relations(policy,id,&first.symbol,&first.location,"normalized 4-token multiset Jaccard",json!({"intersection":evidence.intersection,"union":evidence.union}),">=",
            json!({"minimum_similarity_basis_points":similarity,"minimum_tokens":minimum}),true,
            FindingRelations {
                evidence: json!({"token_counts":evidence.token_counts,"other_location":second.location,"normalization":"syntax_tokens_v1_not_semantic_equivalence"}),
                symbols: vec![second.symbol.clone()],
                locations: vec![second.location.clone()],
            });
    }
}

fn compare_duplicates(
    first: &Function,
    second: &Function,
    tokens: [usize; 2],
    intersection: usize,
    union: usize,
    policy: &Policy,
    report: &mut Report,
) {
    let (first, second, token_counts) = ordered_pair(first, second, tokens[0], tokens[1]);
    let evidence = DuplicateEvidence {
        token_counts,
        intersection,
        union,
    };
    for id in ["rust.duplicate_functions", "rust.alternative_interfaces"] {
        if policy.enabled(id)
            && duplicate_rule_applies(id, "rust.alternative_interfaces", first, second)
        {
            report_duplicate(policy, id, first, second, &evidence, report);
        }
    }
}

fn duplicates(facts: &Facts, policy: &Policy, report: &mut Report) {
    let Some((minimum, similarity)) = duplicate_thresholds(policy) else {
        return;
    };
    let functions = duplicate_functions(facts);
    let token_lists = functions
        .iter()
        .map(|function| normalized_tokens(function))
        .collect::<Vec<_>>();
    metrics::add(
        Counter::NormalizedTokens,
        token_lists.iter().map(Vec::len).sum(),
    );
    let pairs = exact_jaccard_pairs(
        &token_lists,
        minimum,
        similarity,
        policy.limits.maximum_pairs,
        |left, right| overlapping_bodies(functions[left], functions[right]),
    );
    let pairs = match pairs {
        Ok(pairs) => pairs,
        Err(error) => {
            for rule_id in ["rust.duplicate_functions", "rust.alternative_interfaces"] {
                if policy.enabled(rule_id) {
                    report.rule_error(rule_id, error.clone());
                }
            }
            return;
        }
    };
    for pair in pairs {
        compare_duplicates(
            functions[pair.left],
            functions[pair.right],
            [token_lists[pair.left].len(), token_lists[pair.right].len()],
            pair.intersection,
            pair.union,
            policy,
            report,
        );
    }
}

fn variant_path(pattern: &Pat) -> Option<&Path> {
    match pattern {
        Pat::Path(p) => Some(&p.path),
        Pat::Struct(p) => Some(&p.path),
        Pat::TupleStruct(p) => Some(&p.path),
        _ => None,
    }
}

fn push_variant_path(path: &Path, paths: &mut Vec<(Vec<String>, String)>) {
    let mut segments: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    if segments.len() > 1 {
        let variant = segments.pop().expect("qualified path has a variant");
        paths.push((segments, variant));
    }
}

fn variant_paths(pattern: &Pat, paths: &mut Vec<(Vec<String>, String)>) {
    if let Pat::Or(pattern) = pattern {
        for case in &pattern.cases {
            variant_paths(case, paths);
        }
    } else if let Some(path) = variant_path(pattern) {
        push_variant_path(path, paths);
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
        report.finding_with_relations(policy,id,&owner,&first.location,"distinct named callable dispatch sites",json!(support.len()),">=",json!(sites),support.len() as u64>=sites,
            FindingRelations {
                evidence: json!({"minimum_arms":arms,"callables":callables.iter().map(|function|&function.symbol).collect::<Vec<_>>(),"scope":"resolved_local_enum_variant_patterns_not_scrutinee_type_inference"}),
                symbols: callables
                    .iter()
                    .skip(1)
                    .map(|function| function.symbol.clone())
                    .collect(),
                locations: callables
                    .iter()
                    .skip(1)
                    .map(|function| function.location.clone())
                    .collect(),
            },
        );
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
