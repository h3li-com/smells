use super::*;

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

pub(super) struct ClassSummary<'a> {
    pub(super) symbol: &'a str,
    pub(super) location: &'a Location,
    pub(super) fields: usize,
    pub(super) methods: usize,
    pub(super) method_lines: usize,
}

pub(super) fn record_class_metrics(
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

pub(super) fn extract_facts(
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
