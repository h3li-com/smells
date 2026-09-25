use crate::{
    policy::Registry,
    report::{Location, Suppression},
};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use syn::{spanned::Spanned, visit::Visit};
use tree_sitter::{Node, Parser};

pub struct ParsedSuppressions {
    pub directives: Vec<ParsedSuppression>,
    pub errors: Vec<ParsedSuppressionError>,
}

pub struct ParsedSuppressionError {
    pub message: String,
    pub location: Location,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SuppressionLanguage {
    Rust,
    Python,
    TypeScript,
}

impl SuppressionLanguage {
    fn parse(language: &str) -> Self {
        match language {
            "rust" => Self::Rust,
            "python" => Self::Python,
            "typescript" => Self::TypeScript,
            _ => unreachable!("validated language"),
        }
    }

    fn declaration_pattern(self) -> Regex {
        let pattern = match self {
            Self::Rust => {
                r#"^(?:(?:pub(?:\([^)]*\))?|async|unsafe|const|extern(?:\s+"[^"]+")?)\s+)*(?:fn|struct|enum|trait|type)\b"#
            }
            Self::Python => r"^(?:async\s+)?(?:def|class)\s+[A-Za-z_]",
            Self::TypeScript => {
                r"^(?:(?:export|default|declare|abstract|public|private|protected|static|readonly|async|get|set|override)\s+)*(?:(?:class|function|interface|type|enum|namespace)\s+[A-Za-z_$]|(?:constructor|[A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>]+>)?\s*\()"
            }
        };
        Regex::new(pattern).expect("declaration regex")
    }

    fn directive_pattern(self) -> Regex {
        let marker = if self == Self::Python { r"\#" } else { r"//" };
        Regex::new(&format!(
            r"^\s*{marker}\s*smells:\s*ignore\[([a-z][a-z0-9_-]*\.[a-z][a-z0-9_-]*)\]\s+--\s+(\S(?:.*\S)?)\s*$"
        ))
        .expect("directive regex")
    }

    fn is_comment_with_smells(self, line: &str) -> bool {
        let trimmed = line.trim_start();
        let comment = if self == Self::Python {
            trimmed.starts_with('#')
        } else {
            trimmed.starts_with("//")
        };
        comment && trimmed.contains("smells:")
    }

