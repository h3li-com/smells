use crate::{
    input::Input,
    policy::{Policy, Registry},
    report::{Location, Report},
};
use regex::Regex;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::OnceLock,
};
use tree_sitter::{Language, Node, Parser, Tree};

struct FunctionFact {
    symbol: String,
    location: Location,
    parameters: Vec<(String, String)>,
    lines: usize,
    comments: usize,
    tokens: Vec<String>,
    body_range: (usize, usize),
}

struct ClassFact {
    symbol: String,
    location: Location,
    fields: usize,
    methods: usize,
    method_lines: usize,
    operations: usize,
}

type FingerprintFeature<'a> = (&'a [String], usize);
type FeaturePostings<'a> = BTreeMap<FingerprintFeature<'a>, Vec<(usize, usize)>>;

#[derive(Default)]
struct Facts {
    functions: Vec<FunctionFact>,
    classes: Vec<ClassFact>,
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

fn collect_code_rows(node: Node<'_>, rows: &mut BTreeSet<usize>) {
    if node.kind().contains("comment") {
        return;
    }
    if node.child_count() == 0 {
        if matches!(
            node.kind(),
            "{" | "}" | "(" | ")" | "[" | "]" | "," | ";" | ":"
        ) {
            return;
        }
        let start = node.start_position().row;
        let end = node.end_position();
        let exclusive_end = end.row + usize::from(end.column > 0);
        rows.extend(start..exclusive_end.max(start + 1));
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_code_rows(child, rows);
    }
}

fn code_lines(node: Node<'_>, _source: &str) -> usize {
    let mut rows = BTreeSet::new();
    collect_code_rows(node, &mut rows);
    rows.len()
}

fn collect_comment_rows(node: Node<'_>, rows: &mut BTreeSet<usize>) {
    if node.kind().contains("comment") {
        let start = node.start_position().row;
        let end = node.end_position();
        let exclusive_end = end.row + usize::from(end.column > 0);
        rows.extend(start..exclusive_end.max(start + 1));
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_comment_rows(child, rows);
    }
}

fn comment_lines(node: Node<'_>) -> usize {
    let mut comments = BTreeSet::new();
    collect_comment_rows(node, &mut comments);
    let mut code = BTreeSet::new();
    collect_code_rows(node, &mut code);
    comments.difference(&code).count()
}

fn collect_tokens(node: Node<'_>, tokens: &mut Vec<String>) {
    if node.kind().contains("comment") {
        return;
    }
    if node.child_count() == 0 {
        let kind = node.kind();
        tokens.push(if kind.contains("identifier") || kind == "identifier" {
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
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_tokens(child, tokens);
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

fn only_statement(body: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = body.walk();
    let statements: Vec<_> = body.named_children(&mut cursor).collect();
    (statements.len() == 1).then_some(statements[0])
}

fn returned_receiver_field(statement: Node<'_>, source: &str, language: &str) -> bool {
    statement.kind() == "return_statement"
        && statement
            .named_child(0)
            .is_some_and(|field| receiver_field(field, source, language))
}

fn assignment_sides<'tree>(
    statement: Node<'tree>,
    language: &str,
) -> Option<(Node<'tree>, Node<'tree>)> {
    let assignment_kind = if language == "python" {
        "assignment"
    } else {
        "assignment_expression"
    };
    let assignment = first_descendant(statement, &[assignment_kind])?;
    Some((
        assignment.child_by_field_name("left")?,
        assignment.child_by_field_name("right")?,
    ))
}

fn assignment_accessor(
    method: Node<'_>,
    statement: Node<'_>,
    source: &str,
    language: &str,
) -> bool {
    let Some((left, right)) = assignment_sides(statement, language) else {
        return false;
    };
    let ordinary: Vec<_> = parameters(method, source)
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| !matches!(name.as_str(), "self" | "cls" | "this"))
        .collect();
    receiver_field(left, source, language)
        && ordinary.len() == 1
        && right.kind() == "identifier"
        && compact_text(right, source) == ordinary[0]
}

fn strict_accessor(method: Node<'_>, source: &str, language: &str) -> bool {
    let name = node_name(method, source);
    if matches!(name.as_str(), "__init__" | "constructor") {
        return true;
    }
    let Some(body) = method.child_by_field_name("body") else {
        return false;
    };
    let Some(statement) = only_statement(body) else {
        return false;
    };
    returned_receiver_field(statement, source, language)
        || assignment_accessor(method, statement, source, language)
}

fn node_name(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("name")
        .and_then(|name| name.utf8_text(source.as_bytes()).ok())
        .unwrap_or("<anonymous>")
        .to_string()
}

fn function_metrics(
    node: Node<'_>,
    path: &str,
    source: &str,
    prefix: &str,
    input: &Input,
    report: &mut Report,
    facts: &mut Facts,
) {
    let symbol = node_name(node, source);
    let location = location(path, node);
    let declared_parameters = parameters(node, source);
    report.maximum(
        &input.policy,
        &format!("{prefix}.function_arguments"),
        &symbol,
        &location,
        declared_parameters.len(),
        "declared parameters",
    );
    if let Some(body) = node.child_by_field_name("body") {
        let lines = code_lines(body, source);
        report.maximum(
            &input.policy,
            &format!("{prefix}.function_lines"),
            &symbol,
            &location,
            lines,
            "authored body code lines",
        );
        let mut tokens = Vec::new();
        collect_tokens(body, &mut tokens);
        facts.functions.push(FunctionFact {
            symbol,
            location,
            parameters: declared_parameters,
            lines,
            comments: comment_lines(node),
            tokens,
            body_range: (body.start_byte(), body.end_byte()),
        });
    }
}

fn direct_methods<'tree>(body: Node<'tree>, language: &str) -> Vec<Node<'tree>> {
    let mut result = Vec::new();
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        match (language, child.kind()) {
            ("python", "function_definition")
            | ("typescript", "method_definition")
            | ("typescript", "abstract_method_signature")
            | ("typescript", "method_signature") => {
                result.push(child);
            }
            ("python", "decorated_definition") => {
                let mut decorated = child.walk();
                if let Some(function) = child
                    .named_children(&mut decorated)
                    .find(|node| node.kind() == "function_definition")
                {
                    result.push(function);
                }
            }
            _ => {}
        }
    }
    result
}

