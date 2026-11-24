//! Cyclomatic complexity counted on the syntax tree, so keywords and operators
//! in strings, comments and macros never count.
//!
//! Rust follows cargo-crap's decision points, so both tools agree on the same
//! source: `if` (with `else if` and `if let`), `for`, `while`, `loop`, every
//! match arm, `&&`, `||` and each `?`. Python follows radon and TypeScript
//! follows ESLint's `complexity` rule. Closures, lambdas and nested items are
//! their own scope.

use super::model::{class_node, function_node};
use syn::visit::{self, Visit};
use tree_sitter::Node;

pub(super) fn rust(body: &syn::Block) -> u64 {
    let mut decisions = RustDecisions::default();
    decisions.visit_block(body);
    1 + decisions.0
}

#[derive(Default)]
struct RustDecisions(u64);

impl<'ast> Visit<'ast> for RustDecisions {
    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        self.0 += 1;
        visit::visit_expr_if(self, node);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.0 += 1;
        visit::visit_expr_for_loop(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.0 += 1;
        visit::visit_expr_while(self, node);
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.0 += 1;
        visit::visit_expr_loop(self, node);
    }

    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        self.0 += 1;
        visit::visit_arm(self, node);
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        if matches!(node.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) {
            self.0 += 1;
        }
        visit::visit_expr_binary(self, node);
    }

    fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
        self.0 += 1;
        visit::visit_expr_try(self, node);
    }

    fn visit_expr_closure(&mut self, _closure: &'ast syn::ExprClosure) {}

    fn visit_item(&mut self, _item: &'ast syn::Item) {}
}

pub(super) fn portable(function: Node<'_>, body: Node<'_>, language: &str) -> u64 {
    // An expression body, as in `(x) => a && b` or `lambda x: a if b else c`, is
    // itself a decision. TypeScript default parameters branch like ESLint counts them.
    let parameters = function
        .child_by_field_name("parameters")
        .filter(|_| language == "typescript")
        .map_or(0, |parameters| portable_decisions(parameters, language));
    1 + u64::from(decision(body, language)) + portable_decisions(body, language) + parameters
}

fn portable_decisions(node: Node<'_>, language: &str) -> u64 {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| {
            !function_node(child.kind(), language) && !class_node(child.kind(), language)
        })
        .map(|child| u64::from(decision(child, language)) + portable_decisions(child, language))
        .sum()
}

fn decision(node: Node<'_>, language: &str) -> bool {
    match (language, node.kind()) {
        (
            "python",
            "if_statement"
            | "elif_clause"
            | "conditional_expression"
            | "for_statement"
            | "for_in_clause"
            | "while_statement"
            | "except_clause"
            | "assert_statement"
            | "boolean_operator",
        ) => true,
        // A comprehension `if` branches; a `case` guard does not, as in radon.
        ("python", "if_clause") => node
            .parent()
            .is_none_or(|parent| parent.kind() != "case_clause"),
        // `for`, `while` and `try` may take an `else`; `if` already counted its own.
        ("python", "else_clause") => node.parent().is_some_and(|parent| {
            matches!(
                parent.kind(),
                "for_statement" | "while_statement" | "try_statement"
            )
        }),
        ("python", "case_clause") => !irrefutable_case(node),
        (
            "typescript",
            "if_statement"
            | "ternary_expression"
            | "for_statement"
            | "for_in_statement"
            | "while_statement"
            | "do_statement"
            | "switch_case"
            | "catch_clause"
            | "optional_chain"
            | "assignment_pattern"
            | "object_assignment_pattern",
        ) => true,
        // `a?.()` has a bare `?.` token where `a?.b` has an `optional_chain` node.
        ("typescript", "call_expression") => {
            let mut cursor = node.walk();
            node.children(&mut cursor).any(|child| child.kind() == "?.")
        }
        ("typescript", "required_parameter" | "optional_parameter") => {
            node.child_by_field_name("value").is_some()
        }
        ("typescript", "binary_expression" | "augmented_assignment_expression") => node
            .child_by_field_name("operator")
            .is_some_and(|operator| {
                matches!(operator.kind(), "&&" | "||" | "??" | "&&=" | "||=" | "??=")
            }),
        _ => false,
    }
}

