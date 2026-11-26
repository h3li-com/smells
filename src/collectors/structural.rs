use super::{
    common::observation,
    contracts,
    model::{self, ClassFact, CodeScope, FieldFact, FunctionFact, SourceModel},
};
use crate::{
    core::model::Input,
    coverage::LineCoverage,
    evidence::Observation,
    policy::Rule,
    report::Location,
    similarity::{SimilarPair, exact_jaccard_pairs},
};
use regex::Regex;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};
use tree_sitter::Node;

fn normalized_tokens(source: &str) -> Vec<String> {
    static TOKEN: OnceLock<Regex> = OnceLock::new();
    let token = TOKEN.get_or_init(|| {
        Regex::new(r"[A-Za-z_][A-Za-z0-9_]*|\d+(?:\.\d+)?|==|!=|<=|>=|=>|&&|\|\||[^\s]")
            .expect("valid normalized-token expression")
    });
    token
        .find_iter(source)
        .map(|found| normalized_token(found.as_str()).into())
        .collect()
}

fn normalized_token(value: &str) -> &str {
    const KEYWORDS: [&str; 19] = [
        "if", "else", "elif", "for", "while", "match", "case", "switch", "return", "throw",
        "raise", "await", "yield", "and", "or", "not", "in", "is", "new",
    ];
    match value.chars().next() {
        Some(first) if first.is_alphabetic() || first == '_' => {
            if KEYWORDS.contains(&value) {
                value
            } else {
                "identifier"
            }
        }
        Some(first) if first.is_ascii_digit() => "number",
        _ => value,
    }
}

/// Python data-model special methods. The language fixes their names, so
/// implementing different ones (`__bool__` versus `__hash__`) is a protocol choice,
/// not an alternative interface. Library protocol hooks such as `__rich_console__`
/// are not listed and still count.
fn language_special_method(name: &str) -> bool {
    const SPECIAL: [&str; 66] = [
        "new",
        "init",
        "del",
        "repr",
        "str",
        "bytes",
        "format",
        "lt",
        "le",
        "eq",
        "ne",
        "gt",
        "ge",
        "hash",
        "bool",
        "getattr",
        "getattribute",
        "setattr",
        "delattr",
        "dir",
        "get",
        "set",
        "delete",
        "set_name",
        "init_subclass",
        "class_getitem",
        "call",
        "len",
        "length_hint",
        "getitem",
        "setitem",
        "delitem",
        "missing",
        "iter",
        "reversed",
        "contains",
        "next",
        "enter",
        "exit",
        "aenter",
        "aexit",
        "aiter",
        "anext",
        "await",
        "neg",
        "pos",
        "abs",
        "invert",
        "complex",
        "int",
        "float",
        "index",
        "round",
        "trunc",
        "floor",
        "ceil",
        "copy",
        "deepcopy",
        "reduce",
        "reduce_ex",
        "getstate",
        "setstate",
        "getnewargs",
        "sizeof",
        "fspath",
        "post_init",
    ];
    const OPERATORS: [&str; 13] = [
        "add", "sub", "mul", "matmul", "truediv", "floordiv", "mod", "divmod", "pow", "lshift",
        "rshift", "and", "xor",
    ];
    let Some(inner) = name
        .strip_prefix("__")
        .and_then(|rest| rest.strip_suffix("__"))
    else {
        return false;
    };
    let operator = |value: &str| value == "or" || OPERATORS.contains(&value);
    SPECIAL.contains(&inner)
        || operator(inner)
        || inner.strip_prefix(['r', 'i']).is_some_and(operator)
}

/// Each class's effective method-name interface: its own methods and those it
/// inherits from authored bases, so subclasses that override different subsets of
/// one base still expose that one interface.
fn class_interfaces(model: &SourceModel) -> Vec<BTreeSet<&str>> {
    model
        .classes
        .iter()
        .map(|class| {
            let mut names = BTreeSet::new();
            let mut visited = BTreeSet::new();
            let mut pending = vec![class];
            while let Some(current) = pending.pop() {
                if !visited.insert(current.symbol.as_str()) {
                    continue;
                }
                names.extend(
                    current
                        .methods
                        .iter()
                        .map(|method| method.name.as_str())
                        .filter(|name| !language_special_method(name)),
                );
                pending.extend(current.bases.iter().filter_map(|base| {
                    contracts::resolve_declared_base(model, current, base, false)
                }));
            }
            names
        })
        .collect()
}

/// Each class's concrete method bodies as one normalized token sequence, in source
/// order. A boundary token keeps a 4-token shingle from spanning two methods.
fn class_token_lists(model: &SourceModel) -> Vec<Vec<String>> {
    model
        .classes
        .iter()
        .map(|class| {
            let mut tokens = Vec::new();
            for body in class
                .methods
                .iter()
                .filter_map(|method| method.body.as_deref())
            {
                if !tokens.is_empty() {
                    tokens.push("method-boundary".into());
                }
                tokens.extend(normalized_tokens(body));
            }
            tokens
        })
        .collect()
}

fn interface_observation(
    model: &SourceModel,
    token_lists: &[Vec<String>],
    pair: &SimilarPair,
) -> Observation {
    let first = &model.classes[pair.left];
    let second = &model.classes[pair.right];
    let mut candidate = observation(
        first.symbol.clone(),
        first.location.clone(),
        [
            ("first_tokens", token_lists[pair.left].len() as u64),
            ("second_tokens", token_lists[pair.right].len() as u64),
            ("intersection", pair.intersection as u64),
            ("union", pair.union as u64),
        ],
        json!({
            "collector": "authored_class_behavior_similarity_v2",
            "normalization": "code-only lexical tokens; identifiers normalized; 4-token shingle multiset Jaccard",
            "candidate_filter": "exact_jaccard_prefix_and_length_v1",
        }),
    );
    candidate.related_symbols.push(second.symbol.clone());
    candidate.related_locations.push(second.location.clone());
    candidate
}