fn field_name(node: Node<'_>, source: &str, receiver: bool) -> Option<String> {
    if !receiver && matches!(node.kind(), "identifier" | "pattern") {
        return node.utf8_text(source.as_bytes()).ok().map(str::to_string);
    }
    if receiver && node.kind() == "attribute" {
        let owner = node
            .child_by_field_name("object")?
            .utf8_text(source.as_bytes())
            .ok()?;
        if matches!(owner, "self" | "cls") {
            return node
                .child_by_field_name("attribute")?
                .utf8_text(source.as_bytes())
                .ok()
                .map(str::to_string);
        }
    }
    None
}

fn assignment_target_fields(
    node: Node<'_>,
    source: &str,
    receiver: bool,
    fields: &mut BTreeSet<String>,
) {
    if let Some(name) = field_name(node, source, receiver) {
        fields.insert(name);
        return;
    }
    if matches!(node.kind(), "attribute" | "subscript" | "member_expression") {
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        assignment_target_fields(child, source, receiver, fields);
    }
}

fn assignment_fields(node: Node<'_>, source: &str, receiver: bool, fields: &mut BTreeSet<String>) {
    if matches!(node.kind(), "function_definition" | "class_definition") {
        return;
    }
    if matches!(node.kind(), "assignment" | "augmented_assignment")
        && let Some(left) = node.child_by_field_name("left")
    {
        assignment_target_fields(left, source, receiver, fields);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        assignment_fields(child, source, receiver, fields);
    }
}

fn python_fields(body: Node<'_>, methods: &[Node<'_>], source: &str) -> BTreeSet<String> {
    let mut fields = BTreeSet::new();
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if !matches!(child.kind(), "function_definition" | "decorated_definition") {
            assignment_fields(child, source, false, &mut fields);
        }
    }
    for method in methods {
        let Some(method_body) = method.child_by_field_name("body") else {
            continue;
        };
        let mut cursor = method_body.walk();
        for child in method_body.named_children(&mut cursor) {
            assignment_fields(child, source, true, &mut fields);
        }
    }
    fields
}

