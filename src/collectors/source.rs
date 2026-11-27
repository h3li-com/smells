use super::{
    common::{location_at, observation, primitive_type, simple_names},
    model::SourceModel,
};
use crate::{
    core::model::Input, evidence::Observation, names::dunder, policy::Rule, report::Location,
};
use regex::Regex;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::OnceLock,
};

fn navigation_expression() -> &'static Regex {
    static EXPRESSION: OnceLock<Regex> = OnceLock::new();
    EXPRESSION.get_or_init(|| {
        Regex::new(r"[A-Za-z_][A-Za-z0-9_]*(?:\??\.[A-Za-z_][A-Za-z0-9_]*(?:\([^()\n]*\))?){1,}")
            .expect("valid built-in navigation expression")
    })
}

fn member_access() -> &'static Regex {
    static ACCESS: OnceLock<Regex> = OnceLock::new();
    ACCESS.get_or_init(|| {
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\??\s*\.\s*[A-Za-z_]")
            .expect("valid built-in member-access expression")
    })
}

fn private_member_access() -> &'static Regex {
    static ACCESS: OnceLock<Regex> = OnceLock::new();
    ACCESS.get_or_init(|| {
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\??\.(_[A-Za-z_][A-Za-z0-9_]*)")
            .expect("valid built-in private-member expression")
    })
}

fn navigation_hop() -> &'static Regex {
    static HOP: OnceLock<Regex> = OnceLock::new();
    HOP.get_or_init(|| {
        Regex::new(r"\??\.([A-Za-z_][A-Za-z0-9_]*)(\([^()\n]*\))?")
            .expect("valid built-in navigation hop expression")
    })
}

/// Standard-library adapters and conversions reshape a value without reaching
/// into another object, so they neither extend nor break a navigation run.
fn navigation_adapters(language: &str) -> &'static [&'static str] {
    match language {
        "rust" => &[
            "iter",
            "iter_mut",
            "into_iter",
            "clone",
            "cloned",
            "copied",
            "collect",
            "unwrap",
            "expect",
            "as_ref",
            "as_mut",
            "as_str",
            "as_slice",
            "as_deref",
            "as_bytes",
            "to_string",
            "to_owned",
            "into_owned",
            "to_vec",
            "to_str",
            "to_string_lossy",
            "as_path",
            "as_os_str",
            "to_path_buf",
            "into",
            "len",
            "is_empty",
            "is_none",
            "is_some",
            "lock",
            "await",
            "borrow",
            "borrow_mut",
            "keys",
            "values",
            "rev",
            "chars",
            "bytes",
            "lines",
            "trim",
            "count",
            "sum",
            "first",
            "last",
            "unwrap_or_default",
            "ok",
            "some",
            "flatten",
            "peekable",
            "enumerate",
            "display",
        ],
        "python" => &[
            "items",
            "keys",
            "values",
            "copy",
            "strip",
            "lstrip",
            "rstrip",
            "lower",
            "upper",
            "encode",
            "decode",
            "split",
            "splitlines",
            "casefold",
            "title",
        ],
        "typescript" => &[
            "toString",
            "valueOf",
            "trim",
            "toLowerCase",
            "toUpperCase",
            "keys",
            "values",
            "entries",
            "length",
            "slice",
            "flat",
            "reverse",
            "toJSON",
        ],
        _ => &[],
    }
}

const NAVIGATION_SELF_ROOTS: [&str; 3] = ["self", "this", "Self"];

/// The longest run of consecutive navigation hops: field reads and zero-argument
/// calls. A call that takes arguments starts a new run, adapters are transparent,
/// and the first hop from the method's own receiver reads owned state. `open_call`
/// says the expression's last hop opens an argument list the expression regex could
/// not close: nested parentheses or arguments on the following lines.
fn navigation_run(expression: &str, adapters: &[&str], open_call: bool) -> u64 {
    let root_end = expression.find(['.', '?']).unwrap_or(expression.len());
    let mut skip_first = NAVIGATION_SELF_ROOTS.contains(&&expression[..root_end]);
    let (mut run, mut longest) = (0_u64, 0_u64);
    let hops = &expression[root_end..];
    for hop in navigation_hop().captures_iter(hops) {
        if std::mem::take(&mut skip_first) {
            continue;
        }
        let name = hop.get(1).expect("hop name group").as_str();
        let unclosed = open_call && hop.get(0).expect("hop match").end() == hops.len();
        let arguments = hop.get(2).map_or("()", |arguments| arguments.as_str());
        if unclosed || arguments != "()" {
            run = 0;
        } else if !adapters.contains(&name) {
            run += 1;
            longest = longest.max(run);
        }
    }
    longest
}

pub(super) fn visit_navigation_chains(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
    mut emit: impl FnMut(Observation),
) -> Result<(), String> {
    let language = rule.id.split('.').next().unwrap_or_default();
    let adapters = navigation_adapters(language);
    for path in input.files.keys() {
        let code = model
            .code_by_path
            .get(path)
            .ok_or_else(|| format!("missing authored code for {path}"))?;
        for (line_index, line) in code.lines().enumerate() {
            for candidate in navigation_expression().find_iter(line) {
                let open_call = line[candidate.end()..].starts_with('(');
                let transitions = navigation_run(candidate.as_str(), adapters, open_call);
                emit(Observation {
                    symbol: format!("{path}::navigation@{}", line_index + 1),
                    location: Location {
                        path: path.clone(),
                        line: line_index + 1,
                        column: line[..candidate.start()].chars().count() + 1,
                    },
                    related_symbols: Vec::new(),
                    related_locations: Vec::new(),
                    measurements: BTreeMap::from([("transitions".into(), transitions)]),
                    evidence: json!({
                        "collector": "source_navigation_chain_v2",
                        "expression": candidate.as_str(),
                    }),
                });
            }
        }
    }
    Ok(())
}

const OWNED_RECEIVERS: [&str; 5] = ["self", "cls", "this", "Self", "super"];

