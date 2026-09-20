use crate::{
    input::Input,
    metrics::{self, Counter},
    policy::{Policy, Registry},
    report::{Location, Report},
    similarity::exact_jaccard_pairs,
};
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
#[cfg(unix)]
use std::{
    ffi::{CString, OsStr, OsString},
    os::unix::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawFd, FromRawFd},
    },
};
use tree_sitter::{Language, Node, Parser, Tree};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FunctionFact {
    symbol: String,
    location: Location,
    parameters: Vec<(String, String)>,
    lines: usize,
    comments: usize,
    tokens: Vec<String>,
    body_range: (usize, usize),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClassFact {
    symbol: String,
    location: Location,
    fields: usize,
    methods: usize,
    method_lines: usize,
    operations: usize,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Facts {
    functions: Vec<FunctionFact>,
    classes: Vec<ClassFact>,
}

#[derive(Clone, Copy)]
struct RuleNeeds {
    functions: bool,
    parameters: bool,
    lines: bool,
    comments: bool,
    tokens: bool,
    classes: bool,
}

impl RuleNeeds {
    fn from_policy(policy: &Policy, language: &str) -> Self {
        let enabled = |suffix: &str| policy.enabled(&format!("{language}.{suffix}"));
        let parameters = enabled("function_arguments") || enabled("data_clumps");
        let lines = enabled("function_lines") || enabled("comment_share");
        let comments = enabled("comment_share");
        let tokens = enabled("duplicate_functions");
        let functions = parameters || lines || comments || tokens;
        let classes = [
            "class_fields",
            "class_methods",
            "class_method_lines",
            "data_class",
            "lazy_class",
        ]
        .into_iter()
        .any(enabled);
        Self {
            functions,
            parameters,
            lines,
            comments,
            tokens,
            classes,
        }
    }

    fn cache_bytes(self) -> [u8; 6] {
        [
            self.functions as u8,
            self.parameters as u8,
            self.lines as u8,
            self.comments as u8,
            self.tokens as u8,
            self.classes as u8,
        ]
    }
}

fn language(path: &str, registry: &Registry) -> Result<Language, String> {
    match registry.language.as_str() {
        "python" => Ok(tree_sitter_python::LANGUAGE.into()),
        "typescript" if Path::new(path).extension().is_some_and(|ext| ext == "tsx") => {
            Ok(tree_sitter_typescript::LANGUAGE_TSX.into())
        }
        "typescript" => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        other => Err(format!("unsupported portable language: {other}")),
    }
}

fn typescript_parser_compatibility_source(source: &str) -> Option<String> {
    static IMPORT_TYPE_ARGUMENT: OnceLock<Regex> = OnceLock::new();
    let expression = IMPORT_TYPE_ARGUMENT.get_or_init(|| {
        Regex::new(
            r#"(?x)
            <\s*typeof\s+import\s*\(\s*
            (?:"(?:\\.|[^"\\\r\n])*"|'(?:\\.|[^'\\\r\n])*')
            \s*\)\s*>
            "#,
        )
        .expect("fixed TypeScript import-type compatibility regex")
    });
    let mut bytes = source.as_bytes().to_vec();
    let mut changed = false;
    for matched in expression.find_iter(source) {
        if source[matched.end()..]
            .chars()
            .find(|character| !character.is_whitespace())
            != Some('(')
        {
            continue;
        }
        for byte in &mut bytes[matched.start()..matched.end()] {
            if !matches!(*byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
        bytes[matched.start()] = b'<';
        bytes[matched.end() - 1] = b'>';
        if let Some(type_byte) = bytes[matched.start() + 1..matched.end() - 1]
            .iter_mut()
            .find(|byte| !matches!(**byte, b'\n' | b'\r'))
        {
            *type_byte = b'T';
        }
        changed = true;
    }
    changed.then(|| String::from_utf8(bytes).expect("spaces preserve UTF-8"))
}

fn location(path: &str, node: Node<'_>) -> Location {
    Location {
        path: path.into(),
        line: node.start_position().row + 1,
        column: node.start_position().column + 1,
    }
}

#[derive(Default)]
struct FunctionBodyMetrics {
    body_code_rows: BTreeSet<usize>,
    callable_code_rows: BTreeSet<usize>,
    comment_rows: BTreeSet<usize>,
    tokens: Vec<String>,
}

fn extend_node_rows(node: Node<'_>, rows: &mut BTreeSet<usize>) {
    let start = node.start_position().row;
    let end = node.end_position();
    let exclusive_end = end.row + usize::from(end.column > 0);
    rows.extend(start..exclusive_end.max(start + 1));
}

fn normalized_leaf_kind(kind: &str) -> String {
    if kind.contains("identifier") || kind == "identifier" {
        "identifier".into()
    } else if matches!(
        kind,
        "integer"
            | "float"
            | "string"
            | "template_string"
            | "true"
            | "false"
            | "none"
            | "null"
            | "undefined"
    ) {
        format!("literal:{kind}")
    } else {
        kind.into()
    }
}

fn record_comment(node: Node<'_>, needs: RuleNeeds, metrics: &mut FunctionBodyMetrics) {
    if needs.comments {
        extend_node_rows(node, &mut metrics.comment_rows);
    }
}

fn record_body_leaf(
    node: Node<'_>,
    body_range: (usize, usize),
    needs: RuleNeeds,
    metrics: &mut FunctionBodyMetrics,
) {
    let kind = node.kind();
    let substantive = !matches!(kind, "{" | "}" | "(" | ")" | "[" | "]" | "," | ";" | ":");
    if needs.comments && substantive {
        extend_node_rows(node, &mut metrics.callable_code_rows);
    }
    if node.start_byte() < body_range.0 || node.end_byte() > body_range.1 {
        return;
    }
    if needs.lines && substantive {
        extend_node_rows(node, &mut metrics.body_code_rows);
    }
    if needs.tokens {
        metrics.tokens.push(normalized_leaf_kind(kind));
    }
}

fn compact_text(node: Node<'_>, source: &str) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or_default()
        .split_whitespace()
        .collect()
}

fn first_descendant<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    if kinds.contains(&node.kind()) {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| first_descendant(child, kinds))
}

fn parameters(node: Node<'_>, source: &str) -> Vec<(String, String)> {
    if let Some(parameter) = node.child_by_field_name("parameter") {
        return vec![(compact_text(parameter, source), String::new())];
    }
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        if matches!(
            parameter.kind(),
            "keyword_separator" | "positional_separator"
        ) {
            continue;
        }
        let name = parameter
            .child_by_field_name("name")
            .or_else(|| parameter.child_by_field_name("pattern"))
            .or_else(|| {
                first_descendant(
                    parameter,
                    &[
                        "identifier",
                        "rest_pattern",
                        "list_splat_pattern",
                        "dictionary_splat_pattern",
                    ],
                )
            })
            .unwrap_or(parameter);
        let kind = parameter
            .child_by_field_name("type")
            .map(|ty| compact_text(ty, source))
            .unwrap_or_default();
        result.push((compact_text(name, source), kind));
    }
    result
}

fn receiver_field(node: Node<'_>, source: &str, language: &str) -> bool {
    let kind = if language == "python" {
        "attribute"
    } else {
        "member_expression"
    };
    if node.kind() != kind {
        return false;
    }
    node.child_by_field_name("object")
        .and_then(|object| object.utf8_text(source.as_bytes()).ok())
        .is_some_and(|owner| {
            if language == "python" {
                matches!(owner, "self" | "cls")
            } else {
                owner == "this"
            }
        })
}

fn node_name(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("name")
        .and_then(|name| name.utf8_text(source.as_bytes()).ok())
        .unwrap_or("<anonymous>")
        .to_string()
}

fn parameter_property_name(parameter: Node<'_>, source: &str) -> Option<String> {
    let mut children = parameter.walk();
    let property = parameter.children(&mut children).any(|child| {
        matches!(
            child.kind(),
            "accessibility_modifier" | "readonly" | "override_modifier"
        )
    });
    if !property {
        return None;
    }
    parameter
        .child_by_field_name("name")
        .or_else(|| parameter.child_by_field_name("pattern"))?
        .utf8_text(source.as_bytes())
        .ok()
        .map(str::to_string)
}

struct ClassSummary<'a> {
    symbol: &'a str,
    location: &'a Location,
    fields: usize,
    methods: usize,
    method_lines: usize,
}

