use super::extract::{ClassSummary, record_class_metrics};
use super::*;

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
            report.error(error);
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
        report.finding_with_relations(
            policy,
            &id,
            &first.symbol,
            &first.location,
            "repeated named syntax group",
            json!({"slots":group.len(),"declarations":functions.len()}),
            "both >=",
            json!({"minimum_group_size":size,"minimum_declarations":minimum}),
            true,
            FindingRelations {
                evidence: json!({"slots":group,"supporting_symbols":functions.iter().map(|function|&function.symbol).collect::<Vec<_>>(),"type_interpretation":"exact_authored_syntax"}),
                symbols: functions
                    .iter()
                    .skip(1)
                    .map(|function| function.symbol.clone())
                    .collect(),
                locations: functions
                    .iter()
                    .skip(1)
                    .map(|function| function.location.clone())
                    .collect(),
            },
        );
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
    report.finding_with_relations(
        rule.policy,
        rule.id,
        &first.symbol,
        &first.location,
        "normalized 4-token multiset Jaccard",
        json!({"intersection":intersection,"union":union}),
        ">=",
        json!({"minimum_similarity_basis_points":rule.similarity,"minimum_tokens":rule.minimum}),
        true,
        FindingRelations {
            evidence: json!({"token_counts":[first.tokens.len(),second.tokens.len()],"other_location":second.location,"normalization":"tree_sitter_tokens_v1_not_semantic_equivalence"}),
            symbols: vec![second.symbol.clone()],
            locations: vec![second.location.clone()],
        },
    );
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
            report.rule_error(&id, error);
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

pub(super) fn patterns(facts: &Facts, policy: &Policy, language: &str, report: &mut Report) {
    class_patterns(facts, policy, language, report);
    comments(facts, policy, language, report);
    clumps(facts, policy, language, report);
    duplicates(facts, policy, language, report);
}

pub(super) fn source_metrics(
    facts: &Facts,
    registry: &Registry,
    input: &Input,
    report: &mut Report,
) {
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

pub(super) fn validate_required_rules(registry: &Registry, input: &Input, report: &mut Report) {
    for rule in &registry.rules {
        if rule.implementation == "not_implemented" && input.policy.required(&rule.id) {
            report.error(format!("required detector not implemented: {}", rule.id));
        }
    }
}