/// A local bound directly from owned state, such as `let buffer = self.buffer.as_mut();`,
/// `let me = self.project();`, `const def = this._def` or `items = self.items`.
fn owned_alias() -> &'static Regex {
    static ALIAS: OnceLock<Regex> = OnceLock::new();
    ALIAS.get_or_init(|| {
        Regex::new(
            r"(?m)(?:\b(?:let|const|var)\s+(?:mut\s+)?|^[ \t]*)([A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=;\n]*)?=\s*(?:&\s*(?:mut\s+)?)?([A-Za-z_][A-Za-z0-9_]*)\s*\??\s*\.",
        )
        .expect("valid built-in owned-alias expression")
    })
}

/// A Python annotated statement such as `mappings: dict[str, t.Any] = {}`; the
/// annotation names types, not members the method reaches.
fn python_annotation() -> &'static Regex {
    static ANNOTATION: OnceLock<Regex> = OnceLock::new();
    ANNOTATION.get_or_init(|| {
        Regex::new(r"(?m)^[ \t]*([A-Za-z_][A-Za-z0-9_.]*)[ \t]*:([^=\n(){}]+?)[ \t]*(?:=|$)")
            .expect("valid built-in Python annotation expression")
    })
}

/// A destructuring binding such as `let Self { get, fallback, .. } = self;`,
/// `if let Some(inner) = &self.inner` or `const { a, b: c } = this.props`.
fn pattern_alias() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"\b(?:let|const|var)\s+((?:[A-Za-z_][A-Za-z0-9_:]*\s*)?[({\[][^=;]*?[)}\]])\s*=\s*(?:&\s*(?:mut\s+)?)?([A-Za-z_][A-Za-z0-9_]*)\b",
        )
        .expect("valid built-in pattern-alias expression")
    })
}

/// A Rust `match` whose scrutinee starts at a name, possibly borrowed or
/// dereferenced, such as `match &self.legacy {` or `match *self {`.
fn match_scrutinee() -> &'static Regex {
    static SCRUTINEE: OnceLock<Regex> = OnceLock::new();
    SCRUTINEE.get_or_init(|| {
        Regex::new(r"\bmatch\s+(?:&\s*(?:mut\s+)?|\*\s*)?([A-Za-z_][A-Za-z0-9_]*)\b[^{;]*\{")
            .expect("valid built-in match-scrutinee expression")
    })
}

/// A lowercase name in a pattern; whether it binds depends on its neighbours.
fn pattern_name() -> &'static Regex {
    static NAME: OnceLock<Regex> = OnceLock::new();
    NAME.get_or_init(|| {
        Regex::new(r"\b[a-z_][A-Za-z0-9_]*\b").expect("valid built-in pattern-name expression")
    })
}

/// Names a pattern binds: not keywords, paths, field labels (`field: binding`),
/// constructors, or anything in a match guard.
fn pattern_bindings(pattern: &str) -> Vec<&str> {
    let pattern = pattern.split(" if ").next().unwrap_or(pattern);
    pattern_name()
        .find_iter(pattern)
        .filter(|name| !matches!(name.as_str(), "ref" | "mut" | "_" | "box"))
        .filter(|name| {
            let before = pattern[..name.start()].trim_end();
            let after = pattern[name.end()..].trim_start();
            !before.ends_with("::")
                && !before.ends_with('.')
                && !after.starts_with(':')
                && !after.starts_with('(')
                && !after.starts_with('{')
        })
        .map(|name| name.as_str())
        .collect()
}

/// The arm patterns of a `match` block whose body starts at `start`: the text before
/// each top-level `=>`, after the previous arm's `,` or closing brace.
fn match_arm_patterns(body: &str, start: usize) -> Vec<&str> {
    let mut patterns = Vec::new();
    let (mut depth, mut boundary) = (0_usize, start);
    let bytes = body.as_bytes();
    for (offset, byte) in bytes.iter().enumerate().skip(start) {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => break,
            b'}' if depth == 1 => (depth, boundary) = (0, offset + 1),
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => boundary = offset + 1,
            b'=' if depth == 0 && bytes.get(offset + 1) == Some(&b'>') => {
                patterns.push(&body[boundary..offset]);
            }
            _ => {}
        }
    }
    patterns
}

/// A local closure whose first parameter is a name, such as
/// `const removeFromMap = (map: Map<string, any[]>) => {` or `function visit(node) {`.
fn local_closure() -> &'static Regex {
    static CLOSURE: OnceLock<Regex> = OnceLock::new();
    CLOSURE.get_or_init(|| {
        Regex::new(
            r"\b(?:(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:async\s+)?(?:function\b\s*)?|function\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>(]*>)?\s*)\(\s*([A-Za-z_$][A-Za-z0-9_$]*)\b",
        )
        .expect("valid built-in local-closure expression")
    })
}

/// The opening of a call whose first argument starts at a name, such as
/// `(this.listeners)` after `removeFromMap`.
fn call_argument() -> &'static Regex {
    static CALL: OnceLock<Regex> = OnceLock::new();
    CALL.get_or_init(|| {
        Regex::new(r"^\s*\(\s*([A-Za-z_$][A-Za-z0-9_$]*)\b")
            .expect("valid built-in call-argument expression")
    })
}

/// The root each use of closure `name` outside its declaration passes as its first
/// argument, or `None` when any use is not such a call, as in `items.forEach(visit)`.
fn closure_call_roots<'body>(
    body: &'body str,
    name: &str,
    declaration: Range<usize>,
) -> Option<Vec<&'body str>> {
    let identifier = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    body.match_indices(name)
        .map(|(start, _)| start)
        .filter(|start| {
            let (before, after) = (&body[..*start], &body[start + name.len()..]);
            !declaration.contains(start)
                && !before.ends_with(identifier)
                && !after.starts_with(identifier)
                && !before.trim_end().ends_with('.')
        })
        .map(|start| {
            call_argument()
                .captures(&body[start + name.len()..])
                .map(|call| call.get(1).expect("call argument group").as_str())
        })
        .collect()
}