fn record_class_metrics(
    summary: &ClassSummary<'_>,
    registry: &Registry,
    input: &Input,
    report: &mut Report,
) {
    for (suffix, value, description) in [
        ("class_fields", summary.fields, "source-owned class fields"),
        (
            "class_methods",
            summary.methods,
            "source-owned class methods",
        ),
        (
            "class_method_lines",
            summary.method_lines,
            "summed class method code lines",
        ),
    ] {
        report.maximum(
            &input.policy,
            &format!("{}.{}", registry.language, suffix),
            summary.symbol,
            summary.location,
            value,
            description,
        );
    }
}

fn is_class(node: Node<'_>, language: &str) -> bool {
    match language {
        "python" => node.kind() == "class_definition",
        "typescript" => matches!(
            node.kind(),
            "class" | "class_declaration" | "abstract_class_declaration"
        ),
        _ => false,
    }
}

fn is_function(node: Node<'_>, language: &str) -> bool {
    match language {
        "python" => matches!(node.kind(), "function_definition" | "lambda"),
        "typescript" => match node.kind() {
            "method_definition" => node
                .parent()
                .is_some_and(|parent| parent.kind() == "class_body"),
            "arrow_function"
            | "function_declaration"
            | "function_expression"
            | "generator_function"
            | "generator_function_declaration" => true,
            _ => false,
        },
        _ => false,
    }
}

struct ActiveClass {
    node_id: usize,
    order: usize,
    function_depth: usize,
    symbol: String,
    location: Location,
    fields: BTreeSet<String>,
    methods: usize,
    method_lines: usize,
    operations: usize,
}

struct AccessorState {
    constructor: bool,
    sole_statement_id: Option<usize>,
    sole_statement_range: Option<(usize, usize)>,
    assignment_seen: bool,
    ordinary_parameter: Option<String>,
    matched: bool,
}

struct ActiveFunction {
    node_id: usize,
    order: usize,
    record_fact: bool,
    symbol: String,
    location: Location,
    parameters: Vec<(String, String)>,
    body_range: Option<(usize, usize)>,
    metrics: FunctionBodyMetrics,
    metric_needs: RuleNeeds,
    direct_class: Option<usize>,
    accessor: Option<AccessorState>,
}

struct AssignmentTarget {
    assignment_id: usize,
    range: (usize, usize),
    class_index: usize,
    receiver: bool,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceStats {
    syntax_nodes: usize,
    functions: usize,
    normalized_tokens: usize,
}

#[derive(Default)]
struct ExtractionState {
    completed_functions: Vec<(usize, FunctionFact)>,
    completed_classes: Vec<(usize, ClassFact)>,
    classes: Vec<ActiveClass>,
    functions: Vec<ActiveFunction>,
    assignments: Vec<AssignmentTarget>,
    next_class_order: usize,
    next_function_order: usize,
    stats: SourceStats,
}

fn python_class_owner(parent: Node<'_>) -> Option<Node<'_>> {
    if parent.kind() == "block" {
        return parent.parent();
    }
    if parent.kind() != "decorated_definition" {
        return None;
    }
    parent
        .parent()
        .filter(|body| body.kind() == "block")
        .and_then(|body| body.parent())
}

fn direct_class_owner<'tree>(node: Node<'tree>, language: &str) -> Option<Node<'tree>> {
    let parent = node.parent()?;
    match language {
        "typescript" => (parent.kind() == "class_body")
            .then(|| parent.parent())
            .flatten(),
        "python" => python_class_owner(parent),
        _ => None,
    }
}

fn direct_class_index(node: Node<'_>, language: &str, classes: &[ActiveClass]) -> Option<usize> {
    let class_index = classes.len().checked_sub(1)?;
    let owner = direct_class_owner(node, language)?;
    (owner.id() == classes[class_index].node_id).then_some(class_index)
}