/// Classes whose method bodies share at least the configured share of normalized
/// 4-token shingles while exposing different method-name interfaces. Ordered
/// shingles, not a bag of single tokens, so two merely similar-sized classes no
/// longer match on punctuation and identifier counts alone.
pub(super) fn alternative_interfaces(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    let minimum = rule.parameters["minimum_tokens"];
    let similarity = rule.parameters["minimum_similarity_basis_points"];
    let interfaces = class_interfaces(model);
    let token_lists = class_token_lists(model);
    let pairs = exact_jaccard_pairs(
        &token_lists,
        minimum,
        similarity,
        input.policy.limits.maximum_pairs,
        |left, right| {
            interfaces[left].is_empty()
                || interfaces[right].is_empty()
                || interfaces[left] == interfaces[right]
        },
    )?;
    Ok(pairs
        .iter()
        .map(|pair| interface_observation(model, &token_lists, pair))
        .collect())
}

fn dispatch_node(kind: &str, language: &str) -> bool {
    match language {
        "python" => kind == "match_statement",
        "typescript" => kind == "switch_statement",
        _ => false,
    }
}

fn compact_label(node: Node<'_>, source: &str) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or_default()
        .split_whitespace()
        .collect()
}

/// The authored case labels of one `switch`/`match`, read from the raw source so
/// string labels keep their values, sorted so arm order does not split a dispatch.
fn case_labels(dispatch: Node<'_>, source: &str, language: &str) -> Vec<String> {
    let Some(body) = dispatch.child_by_field_name("body") else {
        return Vec::new();
    };
    let mut cursor = body.walk();
    let mut labels = body
        .named_children(&mut cursor)
        .filter_map(|arm| match (language, arm.kind()) {
            ("typescript", "switch_case") => arm
                .child_by_field_name("value")
                .map(|value| compact_label(value, source)),
            ("python", "case_clause") => {
                let mut patterns = arm.walk();
                let label = arm
                    .named_children(&mut patterns)
                    .filter(|pattern| pattern.kind() == "case_pattern")
                    .map(|pattern| compact_label(pattern, source))
                    .collect::<Vec<_>>()
                    .join(",");
                Some(label)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    labels.sort();
    labels
}

/// Every `switch`/`match` with its labels, attributed once to its innermost enclosing
/// function as `(line, column)` of that function's declaration.
fn dispatch_sites(
    node: Node<'_>,
    source: &str,
    language: &str,
    owner: Option<(usize, usize)>,
    sites: &mut Vec<((usize, usize), Vec<String>)>,
) {
    let owner = if model::function_node(node.kind(), language) {
        let start = node.start_position();
        Some((start.row + 1, start.column + 1))
    } else {
        owner
    };
    if let Some(owner) = owner.filter(|_| dispatch_node(node.kind(), language)) {
        let labels = case_labels(node, source, language);
        if !labels.is_empty() {
            sites.push((owner, labels));
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        dispatch_sites(child, source, language, owner, sites);
    }
}

type FunctionsByStart<'model> = BTreeMap<(&'model str, usize, usize), &'model FunctionFact>;

fn file_dispatch_sites<'model>(
    functions: &FunctionsByStart<'model>,
    language: &str,
    path: &str,
    source: &str,
) -> Result<Vec<(&'model FunctionFact, Vec<String>)>, String> {
    let tree = model::parse_portable_tree(language, path, source)?;
    let mut sites = Vec::new();
    dispatch_sites(tree.root_node(), source, language, None, &mut sites);
    Ok(sites
        .into_iter()
        .filter_map(|((line, column), labels)| {
            Some((*functions.get(&(path, line, column))?, labels))
        })
        .collect())
}

fn dispatch_observation(labels: &[String], functions: &[&FunctionFact]) -> Option<Observation> {
    let first = functions.first()?;
    let mut candidate = observation(
        first.symbol.clone(),
        first.location.clone(),
        [
            ("sites", functions.len() as u64),
            ("arms_per_site", labels.len() as u64),
        ],
        json!({
            "collector": "authored_repeated_case_labels_v2",
            "case_labels": labels,
            "scope": "one site per innermost function holding a switch/match with the same sorted authored case labels",
        }),
    );
    for related in functions.iter().skip(1) {
        candidate.related_symbols.push(related.symbol.clone());
        candidate.related_locations.push(related.location.clone());
    }
    Some(candidate)
}

/// Functions whose own `switch`/`match` statements carry the same set of authored
/// case labels. Each statement counts once, for its innermost enclosing function,
/// and a function with several identical statements is one site.
pub(super) fn repeated_dispatch(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    let functions = model
        .functions
        .iter()
        .map(|function| {
            let location = &function.location;
            (
                (location.path.as_str(), location.line, location.column),
                function,
            )
        })
        .collect::<FunctionsByStart<'_>>();
    let mut grouped: BTreeMap<Vec<String>, Vec<&FunctionFact>> = BTreeMap::new();
    for path in model.code_by_path.keys() {
        let Some(source) = input.files.get(path) else {
            continue;
        };
        for (function, labels) in file_dispatch_sites(&functions, language, path, source)? {
            let sites = grouped.entry(labels).or_default();
            if !sites.iter().any(|site| std::ptr::eq(*site, function)) {
                sites.push(function);
            }
        }
    }
    Ok(grouped
        .into_iter()
        .filter_map(|(labels, functions)| dispatch_observation(&labels, &functions))
        .collect())
}

fn lifecycle_methods(class: &ClassFact) -> Vec<&FunctionFact> {
    class
        .methods
        .iter()
        // A constructor initializes every field, so it never shows lifecycle use.
        .filter(|method| {
            method.body.is_some() && !matches!(method.name.as_str(), "__init__" | "constructor")
        })
        .collect()
}

fn field_uses(receiver: &str, field: &FieldFact, methods: &[&FunctionFact]) -> usize {
    let access = Regex::new(&format!(
        r"\b{}\s*\.\s*{}\b",
        regex::escape(receiver),
        regex::escape(&field.name)
    ))
    .expect("escaped temporary-field access expression");
    methods
        .iter()
        .filter(|method| {
            method
                .body
                .as_deref()
                .is_some_and(|body| access.is_match(body))
        })
        .count()
}

fn temporary_field_observation(
    class: &ClassFact,
    receiver: &str,
    own_fields: Option<&BTreeSet<String>>,
) -> Option<Observation> {
    let temporary = class
        .fields
        .iter()
        .filter(|field| field.temporary)
        .filter(|field| own_fields.is_none_or(|names| names.contains(&field.name)))
        .collect::<Vec<_>>();
    let methods = lifecycle_methods(class);
    if temporary.is_empty() || methods.is_empty() {
        return None;
    }
    let uses = temporary
        .iter()
        .map(|field| field_uses(receiver, field, &methods))
        .sum::<usize>();
    Some(observation(
        class.symbol.clone(),
        class.location.clone(),
        [
            ("fields", temporary.len() as u64),
            ("methods", methods.len() as u64),
            ("uses", uses as u64),
            ("possible_uses", (temporary.len() * methods.len()) as u64),
        ],
        json!({
            "collector": "authored_nullable_field_use_v2",
            "fields": temporary.iter().map(|field| &field.name).collect::<Vec<_>>(),
        }),
    ))
}

fn node_text<'source>(node: Node<'_>, source: &'source str) -> &'source str {
    node.utf8_text(source.as_bytes()).unwrap_or_default()
}

fn constructor_parameter_names(method: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
    let Some(parameters) = method.child_by_field_name("parameters") else {
        return;
    };
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        if let Some(name) = parameter
            .child_by_field_name("name")
            .or_else(|| parameter.child_by_field_name("pattern"))
        {
            names.insert(node_text(name, source).into());
        }
    }
}

/// The names a TypeScript class itself declares: direct property declarations,
/// constructor parameters (parameter properties among them), and `this.name = ...`
/// assignments. Members of other declarations, inline object types, parameter
/// lists, and object literals nested inside a member are not the class's fields.
fn typescript_own_field_names(class: Node<'_>, source: &str) -> BTreeSet<String> {
    static ASSIGNMENT: OnceLock<Regex> = OnceLock::new();
    let assignment = ASSIGNMENT.get_or_init(|| {
        Regex::new(r"\bthis\s*\.\s*([A-Za-z_$][A-Za-z0-9_$]*)\s*=[^=]")
            .expect("valid receiver assignment expression")
    });
    let mut names = assignment
        .captures_iter(node_text(class, source))
        .map(|capture| capture[1].to_string())
        .collect::<BTreeSet<_>>();
    let body = class.child_by_field_name("body").unwrap_or(class);
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
        match member.kind() {
            "public_field_definition" | "property_signature" | "abstract_property_signature" => {
                if let Some(name) = member.child_by_field_name("name") {
                    names.insert(node_text(name, source).trim_start_matches('#').into());
                }
            }
            "method_definition"
                if member
                    .child_by_field_name("name")
                    .is_some_and(|name| node_text(name, source) == "constructor") =>
            {
                constructor_parameter_names(member, source, &mut names);
            }
            _ => {}
        }
    }
    names
}