/// The first parameter of a local closure, bound to the root its calls pass in, when
/// every use in the body is a call passing an argument from the same root, as in
/// `removeFromMap(this.a); removeFromMap(this.b)`. A closure that is never called,
/// called with arguments from different roots, or passed on as a value binds nothing.
fn closure_parameter_bindings(body: &str) -> Vec<(usize, &str, Vec<&str>)> {
    local_closure()
        .captures_iter(body)
        .filter_map(|capture| {
            let whole = capture.get(0).expect("closure match");
            let name = capture.get(1).or_else(|| capture.get(2))?.as_str();
            let parameter = capture.get(3).expect("closure parameter group").as_str();
            let roots = closure_call_roots(body, name, whole.range())?;
            let (&root, others) = roots.split_first()?;
            others
                .iter()
                .all(|other| *other == root)
                .then(|| (whole.start(), root, vec![parameter]))
        })
        .collect()
}

/// Bindings in body order with the root they come from: plain aliases, destructuring
/// patterns, the arms of a `match` on a name and local closure parameters.
fn alias_bindings(body: &str) -> Vec<(usize, &str, Vec<&str>)> {
    let mut bindings: Vec<(usize, &str, Vec<&str>)> = Vec::new();
    for capture in owned_alias().captures_iter(body) {
        let name = capture.get(1).expect("alias name group").as_str();
        let root = capture.get(2).expect("alias root group").as_str();
        bindings.push((
            capture.get(0).expect("alias match").start(),
            root,
            vec![name],
        ));
    }
    for capture in pattern_alias().captures_iter(body) {
        let names = pattern_bindings(capture.get(1).expect("pattern group").as_str());
        let root = capture.get(2).expect("pattern root group").as_str();
        bindings.push((capture.get(0).expect("pattern match").start(), root, names));
    }
    for capture in match_scrutinee().captures_iter(body) {
        let whole = capture.get(0).expect("match expression");
        let names = match_arm_patterns(body, whole.end())
            .into_iter()
            .flat_map(pattern_bindings)
            .collect();
        bindings.push((whole.start(), capture.get(1).expect("root").as_str(), names));
    }
    bindings.extend(closure_parameter_bindings(body));
    bindings.sort_by_key(|(start, _, _)| *start);
    bindings
}

/// Names bound from owned state, in body order, so an alias of an alias stays owned.
fn owned_aliases(body: &str) -> BTreeSet<&str> {
    let mut aliases = BTreeSet::new();
    for (_, root, names) in alias_bindings(body) {
        if OWNED_RECEIVERS.contains(&root) || aliases.contains(root) {
            aliases.extend(names);
        }
    }
    aliases
}

/// Byte ranges of Python statement annotations. Block keywords and a trailing comma
/// (a dictionary entry) are not annotations.
fn python_annotation_ranges(body: &str) -> Vec<(usize, usize)> {
    python_annotation()
        .captures_iter(body)
        .filter(|capture| !matches!(&capture[1], "else" | "try" | "finally" | "except"))
        .filter_map(|capture| capture.get(2))
        .filter(|annotation| !annotation.as_str().trim_end().ends_with(','))
        .map(|annotation| (annotation.start(), annotation.end()))
        .collect()
}

/// Whether the receiver at `start` continues a dotted chain of names such as the
/// `innerType` in `this._def.innerType._parse`. A chain resumed after a call or an
/// index, as in `make().value`, starts a new receiver.
fn continues_chain(body: &str, start: usize) -> bool {
    let separator = |character: char| character.is_whitespace() || character == '?';
    body[..start]
        .trim_end_matches(separator)
        .strip_suffix('.')
        .and_then(|before| before.trim_end_matches(separator).chars().next_back())
        .is_some_and(|character| character.is_alphanumeric() || character == '_')
}

/// Ownership of each dotted access receiver in body order: an owned receiver, a local
/// alias of owned state, or a chain continuing from either is owned.
fn receiver_ownership(body: &str, python: bool) -> Vec<bool> {
    let aliases = owned_aliases(body);
    let annotations = if python {
        python_annotation_ranges(body)
    } else {
        Vec::new()
    };
    let mut chain_owned = false;
    member_access()
        .captures_iter(body)
        .map(|capture| capture.get(1).expect("receiver group"))
        .filter(|receiver| {
            !annotations
                .iter()
                .any(|(start, end)| (*start..*end).contains(&receiver.start()))
        })
        .map(|receiver| {
            let name = receiver.as_str();
            if !continues_chain(body, receiver.start()) {
                chain_owned = OWNED_RECEIVERS.contains(&name) || aliases.contains(name);
            }
            chain_owned
        })
        .collect()
}

/// Feature Envy is a property of a method: compare its accesses through foreign
/// receivers with those through its own. Free functions have no own data.
pub(super) fn foreign_accesses(model: &SourceModel) -> Vec<Observation> {
    model
        .functions
        .iter()
        .filter(|function| function.receiver)
        .filter_map(|function| {
            let body = function.body.as_deref()?;
            let python = std::path::Path::new(&function.location.path)
                .extension()
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("py") || extension.eq_ignore_ascii_case("pyi")
                });
            let ownership = receiver_ownership(body, python);
            if ownership.is_empty() {
                return None;
            }
            let foreign = ownership.iter().filter(|owned| !**owned).count();
            Some(observation(
                function.symbol.clone(),
                function.location.clone(),
                [
                    ("foreign_accesses", foreign as u64),
                    ("total_accesses", ownership.len() as u64),
                ],
                json!({
                    "collector": "source_method_member_ownership_v3",
                    "owned_receivers": OWNED_RECEIVERS,
                    "owned_aliases": "locals bound from owned state",
                    "chains": "continuations inherit the chain root's ownership",
                }),
            ))
        })
        .collect()
}

/// Directory holding the child modules of the Rust module defined in `path`.
fn rust_module_dir(path: &str) -> String {
    let (parent, file) = path.rsplit_once('/').unwrap_or(("", path));
    let prefix = if parent.is_empty() {
        String::new()
    } else {
        format!("{parent}/")
    };
    match file {
        "mod.rs" | "lib.rs" | "main.rs" => prefix,
        _ => format!("{prefix}{}/", file.trim_end_matches(".rs")),
    }
}