fn sole_statement(body: Node<'_>) -> Option<Node<'_>> {
    (body.named_child_count() == 1)
        .then(|| body.named_child(0))
        .flatten()
}

fn active_function(
    node: Node<'_>,
    path: &str,
    source: &str,
    language: &str,
    needs: RuleNeeds,
    classes: &[ActiveClass],
    order: usize,
) -> ActiveFunction {
    let direct_class = direct_class_index(node, language, classes);
    let record_fact = needs.functions;
    let needs_parameters = record_fact && needs.parameters || direct_class.is_some();
    let declared_parameters = if needs_parameters {
        parameters(node, source)
    } else {
        Vec::new()
    };
    let body = node.child_by_field_name("body");
    let body_range = body.map(|body| (body.start_byte(), body.end_byte()));
    let metric_needs = RuleNeeds {
        functions: record_fact,
        parameters: needs_parameters,
        lines: record_fact && needs.lines || direct_class.is_some(),
        comments: record_fact && needs.comments,
        tokens: record_fact && needs.tokens,
        classes: needs.classes,
    };
    let symbol = node_name(node, source);
    let accessor = direct_class.map(|_| {
        let statement = body.and_then(sole_statement);
        let ordinary = declared_parameters
            .iter()
            .map(|(name, _)| name)
            .filter(|name| !matches!(name.as_str(), "self" | "cls" | "this"))
            .collect::<Vec<_>>();
        AccessorState {
            constructor: matches!(symbol.as_str(), "__init__" | "constructor"),
            sole_statement_id: statement.map(|node| node.id()),
            sole_statement_range: statement.map(|node| (node.start_byte(), node.end_byte())),
            assignment_seen: false,
            ordinary_parameter: (ordinary.len() == 1).then(|| ordinary[0].clone()),
            matched: false,
        }
    });
    ActiveFunction {
        node_id: node.id(),
        order,
        record_fact,
        symbol,
        location: location(path, node),
        parameters: declared_parameters,
        body_range,
        metrics: FunctionBodyMetrics::default(),
        metric_needs,
        direct_class,
        accessor,
    }
}

fn returned_receiver_field(node: Node<'_>, source: &str, language: &str, statement: usize) -> bool {
    node.id() == statement
        && node.kind() == "return_statement"
        && node
            .named_child(0)
            .is_some_and(|field| receiver_field(field, source, language))
}

fn assignment_in_statement(node: Node<'_>, language: &str, range: (usize, usize)) -> bool {
    let assignment_kind = if language == "python" {
        "assignment"
    } else {
        "assignment_expression"
    };
    node.kind() == assignment_kind && node.start_byte() >= range.0 && node.end_byte() <= range.1
}

fn setter_matches(node: Node<'_>, source: &str, language: &str, parameter: &str) -> bool {
    node.child_by_field_name("left")
        .is_some_and(|left| receiver_field(left, source, language))
        && node.child_by_field_name("right").is_some_and(|right| {
            right.kind() == "identifier" && compact_text(right, source) == parameter
        })
}

fn update_accessor(node: Node<'_>, source: &str, language: &str, function: &mut ActiveFunction) {
    let Some(accessor) = function.accessor.as_mut() else {
        return;
    };
    if accessor.constructor || accessor.matched {
        return;
    }
    if accessor
        .sole_statement_id
        .is_some_and(|statement| returned_receiver_field(node, source, language, statement))
    {
        accessor.matched = true;
        return;
    }
    let Some(range) = accessor.sole_statement_range else {
        return;
    };
    if accessor.assignment_seen || !assignment_in_statement(node, language, range) {
        return;
    }
    accessor.assignment_seen = true;
    let Some(parameter) = accessor.ordinary_parameter.as_deref() else {
        return;
    };
    accessor.matched = setter_matches(node, source, language, parameter);
}

fn python_assignment_target(
    node: Node<'_>,
    classes: &[ActiveClass],
    functions: &[ActiveFunction],
) -> Option<(usize, bool, (usize, usize))> {
    if !matches!(node.kind(), "assignment" | "augmented_assignment") {
        return None;
    }
    let class_index = classes.len().checked_sub(1)?;
    let receiver = if functions.len() == classes[class_index].function_depth {
        false
    } else if functions
        .last()
        .is_some_and(|function| function.direct_class == Some(class_index))
    {
        true
    } else {
        return None;
    };
    let left = node.child_by_field_name("left")?;
    Some((class_index, receiver, (left.start_byte(), left.end_byte())))
}

fn blocked_assignment_identifier(node: Node<'_>, target: &AssignmentTarget) -> bool {
    let mut parent = node.parent();
    while let Some(ancestor) = parent {
        if ancestor.start_byte() < target.range.0 || ancestor.end_byte() > target.range.1 {
            break;
        }
        if matches!(
            ancestor.kind(),
            "attribute" | "subscript" | "member_expression"
        ) {
            return true;
        }
        parent = ancestor.parent();
    }
    false
}

fn update_python_field(
    node: Node<'_>,
    source: &str,
    assignments: &[AssignmentTarget],
    classes: &mut [ActiveClass],
) {
    let Some(target) = assignments.last() else {
        return;
    };
    if node.start_byte() < target.range.0 || node.end_byte() > target.range.1 {
        return;
    }
    let name = if target.receiver && node.kind() == "attribute" {
        receiver_field(node, source, "python").then(|| {
            node.child_by_field_name("attribute")
                .and_then(|field| field.utf8_text(source.as_bytes()).ok())
                .unwrap_or_default()
                .to_string()
        })
    } else if !target.receiver
        && matches!(node.kind(), "identifier" | "pattern")
        && !blocked_assignment_identifier(node, target)
    {
        node.utf8_text(source.as_bytes()).ok().map(str::to_string)
    } else {
        None
    };
    if let Some(name) = name.filter(|name| !name.is_empty()) {
        classes[target.class_index].fields.insert(name);
    }
}

fn update_typescript_field(
    node: Node<'_>,
    source: &str,
    classes: &mut [ActiveClass],
    functions: &[ActiveFunction],
) {
    let Some(class_index) = classes.len().checked_sub(1) else {
        return;
    };
    if node.kind() == "public_field_definition"
        && node.parent().is_some_and(|body| {
            body.kind() == "class_body"
                && body
                    .parent()
                    .is_some_and(|class| class.id() == classes[class_index].node_id)
        })
        && let Some(name) = node.child_by_field_name("name")
        && let Ok(name) = name.utf8_text(source.as_bytes())
    {
        classes[class_index].fields.insert(name.to_string());
    }
    if functions.last().is_some_and(|function| {
        function.direct_class == Some(class_index) && function.symbol == "constructor"
    }) && node
        .parent()
        .is_some_and(|parent| parent.kind() == "formal_parameters")
        && let Some(name) = parameter_property_name(node, source)
    {
        classes[class_index].fields.insert(name);
    }
}

fn update_metrics(node: Node<'_>, functions: &mut [ActiveFunction]) {
    if node.kind().contains("comment") {
        for function in functions {
            record_comment(node, function.metric_needs, &mut function.metrics);
        }
    } else if node.child_count() == 0 {
        for function in functions {
            if let Some(body_range) = function.body_range {
                record_body_leaf(
                    node,
                    body_range,
                    function.metric_needs,
                    &mut function.metrics,
                );
            }
        }
    }
}

fn record_node_stats(node: Node<'_>, language: &str, stats: &mut SourceStats) {
    stats.syntax_nodes += 1;
    stats.functions += usize::from(is_function(node, language));
}

fn start_class(
    node: Node<'_>,
    path: &str,
    source: &str,
    language: &str,
    needs: RuleNeeds,
    state: &mut ExtractionState,
) {
    if !needs.classes || !is_class(node, language) || node.child_by_field_name("body").is_none() {
        return;
    }
    state.classes.push(ActiveClass {
        node_id: node.id(),
        order: state.next_class_order,
        function_depth: state.functions.len(),
        symbol: node_name(node, source),
        location: location(path, node),
        fields: BTreeSet::new(),
        methods: 0,
        method_lines: 0,
        operations: 0,
    });
    state.next_class_order += 1;
}

fn start_function(
    node: Node<'_>,
    path: &str,
    source: &str,
    language: &str,
    needs: RuleNeeds,
    state: &mut ExtractionState,
) {
    if (!needs.functions && !needs.classes) || !is_function(node, language) {
        return;
    }
    state.functions.push(active_function(
        node,
        path,
        source,
        language,
        needs,
        &state.classes,
        state.next_function_order,
    ));
    state.next_function_order += 1;
}

fn record_typescript_signature(
    node: Node<'_>,
    source: &str,
    language: &str,
    needs: RuleNeeds,
    classes: &mut [ActiveClass],
) {
    if !needs.classes
        || language != "typescript"
        || !matches!(
            node.kind(),
            "abstract_method_signature" | "method_signature"
        )
    {
        return;
    }
    let Some(class_index) = direct_class_index(node, language, classes) else {
        return;
    };
    let class = &mut classes[class_index];
    class.methods += 1;
    class.operations += usize::from(node_name(node, source) != "constructor");
}

fn update_accessors(
    node: Node<'_>,
    source: &str,
    language: &str,
    functions: &mut [ActiveFunction],
) {
    for function in functions {
        update_accessor(node, source, language, function);
    }
}

fn start_python_assignment(
    node: Node<'_>,
    language: &str,
    needs: RuleNeeds,
    state: &mut ExtractionState,
) {
    if !needs.classes || language != "python" {
        return;
    }
    let Some((class_index, receiver, range)) =
        python_assignment_target(node, &state.classes, &state.functions)
    else {
        return;
    };
    state.assignments.push(AssignmentTarget {
        assignment_id: node.id(),
        range,
        class_index,
        receiver,
    });
}

fn update_class_field(
    node: Node<'_>,
    source: &str,
    language: &str,
    needs: RuleNeeds,
    state: &mut ExtractionState,
) {
    if !needs.classes {
        return;
    }
    if language == "python" {
        update_python_field(node, source, &state.assignments, &mut state.classes);
    } else {
        update_typescript_field(node, source, &mut state.classes, &state.functions);
    }
}

fn enter_node(
    node: Node<'_>,
    path: &str,
    source: &str,
    registry: &Registry,
    needs: RuleNeeds,
    state: &mut ExtractionState,
) {
    let language = registry.language.as_str();
    record_node_stats(node, language, &mut state.stats);
    start_class(node, path, source, language, needs, state);
    start_function(node, path, source, language, needs, state);
    record_typescript_signature(node, source, language, needs, &mut state.classes);
    update_accessors(node, source, language, &mut state.functions);
    start_python_assignment(node, language, needs, state);
    update_class_field(node, source, language, needs, state);
    update_metrics(node, &mut state.functions);
}

fn exit_node(node: Node<'_>, state: &mut ExtractionState) {
    if state
        .assignments
        .last()
        .is_some_and(|assignment| assignment.assignment_id == node.id())
    {
        state.assignments.pop();
    }
    if state
        .functions
        .last()
        .is_some_and(|function| function.node_id == node.id())
    {
        let function = state.functions.pop().expect("active function");
        let lines = function.metrics.body_code_rows.len();
        if let Some(class_index) = function.direct_class {
            let class = &mut state.classes[class_index];
            class.methods += 1;
            class.method_lines += lines;
            let accessor = function
                .accessor
                .as_ref()
                .is_some_and(|accessor| accessor.constructor || accessor.matched);
            if !accessor {
                class.operations += 1;
            }
        }
        if function.record_fact
            && let Some(body_range) = function.body_range
        {
            let comments = function
                .metrics
                .comment_rows
                .difference(&function.metrics.callable_code_rows)
                .count();
            state.completed_functions.push((
                function.order,
                FunctionFact {
                    symbol: function.symbol,
                    location: function.location,
                    parameters: function.parameters,
                    lines,
                    comments,
                    tokens: function.metrics.tokens,
                    body_range,
                },
            ));
        }
    }
    if state
        .classes
        .last()
        .is_some_and(|class| class.node_id == node.id())
    {
        let class = state.classes.pop().expect("active class");
        state.completed_classes.push((
            class.order,
            ClassFact {
                symbol: class.symbol,
                location: class.location,
                fields: class.fields.len(),
                methods: class.methods,
                method_lines: class.method_lines,
                operations: class.operations,
            },
        ));
    }
}

fn extract_facts(
    tree: &Tree,
    path: &str,
    source: &str,
    registry: &Registry,
    needs: RuleNeeds,
) -> (Facts, SourceStats) {
    let mut facts = Facts::default();
    let mut state = ExtractionState::default();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        enter_node(node, path, source, registry, needs, &mut state);
        if cursor.goto_first_child() {
            continue;
        }
        exit_node(node, &mut state);
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                state.completed_functions.sort_by_key(|(order, _)| *order);
                state.completed_classes.sort_by_key(|(order, _)| *order);
                facts.functions = state
                    .completed_functions
                    .into_iter()
                    .map(|(_, function)| function)
                    .collect();
                facts.classes = state
                    .completed_classes
                    .into_iter()
                    .map(|(_, class)| class)
                    .collect();
                state.stats.normalized_tokens = facts
                    .functions
                    .iter()
                    .map(|function| function.tokens.len())
                    .sum();
                return (facts, state.stats);
            }
            exit_node(cursor.node(), &mut state);
        }
    }
}