fn declared_typescript_fields(body: Node<'_>, source: &str) -> BTreeSet<String> {
    let mut fields = BTreeSet::new();
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind() == "public_field_definition"
            && let Some(name) = child.child_by_field_name("name")
            && let Ok(name) = name.utf8_text(source.as_bytes())
        {
            fields.insert(name.to_string());
        }
    }
    fields
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

fn typescript_fields(body: Node<'_>, methods: &[Node<'_>], source: &str) -> BTreeSet<String> {
    let mut fields = declared_typescript_fields(body, source);
    for method in methods {
        if node_name(*method, source) != "constructor" {
            continue;
        }
        let Some(parameters) = method.child_by_field_name("parameters") else {
            continue;
        };
        let mut cursor = parameters.walk();
        fields.extend(
            parameters
                .named_children(&mut cursor)
                .filter_map(|parameter| parameter_property_name(parameter, source)),
        );
    }
    fields
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

fn class_metrics(
    node: Node<'_>,
    path: &str,
    source: &str,
    registry: &Registry,
    input: &Input,
    report: &mut Report,
    facts: &mut Facts,
) {
    let Some(body) = node.child_by_field_name("body") else {
        return;
    };
    let symbol = node_name(node, source);
    let location = location(path, node);
    let methods = direct_methods(body, &registry.language);
    let fields = if registry.language == "python" {
        python_fields(body, &methods, source)
    } else {
        typescript_fields(body, &methods, source)
    };
    let method_lines: usize = methods
        .iter()
        .filter_map(|method| method.child_by_field_name("body"))
        .map(|body| code_lines(body, source))
        .sum();
    record_class_metrics(
        &ClassSummary {
            symbol: &symbol,
            location: &location,
            fields: fields.len(),
            methods: methods.len(),
            method_lines,
        },
        registry,
        input,
        report,
    );
    facts.classes.push(ClassFact {
        symbol,
        location,
        fields: fields.len(),
        methods: methods.len(),
        method_lines,
        operations: methods
            .iter()
            .filter(|method| !strict_accessor(**method, source, &registry.language))
            .count(),
    });
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
        "typescript" => matches!(
            node.kind(),
            "arrow_function"
                | "function_declaration"
                | "function_expression"
                | "generator_function"
                | "generator_function_declaration"
                | "method_definition"
        ),
        _ => false,
    }
}

fn visit(
    node: Node<'_>,
    path: &str,
    source: &str,
    registry: &Registry,
    input: &Input,
    report: &mut Report,
    facts: &mut Facts,
) {
    if is_class(node, &registry.language) {
        class_metrics(node, path, source, registry, input, report, facts);
    }
    if is_function(node, &registry.language) {
        function_metrics(node, path, source, &registry.language, input, report, facts);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(child, path, source, registry, input, report, facts);
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

fn fingerprint(tokens: &[String]) -> BTreeMap<Vec<String>, usize> {
    let mut result = BTreeMap::new();
    for window in tokens.windows(4) {
        *result.entry(window.to_vec()).or_insert(0) += 1;
    }
    result
}

fn feature_frequencies<'a>(
    facts: &Facts,
    fingerprints: &'a [BTreeMap<Vec<String>, usize>],
    minimum: u64,
) -> BTreeMap<FingerprintFeature<'a>, usize> {
    let mut frequencies = BTreeMap::new();
    for (index, counts) in fingerprints.iter().enumerate() {
        if (facts.functions[index].tokens.len() as u64) < minimum {
            continue;
        }
        for (key, count) in counts {
            for occurrence in 0..*count {
                *frequencies.entry((key.as_slice(), occurrence)).or_default() += 1;
            }
        }
    }
    frequencies
}

