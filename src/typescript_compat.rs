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

fn template_escape() -> &'static Regex {
    static TEMPLATE_ESCAPE: OnceLock<Regex> = OnceLock::new();
    TEMPLATE_ESCAPE.get_or_init(|| {
        Regex::new(r"\A\\(?:u(?P<u>\{[0-9A-Fa-f]+\}|[0-9A-Fa-f]{4})?|x(?P<x>[0-9A-Fa-f]{2})?)")
            .expect("fixed TypeScript template-escape compatibility regex")
    })
}

fn variance_annotation() -> &'static Regex {
    static VARIANCE_ANNOTATION: OnceLock<Regex> = OnceLock::new();
    VARIANCE_ANNOTATION.get_or_init(|| {
        Regex::new(
            r"[<,](?:\s|/\*(?s:.)*?\*/|//[^\n]*)*(?:const\s+)?(?P<modifiers>(?:(?:in|out)\s+)+)[A-Za-z_$]",
        )
            .expect("fixed TypeScript variance-annotation compatibility regex")
    })
}

fn export_type_star() -> &'static Regex {
    static EXPORT_TYPE_STAR: OnceLock<Regex> = OnceLock::new();
    EXPORT_TYPE_STAR.get_or_init(|| {
        Regex::new(r"\bexport\s+(?P<type>type)\s*\*")
            .expect("fixed TypeScript export-type compatibility regex")
    })
}

/// A member named `abstract`, as in `interface Node { abstract: boolean }`.
/// Tree-sitter reads the name as the `abstract` modifier and fails on the `:`,
/// `?`, `!` or `(` after it.
fn abstract_member_name() -> &'static Regex {
    static ABSTRACT_MEMBER_NAME: OnceLock<Regex> = OnceLock::new();
    ABSTRACT_MEMBER_NAME.get_or_init(|| {
        Regex::new(r"[{;,\n](?:[ \t]|(?:readonly|public|private|protected|static|declare)\s)*(?P<name>abstract)\s*[?!]?\s*[:(]")
            .expect("fixed TypeScript abstract-member compatibility regex")
    })
}

/// Same-length repairs found on the first, failed parse tree.
#[derive(Default)]
struct TreeRepairs {
    /// Byte offsets of the `u` or `x` of invalid escapes in tagged templates.
    template_escapes: Vec<usize>,
    /// `in` and `out` variance annotations on type parameters.
    variance_annotations: Vec<Range<usize>>,
    /// Indentation bytes before a generic call signature that needs a `;`.
    call_signature_separators: Vec<usize>,
}

const TEMPLATE_TAGS: [&str; 6] = [
    "identifier",
    "member_expression",
    "call_expression",
    "subscript_expression",
    "parenthesized_expression",
    "non_null_expression",
];

/// A tagged template with a syntax error. Tree-sitter keeps the template node
/// when the error is inside it, but when the template opens on an invalid
/// escape, as in ``String.raw`\u-1` ``, it leaves an error that starts at the
/// backtick right after the tag.
fn is_tagged_template(node: Node<'_>, source: &str) -> bool {
    if node.kind() == "template_string" {
        return node
            .parent()
            .is_some_and(|parent| parent.kind() == "call_expression");
    }
    node.is_error()
        && source.as_bytes()[node.start_byte()] == b'`'
        && node.prev_sibling().is_some_and(ends_with_template_tag)
}

/// Whether `node` ends in an expression that can tag a template. The tag binds
/// tighter than any operator, so in `a + String.raw` it is `String.raw`.
fn ends_with_template_tag(mut node: Node<'_>) -> bool {
    while !TEMPLATE_TAGS.contains(&node.kind()) {
        let Some(last) = node
            .child_count()
            .checked_sub(1)
            .and_then(|last| node.child(last))
        else {
            return false;
        };
        node = last;
    }
    true
}