fn class_patterns(facts: &Facts, policy: &Policy, language: &str, report: &mut Report) {
    let data_class = format!("{language}.data_class");
    let lazy_class = format!("{language}.lazy_class");
    for class in &facts.classes {
        if policy.enabled(&data_class) {
            let minimum = policy.parameter(&data_class, "minimum_fields");
            let maximum = policy.parameter(&data_class, "maximum_operations");
            report.finding(
                policy,
                &data_class,
                &class.symbol,
                &class.location,
                "fields and non-accessor operations",
                json!({"fields":class.fields,"operations":class.operations}),
                "fields >= and operations <=",
                json!({"minimum_fields":minimum,"maximum_operations":maximum}),
                class.fields as u64 >= minimum && class.operations as u64 <= maximum,
                json!({"accessor_definition":"strict_source_shape","constructors_excluded":true}),
            );
        }
        if policy.enabled(&lazy_class) {
            let fields = policy.parameter(&lazy_class, "maximum_fields");
            let methods = policy.parameter(&lazy_class, "maximum_functions");
            let lines = policy.parameter(&lazy_class, "maximum_lines");
            report.finding(
                policy,
                &lazy_class,
                &class.symbol,
                &class.location,
                "fields/methods/code lines",
                json!({"fields":class.fields,"functions":class.methods,"lines":class.method_lines}),
                "all <=",
                json!({"maximum_fields":fields,"maximum_functions":methods,"maximum_lines":lines}),
                class.fields as u64 <= fields
                    && class.methods as u64 <= methods
                    && class.method_lines as u64 <= lines,
                json!({"scope":"source_authored_class_members"}),
            );
        }
    }
}

