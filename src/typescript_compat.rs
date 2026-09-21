use regex::Regex;
use std::{ops::Range, sync::OnceLock};
use tree_sitter::{Parser, Tree};

fn import_type_argument() -> &'static Regex {
    static IMPORT_TYPE_ARGUMENT: OnceLock<Regex> = OnceLock::new();
    IMPORT_TYPE_ARGUMENT.get_or_init(|| {
        Regex::new(
            r#"(?x)
            <\s*typeof\s+import\s*\(\s*
            (?:"(?:\\.|[^"\\\r\n])*"|'(?:\\.|[^'\\\r\n])*')
            \s*\)\s*>
            "#,
        )
        .expect("fixed TypeScript import-type compatibility regex")
    })
}

pub(crate) fn import_type_argument_ranges(source: &str) -> Vec<Range<usize>> {
    import_type_argument()
        .find_iter(source)
        .filter(|matched| {
            source[matched.end()..]
                .chars()
                .find(|character| !character.is_whitespace())
                == Some('(')
        })
        .map(|matched| matched.range())
        .collect()
}

fn compatibility_source(source: &str) -> Option<String> {
    let mut bytes = source.as_bytes().to_vec();
    let mut changed = false;
    for range in import_type_argument_ranges(source) {
        for byte in &mut bytes[range.clone()] {
            if !matches!(*byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
        bytes[range.start] = b'<';
        bytes[range.end - 1] = b'>';
        if let Some(type_byte) = bytes[range.start + 1..range.end - 1]
            .iter_mut()
            .find(|byte| !matches!(**byte, b'\n' | b'\r'))
        {
            *type_byte = b'T';
        }
        changed = true;
    }
    changed.then(|| String::from_utf8(bytes).expect("spaces preserve UTF-8"))
}

fn parsed_tree(parser: &mut Parser, source: &str, path: &str) -> Result<Tree, String> {
    parser
        .parse(source, None)
        .ok_or_else(|| format!("parser cancelled for {path}"))
}

/// Parse TypeScript with the same byte-preserving compatibility treatment at
/// every scanner seam. Tree-sitter currently rejects `typeof import(...)` in a
/// generic call argument even though TypeScript accepts it.
pub(crate) fn parse(parser: &mut Parser, source: &str, path: &str) -> Result<Tree, String> {
    let tree = parsed_tree(parser, source, path)?;
    if !tree.root_node().has_error() {
        return Ok(tree);
    }
    let Some(compatible) = compatibility_source(source) else {
        return Ok(tree);
    };
    parsed_tree(parser, &compatible, path)
}

#[cfg(test)]
mod tests {
    use super::parse;
    use tree_sitter::Parser;

    #[test]
    fn compatibility_reparse_preserves_offsets_and_other_errors() {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let valid = "function load(importOriginal: any) { return importOriginal<typeof import('module')>() }";
        let tree = parse(&mut parser, valid, "valid.ts").unwrap();
        assert!(!tree.root_node().has_error());
        assert_eq!(tree.root_node().end_byte(), valid.len());

        let broken = format!("{valid}\nfunction broken( {{");
        let tree = parse(&mut parser, &broken, "broken.ts").unwrap();
        assert!(tree.root_node().has_error());
    }
}