fn ordered_features<'a>(
    facts: &Facts,
    fingerprints: &'a [BTreeMap<Vec<String>, usize>],
    frequencies: &BTreeMap<FingerprintFeature<'a>, usize>,
    minimum: u64,
) -> Vec<Vec<FingerprintFeature<'a>>> {
    fingerprints
        .iter()
        .enumerate()
        .map(|(index, counts)| {
            if (facts.functions[index].tokens.len() as u64) < minimum {
                return Vec::new();
            }
            let mut features = counts
                .iter()
                .flat_map(|(key, count)| {
                    (0..*count).map(move |occurrence| (key.as_slice(), occurrence))
                })
                .collect::<Vec<_>>();
            features.sort_by(|left, right| {
                frequencies[left]
                    .cmp(&frequencies[right])
                    .then_with(|| left.cmp(right))
            });
            features
        })
        .collect()
}

fn eligible_functions(ordered_features: &[Vec<FingerprintFeature<'_>>]) -> Vec<usize> {
    let mut eligible = (0..ordered_features.len())
        .filter(|index| !ordered_features[*index].is_empty())
        .collect::<Vec<_>>();
    eligible.sort_by_key(|index| (ordered_features[*index].len(), *index));
    eligible
}

fn prefix_length(size: usize, similarity: u64) -> usize {
    let required_overlap = (similarity as u128 * size as u128).div_ceil(10_000) as usize;
    size - required_overlap + 1
}

fn overlapping_functions(first: &FunctionFact, second: &FunctionFact) -> bool {
    first.location.path == second.location.path
        && first.body_range.0 < second.body_range.1
        && second.body_range.0 < first.body_range.1
}

fn candidate_possible(
    left_size: usize,
    right_size: usize,
    left_position: usize,
    right_position: usize,
    similarity: u64,
) -> bool {
    if left_size as u128 * 10_000 < similarity as u128 * right_size as u128 {
        return false;
    }
    let maximum_overlap = 1 + (left_size - left_position - 1).min(right_size - right_position - 1);
    let required_overlap = (similarity as u128 * (left_size + right_size) as u128)
        .div_ceil(10_000 + similarity as u128);
    maximum_overlap as u128 >= required_overlap
}

fn duplicate_candidates(
    facts: &Facts,
    right: usize,
    ordered_features: &[Vec<FingerprintFeature<'_>>],
    postings: &FeaturePostings<'_>,
    similarity: u64,
) -> BTreeSet<usize> {
    let right_size = ordered_features[right].len();
    let mut candidates = BTreeSet::new();
    for (right_position, feature) in ordered_features[right]
        .iter()
        .take(prefix_length(right_size, similarity))
        .enumerate()
    {
        let Some(entries) = postings.get(feature) else {
            continue;
        };
        for (left, left_position) in entries {
            if !overlapping_functions(&facts.functions[*left], &facts.functions[right])
                && candidate_possible(
                    ordered_features[*left].len(),
                    right_size,
                    *left_position,
                    right_position,
                    similarity,
                )
            {
                candidates.insert(*left);
            }
        }
    }
    candidates
}

fn multiset_similarity(
    first: &BTreeMap<Vec<String>, usize>,
    second: &BTreeMap<Vec<String>, usize>,
) -> Option<(usize, usize)> {
    let keys: BTreeSet<_> = first.keys().chain(second.keys()).collect();
    let (mut intersection, mut union) = (0, 0);
    for key in keys {
        let first_count = *first.get(key).unwrap_or(&0);
        let second_count = *second.get(key).unwrap_or(&0);
        intersection += first_count.min(second_count);
        union += first_count.max(second_count);
    }
    (union > 0).then_some((intersection, union))
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

fn compare_duplicate_pair(
    facts: &Facts,
    fingerprints: &[BTreeMap<Vec<String>, usize>],
    left: usize,
    right: usize,
    rule: &DuplicateRule<'_>,
    report: &mut Report,
) {
    let Some((intersection, union)) =
        multiset_similarity(&fingerprints[left], &fingerprints[right])
    else {
        return;
    };
    if intersection as u128 * 10_000 >= rule.similarity as u128 * union as u128 {
        report_duplicate_pair(
            &facts.functions[left],
            &facts.functions[right],
            intersection,
            union,
            rule,
            report,
        );
    }
}

fn add_to_postings<'a>(
    right: usize,
    features: &[FingerprintFeature<'a>],
    prefix_length: usize,
    postings: &mut FeaturePostings<'a>,
) {
    for (position, feature) in features.iter().take(prefix_length).enumerate() {
        postings
            .entry(*feature)
            .or_default()
            .push((right, position));
    }
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
    let fingerprints: Vec<_> = facts
        .functions
        .iter()
        .map(|function| fingerprint(&function.tokens))
        .collect();
    let frequencies = feature_frequencies(facts, &fingerprints, minimum);
    let ordered_features = ordered_features(facts, &fingerprints, &frequencies, minimum);
    let eligible = eligible_functions(&ordered_features);
    let mut postings = FeaturePostings::new();
    let mut comparisons = 0;
    for right in eligible {
        let right_size = ordered_features[right].len();
        let candidates =
            duplicate_candidates(facts, right, &ordered_features, &postings, similarity);
        for left in candidates {
            comparisons += 1;
            if comparisons > policy.limits.maximum_pairs {
                report.errors.push("maximum_pairs budget exceeded".into());
                return;
            }
            compare_duplicate_pair(facts, &fingerprints, left, right, &rule, report);
        }
        add_to_postings(
            right,
            &ordered_features[right],
            prefix_length(right_size, similarity),
            &mut postings,
        );
    }
}

fn patterns(facts: &Facts, policy: &Policy, language: &str, report: &mut Report) {
    class_patterns(facts, policy, language, report);
    comments(facts, policy, language, report);
    clumps(facts, policy, language, report);
    duplicates(facts, policy, language, report);
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

fn configured_parser(path: &str, registry: &Registry, report: &mut Report) -> Option<Parser> {
    let grammar = language(path, registry)
        .map_err(|error| report.errors.push(error))
        .ok()?;
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|error| {
            report
                .errors
                .push(format!("cannot load grammar for {path}: {error}"));
        })
        .ok()?;
    Some(parser)
}

fn parsed_tree(parser: &mut Parser, source: &str, path: &str, report: &mut Report) -> Option<Tree> {
    parser.parse(source, None).or_else(|| {
        report.errors.push(format!("parser cancelled for {path}"));
        None
    })
}

fn compatible_typescript_tree(
    parser: &mut Parser,
    tree: Tree,
    source: &str,
    path: &str,
    registry: &Registry,
    report: &mut Report,
) -> Option<Tree> {
    if !tree.root_node().has_error() || registry.language != "typescript" {
        return Some(tree);
    }
    let Some(compatible) = typescript_parser_compatibility_source(source) else {
        return Some(tree);
    };
    parsed_tree(parser, &compatible, path, report)
}

fn parse_source(
    path: &str,
    source: &str,
    registry: &Registry,
    report: &mut Report,
) -> Option<Tree> {
    let mut parser = configured_parser(path, registry, report)?;
    let tree = parsed_tree(&mut parser, source, path, report)?;
    let tree = compatible_typescript_tree(&mut parser, tree, source, path, registry, report)?;
    if tree.root_node().has_error() {
        report.errors.push(format!("parse error in {path}"));
        return None;
    }
    Some(tree)
}

fn collect_source(
    path: &str,
    source: &str,
    registry: &Registry,
    input: &Input,
    report: &mut Report,
    facts: &mut Facts,
) {
    let Some(tree) = parse_source(path, source, registry, report) else {
        return;
    };
    visit(
        tree.root_node(),
        path,
        source,
        registry,
        input,
        report,
        facts,
    );
}

pub fn check(input: &Input, registry: &Registry) -> Report {
    let mut report = Report::new(registry, &input.policy, input.mode, &input.implementations);
    report.input_sha256 = input.digest.clone();
    report.scanned_files = input.files.keys().cloned().collect();
    validate_required_rules(registry, input, &mut report);
    let mut facts = Facts::default();
    for (path, source) in &input.files {
        collect_source(path, source, registry, input, &mut report, &mut facts);
    }
    patterns(&facts, &input.policy, &registry.language, &mut report);
    report.attach_sources(&input.files);
    report
}