fn comments(facts: &Facts, policy: &Policy, language: &str, report: &mut Report) {
    let id = format!("{language}.comment_share");
    if !policy.enabled(&id) {
        return;
    }
    for function in &facts.functions {
        let code = policy.parameter(&id, "minimum_code_lines");
        let share = policy.parameter(&id, "minimum_share_percent");
        let total = function.lines + function.comments;
        if total == 0 {
            continue;
        }
        report.finding(
            policy,
            &id,
            &function.symbol,
            &function.location,
            "ordinary comment-only body-line share",
            json!({"comment_lines":function.comments,"code_lines":function.lines}),
            "code minimum and share >=",
            json!({"minimum_code_lines":code,"minimum_share_percent":share}),
            function.lines as u64 >= code
                && 100_u128 * function.comments as u128 >= share as u128 * total as u128,
            json!({"denominator":total,"documentation_strings":"excluded"}),
        );
    }
}

fn combinations(
    slots: &[(String, String)],
    size: usize,
    start: usize,
    chosen: &mut Vec<(String, String)>,
    groups: &mut Vec<Vec<(String, String)>>,
    remaining: &mut usize,
) -> Result<(), String> {
    if chosen.len() == size {
        if *remaining == 0 {
            return Err("maximum_group_combinations budget exceeded".into());
        }
        *remaining -= 1;
        groups.push(chosen.clone());
        return Ok(());
    }
    let needed = size - chosen.len();
    if needed > slots.len().saturating_sub(start) {
        return Ok(());
    }
    for index in start..=slots.len() - needed {
        chosen.push(slots[index].clone());
        combinations(slots, size, index + 1, chosen, groups, remaining)?;
        chosen.pop();
    }
    Ok(())
}

fn clumps(facts: &Facts, policy: &Policy, language: &str, report: &mut Report) {
    let id = format!("{language}.data_clumps");
    if !policy.enabled(&id) {
        return;
    }
    let size = policy.parameter(&id, "minimum_group_size") as usize;
    let minimum = policy.parameter(&id, "minimum_declarations");
    let mut remaining = policy.limits.maximum_group_combinations;
    let mut supports: BTreeMap<Vec<(String, String)>, BTreeSet<usize>> = BTreeMap::new();
    for (index, function) in facts.functions.iter().enumerate() {
        let slots: Vec<_> = function
            .parameters
            .iter()
            .filter(|(name, _)| !matches!(name.as_str(), "self" | "cls" | "this"))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if slots.len() < size {
            continue;
        }
        let mut groups = Vec::new();
        if let Err(error) = combinations(
            &slots,
            size,
            0,
            &mut Vec::new(),
            &mut groups,
            &mut remaining,
        ) {
            report.errors.push(error);
            return;
        }
        for group in groups {
            supports.entry(group).or_default().insert(index);
        }
    }
    for (group, support) in supports {
        if (support.len() as u64) < minimum {
            continue;
        }
        let functions: Vec<_> = support
            .into_iter()
            .map(|index| &facts.functions[index])
            .collect();
        let first = functions[0];
        report.finding(
            policy,
            &id,
            &first.symbol,
            &first.location,
            "repeated named syntax group",
            json!({"slots":group.len(),"declarations":functions.len()}),
            "both >=",
            json!({"minimum_group_size":size,"minimum_declarations":minimum}),
            true,
            json!({"slots":group,"supporting_symbols":functions.iter().map(|function|&function.symbol).collect::<Vec<_>>(),"type_interpretation":"exact_authored_syntax"}),
        );
        let finding = report.findings.last_mut().expect("finding just added");
        finding.related_symbols = functions
            .iter()
            .skip(1)
            .map(|function| function.symbol.clone())
            .collect();
        finding.related_locations = functions
            .iter()
            .skip(1)
            .map(|function| function.location.clone())
            .collect();
    }
}

fn overlapping_functions(first: &FunctionFact, second: &FunctionFact) -> bool {
    first.location.path == second.location.path
        && first.body_range.0 < second.body_range.1
        && second.body_range.0 < first.body_range.1
}

fn ordered_functions<'a>(
    first: &'a FunctionFact,
    second: &'a FunctionFact,
) -> (&'a FunctionFact, &'a FunctionFact) {
    if (&first.symbol, &first.location.path, first.location.line)
        <= (&second.symbol, &second.location.path, second.location.line)
    {
        (first, second)
    } else {
        (second, first)
    }
}

struct DuplicateRule<'a> {
    id: &'a str,
    policy: &'a Policy,
    minimum: u64,
    similarity: u64,
}