/// Invalid `\u` and `\x` escapes in the template literal opening at
/// `backtick`. ECMAScript allows them in a tagged template, as in
/// ``String.raw`C:\users\xyz` ``, because the tag receives the raw text, but
/// tree-sitter rejects them. None when the template is unterminated or a
/// substitution holds a string, comment, regex or nested template, which this
/// scan does not lex.
fn template_literal_escapes(source: &str, backtick: usize) -> Option<Vec<usize>> {
    let mut escapes = Vec::new();
    let mut bytes = source.bytes().enumerate().skip(backtick + 1);
    while let Some((index, byte)) = bytes.next() {
        match byte {
            b'`' => return Some(escapes),
            b'\\' => {
                if template_escape()
                    .captures(&source[index..])
                    .is_some_and(|captures| {
                        captures.name("u").is_none() && captures.name("x").is_none()
                    })
                {
                    escapes.push(index + 1);
                }
                bytes.next();
            }
            b'$' if source.as_bytes().get(index + 1) == Some(&b'{') => {
                bytes.next();
                skip_substitution(&mut bytes)?;
            }
            _ => {}
        }
    }
    None
}

fn skip_substitution(bytes: impl Iterator<Item = (usize, u8)>) -> Option<()> {
    let mut depth = 0_usize;
    for (_, byte) in bytes {
        match byte {
            b'{' => depth += 1,
            b'}' if depth == 0 => return Some(()),
            b'}' => depth -= 1,
            b'`' | b'"' | b'\'' | b'/' => return None,
            _ => {}
        }
    }
    None
}

/// The last character of the closest earlier line that holds code, skipping
/// blank and comment lines.
fn previous_code_end(source: &str, line_start: usize) -> Option<char> {
    source[..line_start]
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("//")
                && !line.starts_with("/*")
                && !line.starts_with('*')
        })
        .and_then(|line| line.chars().last())
}

/// Overloaded generic call signatures written one per line, as in
/// `interface Get { <K>(k: K): A\n  <K>(k: K): B }`. TypeScript separates type
/// members by newlines, but tree-sitter reads the second `<` as type arguments
/// of the first signature's return type. Each result is the indentation byte
/// right before such a `<` inside `range`, which can hold a `;`.
fn call_signature_separators(source: &str, range: Range<usize>) -> Vec<usize> {
    let line_starts = std::iter::once(0).chain(
        source
            .get(..range.end)
            .unwrap_or(source)
            .match_indices('\n')
            .map(|(index, _)| index + 1),
    );
    line_starts
        .filter_map(|start| {
            let indent = source[start..].find(|character| character != ' ' && character != '\t')?;
            let first = start + indent;
            (indent > 0 && range.contains(&first) && source[first..].starts_with('<'))
                .then_some(first)
        })
        .filter(|first| {
            !source[..*first].trim_end().ends_with("=>")
                && previous_code_end(source, *first).is_some_and(|end| {
                    matches!(end, ')' | ']' | '>' | '}') || end.is_alphanumeric() || end == '_'
                })
        })
        .map(|first| first - 1)
        .collect()
}

fn tree_repairs(node: Node<'_>, source: &str, repairs: &mut TreeRepairs) {
    if !node.has_error() {
        return;
    }
    if node.is_error() {
        repairs
            .call_signature_separators
            .extend(call_signature_separators(source, node.byte_range()));
    }
    if is_tagged_template(node, source) {
        repairs
            .template_escapes
            .extend(template_literal_escapes(source, node.start_byte()).unwrap_or_default());
        return;
    }
    if node.kind() == "type_parameters" {
        let start = node.start_byte();
        repairs.variance_annotations.extend(
            variance_annotation()
                .captures_iter(&source[node.byte_range()])
                .filter_map(|captures| captures.name("modifiers"))
                .map(|modifiers| start + modifiers.start()..start + modifiers.end()),
        );
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        tree_repairs(child, source, repairs);
    }
}

fn blank(bytes: &mut [u8]) {
    for byte in bytes {
        if !matches!(*byte, b'\n' | b'\r') {
            *byte = b' ';
        }
    }
}

