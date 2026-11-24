use regex::Regex;
use std::{ops::Range, sync::OnceLock};
use tree_sitter::{Node, Parser, Tree};

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

fn import_type() -> &'static Regex {
    static IMPORT_TYPE: OnceLock<Regex> = OnceLock::new();
    IMPORT_TYPE.get_or_init(|| {
        Regex::new(
            r#"(?x)
            (?P<typeof>\btypeof\s+)?
            (?P<import>\bimport\s*\(\s*
                (?:"(?:\\.|[^"\\\r\n])*"|'(?:\\.|[^'\\\r\n])*')
            \s*\))
            (?P<chain>(?:\s*\.\s*[A-Za-z_$][\w$]*)*)
            (?P<next>\s*\(?)
            "#,
        )
        .expect("fixed TypeScript import-type compatibility regex")
    })
}

/// `import("m")` used as a type, as in `typeof import("m").f` or
/// `import("m").T<U>`. Tree-sitter rejects these in type arguments and with
/// type arguments or an index. A dynamic `import("m").then(...)` is a call and
/// is left alone.
fn import_type_ranges(source: &str) -> Vec<Range<usize>> {
    import_type()
        .captures_iter(source)
        .filter(|captures| {
            let start = captures.name("import").map_or(0, |matched| matched.start());
            let member = source[..start].trim_end().ends_with('.');
            let called = captures["next"].ends_with('(');
            !member
                && (captures.name("typeof").is_some() || (!captures["chain"].is_empty() && !called))
        })
        .filter_map(|captures| captures.name("import").map(|matched| matched.range()))
        .collect()
}

fn compatibility_source(source: &str) -> Option<String> {
    let mut bytes = source.as_bytes().to_vec();
    let mut changed = false;
    let argument_ranges = import_type_argument_ranges(source);
    for range in argument_ranges.iter().cloned() {
        for byte in &mut bytes[range.clone()] {
            if !matches!(*byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
        bytes[range.start] = b'<';
        bytes[range.end - 1] = b'>';
        if let Some(type_byte) = bytes[range.start + 1..range.end]
            .iter_mut()
            .find(|byte| !matches!(**byte, b'\n' | b'\r'))
        {
            *type_byte = b'T';
        }
        changed = true;
    }
    for range in import_type_ranges(source) {
        if argument_ranges
            .iter()
            .any(|argument| argument.contains(&range.start))
        {
            continue;
        }
        for byte in &mut bytes[range.start + "import".len()..range.end] {
            if !matches!(*byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
        bytes[range.start] = b'I';
        changed = true;
    }
    changed.then(|| String::from_utf8(bytes).expect("spaces preserve UTF-8"))
}

fn parsed_tree(parser: &mut Parser, source: &str, path: &str) -> Result<Tree, String> {
    parser
        .parse(source, None)
        .ok_or_else(|| format!("parser cancelled for {path}"))
}

/// The first error or missing node. Tree-sitter-typescript 0.23.2, the latest
/// release, cannot parse a tagged template with type arguments such as
/// ``const rows = sql<Row>`SELECT 1`;``. It recovers by inserting a zero-width
/// `!`, a non-null assertion that adds no path and that no rule reads, so a
/// missing `!` is not a syntax error.
pub(crate) fn first_syntax_error(node: Node<'_>) -> Option<Node<'_>> {
    if !node.has_error() {
        return None;
    }
    if node.is_error() || (node.is_missing() && node.kind() != "!") {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(first_syntax_error)
}

pub(crate) fn has_syntax_error(tree: &Tree) -> bool {
    first_syntax_error(tree.root_node()).is_some()
}

/// Parse TypeScript with the same byte-preserving compatibility treatment at
/// every scanner seam. Tree-sitter currently rejects `typeof import(...)` in a
/// generic call argument even though TypeScript accepts it.
pub(crate) fn parse(parser: &mut Parser, source: &str, path: &str) -> Result<Tree, String> {
    let tree = parsed_tree(parser, source, path)?;
    if !has_syntax_error(&tree) {
        return Ok(tree);
    }
    let Some(compatible) = compatibility_source(source) else {
        return Ok(tree);
    };
    parsed_tree(parser, &compatible, path)
}

#[cfg(test)]
mod tests {
    use super::{
        compatibility_source, has_syntax_error, import_type_argument_ranges, import_type_ranges,
        parse,
    };
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

    #[test]
    fn compatibility_replacement_preserves_range_delimiters_and_byte_length() {
        let source = "load< typeof import('module') >()";
        let ranges = import_type_argument_ranges(source);
        assert_eq!(ranges.len(), 1);
        let range = ranges[0].clone();
        let compatible = compatibility_source(source).expect("compatibility rewrite");
        assert_eq!(compatible.len(), source.len());
        assert_eq!(compatible.as_bytes()[range.start], b'<');
        assert_eq!(compatible.as_bytes()[range.end - 1], b'>');
        assert!(compatible[range.start + 1..range.end - 1].contains('T'));
        assert_eq!(&compatible[range.end..], "()");

        assert!(compatibility_source("load<string>()").is_none());
        assert!(import_type_argument_ranges("type X = typeof import('module');").is_empty());
    }

    #[test]
    fn typed_tagged_template_is_not_a_syntax_error() {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let source = "const rows = sql<{ readonly name: string }>`SELECT ${a ? 1 : 2}`;";
        let tree = parse(&mut parser, source, "query.ts").unwrap();
        assert!(tree.root_node().has_error());
        assert!(!has_syntax_error(&tree));
        assert!(tree.root_node().to_sexp().contains(
            "(instantiation_expression (identifier) type_arguments: (type_arguments (object_type"
        ));

        for broken in ["f(a;", "function broken( {", &format!("{source}\nf(a;")] {
            let tree = parse(&mut parser, broken, "broken.ts").unwrap();
            assert!(has_syntax_error(&tree), "{broken}");
        }
    }

    #[test]
    fn import_types_reparse_without_touching_dynamic_imports() {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        for source in [
            "type A = import('m').T<string>;",
            "type A = import(\"m\").U[string];",
            "const f = vi.fn<typeof import('m').g>();",
            "const f = vi.fn<import('m').T>();",
        ] {
            let tree = parse(&mut parser, source, "types.ts").unwrap();
            assert!(!has_syntax_error(&tree), "{source}");
        }

        let source = "f<import('m').T>(); g<typeof import('m')>(); await import('m'); import('m').then(h); x.import('m').a;";
        let compatible = compatibility_source(source).expect("compatibility rewrite");
        assert_eq!(compatible.len(), source.len());
        assert_eq!(
            compatible,
            "f<Import     .T>(); g<T                 >(); await import('m'); import('m').then(h); x.import('m').a;"
        );
        assert_eq!(import_type_ranges("typeof import('m')"), vec![7..18]);
        assert!(import_type_ranges("import('m')").is_empty());

        let tree = parse(
            &mut parser,
            "type A = import('m').T<string>;\nf(a;",
            "broken.ts",
        )
        .unwrap();
        assert!(has_syntax_error(&tree));
    }
}