fn report_duplicate_pair(
    first: &FunctionFact,
    second: &FunctionFact,
    intersection: usize,
    union: usize,
    rule: &DuplicateRule<'_>,
    report: &mut Report,
) {
    let (first, second) = ordered_functions(first, second);
    report.finding(
        rule.policy,
        rule.id,
        &first.symbol,
        &first.location,
        "normalized 4-token multiset Jaccard",
        json!({"intersection":intersection,"union":union}),
        ">=",
        json!({"minimum_similarity_basis_points":rule.similarity,"minimum_tokens":rule.minimum}),
        true,
        json!({"token_counts":[first.tokens.len(),second.tokens.len()],"other_location":second.location,"normalization":"tree_sitter_tokens_v1_not_semantic_equivalence"}),
    );
    let finding = report.findings.last_mut().expect("finding just added");
    finding.related_symbols.push(second.symbol.clone());
    finding.related_locations.push(second.location.clone());
}

fn duplicates(facts: &Facts, policy: &Policy, language: &str, report: &mut Report) {
    let id = format!("{language}.duplicate_functions");
    if !policy.enabled(&id) {
        return;
    }
    let minimum = policy.parameter(&id, "minimum_tokens");
    let similarity = policy.parameter(&id, "minimum_similarity_basis_points");
    let rule = DuplicateRule {
        id: &id,
        policy,
        minimum,
        similarity,
    };
    let token_lists = facts
        .functions
        .iter()
        .map(|function| function.tokens.clone())
        .collect::<Vec<_>>();
    let pairs = exact_jaccard_pairs(
        &token_lists,
        minimum,
        similarity,
        policy.limits.maximum_pairs,
        |left, right| overlapping_functions(&facts.functions[left], &facts.functions[right]),
    );
    let pairs = match pairs {
        Ok(pairs) => pairs,
        Err(error) => {
            report.errors.push(error.into());
            return;
        }
    };
    for pair in pairs {
        report_duplicate_pair(
            &facts.functions[pair.left],
            &facts.functions[pair.right],
            pair.intersection,
            pair.union,
            &rule,
            report,
        );
    }
}

fn patterns(facts: &Facts, policy: &Policy, language: &str, report: &mut Report) {
    class_patterns(facts, policy, language, report);
    comments(facts, policy, language, report);
    clumps(facts, policy, language, report);
    duplicates(facts, policy, language, report);
}

fn source_metrics(facts: &Facts, registry: &Registry, input: &Input, report: &mut Report) {
    for function in &facts.functions {
        report.maximum(
            &input.policy,
            &format!("{}.function_arguments", registry.language),
            &function.symbol,
            &function.location,
            function.parameters.len(),
            "declared parameters",
        );
        report.maximum(
            &input.policy,
            &format!("{}.function_lines", registry.language),
            &function.symbol,
            &function.location,
            function.lines,
            "authored body code lines",
        );
    }
    for class in &facts.classes {
        record_class_metrics(
            &ClassSummary {
                symbol: &class.symbol,
                location: &class.location,
                fields: class.fields,
                methods: class.methods,
                method_lines: class.method_lines,
            },
            registry,
            input,
            report,
        );
    }
}

fn validate_required_rules(registry: &Registry, input: &Input, report: &mut Report) {
    for rule in &registry.rules {
        if rule.implementation == "not_implemented" && input.policy.required(&rule.id) {
            report
                .errors
                .push(format!("required detector not implemented: {}", rule.id));
        }
    }
}

fn configured_parser(path: &str, registry: &Registry) -> Result<Parser, String> {
    let grammar = language(path, registry)?;
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|error| format!("cannot load grammar for {path}: {error}"))?;
    Ok(parser)
}

#[derive(Default)]
struct ParserPool {
    python: Option<Parser>,
    typescript: Option<Parser>,
    tsx: Option<Parser>,
}

impl ParserPool {
    fn parser(&mut self, path: &str, registry: &Registry) -> Result<&mut Parser, String> {
        let slot = match registry.language.as_str() {
            "python" => &mut self.python,
            "typescript" if Path::new(path).extension().is_some_and(|ext| ext == "tsx") => {
                &mut self.tsx
            }
            "typescript" => &mut self.typescript,
            other => return Err(format!("unsupported portable language: {other}")),
        };
        if slot.is_none() {
            *slot = Some(configured_parser(path, registry)?);
        }
        Ok(slot.as_mut().expect("parser initialized"))
    }
}

fn parsed_tree(parser: &mut Parser, source: &str, path: &str) -> Result<Tree, String> {
    parser
        .parse(source, None)
        .ok_or_else(|| format!("parser cancelled for {path}"))
}

fn compatible_typescript_tree(
    parser: &mut Parser,
    tree: Tree,
    source: &str,
    path: &str,
    registry: &Registry,
) -> Result<Tree, String> {
    if !tree.root_node().has_error() || registry.language != "typescript" {
        return Ok(tree);
    }
    let Some(compatible) = typescript_parser_compatibility_source(source) else {
        return Ok(tree);
    };
    parsed_tree(parser, &compatible, path)
}

fn parse_source(
    path: &str,
    source: &str,
    registry: &Registry,
    parsers: &mut ParserPool,
) -> Result<Tree, String> {
    let parser = parsers.parser(path, registry)?;
    let tree = parsed_tree(parser, source, path)?;
    let tree = compatible_typescript_tree(parser, tree, source, path, registry)?;
    if tree.root_node().has_error() {
        return Err(format!("parse error in {path}"));
    }
    Ok(tree)
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceAnalysis {
    facts: Facts,
    errors: Vec<String>,
    stats: SourceStats,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheEntry {
    key: String,
    analysis_sha256: String,
    analysis: SourceAnalysis,
}

#[derive(Serialize)]
struct CacheEntryRef<'a> {
    key: &'a str,
    analysis_sha256: String,
    analysis: &'a SourceAnalysis,
}

struct CacheRoot {
    #[cfg(not(unix))]
    path: PathBuf,
    #[cfg(unix)]
    directory: File,
}

fn cache_root() -> Option<&'static CacheRoot> {
    static ROOT: OnceLock<Option<CacheRoot>> = OnceLock::new();
    ROOT.get_or_init(|| {
        let configured = std::env::var_os("SMELLS_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("smells-cache"));
        initialize_cache_root(&configured)
    })
    .as_ref()
}

fn fact_cache_key(path: &str, source: &str, registry: &Registry, needs: RuleNeeds) -> String {
    static IMPLEMENTATION: OnceLock<String> = OnceLock::new();
    let implementation = IMPLEMENTATION.get_or_init(|| {
        crate::input::digest(&[
            b"portable-facts-v1",
            include_bytes!("portable.rs"),
            include_bytes!("../Cargo.lock"),
        ])
    });
    crate::input::digest(&[
        implementation.as_bytes(),
        registry.language.as_bytes(),
        path.as_bytes(),
        &needs.cache_bytes(),
        source.as_bytes(),
    ])
}

fn fact_cache_relative_path(key: &str) -> PathBuf {
    PathBuf::from("portable-facts-v1")
        .join(&key[..2])
        .join(format!("{key}.json"))
}

fn analysis_digest(key: &str, analysis: &SourceAnalysis) -> Option<String> {
    let bytes = serde_json::to_vec(analysis).ok()?;
    Some(crate::input::digest(&[
        b"portable-fact-payload-v1",
        key.as_bytes(),
        &bytes,
    ]))
}

#[cfg(unix)]
fn cache_components(relative: &Path) -> Option<Vec<CString>> {
    relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(name) => CString::new(name.as_bytes()).ok(),
            _ => None,
        })
        .collect()
}

