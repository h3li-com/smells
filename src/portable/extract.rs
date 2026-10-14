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

fn occupied_row_range(
    start_row: usize,
    end_row: usize,
    end_column: usize,
) -> std::ops::Range<usize> {
    let exclusive_end = end_row + usize::from(end_column > 0);
    start_row..exclusive_end.max(start_row + 1)
}

fn extend_node_rows(node: Node<'_>, rows: &mut BTreeSet<usize>) {
    let start = node.start_position().row;
    let end = node.end_position();
    rows.extend(occupied_row_range(start, end.row, end.column));
}

fn normalized_leaf_kind(kind: &str) -> String {
    if kind.contains("identifier") {
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

fn should_record_substantive_row(enabled: bool, substantive: bool) -> bool {
    enabled && substantive
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
    if should_record_substantive_row(needs.comments, substantive) {
        extend_node_rows(node, &mut metrics.callable_code_rows);
    }
    if node.start_byte() < body_range.0 || node.end_byte() > body_range.1 {
        return;
    }
    if should_record_substantive_row(needs.lines, substantive) {
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
        .or_else(|| {
            let parent = node.parent()?;
            (parent.kind() == "variable_declarator"
                && parent
                    .child_by_field_name("value")
                    .is_some_and(|value| value.id() == node.id()))
            .then(|| parent.child_by_field_name("name"))
            .flatten()
        })
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
    report.maximum_with_role(
        &input.policy,
        &format!("{}.class_method_lines", registry.language),
        summary.symbol,
        summary.location,
        crate::report::RoleAdjustedMaximum::new(
            summary.method_lines,
            "summed class method code lines",
            crate::migration::class_role(
                input,
                &summary.location.path,
                summary.symbol,
                summary.location,
            ),
        ),
    );
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
    owner: Option<String>,
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
    let body = if parent.kind() == "decorated_definition" {
        parent.parent()?
    } else {
        parent
    };
    (body.kind() == "block")
        .then(|| body.parent())
        .flatten()
        .filter(|owner| owner.kind() == "class_definition")
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

fn active_function_needs(needs: RuleNeeds, direct_class: bool) -> RuleNeeds {
    RuleNeeds {
        functions: needs.functions,
        parameters: needs.functions && needs.parameters || direct_class,
        lines: needs.functions && needs.lines || direct_class,
        comments: needs.functions && needs.comments,
        tokens: needs.functions && needs.tokens,
        classes: needs.classes,
    }
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
    let owner = direct_class_owner(node, language).map(|owner| node_name(owner, source));
    let direct_class = direct_class_index(node, language, classes);
    let record_fact = needs.functions;
    let metric_needs = active_function_needs(needs, direct_class.is_some());
    let declared_parameters = if metric_needs.parameters {
        parameters(node, source)
    } else {
        Vec::new()
    };
    let body = node.child_by_field_name("body");
    let body_range = body.map(|body| (body.start_byte(), body.end_byte()));
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
        owner,
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
    if accessor.matched {
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

fn span_is_within(inner: (usize, usize), outer: (usize, usize)) -> bool {
    inner.0 >= outer.0 && inner.1 <= outer.1
}

fn blocked_assignment_identifier(node: Node<'_>, target: &AssignmentTarget) -> bool {
    let mut parent = node.parent();
    while let Some(ancestor) = parent {
        if !span_is_within((ancestor.start_byte(), ancestor.end_byte()), target.range) {
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
    if !span_is_within((node.start_byte(), node.end_byte()), target.range) {
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

fn is_direct_constructor(direct_class: Option<usize>, symbol: &str, class_index: usize) -> bool {
    direct_class == Some(class_index) && symbol == "constructor"
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
            body.parent()
                .is_some_and(|class| class.id() == classes[class_index].node_id)
        })
        && let Some(name) = node.child_by_field_name("name")
        && let Ok(name) = name.utf8_text(source.as_bytes())
    {
        classes[class_index].fields.insert(name.to_string());
    }
    if functions.last().is_some_and(|function| {
        is_direct_constructor(function.direct_class, &function.symbol, class_index)
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

fn should_record_typescript_signature(classes: bool, language: &str, kind: &str) -> bool {
    classes
        && language == "typescript"
        && matches!(kind, "abstract_method_signature" | "method_signature")
}

fn record_typescript_signature(
    node: Node<'_>,
    source: &str,
    language: &str,
    needs: RuleNeeds,
    classes: &mut [ActiveClass],
) {
    if !should_record_typescript_signature(needs.classes, language, node.kind()) {
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

fn should_track_python_assignments(classes: bool, language: &str) -> bool {
    classes && language == "python"
}

fn start_python_assignment(
    node: Node<'_>,
    language: &str,
    needs: RuleNeeds,
    state: &mut ExtractionState,
) {
    if !should_track_python_assignments(needs.classes, language) {
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
                    owner: function.owner,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_nodes_of_kind<'tree>(node: Node<'tree>, kind: &str, matches: &mut Vec<Node<'tree>>) {
        if node.kind() == kind {
            matches.push(node);
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect_nodes_of_kind(child, kind, matches);
        }
    }

    #[test]
    fn occupied_rows_use_tree_sitters_exclusive_end_position() {
        assert_eq!(occupied_row_range(3, 3, 0), 3..4);
        assert_eq!(occupied_row_range(3, 3, 8), 3..4);
        assert_eq!(occupied_row_range(2, 5, 0), 2..5);
        assert_eq!(occupied_row_range(2, 5, 1), 2..6);
    }

    #[test]
    fn substantive_rows_are_recorded_only_when_the_measurement_is_enabled() {
        assert!(!should_record_substantive_row(false, false));
        assert!(!should_record_substantive_row(false, true));
        assert!(!should_record_substantive_row(true, false));
        assert!(should_record_substantive_row(true, true));
    }

    #[test]
    fn arrow_functions_inherit_only_a_direct_variable_declarator_name() {
        let source = concat!(
            "const assigned = () => 1;\n",
            "const object = { nested: () => 2 };\n",
            "class Service { handler = () => 3; }\n",
        );
        let registry = crate::policy::registry("typescript-v1").unwrap();
        let mut parsers = ParserPool::default();
        let tree = parse_source("arrows.ts", source, &registry, &mut parsers)
            .unwrap_or_else(|error| panic!("{}", error.message));
        let mut arrows = Vec::new();
        collect_nodes_of_kind(tree.root_node(), "arrow_function", &mut arrows);

        assert_eq!(arrows.len(), 3);
        assert_eq!(node_name(arrows[0], source), "assigned");
        assert_eq!(node_name(arrows[1], source), "<anonymous>");
        assert_eq!(node_name(arrows[2], source), "<anonymous>");
    }

    #[test]
    fn python_function_owner_means_direct_class_owner_only() {
        let source = concat!(
            "def top_level():\n",
            "    return 1\n",
            "\n",
            "class Service:\n",
            "    @staticmethod\n",
            "    def decorated():\n",
            "        return 2\n",
            "\n",
            "    def outer(self):\n",
            "        def nested():\n",
            "            return 3\n",
            "        return nested()\n",
        );
        let (facts, _) = facts("python-v1", "owners.py", source);
        let owners = facts
            .functions
            .iter()
            .map(|function| (function.symbol.as_str(), function.owner.as_deref()))
            .collect::<Vec<_>>();

        assert_eq!(
            owners,
            [
                ("top_level", None),
                ("decorated", Some("Service")),
                ("outer", Some("Service")),
                ("nested", None),
            ]
        );
    }

    #[test]
    fn active_function_measurements_follow_the_rule_needs_matrix() {
        for mask in 0_u8..64 {
            let needs = RuleNeeds {
                functions: mask & 1 != 0,
                parameters: mask & 2 != 0,
                lines: mask & 4 != 0,
                comments: mask & 8 != 0,
                tokens: mask & 16 != 0,
                classes: mask & 32 != 0,
            };
            for direct_class in [false, true] {
                let active = active_function_needs(needs, direct_class);
                assert_eq!(active.functions, needs.functions);
                assert_eq!(
                    active.parameters,
                    needs.functions && needs.parameters || direct_class
                );
                assert_eq!(active.lines, needs.functions && needs.lines || direct_class);
                assert_eq!(active.comments, needs.functions && needs.comments);
                assert_eq!(active.tokens, needs.functions && needs.tokens);
                assert_eq!(active.classes, needs.classes);
            }
        }
    }

    #[test]
    fn source_spans_are_within_a_target_only_when_both_boundaries_fit() {
        assert!(span_is_within((10, 20), (10, 20)));
        assert!(span_is_within((11, 19), (10, 20)));
        assert!(!span_is_within((9, 19), (10, 20)));
        assert!(!span_is_within((11, 21), (10, 20)));
        assert!(!span_is_within((9, 21), (10, 20)));
    }

    #[test]
    fn parameter_properties_belong_only_to_the_current_direct_constructor() {
        assert!(is_direct_constructor(Some(2), "constructor", 2));
        assert!(!is_direct_constructor(Some(2), "work", 2));
        assert!(!is_direct_constructor(Some(1), "constructor", 2));
        assert!(!is_direct_constructor(None, "constructor", 2));
    }

    #[test]
    fn typescript_signatures_require_the_rule_language_and_signature_kind() {
        assert!(should_record_typescript_signature(
            true,
            "typescript",
            "method_signature"
        ));
        assert!(should_record_typescript_signature(
            true,
            "typescript",
            "abstract_method_signature"
        ));
        assert!(!should_record_typescript_signature(
            false,
            "typescript",
            "method_signature"
        ));
        assert!(!should_record_typescript_signature(
            true,
            "python",
            "method_signature"
        ));
        assert!(!should_record_typescript_signature(
            true,
            "typescript",
            "method_definition"
        ));
    }

    #[test]
    fn python_assignment_tracking_requires_class_rules_and_python() {
        assert!(should_track_python_assignments(true, "python"));
        assert!(!should_track_python_assignments(false, "python"));
        assert!(!should_track_python_assignments(true, "typescript"));
        assert!(!should_track_python_assignments(false, "typescript"));
    }

    fn facts(pack: &str, path: &str, source: &str) -> (Facts, SourceStats) {
        let registry = crate::policy::registry(pack).unwrap();
        let mut parsers = ParserPool::default();
        let tree = parse_source(path, source, &registry, &mut parsers)
            .unwrap_or_else(|error| panic!("{}", error.message));
        extract_facts(
            &tree,
            path,
            source,
            &registry,
            RuleNeeds {
                functions: true,
                parameters: true,
                lines: true,
                comments: true,
                tokens: true,
                classes: true,
            },
        )
    }

    #[test]
    fn typescript_method_fact_keeps_its_class_owner_without_class_rules() {
        let source = concat!(
            "export class Accounts implements AppMigration {\n",
            "  async up(queryRunner: QueryRunner) {\n",
            "    await queryRunner.query('CREATE TABLE accounts (id integer)');\n",
            "  }\n",
            "}\n",
        );
        let registry = crate::policy::registry("typescript-v1").unwrap();
        let mut parsers = ParserPool::default();
        let tree = parse_source("accounts.ts", source, &registry, &mut parsers)
            .unwrap_or_else(|error| panic!("{}", error.message));
        let needs = RuleNeeds {
            functions: true,
            parameters: false,
            lines: true,
            comments: false,
            tokens: false,
            classes: false,
        };

        let (facts, _) = extract_facts(&tree, "accounts.ts", source, &registry, needs);

        assert_eq!(facts.functions[0].symbol, "up");
        assert_eq!(facts.functions[0].owner.as_deref(), Some("Accounts"));
    }

    #[test]
    fn python_extraction_preserves_owned_methods_fields_accessors_and_metrics() {
        let source = concat!(
            "class Account:\n",
            "    class_field = 1\n",
            "    external.field = 2\n",
            "    self.phantom = 3\n",
            "    left_field, right_field = 1, 2\n",
            "\n",
            "    def __init__(self, value: int):\n",
            "        self.value = value\n",
            "        self.mapping['key'] = value\n",
            "\n",
            "    def current(self) -> int:\n",
            "        return self.value\n",
            "\n",
            "    def set_name(self, name: str):\n",
            "        self.name = name\n",
            "\n",
            "    def work(self, count: int):\n",
            "        # explanation only\n",
            "        result = count + 1\n",
            "        return result\n",
        );
        let (facts, stats) = facts("python-v1", "account.py", source);

        assert_eq!(
            facts
                .functions
                .iter()
                .map(|function| function.symbol.as_str())
                .collect::<Vec<_>>(),
            ["__init__", "current", "set_name", "work"]
        );
        assert!(
            facts
                .functions
                .iter()
                .all(|function| function.owner.as_deref() == Some("Account"))
        );
        assert_eq!(facts.functions[0].location.line, 7);
        assert_eq!(
            facts.functions[0].parameters[1],
            ("value".into(), "int".into())
        );
        assert_eq!(facts.functions[3].comments, 1);
        assert!(facts.functions[3].lines >= 2);
        assert!(!facts.functions[3].tokens.is_empty());

        assert_eq!(facts.classes.len(), 1);
        let account = &facts.classes[0];
        assert_eq!(account.symbol, "Account");
        assert_eq!(account.location.line, 1);
        assert_eq!(account.fields, 6);
        assert_eq!(account.methods, 4);
        assert_eq!(account.operations, 1);
        assert!(account.method_lines >= 5);
        assert_eq!(stats.functions, 4);
        assert!(stats.syntax_nodes > stats.functions);
        assert_eq!(
            stats.normalized_tokens,
            facts
                .functions
                .iter()
                .map(|function| function.tokens.len())
                .sum::<usize>()
        );
    }

    #[test]
    fn typescript_extraction_counts_parameter_properties_and_strict_accessors() {
        let source = concat!(
            "export class Service {\n",
            "  public field: string;\n",
            "  constructor(public id: number, private readonly token: string) {\n",
            "    this.field = token;\n",
            "  }\n",
            "  current(): string { return this.field; }\n",
            "  setField(field: string): void { this.field = field; }\n",
            "  work(value: number): number { return value + 1; }\n",
            "}\n",
        );
        let (facts, stats) = facts("typescript-v1", "service.ts", source);

        assert_eq!(
            facts
                .functions
                .iter()
                .map(|function| function.symbol.as_str())
                .collect::<Vec<_>>(),
            ["constructor", "current", "setField", "work"]
        );
        assert!(
            facts
                .functions
                .iter()
                .all(|function| function.owner.as_deref() == Some("Service"))
        );
        assert_eq!(
            facts.functions[0].parameters[0],
            ("id".into(), ":number".into())
        );
        assert_eq!(
            facts.functions[0].parameters[1],
            ("token".into(), ":string".into())
        );

        assert_eq!(facts.classes.len(), 1);
        let service = &facts.classes[0];
        assert_eq!(service.symbol, "Service");
        assert_eq!(service.fields, 3);
        assert_eq!(service.methods, 4);
        assert_eq!(service.operations, 1);
        assert!(service.method_lines >= 4);
        assert_eq!(stats.functions, 4);
        assert_eq!(
            stats.normalized_tokens,
            facts
                .functions
                .iter()
                .map(|function| function.tokens.len())
                .sum::<usize>()
        );
    }

    #[test]
    fn extraction_preserves_source_order_across_multiple_classes_and_functions() {
        let source = concat!(
            "def before():\n    return 1\n\n",
            "class First:\n",
            "    def run(self):\n        return 2\n\n",
            "def between():\n    return 3\n\n",
            "class Second:\n",
            "    def run(self):\n        return 4\n",
        );
        let (facts, stats) = facts("python-v1", "ordered.py", source);

        assert_eq!(
            facts
                .functions
                .iter()
                .map(|function| (function.symbol.as_str(), function.owner.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("before", None),
                ("run", Some("First")),
                ("between", None),
                ("run", Some("Second")),
            ]
        );
        assert_eq!(
            facts
                .classes
                .iter()
                .map(|class| class.symbol.as_str())
                .collect::<Vec<_>>(),
            ["First", "Second"]
        );
        assert_eq!(stats.functions, 4);
    }

    #[test]
    fn nested_classes_preserve_lexical_order_in_extracted_facts() {
        let source = concat!(
            "class Outer:\n",
            "    class Inner:\n",
            "        pass\n",
            "\n",
            "class After:\n",
            "    pass\n",
        );
        let (facts, _) = facts("python-v1", "nested.py", source);

        assert_eq!(
            facts
                .classes
                .iter()
                .map(|class| class.symbol.as_str())
                .collect::<Vec<_>>(),
            ["Outer", "Inner", "After"]
        );
    }

    #[test]
    fn accessor_classification_rejects_near_miss_getters_and_setters() {
        let source = concat!(
            "class Account:\n",
            "    def __init__(self, value):\n        self.value = value\n",
            "    def get_exact(self):\n        return self.value\n",
            "    def get_other(self):\n        return other.value\n",
            "    def get_with_setup(self):\n        prepared = 1\n        return self.value\n",
            "    def set_exact(self, value):\n        self.value = value\n",
            "    def set_wrong_value(self, value):\n        self.value = other\n",
            "    def set_with_return(self, value):\n        self.value = value\n        return value\n",
        );
        let (python_facts, _) = facts("python-v1", "accessors.py", source);

        let account = &python_facts.classes[0];
        assert_eq!(account.methods, 7);
        assert_eq!(account.fields, 1);
        assert_eq!(account.operations, 4);

        let typescript = concat!(
            "class Account {\n",
            "  setExact(value: number) { this.value = value; }\n",
            "  setChained(value: number) { this.value = this.other = value; }\n",
            "}\n",
        );
        let (facts, _) = facts("typescript-v1", "accessors.ts", typescript);
        assert_eq!(facts.classes[0].methods, 2);
        assert_eq!(facts.classes[0].operations, 1);
    }

    #[test]
    fn typescript_abstract_signatures_count_only_direct_class_operations() {
        let source = concat!(
            "export abstract class Contract {\n",
            "  constructor(public id: number);\n",
            "  abstract execute(value: number): void;\n",
            "  abstract reset(): void;\n",
            "}\n",
            "export class Concrete {\n",
            "  run(): void {}\n",
            "}\n",
        );
        let (facts, stats) = facts("typescript-v1", "contract.ts", source);

        assert_eq!(facts.classes.len(), 2);
        assert_eq!(facts.classes[0].symbol, "Contract");
        assert_eq!(facts.classes[0].methods, 3);
        assert_eq!(facts.classes[0].operations, 2);
        assert_eq!(facts.classes[1].symbol, "Concrete");
        assert_eq!(facts.classes[1].methods, 1);
        assert_eq!(facts.classes[1].operations, 1);
        assert_eq!(stats.functions, 1);
    }
}
