use super::extract::{ClassSummary, record_class_metrics};
use super::*;

fn data_class_match(fields: usize, operations: usize, minimum: u64, maximum: u64) -> bool {
    fields as u64 >= minimum && operations as u64 <= maximum
}

fn lazy_class_match(
    fields: usize,
    methods: usize,
    lines: usize,
    maximum_fields: u64,
    maximum_methods: u64,
    maximum_lines: u64,
) -> bool {
    fields as u64 <= maximum_fields
        && methods as u64 <= maximum_methods
        && lines as u64 <= maximum_lines
}

fn body_line_total(lines: usize, comments: usize) -> usize {
    lines + comments
}

fn comment_share_match(lines: usize, comments: usize, minimum_lines: u64, share: u64) -> bool {
    let total = body_line_total(lines, comments);
    lines as u64 >= minimum_lines && 100_u128 * comments as u128 >= share as u128 * total as u128
}

fn enough_clump_slots(slots: usize, minimum: usize) -> bool {
    slots >= minimum
}

fn enough_clump_support(declarations: usize, minimum: u64) -> bool {
    declarations as u64 >= minimum
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
                data_class_match(class.fields, class.operations, minimum, maximum),
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
                lazy_class_match(
                    class.fields,
                    class.methods,
                    class.method_lines,
                    fields,
                    methods,
                    lines,
                ),
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
        let total = body_line_total(function.lines, function.comments);
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
            comment_share_match(function.lines, function.comments, code, share),
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
        if !enough_clump_slots(slots.len(), size) {
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
        if !enough_clump_support(support.len(), minimum) {
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

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        FunctionFact, Location, body_line_total, combinations, comment_share_match,
        data_class_match, enough_clump_slots, enough_clump_support, lazy_class_match,
        ordered_functions, overlapping_functions,
    };

    fn function(symbol: &str, path: &str, line: usize, body_range: (usize, usize)) -> FunctionFact {
        FunctionFact {
            symbol: symbol.into(),
            owner: None,
            location: Location {
                path: path.into(),
                line,
                column: 1,
            },
            parameters: Vec::new(),
            lines: 1,
            comments: 0,
            tokens: vec!["identifier".into()],
            body_range,
        }
    }

    #[test]
    fn data_class_requires_both_independent_boundaries() {
        assert!(data_class_match(5, 2, 5, 2));
        assert!(!data_class_match(4, 2, 5, 2));
        assert!(!data_class_match(5, 3, 5, 2));
        assert!(!data_class_match(4, 3, 5, 2));
    }

    #[test]
    fn lazy_class_requires_every_maximum_boundary() {
        assert!(lazy_class_match(3, 4, 20, 3, 4, 20));
        assert!(!lazy_class_match(4, 4, 20, 3, 4, 20));
        assert!(!lazy_class_match(3, 5, 20, 3, 4, 20));
        assert!(!lazy_class_match(3, 4, 21, 3, 4, 20));
        assert!(!lazy_class_match(4, 5, 21, 3, 4, 20));
    }

    #[test]
    fn comment_share_requires_the_code_minimum_and_exact_integer_ratio() {
        assert!(comment_share_match(20, 5, 20, 20));
        assert!(!comment_share_match(19, 5, 20, 20));
        assert!(!comment_share_match(20, 4, 20, 20));
        assert!(comment_share_match(80, 20, 1, 20));
        assert!(!comment_share_match(81, 19, 1, 20));
    }

    #[test]
    fn body_line_total_adds_code_and_comment_rows_independently() {
        assert_eq!(body_line_total(0, 0), 0);
        assert_eq!(body_line_total(5, 0), 5);
        assert_eq!(body_line_total(0, 3), 3);
        assert_eq!(body_line_total(5, 3), 8);
    }

    #[test]
    fn group_combinations_are_unique_and_charge_the_exact_budget() {
        let slots = [
            ("a".into(), "A".into()),
            ("b".into(), "B".into()),
            ("c".into(), "C".into()),
            ("d".into(), "D".into()),
        ];
        let mut groups = Vec::new();
        let mut remaining = 6;
        combinations(&slots, 2, 0, &mut Vec::new(), &mut groups, &mut remaining).unwrap();
        assert_eq!(groups.len(), 6);
        assert_eq!(remaining, 0);
        assert_eq!(groups[0], [slots[0].clone(), slots[1].clone()]);
        assert_eq!(groups[5], [slots[2].clone(), slots[3].clone()]);

        let mut groups = Vec::new();
        let mut remaining = 5;
        let error =
            combinations(&slots, 2, 0, &mut Vec::new(), &mut groups, &mut remaining).unwrap_err();
        assert_eq!(error, "maximum_group_combinations budget exceeded");
        assert_eq!(groups.len(), 5);
        assert_eq!(remaining, 0);

        let mut groups = Vec::new();
        let mut remaining = 1;
        combinations(&slots, 5, 0, &mut Vec::new(), &mut groups, &mut remaining).unwrap();
        assert!(groups.is_empty());
        assert_eq!(remaining, 1);
    }

    #[test]
    fn clump_candidate_and_support_thresholds_are_inclusive_and_independent() {
        assert!(enough_clump_slots(3, 3));
        assert!(enough_clump_slots(4, 3));
        assert!(!enough_clump_slots(2, 3));
        assert!(enough_clump_support(3, 3));
        assert!(enough_clump_support(4, 3));
        assert!(!enough_clump_support(2, 3));
    }

    #[test]
    fn duplicate_function_overlap_uses_strict_ranges_within_the_same_file() {
        let first = function("zeta", "same.py", 10, (10, 20));
        let overlapping = function("alpha", "same.py", 2, (19, 30));
        let touching = function("touching", "same.py", 20, (20, 30));
        let touching_before = function("touching_before", "same.py", 1, (0, 10));
        let containing = function("containing", "same.py", 1, (0, 40));
        let other_file = function("other", "other.py", 1, (10, 20));

        assert!(overlapping_functions(&first, &overlapping));
        assert!(overlapping_functions(&first, &containing));
        assert!(!overlapping_functions(&first, &touching));
        assert!(!overlapping_functions(&first, &touching_before));
        assert!(!overlapping_functions(&first, &other_file));

        let (ordered_first, ordered_second) = ordered_functions(&first, &overlapping);
        assert_eq!(ordered_first.symbol, "alpha");
        assert_eq!(ordered_second.symbol, "zeta");
        let (ordered_first, ordered_second) = ordered_functions(&overlapping, &first);
        assert_eq!(ordered_first.symbol, "alpha");
        assert_eq!(ordered_second.symbol, "zeta");
    }
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
        report.maximum_with_role(
            &input.policy,
            &format!("{}.function_lines", registry.language),
            &function.symbol,
            &function.location,
            crate::report::RoleAdjustedMaximum::new(
                function.lines,
                "authored body code lines",
                crate::migration::function_role(
                    input,
                    &function.location.path,
                    &function.symbol,
                    function.owner.as_deref(),
                    &function.location,
                ),
            ),
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