/// Private fields are visible to the owner's module and its descendants, so only a
/// descendant file can compile against them. Same-file access is the owner's own, and
/// any other match is a receiver-name collision.
fn rust_descendant_access(owner_paths: &BTreeSet<String>, path: &str) -> bool {
    owner_paths
        .iter()
        .all(|owner| owner != path && path.starts_with(&rust_module_dir(owner)))
}

fn rust_dependency_contract(
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    let mut owners: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for (receiver, field, path) in model.classes.iter().flat_map(|class| {
        let mut receiver = String::new();
        for (index, character) in class.name.chars().enumerate() {
            if index > 0 && character.is_ascii_uppercase() {
                receiver.push('_');
            }
            receiver.push(character.to_ascii_lowercase());
        }
        class
            .fields
            .iter()
            .filter(|field| field.private && simple_names(&field.name).len() == 1)
            .map(move |field| {
                (
                    receiver.clone(),
                    field.name.clone(),
                    class.location.path.clone(),
                )
            })
    }) {
        owners.entry((receiver, field)).or_default().insert(path);
    }
    let private_fields = owners.keys().cloned().collect::<BTreeSet<_>>();
    let mut observations = Vec::new();
    for (path, source) in &input.files {
        let code = model
            .code_by_path
            .get(path)
            .ok_or_else(|| format!("missing authored code for {path}"))?;
        let mut matches = Vec::new();
        for ((receiver, field), owner_paths) in &owners {
            if !rust_descendant_access(owner_paths, path) {
                continue;
            }
            let access = Regex::new(&format!(
                r"\b{}\s*\.\s*{}\b",
                regex::escape(receiver),
                regex::escape(field)
            ))
            .expect("escaped Rust private-field expression");
            matches.extend(access.find_iter(code).map(|found| found.start()));
        }
        matches.sort_unstable();
        let Some(first) = matches.first() else {
            continue;
        };
        observations.push(observation(
            format!("{path}::foreign-private-field-accesses"),
            location_at(path, source, *first),
            [("forbidden_accesses", matches.len() as u64)],
            json!({
                "collector": "rust_private_field_name_access_v2",
                "scope": "dotted access in a descendant module file whose receiver name matches the snake-case private-field owner",
                "private_owner_fields": private_fields,
            }),
        ));
    }
    Ok(observations)
}

/// Whether a `_`-prefixed member name is public by convention: a dunder protocol or
/// builtin such as `__html__`, `__version__` or `__proto__`; in Python, a sunder enum
/// protocol name such as `_value_` or `_missing_`; or, in TypeScript, an all-caps
/// `__NAME` binding constant such as a Cloudflare `__STATIC_CONTENT`.
fn public_underscore_member(language: &str, member: &str) -> bool {
    let sunder = language == "python"
        && member
            .strip_prefix('_')
            .and_then(|rest| rest.strip_suffix('_'))
            .is_some_and(|name| !name.is_empty() && !name.starts_with('_') && !name.ends_with('_'));
    let constant = language == "typescript"
        && member.strip_prefix("__").is_some_and(|name| {
            name.starts_with(|character: char| character.is_ascii_uppercase())
                && name.chars().all(|character| {
                    character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
                })
        });
    dunder(member) || sunder || constant
}

/// Whether the innermost class around `offset`, the one that starts last, in `path` declares `member` as a
/// field or method or uses it on its own receiver. Access to that member on
/// another instance of the class, as in `other._key` inside `__eq__`, is the
/// class's own.
fn enclosing_class_member(model: &SourceModel, path: &str, offset: usize, member: &str) -> bool {
    let Some(class) = model
        .classes
        .iter()
        .filter(|class| class.location.path == path)
        .filter(|class| class.range.0 <= offset && offset < class.range.1)
        .max_by_key(|class| class.range.0)
    else {
        return false;
    };
    class.fields.iter().any(|field| field.name == member)
        || class.methods.iter().any(|method| method.name == member)
        || receiver_member(&class.source, member)
}

/// Whether `source` uses `member` on `self` or `this`, as in `self._key`.
fn receiver_member(source: &str, member: &str) -> bool {
    let word = |character: char| character.is_alphanumeric() || character == '_';
    source.match_indices(member).any(|(start, _)| {
        let whole = !source[start + member.len()..].starts_with(word);
        let receiver = source[..start]
            .trim_end()
            .strip_suffix('.')
            .map(str::trim_end)
            .is_some_and(|before| {
                ["self", "this"].iter().any(|name| {
                    before
                        .strip_suffix(name)
                        .is_some_and(|rest| !rest.ends_with(word))
                })
            });
        whole && receiver
    })
}

pub(super) fn dependency_contract(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    if rule.id.starts_with("rust.") {
        return rust_dependency_contract(model, input);
    }
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    let mut observations = Vec::new();
    for (path, source) in &input.files {
        let code = model
            .code_by_path
            .get(path)
            .ok_or_else(|| format!("missing authored code for {path}"))?;
        let accesses = private_member_access()
            .captures_iter(code)
            .filter(|capture| !matches!(&capture[1], "self" | "cls" | "this" | "Self" | "super"))
            .filter(|capture| !public_underscore_member(language, &capture[2]))
            .filter(|capture| {
                let start = capture.get(0).expect("private member match").start();
                !enclosing_class_member(model, path, start, &capture[2])
            })
            .collect::<Vec<_>>();
        let Some(first) = accesses.first() else {
            continue;
        };
        let start = first.get(0).expect("private member match").start();
        let prefix = &source[..start];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix
            .rsplit_once('\n')
            .map_or(prefix, |(_, suffix)| suffix)
            .chars()
            .count()
            + 1;
        observations.push(Observation {
            symbol: format!("{path}::foreign-private-accesses"),
            location: Location {
                path: path.clone(),
                line,
                column,
            },
            related_symbols: Vec::new(),
            related_locations: Vec::new(),
            measurements: BTreeMap::from([("forbidden_accesses".into(), accesses.len() as u64)]),
            evidence: json!({
                "collector": "source_private_member_access_v2",
                "contract": "language_private_naming_or_syntax",
                "public_by_convention": "dunder names; Python sunder names; TypeScript all-caps __NAME constants",
            }),
        });
    }
    Ok(observations)
}