/// `case _:` and a bare capture such as `case other:` match anything, so like
/// `else` they add no path.
fn irrefutable_case(case: Node<'_>) -> bool {
    let mut cursor = case.walk();
    let patterns: Vec<_> = case
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "case_pattern")
        .collect();
    let [pattern] = patterns.as_slice() else {
        return false;
    };
    let mut cursor = pattern.walk();
    let parts: Vec<_> = pattern.named_children(&mut cursor).collect();
    match parts.as_slice() {
        [] => true,
        [name] => name.kind() == "dotted_name" && name.named_child_count() == 1,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_cc(body: &str) -> u64 {
        rust(&syn::parse_str(body).unwrap())
    }

    fn portable_cc(language: &str, path: &str, source: &str) -> u64 {
        portable_function_cc(language, path, source, |node| node.start_byte())
    }

    fn portable_function_cc(
        language: &str,
        path: &str,
        source: &str,
        order: impl Fn(&Node<'_>) -> usize,
    ) -> u64 {
        let tree = super::super::model::parse_portable_tree(language, path, source).unwrap();
        let mut functions = Vec::new();
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if function_node(node.kind(), language) {
                functions.push(node);
            }
            let mut cursor = node.walk();
            stack.extend(node.named_children(&mut cursor));
        }
        let outer = functions.into_iter().min_by_key(order).unwrap();
        portable(outer, outer.child_by_field_name("body").unwrap(), language)
    }

    #[test]
    fn rust_counts_each_cargo_crap_decision_point_once() {
        assert_eq!(rust_cc("{}"), 1);
        assert_eq!(rust_cc("{ if a {} }"), 2);
        assert_eq!(rust_cc("{ if a {} else if b {} else {} }"), 3);
        assert_eq!(rust_cc("{ if let Some(x) = y {} }"), 2);
        assert_eq!(rust_cc("{ for x in y {} }"), 2);
        assert_eq!(rust_cc("{ while a {} }"), 2);
        assert_eq!(rust_cc("{ loop { break; } }"), 2);
        assert_eq!(rust_cc("{ match v { A => 1, B => 2, _ => 3 } }"), 4);
        assert_eq!(rust_cc("{ a && b || c }"), 3);
        assert_eq!(rust_cc("{ a & b | c }"), 1);
        assert_eq!(rust_cc("{ let x = a()?; b(x)?.c()? }"), 4);
    }

    #[test]
    fn rust_ignores_text_closures_nested_items_and_macros() {
        assert_eq!(rust_cc(r#"{ let s = "if a && b || c"; /* while */ s }"#), 1);
        assert_eq!(rust_cc("{ v.iter().all(|x| x.a() && x.b()) }"), 1);
        assert_eq!(rust_cc("{ v.ok_or_else(|| e())? }"), 2);
        assert_eq!(
            rust_cc("{ fn nested(a: bool) { if a {} } nested(true) }"),
            1
        );
        assert_eq!(rust_cc("{ assert!(a && b); }"), 1);
    }

    #[test]
    fn python_counts_radon_decisions_on_the_tree() {
        let cc = |body: &str| portable_cc("python", "m.py", &format!("def f(a, b, c):\n{body}"));
        assert_eq!(cc("    return a\n"), 1);
        assert_eq!(
            cc("    if a:\n        pass\n    elif b:\n        pass\n    else:\n        pass\n"),
            3
        );
        assert_eq!(cc("    return a if b else c\n"), 2);
        assert_eq!(cc("    return [x for x in a if x]\n"), 3);
        assert_eq!(
            cc("    for x in a:\n        pass\n    while b:\n        pass\n"),
            3
        );
        assert_eq!(
            cc("    try:\n        pass\n    except E:\n        pass\n    finally:\n        pass\n"),
            2
        );
        assert_eq!(cc("    return a and b or c\n"), 3);
        assert_eq!(
            cc(
                "    match a:\n        case 1:\n            pass\n        case _:\n            pass\n"
            ),
            2
        );
        assert_eq!(
            cc(
                "    match a:\n        case 1 if b:\n            pass\n        case [x]:\n            pass\n        case other:\n            pass\n"
            ),
            3
        );
        assert_eq!(cc("    assert a\n    assert b, c\n"), 3);
        assert_eq!(
            cc("    for x in a:\n        pass\n    else:\n        pass\n"),
            3
        );
        assert_eq!(
            cc("    while a:\n        pass\n    else:\n        pass\n"),
            3
        );
        assert_eq!(
            cc("    try:\n        pass\n    except E:\n        pass\n    else:\n        pass\n"),
            3
        );
        assert_eq!(cc("    s = 'if a and b'  # while\n    return s\n"), 1);
        let lambda = |source: &str| {
            portable_function_cc("python", "m.py", source, |node| {
                usize::MAX - node.start_byte()
            })
        };
        assert_eq!(lambda("g = lambda x: x if x else 0\n"), 2);
        assert_eq!(lambda("g = lambda x: x and x.y\n"), 2);
        assert_eq!(
            cc(
                "    g = lambda x: x if x else a\n    def h():\n        if b:\n            pass\n    return g\n"
            ),
            1
        );
    }

    #[test]
    fn typescript_counts_eslint_decisions_on_the_tree() {
        let cc = |body: &str| {
            portable_cc(
                "typescript",
                "m.ts",
                &format!("function f(a: any, b: any, c: any) {{ {body} }}"),
            )
        };
        assert_eq!(cc("return a;"), 1);
        let arrow = |source: &str| {
            portable_function_cc("typescript", "m.ts", source, |node| {
                usize::MAX - node.start_byte()
            })
        };
        assert_eq!(arrow("const g = (x: any) => x.a && x.b;"), 2);
        assert_eq!(arrow("const g = (x: any) => (x ? 1 : 2);"), 2);
        assert_eq!(arrow("const g = (x: any) => x?.a;"), 2);
        assert_eq!(
            arrow("const g = (x: any = 1, { y = 2 }: any = {}) => x;"),
            4
        );
        assert_eq!(arrow("const g = ([x = 1]: any) => x?.();"), 3);
        assert_eq!(cc("if (a) {} else if (b) {} else {}"), 3);
        assert_eq!(cc("return a ? b : c;"), 2);
        assert_eq!(
            cc("for (;;) {} for (const x of a) {} for (const k in a) {}"),
            4
        );
        assert_eq!(cc("while (a) {} do {} while (b);"), 3);
        assert_eq!(
            cc("switch (a) { case 1: break; case 2: break; default: break; }"),
            3
        );
        assert_eq!(cc("try {} catch (e) {} finally {}"), 2);
        assert_eq!(cc("return a && b || c ?? 1;"), 4);
        assert_eq!(cc("return a & b | c + 1;"), 1);
        assert_eq!(cc("a ??= 1; b ||= 2; c &&= 3; a += 1;"), 4);
        assert_eq!(cc("const s = 'if (a && b)'; // while\n return a?.b?.c;"), 3);
        assert_eq!(
            cc("const g = (x: any) => (x ? a : b); class K { m() { if (a) {} } } return g;"),
            1
        );
    }
}
