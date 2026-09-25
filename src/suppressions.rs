use crate::{
    policy::Registry,
    report::{Location, Suppression},
};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::{Node, Parser};

pub struct ParsedSuppressions {
    pub directives: Vec<Suppression>,
    pub errors: Vec<String>,
}

fn declaration_pattern(language: &str) -> Regex {
    let pattern = match language {
        "rust" => {
            r#"^(?:(?:pub(?:\([^)]*\))?|async|unsafe|const|extern(?:\s+"[^"]+")?)\s+)*(?:fn|struct|enum|trait|impl|type|mod|const|static)\b"#
        }
        "python" => r"^(?:async\s+)?(?:def|class)\s+[A-Za-z_]",
        "typescript" => {
            r"^(?:(?:export|default|declare|abstract|public|private|protected|static|readonly|async|get|set|override)\s+)*(?:(?:class|function|interface|type|enum|namespace)\s+[A-Za-z_$]|(?:constructor|[A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>]+>)?\s*\()"
        }
        _ => unreachable!("validated language"),
    };
    Regex::new(pattern).expect("declaration regex")
}

fn directive_pattern(language: &str) -> Regex {
    let marker = if language == "python" { r"\#" } else { r"//" };
    Regex::new(&format!(
        r"^\s*{marker}\s*smells:\s*ignore\[([a-z][a-z0-9_-]*\.[a-z][a-z0-9_-]*)\]\s+--\s+(\S(?:.*\S)?)\s*$"
    ))
    .expect("directive regex")
}

fn is_comment_with_smells(line: &str, language: &str) -> bool {
    let trimmed = line.trim_start();
    let comment = if language == "python" {
        trimmed.starts_with('#')
    } else {
        trimmed.starts_with("//")
    };
    comment && trimmed.contains("smells:")
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

fn actual_comment_lines(path: &str, source: &str, language: &str) -> BTreeSet<usize> {
    if !source.contains("smells:") {
        return BTreeSet::new();
    }
    if language == "rust" {
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

    let grammar = if language == "python" {
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

fn permitted_gap_line(line: &str, language: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty()
        || is_comment_with_smells(line, language)
        || (language == "rust" && trimmed.starts_with("#["))
        || (language != "rust" && trimmed.starts_with('@'))
}

fn next_declaration(
    lines: &[&str],
    directive_line: usize,
    language: &str,
) -> Option<(usize, usize)> {
    let declaration = declaration_pattern(language);
    for (offset, line) in lines.iter().enumerate().skip(directive_line) {
        let trimmed = line.trim_start();
        if declaration.is_match(trimmed) {
            return Some((offset + 1, line.len() - trimmed.len() + 1));
        }
        if !permitted_gap_line(line, language) {
            return None;
        }
    }
    None
}

fn python_declaration_end_line(lines: &[&str], start_line: usize) -> usize {
    let indentation = lines[start_line - 1].len() - lines[start_line - 1].trim_start().len();
    let mut end = start_line;
    for (offset, line) in lines.iter().enumerate().skip(start_line) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let current = line.len() - line.trim_start().len();
        if current <= indentation {
            break;
        }
        end = offset + 1;
    }
    end
}

fn update_brace_depth(line: &str, opened: &mut bool, depth: &mut isize) {
    for character in line.chars() {
        match character {
            '{' => {
                *opened = true;
                *depth += 1;
            }
            '}' if *opened => *depth -= 1,
            _ => {}
        }
    }
}

fn braced_declaration_end_line(lines: &[&str], start_line: usize) -> usize {
    let mut depth = 0isize;
    let mut opened = false;
    for (offset, line) in lines.iter().enumerate().skip(start_line - 1) {
        update_brace_depth(line, &mut opened, &mut depth);
        if opened && depth <= 0 {
            return offset + 1;
        }
        if !opened && line.contains(';') {
            return offset + 1;
        }
    }
    start_line
}

fn declaration_end_line(lines: &[&str], start_line: usize, language: &str) -> usize {
    if language == "python" {
        python_declaration_end_line(lines, start_line)
    } else {
        braced_declaration_end_line(lines, start_line)
    }
}

pub fn parse(files: &BTreeMap<String, String>, registry: &Registry) -> ParsedSuppressions {
    let known_rules: BTreeSet<_> = registry.rules.iter().map(|rule| rule.id.as_str()).collect();
    let directive = directive_pattern(&registry.language);
    let mut directives = Vec::new();
    let mut errors = Vec::new();
    let mut targets = BTreeSet::new();

    for (path, source) in files {
        let lines: Vec<_> = source.lines().collect();
        let comment_lines = actual_comment_lines(path, source, &registry.language);
        for (offset, line) in lines.iter().enumerate() {
            if !comment_lines.contains(&(offset + 1))
                || !is_comment_with_smells(line, &registry.language)
            {
                continue;
            }
            let line_number = offset + 1;
            let Some(captures) = directive.captures(line) else {
                errors.push(format!(
                    "invalid suppression at {path}:{line_number}: expected `smells: ignore[exact-rule-id] -- non-empty reason`"
                ));
                continue;
            };
            let rule_id = captures[1].to_string();
            let reason = captures[2].to_string();
            if !known_rules.contains(rule_id.as_str()) {
                errors.push(format!(
                    "unknown suppression rule at {path}:{line_number}: {rule_id}"
                ));
                continue;
            }
            let Some((target_line, target_column)) =
                next_declaration(&lines, line_number, &registry.language)
            else {
                errors.push(format!(
                    "misplaced suppression at {path}:{line_number}: no next declaration"
                ));
                continue;
            };
            if !targets.insert((path.clone(), target_line, rule_id.clone())) {
                errors.push(format!(
                    "duplicate suppression at {path}:{line_number}: {rule_id} already targets the next declaration"
                ));
                continue;
            }
            directives.push(Suppression {
                suppression_id: format!("S{:06}", directives.len() + 1),
                rule_id,
                reason,
                directive_location: Location {
                    path: path.clone(),
                    line: line_number,
                    column: line.find("smells:").map_or(1, |column| column + 1),
                },
                target_location: Location {
                    path: path.clone(),
                    line: target_line,
                    column: target_column,
                },
                target_end_line: declaration_end_line(&lines, target_line, &registry.language),
                state: "unused".into(),
            });
        }
    }

    ParsedSuppressions { directives, errors }
}