fn compatibility_source(source: &str, repairs: &TreeRepairs) -> Option<String> {
    let mut bytes = source.as_bytes().to_vec();
    repair_tree_errors(&mut bytes, repairs);
    repair_keywords(&mut bytes, source);
    repair_import_types(&mut bytes, source);
    // Every repair rewrites a byte to a different one, so equal bytes mean no repair.
    (bytes != source.as_bytes()).then(|| String::from_utf8(bytes).expect("spaces preserve UTF-8"))
}

/// Repairs located from the failed parse tree.
fn repair_tree_errors(bytes: &mut [u8], repairs: &TreeRepairs) {
    for &escape in &repairs.template_escapes {
        bytes[escape] = b'q';
    }
    for range in repairs.variance_annotations.iter().cloned() {
        blank(&mut bytes[range]);
    }
    for &separator in &repairs.call_signature_separators {
        bytes[separator] = b';';
    }
}

/// Members named `abstract` and `export type *` re-exports.
fn repair_keywords(bytes: &mut [u8], source: &str) {
    for captures in abstract_member_name().captures_iter(source) {
        if let Some(name) = captures.name("name") {
            bytes[name.end() - 1] = b'$';
        }
    }
    for captures in export_type_star().captures_iter(source) {
        if let Some(keyword) = captures.name("type") {
            blank(&mut bytes[keyword.range()]);
        }
    }
}

