use std::path::Path;
use tree_sitter::{Language, Node, Parser};

fn mask(bytes: &mut [u8], range: std::ops::Range<usize>) {
    for byte in &mut bytes[range] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

fn rust_code_only(source: &str) -> String {
    use rustc_lexer::{LiteralKind, TokenKind};

    let mut bytes = source.as_bytes().to_vec();
    let mut offset = 0;
    for token in rustc_lexer::tokenize(source, rustc_lexer::FrontmatterAllowed::No) {
        let end = offset + token.len as usize;
        let trivia_or_text = matches!(
            token.kind,
            TokenKind::LineComment { .. }
                | TokenKind::BlockComment { .. }
                | TokenKind::Literal {
                    kind: LiteralKind::Char { .. }
                        | LiteralKind::Byte { .. }
                        | LiteralKind::Str { .. }
                        | LiteralKind::ByteStr { .. }
                        | LiteralKind::CStr { .. }
                        | LiteralKind::RawStr { .. }
                        | LiteralKind::RawByteStr { .. }
                        | LiteralKind::RawCStr { .. },
                    ..
                }
        );
        if trivia_or_text {
            mask(&mut bytes, offset..end);
        }
        offset = end;
    }
    String::from_utf8(bytes).expect("masking preserves UTF-8 validity")
}

fn grammar(path: &str) -> Result<Language, String> {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("py" | "pyi") => Ok(tree_sitter_python::LANGUAGE.into()),
        Some("tsx") => Ok(tree_sitter_typescript::LANGUAGE_TSX.into()),
        Some("ts" | "mts" | "cts") => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        _ => Err(format!("no collector grammar for {path}")),
    }
}

fn entirely_non_code_node(kind: &str) -> bool {
    kind == "comment"
        || kind == "regex"
        || matches!(kind, "import_statement" | "import_from_statement")
}

fn string_node(kind: &str) -> bool {
    kind.contains("string")
}

fn interpolation_node(kind: &str) -> bool {
    matches!(kind, "interpolation" | "template_substitution")
}

fn restore_interpolations(node: Node<'_>, source: &[u8], bytes: &mut [u8]) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if interpolation_node(child.kind()) {
            bytes[child.byte_range()].copy_from_slice(&source[child.byte_range()]);
            let mut child_cursor = child.walk();
            for expression_child in child.children(&mut child_cursor) {
                mask_non_code_nodes(expression_child, source, bytes);
            }
        } else {
            restore_interpolations(child, source, bytes);
        }
    }
}

fn mask_non_code_nodes(node: Node<'_>, source: &[u8], bytes: &mut [u8]) {
    if entirely_non_code_node(node.kind()) {
        mask(bytes, node.byte_range());
        return;
    }
    if string_node(node.kind()) {
        mask(bytes, node.byte_range());
        restore_interpolations(node, source, bytes);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        mask_non_code_nodes(child, source, bytes);
    }
}

pub(super) fn code_only_from_tree(path: &str, source: &str, root: Node<'_>) -> String {
    let source_bytes = source.as_bytes();
    let mut bytes = source_bytes.to_vec();
    mask_non_code_nodes(root, source_bytes, &mut bytes);
    if Path::new(path)
        .extension()
        .is_some_and(|extension| matches!(extension.to_str(), Some("ts" | "tsx" | "mts" | "cts")))
    {
        for range in crate::typescript_compat::import_type_argument_ranges(source) {
            mask(&mut bytes, range);
        }
    }
    String::from_utf8(bytes).expect("masking preserves UTF-8 validity")
}

pub(super) fn code_only(path: &str, source: &str) -> Result<String, String> {
    if Path::new(path)
        .extension()
        .is_some_and(|extension| extension == "rs")
    {
        return Ok(rust_code_only(source));
    }
    let language = grammar(path)?;
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|error| format!("cannot load collector grammar for {path}: {error}"))?;
    let tree = if Path::new(path)
        .extension()
        .is_some_and(|extension| matches!(extension.to_str(), Some("ts" | "tsx" | "mts" | "cts")))
    {
        crate::typescript_compat::parse(&mut parser, source, path)?
    } else {
        parser
            .parse(source, None)
            .ok_or_else(|| format!("collector parser cancelled for {path}"))?
    };
    if tree.root_node().has_error() {
        return Err(format!("collector parse error in {path}"));
    }
    Ok(code_only_from_tree(path, source, tree.root_node()))
}

#[cfg(test)]
mod tests {
    use super::code_only;

    #[test]
    fn masks_comments_strings_and_imports_without_changing_offsets() {
        for (path, source) in [
            (
                "sample.py",
                "import a.b.c\n# x.y.z\nvalue = 'p.q.r'\nreal.x.y\n",
            ),
            (
                "sample.ts",
                "import { x } from 'a.b';\n// x.y.z\nconst s = `p.q.r`;\nreal.x.y;\n",
            ),
            ("sample.rs", "// x.y.z\nlet s = \"p.q.r\";\nreal.x.y;\n"),
        ] {
            let masked = code_only(path, source).unwrap();
            assert_eq!(masked.len(), source.len());
            assert!(!masked.contains("x.y.z"));
            assert!(!masked.contains("p.q.r"));
            assert!(masked.contains("real.x.y"));
        }
    }

    #[test]
    fn preserves_exported_code_and_executable_string_interpolations() {
        for (path, source, expected) in [
            (
                "sample.py",
                "value = f'literal.a.b {real.x.y}'\n",
                "real.x.y",
            ),
            (
                "sample.ts",
                "export function read() { return `literal.a.b ${real.x.y}`; }\n",
                "real.x.y",
            ),
        ] {
            let masked = code_only(path, source).unwrap();
            assert_eq!(masked.len(), source.len());
            assert!(masked.contains(expected), "{path}: {masked:?}");
            assert!(!masked.contains("literal.a.b"), "{path}: {masked:?}");
        }
    }

    #[test]
    fn masks_import_type_literals_after_the_typescript_compatibility_reparse() {
        let source = concat!(
            "function load(importOriginal: any) {\n",
            "  return importOriginal<typeof import('fake.deep.navigation')>()\n",
            "}\n",
        );
        let masked = code_only("sample.ts", source).unwrap();
        assert_eq!(masked.len(), source.len());
        assert!(!masked.contains("fake.deep.navigation"));
    }
}