    fn permitted_gap_line(
        self,
        line: &str,
        line_number: usize,
        annotation_lines: &BTreeSet<usize>,
    ) -> bool {
        let trimmed = line.trim();
        trimmed.is_empty()
            || self.is_comment_with_smells(line)
            || annotation_lines.contains(&line_number)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DeclarationSpan {
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
}

impl DeclarationSpan {
    fn contains_position(self, line: usize, column: usize) -> bool {
        let position = (line, column);
        position >= (self.start_line, self.start_column)
            && position < (self.end_line, self.end_column)
    }

    fn contains(self, location: &Location) -> bool {
        self.contains_position(location.line, location.column)
    }
}

pub struct ParsedSuppression {
    pub suppression: Suppression,
    target_span: DeclarationSpan,
    nested_spans: Vec<DeclarationSpan>,
}

impl ParsedSuppression {
    pub fn owns(&self, rule_id: &str, location: &Location) -> bool {
        self.suppression.rule_id == rule_id
            && self.suppression.target_location.path == location.path
            && self.target_span.contains(location)
            && !self.nested_spans.iter().any(|span| span.contains(location))
    }
}

fn comment_nodes(node: Node<'_>, lines: &mut BTreeSet<usize>) {
    if node.kind() == "comment" {
        lines.insert(node.start_position().row + 1);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        comment_nodes(child, lines);
    }
}

fn actual_comment_lines(
    path: &str,
    source: &str,
    language: SuppressionLanguage,
) -> BTreeSet<usize> {
    if !source.contains("smells:") {
        return BTreeSet::new();
    }
    if language == SuppressionLanguage::Rust {
        let mut lines = BTreeSet::new();
        let mut offset = 0;
        let mut line = 1;
        for token in rustc_lexer::tokenize(source, rustc_lexer::FrontmatterAllowed::No) {
            let length = token.len as usize;
            let text = &source[offset..offset + length];
            if matches!(
                token.kind,
                rustc_lexer::TokenKind::LineComment { doc_style: None }
            ) {
                lines.insert(line);
            }
            line += text.bytes().filter(|byte| *byte == b'\n').count();
            offset += length;
        }
        return lines;
    }

    let grammar = if language == SuppressionLanguage::Python {
        tree_sitter_python::LANGUAGE.into()
    } else if path.ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .expect("embedded suppression grammar must load");
    let mut lines = BTreeSet::new();
    if let Some(tree) = parser.parse(source, None) {
        comment_nodes(tree.root_node(), &mut lines);
    }
    lines
}

fn next_declaration(
    lines: &[&str],
    directive_line: usize,
    language: SuppressionLanguage,
    annotation_lines: &BTreeSet<usize>,
) -> Option<(usize, usize)> {
    let declaration = language.declaration_pattern();
    for (offset, line) in lines.iter().enumerate().skip(directive_line) {
        let trimmed = line.trim_start();
        if declaration.is_match(trimmed) {
            return Some((offset + 1, line.len() - trimmed.len() + 1));
        }
        if !language.permitted_gap_line(line, offset + 1, annotation_lines) {
            return None;
        }
    }
    None
}

fn tree_sitter_declaration(kind: &str, language: SuppressionLanguage) -> bool {
    match language {
        SuppressionLanguage::Python => {
            matches!(kind, "function_definition" | "class_definition" | "lambda")
        }
        SuppressionLanguage::TypeScript => matches!(
            kind,
            "function_declaration"
                | "generator_function_declaration"
                | "class_declaration"
                | "abstract_class_declaration"
                | "interface_declaration"
                | "type_alias_declaration"
                | "enum_declaration"
                | "internal_module"
                | "method_definition"
                | "method_signature"
                | "abstract_method_signature"
                | "function_signature"
                | "arrow_function"
                | "function_expression"
                | "generator_function"
        ),
        SuppressionLanguage::Rust => false,
    }
}

#[derive(Default)]
struct SourceSyntax {
    spans: Vec<DeclarationSpan>,
    annotation_lines: BTreeSet<usize>,
}

fn collect_tree_sitter_syntax(
    node: Node<'_>,
    language: SuppressionLanguage,
    syntax: &mut SourceSyntax,
) {
    if tree_sitter_declaration(node.kind(), language) {
        syntax.spans.push(DeclarationSpan {
            start_line: node.start_position().row + 1,
            start_column: node.start_position().column + 1,
            end_line: node.end_position().row + 1,
            end_column: node.end_position().column + 1,
        });
    }
    if node.kind() == "decorator" {
        syntax
            .annotation_lines
            .extend((node.start_position().row + 1)..=(node.end_position().row + 1));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_tree_sitter_syntax(child, language, syntax);
    }
}

#[derive(Default)]
struct RustDeclarations {
    syntax: SourceSyntax,
}

impl RustDeclarations {
    fn push(
        &mut self,
        declaration: proc_macro2::Span,
        whole: proc_macro2::Span,
        attributes: &[syn::Attribute],
    ) {
        let start = declaration.start();
        let end = whole.end();
        self.syntax.spans.push(DeclarationSpan {
            start_line: start.line,
            start_column: start.column + 1,
            end_line: end.line,
            end_column: end.column + 1,
        });
        for attribute in attributes {
            let span = attribute.span();
            self.syntax
                .annotation_lines
                .extend(span.start().line..=span.end().line);
        }
    }
}

impl<'ast> Visit<'ast> for RustDeclarations {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.push(node.sig.span(), node.span(), &node.attrs);
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.push(node.sig.span(), node.span(), &node.attrs);
        syn::visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.push(node.sig.span(), node.span(), &node.attrs);
        syn::visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.push(node.struct_token.span(), node.span(), &node.attrs);
        syn::visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        self.push(node.enum_token.span(), node.span(), &node.attrs);
        syn::visit::visit_item_enum(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.push(node.trait_token.span(), node.span(), &node.attrs);
        syn::visit::visit_item_trait(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        self.push(node.type_token.span(), node.span(), &node.attrs);
        syn::visit::visit_item_type(self, node);
    }
}

fn source_syntax(path: &str, source: &str, language: SuppressionLanguage) -> SourceSyntax {
    if language == SuppressionLanguage::Rust {
        let Ok(file) = syn::parse_file(source) else {
            return SourceSyntax::default();
        };
        let mut declarations = RustDeclarations::default();
        declarations.visit_file(&file);
        return declarations.syntax;
    }

    let grammar = if language == SuppressionLanguage::Python {
        tree_sitter_python::LANGUAGE.into()
    } else if path.ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .expect("embedded suppression grammar must load");
    let mut syntax = SourceSyntax::default();
    if let Some(tree) = parser.parse(source, None) {
        collect_tree_sitter_syntax(tree.root_node(), language, &mut syntax);
    }
    syntax
}

fn target_span(spans: &[DeclarationSpan], line: usize, column: usize) -> Option<DeclarationSpan> {
    spans
        .iter()
        .copied()
        .filter(|span| span.start_line == line && span.start_column >= column)
        .min_by_key(|span| span.start_column)
}

fn suppression_error(
    path: &str,
    line: usize,
    column: usize,
    message: String,
) -> ParsedSuppressionError {
    ParsedSuppressionError {
        message,
        location: Location {
            path: path.into(),
            line,
            column,
        },
    }
}

pub fn parse(files: &BTreeMap<String, String>, registry: &Registry) -> ParsedSuppressions {
    let language = SuppressionLanguage::parse(&registry.language);
    let known_rules: BTreeSet<_> = registry.rules.iter().map(|rule| rule.id.as_str()).collect();
    let directive = language.directive_pattern();
    let mut directives = Vec::new();
    let mut errors = Vec::new();
    let mut targets = BTreeSet::new();

    for (path, source) in files {
        let lines: Vec<_> = source.lines().collect();
        let comment_lines = actual_comment_lines(path, source, language);
        if comment_lines.is_empty() {
            continue;
        }
        let syntax = source_syntax(path, source, language);
        for (offset, line) in lines.iter().enumerate() {
            if !comment_lines.contains(&(offset + 1)) || !language.is_comment_with_smells(line) {
                continue;
            }
            let line_number = offset + 1;
            let directive_column = line.find("smells:").map_or(1, |column| column + 1);
            let Some(captures) = directive.captures(line) else {
                errors.push(suppression_error(
                    path,
                    line_number,
                    directive_column,
                    format!(
                        "invalid suppression at {path}:{line_number}: expected `smells: ignore[exact-rule-id] -- non-empty reason`"
                    ),
                ));
                continue;
            };
            let rule_id = captures[1].to_string();
            let reason = captures[2].to_string();
            if !known_rules.contains(rule_id.as_str()) {
                errors.push(suppression_error(
                    path,
                    line_number,
                    directive_column,
                    format!("unknown suppression rule at {path}:{line_number}: {rule_id}"),
                ));
                continue;
            }
            let Some((target_line, target_column)) =
                next_declaration(&lines, line_number, language, &syntax.annotation_lines)
            else {
                errors.push(suppression_error(
                    path,
                    line_number,
                    directive_column,
                    format!("misplaced suppression at {path}:{line_number}: no next declaration"),
                ));
                continue;
            };
            let Some(target_span) = target_span(&syntax.spans, target_line, target_column) else {
                errors.push(suppression_error(
                    path,
                    line_number,
                    directive_column,
                    format!(
                        "misplaced suppression at {path}:{line_number}: next declaration could not be resolved"
                    ),
                ));
                continue;
            };
            if !targets.insert((path.clone(), target_line, rule_id.clone())) {
                errors.push(suppression_error(
                    path,
                    line_number,
                    directive_column,
                    format!(
                        "duplicate suppression at {path}:{line_number}: {rule_id} already targets the next declaration"
                    ),
                ));
                continue;
            }
            let nested_spans = syntax
                .spans
                .iter()
                .copied()
                .filter(|span| {
                    *span != target_span
                        && target_span.contains_position(span.start_line, span.start_column)
                })
                .collect();
            directives.push(ParsedSuppression {
                suppression: Suppression {
                    suppression_id: format!("S{:06}", directives.len() + 1),
                    rule_id,
                    reason,
                    directive_location: Location {
                        path: path.clone(),
                        line: line_number,
                        column: directive_column,
                    },
                    target_location: Location {
                        path: path.clone(),
                        line: target_line,
                        column: target_column,
                    },
                    target_end_line: target_span.end_line,
                    state: "unused".into(),
                },
                target_span,
                nested_spans,
            });
        }
    }

    ParsedSuppressions { directives, errors }
}