fn class_nodes_by_range<'tree>(
    node: Node<'tree>,
    ranges: &BTreeSet<(usize, usize)>,
    found: &mut BTreeMap<(usize, usize), Node<'tree>>,
) {
    let range = (node.start_byte(), node.end_byte());
    if ranges.contains(&range) && model::class_node(node.kind(), "typescript") {
        found.insert(range, node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        class_nodes_by_range(child, ranges, found);
    }
}

/// Own field names keyed by class path and byte range.
type OwnFieldsByClass = BTreeMap<(String, (usize, usize)), BTreeSet<String>>;

/// Own field names of each TypeScript class that has a temporary-field candidate,
/// keyed by class symbol and byte range. Files without candidates are not reparsed.
fn typescript_own_fields(model: &SourceModel, input: &Input) -> Result<OwnFieldsByClass, String> {
    let mut ranges = BTreeMap::<&str, BTreeSet<(usize, usize)>>::new();
    for class in &model.classes {
        if class.fields.iter().any(|field| field.temporary) {
            ranges
                .entry(class.location.path.as_str())
                .or_default()
                .insert(class.range);
        }
    }
    let mut own = BTreeMap::new();
    for (path, ranges) in ranges {
        let Some(source) = input.files.get(path) else {
            continue;
        };
        let tree = model::parse_portable_tree("typescript", path, source)?;
        let mut found = BTreeMap::new();
        class_nodes_by_range(tree.root_node(), &ranges, &mut found);
        for (range, node) in found {
            own.insert(
                (path.to_string(), range),
                typescript_own_field_names(node, source),
            );
        }
    }
    Ok(own)
}

pub(super) fn temporary_fields(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    let receiver = if language == "python" { "self" } else { "this" };
    let own = if language == "typescript" {
        Some(typescript_own_fields(model, input)?)
    } else {
        None
    };
    Ok(model
        .classes
        .iter()
        .filter_map(|class| {
            let names = own.as_ref().map(|own| {
                own.get(&(class.location.path.clone(), class.range))
                    .cloned()
                    .unwrap_or_default()
            });
            temporary_field_observation(class, receiver, names.as_ref())
        })
        .collect())
}

fn forwarding_method(body: &str, language: &str) -> bool {
    let receiver = if language == "python" { "self" } else { "this" };
    let compact = body
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim()
        .trim_end_matches(';')
        .trim();
    let compact = compact.split_whitespace().collect::<Vec<_>>().join(" ");
    let expression = Regex::new(&format!(
        r"(?s)^(?:return\s+)?{receiver}\s*\.\s*[A-Za-z_][A-Za-z0-9_]*\s*\.\s*[A-Za-z_][A-Za-z0-9_]*\([^;{{}}]*\)$"
    ))
    .expect("valid forwarding expression");
    expression.is_match(&compact)
}

pub(super) fn forwarding_share(rule: &Rule, model: &SourceModel) -> Vec<Observation> {
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    model
        .classes
        .iter()
        .filter_map(|class| {
            let methods = class
                .methods
                .iter()
                .filter(|method| {
                    method.body.is_some()
                        && !matches!(method.name.as_str(), "__init__" | "constructor")
                })
                .collect::<Vec<_>>();
            if methods.is_empty() {
                return None;
            }
            let forwarders = methods
                .iter()
                .filter(|method| {
                    method
                        .body
                        .as_deref()
                        .is_some_and(|body| forwarding_method(body, language))
                })
                .count();
            Some(observation(
                class.symbol.clone(),
                class.location.clone(),
                [
                    ("methods", methods.len() as u64),
                    ("forwarders", forwarders as u64),
                ],
                json!({"collector": "authored_single_delegate_method_v1"}),
            ))
        })
        .collect()
}

pub(super) fn function_crap(
    model: &SourceModel,
    coverage: Option<&LineCoverage>,
) -> Vec<Observation> {
    model
        .functions
        .iter()
        .filter_map(|function| {
            let complexity = function.complexity?;
            let measured = coverage.and_then(|coverage| {
                let (covered, instrumented) = coverage.span(
                    &function.location.path,
                    function.body_line,
                    function.end_line,
                )?;
                Some((covered_crap(complexity, covered, instrumented)?, covered, instrumented, &coverage.sha256))
            });
            let (numerator, denominator, evidence) = match measured {
                Some(((numerator, denominator), covered, instrumented, sha256)) => (
                    numerator,
                    denominator,
                    json!({
                        "collector": "lcov_line_coverage_crap_v1",
                        "cyclomatic_complexity": complexity,
                        "covered_lines": covered,
                        "instrumented_lines": instrumented,
                        "coverage_sha256": sha256,
                        "interpretation": "CRAP from supplied LCOV line coverage over the function body",
                    }),
                ),
                None => (
                    complexity * complexity + complexity,
                    1,
                    json!({
                        "collector": "authored_zero_coverage_crap_upper_bound_v1",
                        "cyclomatic_complexity": complexity,
                        "coverage_assumption_percent": 0,
                        "interpretation": "conservative upper bound when no complete coverage artifact is supplied",
                    }),
                ),
            };
            observation(
                function.symbol.clone(),
                function.location.clone(),
                [("numerator", numerator), ("denominator", denominator)],
                evidence,
            )
            .into()
        })
        .collect()
}

/// Exact `CC^2 * (1 - c)^3 + CC` for line coverage `c = covered / instrumented`, as a
/// reduced rational. `None` asks the caller to keep the zero-coverage upper bound.
fn covered_crap(complexity: u64, covered: u64, instrumented: u64) -> Option<(u64, u64)> {
    let (complexity, total) = (u128::from(complexity), u128::from(instrumented));
    let uncovered = total - u128::from(covered);
    let numerator = complexity
        .checked_mul(complexity)?
        .checked_mul(uncovered.checked_pow(3)?)?
        .checked_add(complexity.checked_mul(total.checked_pow(3)?)?)?;
    let denominator = total.pow(3);
    let divisor = gcd(numerator, denominator);
    Some((
        u64::try_from(numerator / divisor).ok()?,
        u64::try_from(denominator / divisor).ok()?,
    ))
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn corpus_occurrences(model: &SourceModel, name: &str) -> usize {
    let expression = Regex::new(&format!(r"\b{}\b", regex::escape(name)))
        .expect("escaped identifier expression");
    model
        .code_by_path
        .values()
        .map(|source| expression.find_iter(source).count())
        .sum()
}

/// A private declaration is unused when nothing reaches it: no second corpus
/// occurrence, no attribute path naming it, and no harness invoking it.
fn unreferenced(model: &SourceModel, private: bool, name: &str) -> bool {
    private
        && !name.starts_with("__")
        && !model.attribute_references.contains(name)
        && corpus_occurrences(model, name) == 1
}

pub(super) fn unused_code(model: &SourceModel) -> Vec<Observation> {
    let functions = model
        .functions
        .iter()
        .filter(|function| !function.entry_point)
        .filter(|function| unreferenced(model, function.private, &function.name))
        .map(|function| (&function.location, &function.symbol, function.scope));
    let classes = model
        .classes
        .iter()
        .filter(|class| unreferenced(model, class.private, &class.name))
        .map(|class| (&class.location, &class.symbol, class.scope));
    // Test declarations group apart from production ones, so a file that mixes both
    // (an inline `#[cfg(test)]` module) keeps a blocking production finding.
    let mut by_file: BTreeMap<(String, CodeScope), (Location, Vec<String>)> = BTreeMap::new();
    for (location, symbol, scope) in functions.chain(classes) {
        by_file
            .entry((location.path.clone(), scope))
            .or_insert_with(|| (location.clone(), Vec::new()))
            .1
            .push(symbol.clone());
    }
    by_file
        .into_iter()
        .map(|((path, scope), (location, symbols))| {
            let test_code = scope == CodeScope::Test;
            let kind = if test_code { "test" } else { "private" };
            observation(
                format!("{path}::unused-{kind}-declarations"),
                location,
                [("findings", symbols.len() as u64)],
                json!({
                    "collector": "authored_private_declaration_references_v2",
                    "symbols": symbols,
                    "test_code": test_code,
                    "scope": "private declarations with no corpus, attribute-path, or harness reference; Rust `_` names, dead_code allowances, exports, and protocol hooks are not private",
                }),
            )
        })
        .collect()
}

pub(super) fn unused_type_parameters(model: &SourceModel) -> Vec<Observation> {
    let mut observations = Vec::new();
    for function in &model.functions {
        let unused = function
            .type_parameters
            .iter()
            .filter(|parameter| {
                Regex::new(&format!(r"\b{}\b", regex::escape(parameter)))
                    .expect("escaped type parameter")
                    .find_iter(&function.source)
                    .count()
                    == 1
            })
            .cloned()
            .collect::<Vec<_>>();
        if !unused.is_empty() {
            observations.push(observation(
                function.symbol.clone(),
                function.location.clone(),
                [("findings", unused.len() as u64)],
                json!({
                    "collector": "authored_generic_parameter_use_v1",
                    "unused_parameters": unused,
                }),
            ));
        }
    }
    for class in &model.classes {
        let unused = class
            .type_parameters
            .iter()
            .filter(|parameter| {
                Regex::new(&format!(r"\b{}\b", regex::escape(parameter)))
                    .expect("escaped type parameter")
                    .find_iter(&class.source)
                    .count()
                    == 1
            })
            .cloned()
            .collect::<Vec<_>>();
        if !unused.is_empty() {
            observations.push(observation(
                class.symbol.clone(),
                class.location.clone(),
                [("findings", unused.len() as u64)],
                json!({
                    "collector": "authored_generic_parameter_use_v1",
                    "unused_parameters": unused,
                }),
            ));
        }
    }
    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn class_nodes_by_range_finds_only_requested_classes() {
        let source = "class A { x?: number }\nclass B { y?: number }\n";
        let tree = model::parse_portable_tree("typescript", "a.ts", source).expect("parses");
        let b = (source.find("class B").expect("class B"), source.len() - 1);
        let mut found = BTreeMap::new();
        class_nodes_by_range(tree.root_node(), &BTreeSet::from([b]), &mut found);
        assert_eq!(found.keys().copied().collect::<Vec<_>>(), [b]);
    }

    #[test]
    fn field_uses_count_each_method_that_reads_the_field() {
        let field = FieldFact {
            name: "cache".into(),
            authored_type: "dict".into(),
            temporary: true,
            private: false,
        };
        let reads = function("reads", Some("return self.cache"));
        let writes = function("writes", Some("self.cache = {}"));
        let other = function("other", Some("return self.cached"));
        assert_eq!(field_uses("self", &field, &[&reads, &writes, &other]), 2);
        assert_eq!(field_uses("self", &field, &[&other]), 0);
    }

    #[test]
    fn python_case_labels_come_from_case_patterns() {
        let source = "def f(x):\n    match x:\n        case 1:\n            pass\n        case \"a\" | \"b\":\n            pass\n";
        let tree = model::parse_portable_tree("python", "a.py", source).expect("parses");
        let mut sites = Vec::new();
        dispatch_sites(tree.root_node(), source, "python", None, &mut sites);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].1, ["\"a\"|\"b\"", "1"]);
    }

    #[test]
    fn dispatch_nodes_are_language_specific() {
        assert!(dispatch_node("match_statement", "python"));
        assert!(dispatch_node("switch_statement", "typescript"));
        assert!(!dispatch_node("if_statement", "python"));
        assert!(!dispatch_node("if_statement", "typescript"));
        assert!(!dispatch_node("switch_statement", "rust"));
    }

    #[test]
    fn typescript_own_fields_take_constructor_parameters_only() {
        let source =
            "class A {\n  x = 1;\n  constructor(private a: number) {}\n  m(b: number) {}\n}\n";
        let tree = model::parse_portable_tree("typescript", "a.ts", source).expect("parses");
        let class = tree.root_node().named_child(0).expect("class");
        let names = typescript_own_field_names(class, source);
        assert_eq!(names.into_iter().collect::<Vec<_>>(), ["a", "x"]);
    }

    #[test]
    fn class_token_lists_separate_method_bodies() {
        let model = SourceModel {
            classes: vec![class(
                "A",
                vec![function("a", Some("one")), function("b", Some("two"))],
            )],
            ..SourceModel::default()
        };
        let tokens = class_token_lists(&model).remove(0);
        let boundaries = tokens
            .iter()
            .filter(|token| *token == "method-boundary")
            .count();
        assert_eq!(boundaries, 1);
        assert_ne!(tokens[0], "method-boundary");
    }

    fn location(line: usize) -> Location {
        Location {
            path: "src/domain.rs".into(),
            line,
            column: 1,
        }
    }

    fn function(name: &str, body: Option<&str>) -> FunctionFact {
        FunctionFact {
            name: name.into(),
            symbol: format!("src/domain.rs::{name}"),
            location: location(1),
            body_line: 1,
            end_line: 1,
            type_parameters: Vec::new(),
            body: body.map(Arc::from),
            complexity: body.map(|_| 1),
            source: Arc::from(body.unwrap_or_default()),
            private: false,
            receiver: false,
            entry_point: false,
            scope: CodeScope::Production,
        }
    }

    fn class(name: &str, methods: Vec<FunctionFact>) -> ClassFact {
        ClassFact {
            name: name.into(),
            symbol: format!("src/domain.rs::{name}"),
            location: location(1),
            type_parameters: Vec::new(),
            source: Arc::from(""),
            bases: Vec::new(),
            interface: false,
            private: false,
            fields: Vec::new(),
            methods,
            range: (0, 0),
            scope: CodeScope::Production,
        }
    }

    fn portable_rule(id: &str, parameters: &[(&str, u64)]) -> Rule {
        Rule {
            id: id.into(),
            version: 2,
            smell: "test".into(),
            kind: "indicator".into(),
            implementation: "implemented".into(),
            inputs: Vec::new(),
            parameters: parameters
                .iter()
                .map(|(name, value)| ((*name).into(), *value))
                .collect(),
            contract: "test".into(),
        }
    }

    fn portable_input(rule_pack: &str, files: &[(&str, &str)]) -> Input {
        Input::fixture(
            rule_pack,
            files
                .iter()
                .map(|(path, source)| ((*path).into(), (*source).into()))
                .collect(),
        )
    }

    fn portable_observations(
        collect: fn(&Rule, &SourceModel, &Input) -> Result<Vec<Observation>, String>,
        rule: &Rule,
        input: &Input,
    ) -> Vec<Observation> {
        let model = model::source_model(rule, input).expect("fixture source model");
        collect(rule, &model, input).expect("fixture observations")
    }

    #[test]
    fn language_special_methods_are_protocols_not_interface_names() {
        for special in [
            "__init__", "__eq__", "__hash__", "__bool__", "__radd__", "__ior__",
        ] {
            assert!(language_special_method(special), "{special}");
        }
        for named in ["__rich_console__", "__private", "close", "__rxor", "__ip__"] {
            assert!(!language_special_method(named), "{named}");
        }
    }

    #[test]
    fn alternative_interfaces_pair_ordered_shingles_not_similar_token_bags() {
        let body = "        total = 0\n        for item in self.items:\n            if item.ready:\n                total = total + item.amount\n        return total\n";
        let reordered = "        if self.ready:\n            return 0\n        for item in self.items:\n            total = item.amount + total\n        total = 0\n        return total\n";
        let source = format!(
            "class Ledger:\n    def balance(self):\n{body}\n\
             class Wallet:\n    def funds(self):\n{body}\n\
             class Shuffled:\n    def other(self):\n{reordered}\n\
             class Placeholder:\n    def __bool__(self):\n{body}\n\
             class Identity:\n    def __hash__(self):\n{body}\n"
        );
        let rule = portable_rule(
            "python.alternative_interfaces",
            &[
                ("minimum_tokens", 20),
                ("minimum_similarity_basis_points", 6200),
            ],
        );
        let input = portable_input("python-v1", &[("src/money.py", &source)]);
        let observations = portable_observations(alternative_interfaces, &rule, &input);

        let pairs = observations
            .iter()
            .map(|found| (found.symbol.as_str(), found.related_symbols[0].as_str()))
            .collect::<Vec<_>>();
        assert_eq!(pairs, [("src/money.py::Ledger", "src/money.py::Wallet")]);
        assert_eq!(
            observations[0].measurements["intersection"],
            observations[0].measurements["union"]
        );
    }

    #[test]
    fn alternative_interfaces_compare_inherited_authored_methods() {
        let body = "        total = 0\n        for item in self.items:\n            if item.ready:\n                total = total + item.amount\n        return total\n";
        let source = format!(
            "class Tag:\n    def check(self):\n        return 1\n    def convert(self):\n        return 2\n\
             class TagTuple(Tag):\n    def check(self):\n{body}\n    def convert(self):\n        return 3\n\
             class PassList(Tag):\n    def check(self):\n{body}\n\
             class Loop(Cycle):\n    def funds(self):\n{body}\n\
             class Cycle(Loop):\n    def other(self):\n        return 4\n"
        );
        let rule = portable_rule(
            "python.alternative_interfaces",
            &[
                ("minimum_tokens", 20),
                ("minimum_similarity_basis_points", 6000),
            ],
        );
        let input = portable_input("python-v1", &[("src/tag.py", &source)]);
        let observations = portable_observations(alternative_interfaces, &rule, &input);

        let pairs = observations
            .iter()
            .map(|found| (found.symbol.as_str(), found.related_symbols[0].as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            pairs,
            [
                ("src/tag.py::PassList", "src/tag.py::Loop"),
                ("src/tag.py::Loop", "src/tag.py::TagTuple"),
            ]
        );
    }

    #[test]
    fn repeated_dispatch_groups_real_labels_once_per_innermost_function() {
        let kinds = "case 'add': return 1; case 'remove': return 2; case 'move': return 3; case 'copy': return 4;";
        let levels = "case 'info': return 1; case 'warn': return 2; case 'error': return 3; case 'debug': return 4;";
        let source = format!(
            "export function first(kind: string) {{ switch (kind) {{ {kinds} }} }}\n\
             export function second(kind: string) {{ switch (kind) {{ {kinds} }} }}\n\
             export function outer() {{ return (kind: string) => {{ switch (kind) {{ {kinds} }} }}; }}\n\
             export function log(level: string) {{ switch (level) {{ {levels} }} }}\n"
        );
        let rule = portable_rule("typescript.repeated_dispatch", &[]);
        let input = portable_input("typescript-v1", &[("src/dispatch.ts", &source)]);
        let observations = portable_observations(repeated_dispatch, &rule, &input);

        let shared = observations
            .iter()
            .find(|found| found.evidence["case_labels"][0] == "'add'")
            .expect("shared kind dispatch");
        assert_eq!(shared.measurements["sites"], 3);
        assert_eq!(shared.measurements["arms_per_site"], 4);
        assert!(
            shared
                .related_symbols
                .iter()
                .all(|symbol| !symbol.ends_with("::outer"))
        );
        let logging = observations
            .iter()
            .find(|found| found.evidence["case_labels"][0] == "'debug'")
            .expect("separate level dispatch");
        assert_eq!(logging.measurements["sites"], 1);
    }

    #[test]
    fn typescript_temporary_fields_are_only_the_class_own_declarations() {
        let source = "export class Schema {\n\
             \x20 ready?: string;\n\
             \x20 pending: number | null = null;\n\
             \x20 cache: Map<string, {\n    request?: string\n  }> = new Map();\n\
             \x20 constructor(private retry?: number) {}\n\
             \x20 static create = (\n    params?: string,\n  ) => new Schema({\n    exactLength: null,\n    maxLength: null,\n  });\n\
             \x20 first() { return 1; }\n\
             \x20 second() { return 2; }\n\
             \x20 third() { return this.ready; }\n\
             \x20 fourth() { return 4; }\n\
             }\n";
        let rule = portable_rule("typescript.temporary_fields", &[]);
        let input = portable_input("typescript-v1", &[("src/schema.ts", source)]);
        let model = model::source_model(&rule, &input).expect("fixture source model");
        let observations = temporary_fields(&rule, &model, &input).expect("fixture observations");

        assert_eq!(observations.len(), 1);
        let mut fields = observations[0].evidence["fields"]
            .as_array()
            .expect("field names")
            .iter()
            .map(|name| name.as_str().expect("field name"))
            .collect::<Vec<_>>();
        fields.sort_unstable();
        for nested in ["request", "params", "exactLength", "maxLength"] {
            assert!(!fields.contains(&nested), "{nested} in {fields:?}");
        }
        assert!(fields.contains(&"ready"), "{fields:?}");
        assert!(fields.contains(&"pending"), "{fields:?}");
    }

    #[test]
    fn token_sequences_keep_order_keywords_and_normalized_values() {
        let first = normalized_tokens("if account == 42 { return _account; }");
        let second = normalized_tokens("if customer == 7 { return other; }");
        assert_eq!(first, second);
        assert_eq!(
            first,
            [
                "if",
                "identifier",
                "==",
                "number",
                "{",
                "return",
                "identifier",
                ";",
                "}"
            ]
        );
    }

    #[test]
    fn forwarding_and_complexity_contracts_reject_near_misses() {
        assert!(forwarding_method(
            "return self.client.load(value)",
            "python"
        ));
        assert!(forwarding_method(
            "{ return this.client.load(value); }",
            "typescript"
        ));
        assert!(!forwarding_method("return client.load(value)", "python"));
        assert!(!forwarding_method(
            "return self.client.load(value); self.audit()",
            "python"
        ));
    }

    #[test]
    fn forwarding_share_counts_only_concrete_non_constructor_methods() {
        let model = SourceModel {
            classes: vec![class(
                "Gateway",
                vec![
                    function("constructor", Some("this.client.load(value)")),
                    function("declared_only", None),
                    function("load", Some("this.client.load(value)")),
                ],
            )],
            ..SourceModel::default()
        };
        let rule = Rule {
            id: "typescript.forwarding_share".into(),
            version: 1,
            smell: "middle_man".into(),
            kind: "ratio".into(),
            implementation: "forwarding_share".into(),
            inputs: Vec::new(),
            parameters: BTreeMap::new(),
            contract: "test".into(),
        };

        let observations = forwarding_share(&rule, &model);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["methods"], 1);
        assert_eq!(observations[0].measurements["forwarders"], 1);
    }

    #[test]
    fn temporary_field_observation_counts_the_full_use_matrix() {
        let mut account = class(
            "Account",
            vec![
                function("__init__", Some("self.pending = None")),
                function("first", Some("self.pending = 1")),
                function("second", Some("return self.other")),
                function("abstract", None),
            ],
        );
        account.fields = vec![
            FieldFact {
                name: "pending".into(),
                authored_type: "Optional[int]".into(),
                temporary: true,
                private: false,
            },
            FieldFact {
                name: "other".into(),
                authored_type: "int".into(),
                temporary: false,
                private: false,
            },
        ];
        let model = SourceModel {
            classes: vec![account],
            ..SourceModel::default()
        };
        let rule = Rule {
            id: "python.temporary_fields".into(),
            version: 1,
            smell: "temporary_field".into(),
            kind: "ratio".into(),
            implementation: "temporary_fields".into(),
            inputs: Vec::new(),
            parameters: BTreeMap::new(),
            contract: "test".into(),
        };
        let observations =
            temporary_fields(&rule, &model, &Input::fixture("python-v1", BTreeMap::new()))
                .expect("python temporary fields need no reparse");
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["fields"], 1);
        assert_eq!(observations[0].measurements["methods"], 2);
        assert_eq!(observations[0].measurements["uses"], 1);
        assert_eq!(observations[0].measurements["possible_uses"], 2);
    }

    #[test]
    fn unused_declarations_and_type_parameters_require_exactly_one_corpus_occurrence() {
        let mut hidden = function("_hidden", Some("return 1"));
        hidden.private = true;
        hidden.source = Arc::from("fn _hidden<T>() { return 1; }");
        hidden.type_parameters = vec!["T".into()];
        let mut used = function("_used", Some("return 2"));
        used.private = true;
        let model = SourceModel {
            functions: vec![hidden, used],
            code_by_path: BTreeMap::from([(
                "src/domain.rs".into(),
                Arc::from("fn _hidden() {} fn _used() {} fn call() { _used(); }") as Arc<str>,
            )]),
            ..SourceModel::default()
        };

        let unused = unused_code(&model);
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].measurements["findings"], 1);
        assert_eq!(unused[0].evidence["symbols"][0], "src/domain.rs::_hidden");
        let unused_parameters = unused_type_parameters(&model);
        assert_eq!(unused_parameters.len(), 1);
        assert_eq!(unused_parameters[0].evidence["unused_parameters"][0], "T");
    }

    #[test]
    fn unused_code_skips_harness_entry_points_and_attribute_path_references() {
        let mut entry = function("runs", Some("{}"));
        entry.private = true;
        entry.entry_point = true;
        let mut attribute = function("defaults", Some("{}"));
        attribute.private = true;
        let mut unused = function("orphan", Some("{}"));
        unused.private = true;
        let model = SourceModel {
            functions: vec![entry, attribute, unused],
            attribute_references: BTreeSet::from(["defaults".into()]),
            code_by_path: BTreeMap::from([(
                "src/domain.rs".into(),
                Arc::from("fn runs() {}\nfn defaults() {}\nfn orphan() {}\n") as Arc<str>,
            )]),
            ..SourceModel::default()
        };

        let observations = unused_code(&model);
        assert_eq!(
            observations[0].evidence["symbols"],
            json!(["src/domain.rs::orphan"])
        );
    }

    #[test]
    fn unused_code_applies_the_same_privacy_name_and_reference_contract_to_classes() {
        let mut unused = class("_Unused", Vec::new());
        unused.private = true;
        let mut referenced = class("_Referenced", Vec::new());
        referenced.private = true;
        let mut public = class("Public", Vec::new());
        public.private = false;
        let mut dunder = class("__Generated", Vec::new());
        dunder.private = true;
        let model = SourceModel {
            classes: vec![unused, referenced, public, dunder],
            code_by_path: BTreeMap::from([(
                "src/domain.rs".into(),
                Arc::from(concat!(
                    "class _Unused: pass\n",
                    "class _Referenced: pass\n",
                    "use(_Referenced)\n",
                    "class Public: pass\n",
                    "class __Generated: pass\n",
                )) as Arc<str>,
            )]),
            ..SourceModel::default()
        };

        let observations = unused_code(&model);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["findings"], 1);
        assert_eq!(
            observations[0].evidence["symbols"],
            json!(["src/domain.rs::_Unused"])
        );
    }

    #[test]
    fn unused_type_parameters_are_reported_for_classes_only_when_unreferenced() {
        let mut generic = class("Envelope", Vec::new());
        generic.type_parameters = vec!["Unused".into(), "Value".into()];
        generic.source = Arc::from("class Envelope<Unused, Value> { value: Value }");
        let mut complete = class("Pair", Vec::new());
        complete.type_parameters = vec!["Left".into(), "Right".into()];
        complete.source = Arc::from("class Pair<Left, Right> { left: Left; right: Right }");
        let model = SourceModel {
            classes: vec![generic, complete],
            ..SourceModel::default()
        };

        let observations = unused_type_parameters(&model);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].symbol, "src/domain.rs::Envelope");
        assert_eq!(observations[0].measurements["findings"], 1);
        assert_eq!(
            observations[0].evidence["unused_parameters"],
            json!(["Unused"])
        );
    }

    #[test]
    fn function_crap_uses_measured_line_coverage_and_keeps_the_bound_without_records() {
        let mut covered = function("covered", Some("{ if ready { 1 } else { 2 } }"));
        covered.body_line = 2;
        covered.end_line = 4;
        covered.complexity = Some(2);
        let mut unrecorded = function("unrecorded", Some("{ if ready { 1 } else { 2 } }"));
        unrecorded.location = location(10);
        unrecorded.body_line = 11;
        unrecorded.end_line = 12;
        unrecorded.complexity = Some(2);
        let model = SourceModel {
            functions: vec![covered, unrecorded],
            ..SourceModel::default()
        };
        let coverage = LineCoverage {
            sha256: "abc".into(),
            files: BTreeMap::from([(
                "src/domain.rs".to_string(),
                BTreeMap::from([(2, 3), (3, 0), (13, 1)]),
            )]),
        };

        let observations = function_crap(&model, Some(&coverage));
        assert_eq!(observations[0].measurements["numerator"], 5);
        assert_eq!(observations[0].measurements["denominator"], 2);
        assert_eq!(
            observations[0].evidence,
            json!({
                "collector": "lcov_line_coverage_crap_v1",
                "cyclomatic_complexity": 2,
                "covered_lines": 1,
                "instrumented_lines": 2,
                "coverage_sha256": "abc",
                "interpretation": "CRAP from supplied LCOV line coverage over the function body",
            })
        );
        assert_eq!(observations[1].measurements["numerator"], 6);
        assert_eq!(observations[1].measurements["denominator"], 1);
        assert_eq!(observations[1].evidence["coverage_assumption_percent"], 0);
        let uncovered = function_crap(&model, None);
        assert_eq!(uncovered[0].measurements["numerator"], 6);
    }

    #[test]
    fn covered_crap_is_exact_reduced_and_overflow_falls_back() {
        assert_eq!(covered_crap(3, 5, 5), Some((3, 1)));
        assert_eq!(covered_crap(3, 0, 5), Some((12, 1)));
        assert_eq!(covered_crap(4, 1, 3), Some((236, 27)));
        assert_eq!(covered_crap(u64::MAX, 0, u64::MAX), None);
        assert_eq!(gcd(12, 18), 6);
    }
}