#[cfg(unix)]
fn open_at(parent: &File, name: &OsStr, flags: libc::c_int) -> Option<File> {
    let name = CString::new(name.as_bytes()).ok()?;
    // SAFETY: parent is an open directory descriptor, name is NUL-terminated,
    // and a successful openat returns a uniquely owned descriptor.
    let descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if descriptor < 0 {
        None
    } else {
        // SAFETY: the successful openat call transferred ownership of descriptor.
        Some(unsafe { File::from_raw_fd(descriptor) })
    }
}

#[cfg(unix)]
fn open_absolute_directory(path: &Path) -> Option<File> {
    if !path.is_absolute() {
        return None;
    }
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")
        .ok()?;
    for component in path.components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                directory = open_at(
                    &directory,
                    name,
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )?;
            }
            _ => return None,
        }
    }
    Some(directory)
}

#[cfg(unix)]
fn absolute_cache_path(configured: &Path) -> Option<PathBuf> {
    if configured.is_absolute() {
        Some(configured.to_path_buf())
    } else {
        Some(std::env::current_dir().ok()?.join(configured))
    }
}

#[cfg(unix)]
fn prepared_cache_parent(configured: &Path) -> Option<(File, OsString)> {
    let absolute = absolute_cache_path(configured)?;
    let parent = absolute.parent()?;
    fs::create_dir_all(parent).ok()?;
    let canonical_parent = parent.canonicalize().ok()?;
    Some((
        open_absolute_directory(&canonical_parent)?,
        absolute.file_name()?.to_os_string(),
    ))
}

#[cfg(unix)]
fn create_or_open_cache_root(parent: &File, name: &OsStr) -> Option<File> {
    let name_c = CString::new(name.as_bytes()).ok()?;
    // SAFETY: parent and name_c remain valid for the call.
    let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name_c.as_ptr(), 0o700) };
    if created != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
        return None;
    }
    open_at(
        parent,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )
}

#[cfg(unix)]
fn initialize_cache_root(configured: &Path) -> Option<CacheRoot> {
    let (parent, name) = prepared_cache_parent(configured)?;
    Some(CacheRoot {
        directory: create_or_open_cache_root(&parent, &name)?,
    })
}

#[cfg(not(unix))]
fn absolute_cache_path(configured: &Path) -> Option<PathBuf> {
    if configured.is_absolute() {
        Some(configured.to_path_buf())
    } else {
        Some(std::env::current_dir().ok()?.join(configured))
    }
}

#[cfg(not(unix))]
fn initialize_cache_root(configured: &Path) -> Option<CacheRoot> {
    let configured = absolute_cache_path(configured)?;
    fs::create_dir_all(&configured).ok()?;
    let path = configured.canonicalize().ok()?;
    fs::symlink_metadata(&path)
        .ok()?
        .file_type()
        .is_dir()
        .then_some(CacheRoot { path })
}

#[cfg(unix)]
fn open_cache_parent(root: &CacheRoot, relative: &Path, create: bool) -> Option<(File, CString)> {
    let mut components = cache_components(relative)?;
    let target = components.pop()?;
    let mut directory = root.directory.try_clone().ok()?;
    for component in components {
        if create {
            // SAFETY: directory and component remain valid for the duration of the call.
            let created =
                unsafe { libc::mkdirat(directory.as_raw_fd(), component.as_ptr(), 0o700) };
            if created != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
            {
                return None;
            }
        }
        directory = open_at(
            &directory,
            OsStr::from_bytes(component.as_bytes()),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )?;
    }
    Some((directory, target))
}

#[cfg(unix)]
fn open_cached_file(root: &CacheRoot, relative: &Path) -> Option<File> {
    let (parent, target) = open_cache_parent(root, relative, false)?;
    open_at(
        &parent,
        OsStr::from_bytes(target.as_bytes()),
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )
    .filter(|file| file.metadata().is_ok_and(|metadata| metadata.is_file()))
}

#[cfg(not(unix))]
fn open_cached_file(root: &CacheRoot, relative: &Path) -> Option<File> {
    let path = root.path.join(relative);
    let metadata = fs::symlink_metadata(&path).ok()?;
    metadata
        .file_type()
        .is_file()
        .then(|| File::open(path).ok())?
}

fn cached_analysis(root: &CacheRoot, relative: &Path, key: &str) -> Option<SourceAnalysis> {
    let entry: CacheEntry = serde_json::from_reader(open_cached_file(root, relative)?).ok()?;
    if entry.key != key || analysis_digest(key, &entry.analysis)? != entry.analysis_sha256 {
        return None;
    }
    Some(entry.analysis)
}

#[cfg(unix)]
fn write_cache_entry(root: &CacheRoot, relative: &Path, bytes: &[u8], nonce: usize) -> Option<()> {
    let (parent, target) = open_cache_parent(root, relative, true)?;
    let temporary = CString::new(format!(
        ".{}.{}.{}.tmp",
        target.to_string_lossy(),
        std::process::id(),
        nonce
    ))
    .ok()?;
    let mut file = open_at(
        &parent,
        OsStr::from_bytes(temporary.as_bytes()),
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )?;
    if file.write_all(bytes).is_err() {
        // SAFETY: parent and temporary remain valid for the duration of the call.
        unsafe { libc::unlinkat(parent.as_raw_fd(), temporary.as_ptr(), 0) };
        return None;
    }
    drop(file);
    // SAFETY: both names are relative to the same open directory descriptor.
    let renamed = unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            temporary.as_ptr(),
            parent.as_raw_fd(),
            target.as_ptr(),
        )
    };
    if renamed != 0 {
        // SAFETY: parent and temporary remain valid for the duration of the call.
        unsafe { libc::unlinkat(parent.as_raw_fd(), temporary.as_ptr(), 0) };
        return None;
    }
    Some(())
}