fn repair_import_types(bytes: &mut [u8], source: &str) {
    let argument_ranges = import_type_argument_ranges(source);
    for range in argument_ranges.iter().cloned() {
        blank(&mut bytes[range.clone()]);
        bytes[range.start] = b'<';
        bytes[range.end - 1] = b'>';
        if let Some(type_byte) = bytes[range.start + 1..range.end]
            .iter_mut()
            .find(|byte| !matches!(**byte, b'\n' | b'\r'))
        {
            *type_byte = b'T';
        }
    }
    for range in import_type_ranges(source) {
        if argument_ranges
            .iter()
            .any(|argument| argument.contains(&range.start))
        {
            continue;
        }
        blank(&mut bytes[range.start + "import".len()..range.end]);
        bytes[range.start] = b'I';
    }
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

/// The one-based line and column of the first syntax error, or of the start of
/// the file when the tree has none.
pub(crate) fn syntax_error_position(tree: &Tree) -> (usize, usize) {
    let point = first_syntax_error(tree.root_node()).map_or_else(
        || tree.root_node().start_position(),
        |node| node.start_position(),
    );
    (point.row + 1, point.column + 1)
}

/// `parse error in <path>:<line>:<column>`, shared by every scanner seam so one
/// broken file reports one message.
pub(crate) fn parse_error_message(path: &str, tree: &Tree) -> String {
    let (line, column) = syntax_error_position(tree);
    format!("parse error in {path}:{line}:{column}")
}

/// Parse TypeScript with the same byte-preserving compatibility treatment at
/// every scanner seam. Tree-sitter currently rejects `typeof import(...)` in a
/// generic call argument, some other import types, invalid escapes in tagged
/// templates, variance annotations, overloaded generic call signatures on
/// separate lines, `export type *`, and members named `abstract`, all of which
/// TypeScript accepts.
pub(crate) fn parse(parser: &mut Parser, source: &str, path: &str) -> Result<Tree, String> {
    let tree = parsed_tree(parser, source, path)?;
    if !has_syntax_error(&tree) {
        return Ok(tree);
    }
    let mut repairs = TreeRepairs::default();
    tree_repairs(tree.root_node(), source, &mut repairs);
    let Some(compatible) = compatibility_source(source, &repairs) else {
        return Ok(tree);
    };
    parsed_tree(parser, &compatible, path)
}

#[cfg(test)]
mod tests {
    use super::{
        TreeRepairs, call_signature_separators, compatibility_source, has_syntax_error,
        import_type_argument_ranges, import_type_ranges, parse, tree_repairs,
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
        let compatible =
            compatibility_source(source, &TreeRepairs::default()).expect("compatibility rewrite");
        assert_eq!(compatible.len(), source.len());
        assert_eq!(compatible.as_bytes()[range.start], b'<');
        assert_eq!(compatible.as_bytes()[range.end - 1], b'>');
        assert!(compatible[range.start + 1..range.end - 1].contains('T'));
        assert_eq!(&compatible[range.end..], "()");

        assert!(compatibility_source("load<string>()", &TreeRepairs::default()).is_none());
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
        let compatible =
            compatibility_source(source, &TreeRepairs::default()).expect("compatibility rewrite");
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

    fn typescript_parser() -> Parser {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser
    }

    fn first_repairs(parser: &mut Parser, source: &str) -> TreeRepairs {
        let tree = parser.parse(source, None).unwrap();
        assert!(has_syntax_error(&tree), "{source}");
        let mut repairs = TreeRepairs::default();
        tree_repairs(tree.root_node(), source, &mut repairs);
        repairs
    }

    #[test]
    fn tagged_template_escapes_reparse_without_hiding_untagged_errors() {
        let mut parser = typescript_parser();
        let source = r"const a = String.raw`C:\users\x ${b} \u{41}\x41\u0041\u{zz}`;";
        let repairs = first_repairs(&mut parser, source);
        assert_eq!(repairs.template_escapes, vec![24, 30, 54]);
        let tree = parse(&mut parser, source, "raw.ts").unwrap();
        assert!(!has_syntax_error(&tree));
        assert_eq!(
            compatibility_source(source, &repairs).unwrap(),
            r"const a = String.raw`C:\qsers\q ${b} \u{41}\x41\u0041\q{zz}`;"
        );

        let source = r#"f(String.raw`{\rtf1 ` + "x" + String.raw`\u-10179?\x-8576? ${a} After}`);"#;
        assert_eq!(
            first_repairs(&mut parser, source).template_escapes,
            vec![42, 51]
        );
        for source in [
            r"const a = String.raw`\u-1`;",
            r"const a = tag.raw()[0]!`\u-1 ${{ a: 1 }}`;",
            r"const a = String.raw`$5 ${ {}.a } \`\u-1`;",
            "const a = String.raw `\\x`;",
        ] {
            let tree = parse(&mut parser, source, "raw.ts").unwrap();
            assert!(!has_syntax_error(&tree), "{source}");
        }

        for source in [
            r"const a = `C:\users\x`;",
            r"const a = `\u-1`;",
            r#"const a = "C:\users\x";"#,
            "const a = String.raw`C:\\users`;\nf(a;",
            r#"const a = String.raw`\u-1 ${"\u"}`;"#,
            r#"const a = String.raw`${ "}\u" } \u-1`;"#,
            r#"const a = String.raw`${ {}["\u"] } \u-1`;"#,
            r"const a = String.raw`\u-1",
        ] {
            let tree = parse(&mut parser, source, "broken.ts").unwrap();
            assert!(has_syntax_error(&tree), "{source}");
        }
    }

    #[test]
    fn variance_annotations_reparse_without_hiding_other_errors() {
        let mut parser = typescript_parser();
        let source = "interface I<in out H, const K> { h: H }";
        let repairs = first_repairs(&mut parser, source);
        assert_eq!(repairs.variance_annotations, vec![12..19]);
        assert_eq!(
            compatibility_source(source, &repairs).unwrap(),
            "interface I<       H, const K> { h: H }"
        );
        for source in [
            "export class Declaration<out Handler> { h!: Handler; }",
            "type T<\n  in\n  H,\n  const out K extends string = 'k'> = (h: H) => K;",
            "interface O<\n  /** @ts-ignore Cast variance */\n  out S extends B = B,\n  // why\n  out C = B,\n> {}",
        ] {
            let tree = parse(&mut parser, source, "variance.ts").unwrap();
            assert!(!has_syntax_error(&tree), "{source}");
        }
        for source in [
            "class D<out H> { h: H; }\nf(a;",
            "class D<out H K> {}",
            "class D<H out> {}",
        ] {
            let tree = parse(&mut parser, source, "broken.ts").unwrap();
            assert!(has_syntax_error(&tree), "{source}");
        }
    }

    #[test]
    fn overloaded_call_signatures_reparse_without_hiding_other_errors() {
        let mut parser = typescript_parser();
        let source = "interface Get<E> {\n  <K extends keyof E>(key: K): E[K]\n  // other\n  <K>(key: K): K\n}";
        let repairs = first_repairs(&mut parser, source);
        assert_eq!(repairs.call_signature_separators, vec![67]);
        assert_eq!(
            compatibility_source(source, &repairs).unwrap(),
            "interface Get<E> {\n  <K extends keyof E>(key: K): E[K]\n  // other\n ;<K>(key: K): K\n}"
        );
        for source in [
            "type U = {\n  <T>(s: T): [T, F<T>]\n  <T = undefined>(): [T | undefined]\n}",
            "interface F {\n  (h: H): [H]\n  <\n    I extends Input = {},\n  >(h: H): [H, I]\n}",
            "interface M {\n  <E>(p: (c: C<E>) => P): M<E>\n  <E>(p: P): M<E>\n  name: string\n}",
        ] {
            let tree = parse(&mut parser, source, "signatures.ts").unwrap();
            assert!(!has_syntax_error(&tree), "{source}");
        }
        for source in [
            "interface G {\n  <K>(key: K): K\n<K>(key: K): K\n}",
            "interface G {\n  <K>(key: K): K\n  <K>(key: K: K\n}",
            "const f = (a: A) =>\n  <T>(b: T) => b\nf(a;",
        ] {
            let tree = parse(&mut parser, source, "broken.ts").unwrap();
            assert!(has_syntax_error(&tree), "{source}");
        }
        assert!(call_signature_separators("a\n  <b", 0..1).is_empty());
        assert!(call_signature_separators("{\n  <b", 0..6).is_empty());
        assert!(call_signature_separators("x =>\n  <b", 0..9).is_empty());
        assert_eq!(call_signature_separators("x)\n\t<b", 0..7), vec![3]);
        assert_eq!(call_signature_separators("x_\n <b", 0..6), vec![3]);
        assert_eq!(call_signature_separators("K\n <b", 0..5), vec![2]);
        // A `<` at column zero has no indentation byte to hold a separator.
        assert!(call_signature_separators("x)\n<b", 0..5).is_empty());
    }

    #[test]
    fn export_type_star_reparses() {
        let mut parser = typescript_parser();
        for source in [
            "export type * from './types'",
            "export type * as types from './types'\nexport * from './x'",
        ] {
            let tree = parse(&mut parser, source, "index.ts").unwrap();
            assert!(!has_syntax_error(&tree), "{source}");
        }
        assert_eq!(
            compatibility_source("export type * from './t'", &TreeRepairs::default()).unwrap(),
            "export      * from './t'"
        );
        let tree = parse(&mut parser, "export type * from './t'\nf(a;", "broken.ts").unwrap();
        assert!(has_syntax_error(&tree));
    }

    #[test]
    fn members_named_abstract_reparse() {
        let mut parser = typescript_parser();
        for source in [
            "interface A {\n  abstract: boolean;\n}",
            "interface A {\n  readonly abstract?: false;\n  abstract(): void;\n}",
            "type A = { abstract: boolean };",
            "class A {\n  abstract!: boolean;\n  static abstract = 1;\n}",
            "abstract class B {\n  abstract run(): void;\n}\ninterface A { x: 1, abstract: 2 }",
        ] {
            let tree = parse(&mut parser, source, "index.ts").unwrap();
            assert!(!has_syntax_error(&tree), "{source}");
        }
        assert_eq!(
            compatibility_source(
                "abstract class B { abstract run(): void }\ntype A = { abstract: 1 }",
                &TreeRepairs::default()
            )
            .unwrap(),
            "abstract class B { abstract run(): void }\ntype A = { abstrac$: 1 }"
        );
        let tree = parse(&mut parser, "type A = { abstract: 1 }\nf(a;", "broken.ts").unwrap();
        assert!(has_syntax_error(&tree));
    }
}