pub(super) fn primitive_slots(rule: &Rule, model: &SourceModel) -> Vec<Observation> {
    let language = rule.id.split_once('.').map_or("", |(language, _)| language);
    model
        .classes
        .iter()
        .filter(|class| !class.fields.is_empty())
        .filter(|class| !(language == "typescript" && class.interface))
        .map(|class| {
            let raw = class
                .fields
                .iter()
                .filter(|field| primitive_type(language, &field.authored_type))
                .count();
            observation(
                class.symbol.clone(),
                class.location.clone(),
                [
                    ("raw_slots", raw as u64),
                    ("total_slots", class.fields.len() as u64),
                ],
                json!({
                    "collector": "authored_primitive_annotations_v2",
                    "scope": "annotated class slots; TypeScript interfaces are not classes",
                    "language": language,
                }),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::model::{ClassFact, CodeScope, FieldFact};
    use std::sync::Arc;

    #[test]
    fn pattern_bindings_skip_paths_labels_and_guards() {
        assert_eq!(pattern_bindings("state::Ready(ref mut x) if x.ok"), ["x"]);
        assert_eq!(pattern_bindings("Point { x: px, y, .. }"), ["px", "y"]);
    }

    #[test]
    fn match_arm_patterns_resume_after_nested_block_arms() {
        let body = "match s { A => { if y { z } } B(b) => b, }";
        let start = body.find('{').expect("match body") + 1;
        let arms: Vec<&str> = match_arm_patterns(body, start)
            .into_iter()
            .map(str::trim)
            .collect();
        assert_eq!(arms, ["A", "B(b)"]);
    }

    #[test]
    fn match_arm_patterns_split_on_top_level_commas_and_arrows_only() {
        let body = "match s { A => 1, B(b, c) => b, x if x == 1 => x, }";
        let start = body.find('{').expect("match body") + 1;
        let arms: Vec<&str> = match_arm_patterns(body, start)
            .into_iter()
            .map(str::trim)
            .collect();
        assert_eq!(arms, ["A", "B(b, c)", "x if x == 1"]);
    }

    #[test]
    fn public_underscore_members_need_a_name_inside_the_dunder() {
        assert!(public_underscore_member("python", "__html__"));
        assert!(public_underscore_member("python", "__a__"));
        assert!(!public_underscore_member("python", "____"));
        assert!(public_underscore_member("python", "_value_"));
        assert!(public_underscore_member("python", "_a_"));
        assert!(!public_underscore_member("python", "_value"));
        assert!(!public_underscore_member("python", "___"));
        assert!(!public_underscore_member("python", "__"));
        assert!(!public_underscore_member("python", "_"));
        assert!(!public_underscore_member("python", "_a__"));
        assert!(!public_underscore_member("python", "__a_"));
        assert!(!public_underscore_member("typescript", "_value_"));
        assert!(public_underscore_member("typescript", "__STATIC_CONTENT"));
        assert!(!public_underscore_member("typescript", "__1A"));
        assert!(!public_underscore_member("typescript", "__Ab"));
    }

    #[test]
    fn continues_chain_follows_names_but_not_calls() {
        let chain = "this._def.innerType._parse";
        assert!(continues_chain(
            chain,
            chain.find("innerType").expect("member")
        ));
        assert!(continues_chain(
            chain,
            chain.find("_parse").expect("member")
        ));
        let optional = "this._def?.innerType";
        assert!(continues_chain(
            optional,
            optional.find("innerType").expect("member")
        ));
        let call = "make().value";
        assert!(!continues_chain(call, call.find("value").expect("member")));
    }

    fn rule(id: &str) -> Rule {
        Rule {
            id: id.into(),
            version: 1,
            smell: "test".into(),
            kind: "threshold".into(),
            implementation: "test".into(),
            inputs: Vec::new(),
            parameters: BTreeMap::new(),
            contract: "test".into(),
        }
    }

    fn input(rule_pack: &str, files: &[(&str, &str)]) -> Input {
        let files = files
            .iter()
            .map(|(path, source)| ((*path).into(), (*source).into()))
            .collect();
        Input::fixture(rule_pack, files)
    }

    fn model_with_code(path: &str, source: &str) -> SourceModel {
        SourceModel {
            code_by_path: BTreeMap::from([(path.into(), Arc::from(source))]),
            ..SourceModel::default()
        }
    }

    #[test]
    fn navigation_adapters_are_transparent_only_in_their_own_language() {
        for (language, expression) in [
            ("rust", "a.b.iter().c"),
            ("rust", "a.b.into_owned().c"),
            ("python", "a.b.items().c"),
            ("typescript", "a.b.toString().c"),
        ] {
            assert_eq!(
                navigation_run(expression, navigation_adapters(language), false),
                2,
                "{language}"
            );
            assert_eq!(navigation_run(expression, &[], false), 3, "{language}");
        }
    }

    #[test]
    fn navigation_chains_count_the_longest_run_of_navigation_hops() {
        let source = concat!(
            "let value = root.first().second.third;\n",
            "  other?.child.run();\n",
            "  a.b.c.find(x).d.e;\n",
            "  self.config.items.iter().cloned().last_seen;\n",
            "  this.a.b.c;\n",
        );
        let input = input("typescript-v1", &[("sample.ts", source)]);
        let model = model_with_code("sample.ts", source);
        let mut observations = Vec::new();
        visit_navigation_chains(
            &rule("typescript.navigation_chains"),
            &model,
            &input,
            |observation| observations.push(observation),
        )
        .unwrap();

        let measured = observations
            .iter()
            .map(|observation| {
                (
                    observation.location.line,
                    observation.location.column,
                    observation.measurements["transitions"],
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            measured,
            [(1, 13, 3), (2, 3, 2), (3, 3, 2), (4, 3, 4), (5, 3, 2)]
        );
        assert_eq!(
            navigation_run(
                "self.config.items.iter().cloned().last_seen",
                navigation_adapters("rust"),
                false
            ),
            2
        );
        assert_eq!(navigation_run("Self.a.b.c.d", &[], false), 3);
    }

    #[test]
    fn foreign_accesses_measure_each_receiver_method_against_its_own_accesses() {
        let source = concat!(
            "class Service:\n",
            "    def envy(self, order):\n",
            "        return order.a + order.b + order.c + self.x\n",
            "    def own(self):\n",
            "        return self.x + cls.y + super.z\n",
            "    @staticmethod\n",
            "    def helper(order):\n",
            "        return order.a + order.b\n",
            "def free(order):\n",
            "    return order.a + order.b\n",
        );
        let model = super::super::model::source_model(
            &rule("python.foreign_accesses"),
            &input("python-v1", &[("sample.py", source)]),
        )
        .unwrap();
        let observations = foreign_accesses(&model);
        let measured = observations
            .iter()
            .map(|observation| {
                (
                    observation.symbol.as_str(),
                    observation.location.line,
                    observation.measurements["foreign_accesses"],
                    observation.measurements["total_accesses"],
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            measured,
            [
                ("sample.py::Service.envy", 2, 3, 4),
                ("sample.py::Service.own", 4, 0, 3),
            ]
        );
    }

    #[test]
    fn foreign_accesses_read_token_stream_rust_method_bodies() {
        let source = concat!(
            "pub struct S { x: u8 }\n",
            "impl S {\n",
            "    pub fn envy(&self, o: &O) -> u8 { o.a + o.b + o.c + self.x + (0..o.d).len() as u8 }\n",
            "    pub fn new() -> Self { S { x: 0 } }\n",
            "}\n",
        );
        let model = super::super::model::source_model(
            &rule("rust.foreign_accesses"),
            &input("rust-v1", &[("src/lib.rs", source)]),
        )
        .unwrap();
        let observations = foreign_accesses(&model);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].symbol, "src/lib.rs::S.envy");
        assert_eq!(observations[0].measurements["foreign_accesses"], 4);
        assert_eq!(observations[0].measurements["total_accesses"], 5);
    }

    #[test]
    fn dependency_contract_counts_only_foreign_private_accesses() {
        let source = concat!(
            "self._owned\n",
            "service._token\n",
            "service._other\n",
            "this.#owned\n",
        );
        let observations = dependency_contract(
            &rule("python.dependency_contract"),
            &model_with_code("sample.py", source),
            &input("python-v1", &[("sample.py", source)]),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["forbidden_accesses"], 2);
        assert_eq!(observations[0].location.line, 2);
        assert_eq!(observations[0].location.column, 1);
    }

    #[test]
    fn rust_dependency_contract_uses_snake_case_private_field_owners() {
        let source = concat!(
            "fn inspect(user_account: UserAccount, other: UserAccount) {\n",
            "  let first = user_account.secret;\n",
            "  let second = other.secret;\n",
            "  let third = user_account.secret;\n",
            "  let public = user_account.public_value;\n",
            "  let nested = user_account.nested.secret;\n",
            "}\n",
        );
        let mut model = model_with_code("src/domain/check.rs", source);
        model
            .code_by_path
            .insert("src/domain.rs".into(), source.into());
        model
            .code_by_path
            .insert("src/billing.rs".into(), source.into());
        model.classes.push(ClassFact {
            name: "UserAccount".into(),
            symbol: "src/domain.rs::UserAccount".into(),
            location: Location {
                path: "src/domain.rs".into(),
                line: 1,
                column: 1,
            },
            type_parameters: Vec::new(),
            source: Arc::from(""),
            bases: Vec::new(),
            interface: false,
            private: false,
            scope: CodeScope::Production,
            fields: vec![
                FieldFact {
                    name: "secret".into(),
                    authored_type: "String".into(),
                    temporary: false,
                    private: true,
                },
                FieldFact {
                    name: "public_value".into(),
                    authored_type: "String".into(),
                    temporary: false,
                    private: false,
                },
                FieldFact {
                    name: "nested.secret".into(),
                    authored_type: "String".into(),
                    temporary: false,
                    private: true,
                },
            ],
            methods: Vec::new(),
            range: (0, 0),
        });
        let observations = dependency_contract(
            &rule("rust.dependency_contract"),
            &model,
            &input(
                "rust-v1",
                &[
                    ("src/billing.rs", source),
                    ("src/domain.rs", source),
                    ("src/domain/check.rs", source),
                ],
            ),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0].symbol,
            "src/domain/check.rs::foreign-private-field-accesses"
        );
        assert_eq!(observations[0].measurements["forbidden_accesses"], 2);
        assert_eq!(observations[0].location.line, 2);
        assert_eq!(observations[0].location.column, 15);
    }

    #[test]
    fn navigation_treats_option_queries_guards_and_await_as_transparent() {
        let rust = navigation_adapters("rust");
        for expression in [
            "self.config.colors.matched().is_none",
            "orphanage.sigchild.matched.lock().is_none()",
            "field.attrs.default().is_some()",
            "response.into_body().collect().await.unwrap().to_bytes()",
        ] {
            assert_eq!(navigation_run(expression, rust, false), 2, "{expression}");
        }
    }

    #[test]
    fn navigation_calls_with_nested_or_multi_line_arguments_break_the_run() {
        let source = concat!(
            "app.router._routes.append(Route(\"/admin\", endpoint))\n",
            "table.box.__dict__.update(\n",
            "    top_left=\"a\",\n",
            ")\n",
            "a.b.c.d\n",
        );
        let input = input("python-v1", &[("sample.py", source)]);
        let model = model_with_code("sample.py", source);
        let mut transitions = Vec::new();
        visit_navigation_chains(
            &rule("python.navigation_chains"),
            &model,
            &input,
            |observation| transitions.push(observation.measurements["transitions"]),
        )
        .unwrap();
        assert_eq!(transitions, [2, 2, 3]);
    }

    fn foreign_measurements(
        rule_id: &str,
        rule_pack: &str,
        path: &str,
        source: &str,
    ) -> Vec<(String, u64, u64)> {
        let model =
            super::super::model::source_model(&rule(rule_id), &input(rule_pack, &[(path, source)]))
                .unwrap();
        foreign_accesses(&model)
            .into_iter()
            .map(|observation| {
                (
                    observation.symbol,
                    observation.measurements["foreign_accesses"],
                    observation.measurements["total_accesses"],
                )
            })
            .collect()
    }

    #[test]
    fn foreign_accesses_treat_locals_bound_from_owned_state_as_owned() {
        let source = concat!(
            "pub struct S { buffer: Vec<u8> }\n",
            "impl S {\n",
            "    fn field(&mut self, name: &str) {\n",
            "        let buffer = self.buffer.as_mut();\n",
            "        let me = self.project();\n",
            "        let inner = me.inner;\n",
            "        buffer.push(1); buffer.push(2); me.a.go(); inner.b(); name.len();\n",
            "    }\n",
            "}\n",
        );
        assert_eq!(
            foreign_measurements("rust.foreign_accesses", "rust-v1", "src/lib.rs", source),
            [("src/lib.rs::S.field".into(), 1, 8)]
        );
    }

    #[test]
    fn foreign_accesses_treat_pattern_bindings_from_owned_state_as_owned() {
        let source = concat!(
            "pub struct S { get: Route, legacy: Option<L> }\n",
            "impl S {\n",
            "    fn field(self, other: &O) {\n",
            "        let Self { get, legacy: old, .. } = self;\n",
            "        get.a(); get.b(); old.c();\n",
            "        match &self.legacy {\n",
            "            Some(legacy) if legacy.on => { legacy.d(); }\n",
            "            None => other.e(),\n",
            "        }\n",
            "        if let Some(inner) = &self.legacy { inner.f(); }\n",
            "        other.g();\n",
            "    }\n",
            "}\n",
        );
        assert_eq!(
            foreign_measurements("rust.foreign_accesses", "rust-v1", "src/lib.rs", source),
            [("src/lib.rs::S.field".into(), 2, 10)]
        );
    }

    #[test]
    fn foreign_accesses_treat_bindings_from_a_dereferenced_self_match_as_owned() {
        let source = concat!(
            "pub enum E { A(Inner), B(Inner) }\n",
            "impl E {\n",
            "    fn field(&self, other: &O) {\n",
            "        match *self {\n",
            "            E::A(ref s) => { s.a(); s.b(); }\n",
            "            E::B(ref s) => { s.c(); other.d(); }\n",
            "        }\n",
            "        other.e();\n",
            "    }\n",
            "}\n",
        );
        assert_eq!(
            foreign_measurements("rust.foreign_accesses", "rust-v1", "src/lib.rs", source),
            [("src/lib.rs::E.field".into(), 2, 5)]
        );
    }

    #[test]
    fn foreign_accesses_ignore_python_statement_annotations() {
        let source = concat!(
            "class Config:\n",
            "    def from_mapping(self, mapping):\n",
            "        mappings: dict[str, t.Any] = {}\n",
            "        names: t.List[str]\n",
            "        table = {key: value.x, other: value.y,\n",
            "                 last: value.z}\n",
            "        mappings.update(mapping)\n",
            "        if mapping:\n",
            "            pass\n",
            "        else: other.run()\n",
        );
        assert_eq!(
            foreign_measurements("python.foreign_accesses", "python-v1", "sample.py", source),
            [("sample.py::Config.from_mapping".into(), 5, 5)]
        );
    }

    #[test]
    fn foreign_accesses_inherit_ownership_along_a_chain_of_names() {
        let source = concat!(
            "class ZodReadonly {\n",
            "  _parse(input: ParseInput) {\n",
            "    const result = this._def.innerType._parse(input);\n",
            "    make().value.other.run();\n",
            "    return data.value.x;\n",
            "  }\n",
            "}\n",
        );
        assert_eq!(
            foreign_measurements(
                "typescript.foreign_accesses",
                "typescript-v1",
                "sample.ts",
                source
            ),
            [("sample.ts::ZodReadonly._parse".into(), 2, 4)]
        );
    }

    #[test]
    fn foreign_accesses_treat_closure_parameters_given_only_owned_state_as_owned() {
        let source = concat!(
            "class HMRContext {\n",
            "  off(event: string, other: Other) {\n",
            "    const removeFromMap = (map: Map<string, any[]>) => {\n",
            "      const existing = map.get(event)\n",
            "      map.set(event, existing.filter(Boolean))\n",
            "    }\n",
            "    removeFromMap(this.client.listeners)\n",
            "    removeFromMap(this.newListeners)\n",
            "    other.removeFromMap(other.listeners)\n",
            "    const mixed = (m: Map<string, any[]>) => m.clear()\n",
            "    mixed(this.newListeners)\n",
            "    mixed(other.listeners)\n",
            "    function visit(node: Node) { node.a(); node.b() }\n",
            "    visit(other.root)\n",
            "    const unused = (x: X) => x.run()\n",
            "    const walk = (n: Node) => { n.a(); n.b() }\n",
            "    walk(this.root)\n",
            "    if (walk(other.root)) {}\n",
            "    const each = (n: Node) => { n.a(); n.b() }\n",
            "    each(this.root)\n",
            "    this.items.forEach(each)\n",
            "    const keep = (k: Node) => { k.a(); k.b() }\n",
            "    keep(this.root)\n",
            "    const my_keep = 1, $keep = 2\n",
            "  }\n",
            "}\n",
        );
        assert_eq!(
            foreign_measurements(
                "typescript.foreign_accesses",
                "typescript-v1",
                "sample.ts",
                source
            ),
            [("sample.ts::HMRContext.off".into(), 13, 25)]
        );
    }

    #[test]
    fn dependency_contract_treats_dunders_and_binding_constants_as_public() {
        let source = concat!(
            "o.__html__()\n",
            "httpx.__version__\n",
            "delete result.__proto__\n",
            "c.env.__STATIC_CONTENT\n",
            "obj.__mangled\n",
            "obj.__SECRET\n",
            "obj._token\n",
        );
        for (rule_id, rule_pack, path, expected, first_line) in [
            ("python.dependency_contract", "python-v1", "sample.py", 4, 4),
            (
                "typescript.dependency_contract",
                "typescript-v1",
                "sample.ts",
                2,
                5,
            ),
        ] {
            let observations = dependency_contract(
                &rule(rule_id),
                &model_with_code(path, source),
                &input(rule_pack, &[(path, source)]),
            )
            .unwrap();
            assert_eq!(
                observations[0].measurements["forbidden_accesses"], expected,
                "{rule_id}"
            );
            assert_eq!(observations[0].location.line, first_line, "{rule_id}");
        }
    }

    #[test]
    fn dependency_contract_treats_enclosing_class_members_as_owned() {
        let cases = [
            (
                "typescript.dependency_contract",
                "typescript-v1",
                "app.ts",
                concat!(
                    "export class App {\n",
                    "  _basePath = '/'\n",
                    "  #handler = 1\n",
                    "  #clone(): App {\n",
                    "    const clone = new App()\n",
                    "    clone.#handler = this.#handler\n",
                    "    clone._basePath = this._basePath\n",
                    "    clone._route = 1\n",
                    "    return clone\n",
                    "  }\n",
                    "}\n",
                    "other._basePath\n",
                ),
                (2, 8),
            ),
            (
                "python.dependency_contract",
                "python-v1",
                "key.py",
                concat!(
                    "class Key:\n",
                    "    def __init__(self, value):\n",
                    "        self._value = value\n",
                    "    def _hash(self):\n",
                    "        return 1\n",
                    "    def __eq__(self, other):\n",
                    "        return other._value == self._value and other._hash() and other._raw\n",
                    "key._value\n",
                ),
                (2, 7),
            ),
        ];
        for (rule_id, rule_pack, path, source, (expected, line)) in cases {
            let input = input(rule_pack, &[(path, source)]);
            let model = super::super::model::source_model(&rule(rule_id), &input).unwrap();
            let observations = dependency_contract(&rule(rule_id), &model, &input).unwrap();
            assert_eq!(
                observations[0].measurements["forbidden_accesses"], expected,
                "{rule_id}"
            );
            assert_eq!(observations[0].location.line, line, "{rule_id}");
        }
    }

    #[test]
    fn receiver_members_are_whole_names_on_self_or_this() {
        assert!(receiver_member("return self._key", "_key"));
        assert!(receiver_member("this . _key = 1", "_key"));
        assert!(receiver_member("x = other._key + self._key", "_key"));
        assert!(!receiver_member("return self._keys", "_key"));
        assert!(!receiver_member("return myself._key", "_key"));
        assert!(!receiver_member("return other._key", "_key"));
        assert!(!receiver_member("return self_key", "_key"));
        assert!(!receiver_member("_key = 1", "_key"));
    }

    #[test]
    fn enclosing_class_members_come_from_the_innermost_class() {
        let source = concat!(
            "class Outer:\n",
            "    _a = 1\n",
            "    class Inner:\n",
            "        def _m(self):\n",
            "            return 1\n",
            "    def run(self):\n",
            "        return self._a\n",
            "x = 1\n",
        );
        let model = super::super::model::source_model(
            &rule("python.dependency_contract"),
            &input("python-v1", &[("a.py", source)]),
        )
        .unwrap();
        let inner = source.find("return 1").unwrap();
        assert!(enclosing_class_member(&model, "a.py", inner, "_m"));
        assert!(!enclosing_class_member(&model, "a.py", inner, "_a"));
        assert!(!enclosing_class_member(&model, "a.py", inner, "_z"));
        let outer = model
            .classes
            .iter()
            .find(|class| class.name == "Outer")
            .unwrap()
            .range;
        assert!(enclosing_class_member(&model, "a.py", outer.1 - 1, "_a"));
        assert!(!enclosing_class_member(&model, "a.py", outer.1, "_a"));

        let source = "class Box {\n  _a: number = 1\n  run() { return 2 }\n}\n";
        let model = super::super::model::source_model(
            &rule("typescript.dependency_contract"),
            &input("typescript-v1", &[("a.ts", source)]),
        )
        .unwrap();
        let body = source.find("return").unwrap();
        assert!(enclosing_class_member(&model, "a.ts", body, "_a"));
        assert!(!enclosing_class_member(&model, "a.ts", body, "_z"));
    }

    #[test]
    fn primitive_slots_skip_typescript_interfaces() {
        let source = concat!(
            "interface Options { a: string; b: string; c: number; d: boolean; e: string }\n",
            "class Row { a: string; b: string; c: number; d: boolean; e: string }\n",
        );
        let model = super::super::model::source_model(
            &rule("typescript.primitive_slots"),
            &input("typescript-v1", &[("sample.ts", source)]),
        )
        .unwrap();
        let symbols = primitive_slots(&rule("typescript.primitive_slots"), &model)
            .into_iter()
            .map(|observation| observation.symbol)
            .collect::<Vec<_>>();
        assert_eq!(symbols, ["sample.ts::Row"]);
    }

    #[test]
    fn rust_module_dirs_follow_module_file_conventions() {
        assert_eq!(rust_module_dir("src/lib.rs"), "src/");
        assert_eq!(rust_module_dir("src/main.rs"), "src/");
        assert_eq!(rust_module_dir("src/core/mod.rs"), "src/core/");
        assert_eq!(rust_module_dir("src/core/scanner.rs"), "src/core/scanner/");
        assert_eq!(rust_module_dir("build.rs"), "build/");
        assert_eq!(rust_module_dir("lib.rs"), "");
        let shared = BTreeSet::from(["src/a.rs".to_string(), "src/b.rs".to_string()]);
        assert!(!rust_descendant_access(&shared, "src/a/inner.rs"));
        let single = BTreeSet::from(["src/a.rs".to_string()]);
        assert!(rust_descendant_access(&single, "src/a/inner.rs"));
        assert!(!rust_descendant_access(&single, "src/a.rs"));
        assert!(!rust_descendant_access(&single, "src/ab/inner.rs"));
    }
}