#[cfg(not(unix))]
fn write_cache_entry(root: &CacheRoot, relative: &Path, bytes: &[u8], nonce: usize) -> Option<()> {
    let path = root.path.join(relative);
    let parent = path.parent()?;
    fs::create_dir_all(parent).ok()?;
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()?.to_string_lossy(),
        std::process::id(),
        nonce
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| file.write_all(bytes))
        .ok()?;
    fs::rename(&temporary, path).ok()?;
    Some(())
}

fn store_analysis(root: &CacheRoot, relative: &Path, key: &str, analysis: &SourceAnalysis) {
    static NEXT_TEMPORARY: AtomicUsize = AtomicUsize::new(0);
    let Some(analysis_sha256) = analysis_digest(key, analysis) else {
        return;
    };
    let bytes = match serde_json::to_vec(&CacheEntryRef {
        key,
        analysis_sha256,
        analysis,
    }) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    let _ = write_cache_entry(
        root,
        relative,
        &bytes,
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed),
    );
}

fn cached_or_analyze_source(
    path: &str,
    source: &str,
    registry: &Registry,
    needs: RuleNeeds,
    parsers: &mut ParserPool,
) -> SourceAnalysis {
    let key = fact_cache_key(path, source, registry, needs);
    let cache_relative = fact_cache_relative_path(&key);
    if let Some(root) = cache_root()
        && let Some(analysis) = cached_analysis(root, &cache_relative, &key)
    {
        metrics::add(Counter::FactCacheHits, 1);
        record_analysis_stats(&analysis, false);
        return analysis;
    }
    metrics::add(Counter::FactCacheMisses, 1);
    let analysis = analyze_source(path, source, registry, needs, parsers);
    if let Some(root) = cache_root() {
        store_analysis(root, &cache_relative, &key, &analysis);
    }
    record_analysis_stats(&analysis, true);
    analysis
}

fn record_analysis_stats(analysis: &SourceAnalysis, parsed: bool) {
    if parsed {
        metrics::add(Counter::SyntaxNodes, analysis.stats.syntax_nodes);
    }
    metrics::add(Counter::Functions, analysis.stats.functions);
    metrics::add(Counter::NormalizedTokens, analysis.stats.normalized_tokens);
}

fn analyze_source(
    path: &str,
    source: &str,
    registry: &Registry,
    needs: RuleNeeds,
    parsers: &mut ParserPool,
) -> SourceAnalysis {
    let tree = match parse_source(path, source, registry, parsers) {
        Ok(tree) => tree,
        Err(error) => {
            return SourceAnalysis {
                errors: vec![error],
                ..SourceAnalysis::default()
            };
        }
    };
    let (facts, stats) = extract_facts(&tree, path, source, registry, needs);
    SourceAnalysis {
        facts,
        errors: Vec::new(),
        stats,
    }
}

pub fn check(input: &Input, registry: &Registry) -> Report {
    let mut report = Report::new(registry, &input.policy, input.mode, &input.implementations);
    report.input_sha256 = input.digest.clone();
    report.scanned_files = input.files.keys().cloned().collect();
    validate_required_rules(registry, input, &mut report);
    let needs = RuleNeeds::from_policy(&input.policy, &registry.language);
    let sources = input.files.iter().collect::<Vec<_>>();
    let analyses = sources
        .par_iter()
        .map_init(ParserPool::default, |parsers, (path, source)| {
            cached_or_analyze_source(path, source, registry, needs, parsers)
        })
        .collect::<Vec<_>>();
    let mut facts = Facts::default();
    for mut analysis in analyses {
        facts.functions.append(&mut analysis.facts.functions);
        facts.classes.append(&mut analysis.facts.classes);
        report.errors.append(&mut analysis.errors);
    }
    source_metrics(&facts, registry, input, &mut report);
    patterns(&facts, &input.policy, &registry.language, &mut report);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_rejects_tampering_and_symlink_redirection() {
        let base =
            std::env::temp_dir().join(format!("smells-portable-cache-test-{}", std::process::id()));
        fs::create_dir(&base).unwrap();
        let root_path = base.join("cache");
        let root = initialize_cache_root(&root_path).unwrap();
        let relative = Path::new("entry.json");
        let path = root_path.join(relative);
        let key = "a".repeat(64);
        let analysis = SourceAnalysis::default();
        store_analysis(&root, relative, &key, &analysis);
        assert!(cached_analysis(&root, relative, &key).is_some());

        let mut entry: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        entry["analysis_sha256"] = json!("0".repeat(64));
        fs::write(&path, serde_json::to_vec(&entry).unwrap()).unwrap();
        assert!(cached_analysis(&root, relative, &key).is_none());
        store_analysis(&root, relative, &key, &analysis);
        assert!(cached_analysis(&root, relative, &key).is_some());

        #[cfg(unix)]
        {
            let link = root_path.join("entry-link.json");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(cached_analysis(&root, Path::new("entry-link.json"), &key).is_none());

            let actual = root_path.join("actual");
            fs::create_dir(&actual).unwrap();
            fs::copy(&path, actual.join(relative)).unwrap();
            std::os::unix::fs::symlink(&actual, root_path.join("linked-directory")).unwrap();
            let linked_entry = Path::new("linked-directory/entry.json");
            assert!(cached_analysis(&root, linked_entry, &key).is_none());
            store_analysis(&root, linked_entry, &key, &analysis);
            assert!(cached_analysis(&root, linked_entry, &key).is_none());

            let moved = base.join("moved-cache");
            let attacker = base.join("attacker-cache");
            fs::create_dir(&attacker).unwrap();
            fs::rename(&root_path, &moved).unwrap();
            std::os::unix::fs::symlink(&attacker, &root_path).unwrap();
            fs::write(attacker.join(relative), b"not cache evidence").unwrap();
            assert!(cached_analysis(&root, relative, &key).is_some());
            store_analysis(&root, relative, &key, &analysis);
            assert!(cached_analysis(&root, relative, &key).is_some());
            assert_eq!(
                fs::read(attacker.join(relative)).unwrap(),
                b"not cache evidence"
            );
            assert!(initialize_cache_root(&root_path).is_none());
            fs::remove_file(&root_path).unwrap();
        }

        fs::remove_dir_all(base).unwrap();
    }
}
