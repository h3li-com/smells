use super::{
    common::{location_at, observation, primitive_type},
    model::{ClassFact, SourceModel},
    syntax::{code_only, import_statements},
};
use crate::{core::model::Input, evidence::Observation, names::dunder, policy::Rule};
use regex::Regex;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
    sync::OnceLock,
};
use syn::visit::Visit;

fn rule_language(rule: &Rule) -> &str {
    rule.id.split_once('.').map_or("", |(language, _)| language)
}

fn typescript_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| matches!(extension.to_str(), Some("ts" | "tsx" | "mts" | "cts")))
}

fn semantically_nominal_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    // `zip` names a postcode only as the whole name; `search_zip` is a compression flag.
    name == "zip"
        || [
            "id", "uuid", "email", "phone", "amount", "price", "currency", "date", "time",
            "address", "country", "postcode", "zipcode", "zip_code",
        ]
        .iter()
        .any(|role| name == *role || name.ends_with(&format!("_{role}")))
}

fn boolean_type(authored_type: &str) -> bool {
    matches!(authored_type.trim(), "bool" | "boolean")
}

/// A TypeScript `interface` describes a shape, not a class with nominal field slots.
fn typescript_interface(language: &str, class: &ClassFact) -> bool {
    language == "typescript"
        && class.interface
        && class.source.trim_start().starts_with("interface")
}

pub(super) fn nominal_slot_contract(rule: &Rule, model: &SourceModel) -> Vec<Observation> {
    let language = rule_language(rule);
    model
        .classes
        .iter()
        .filter(|class| !typescript_interface(language, class))
        .filter_map(|class| {
            let mismatches = class
                .fields
                .iter()
                .filter(|field| {
                    semantically_nominal_name(&field.name)
                        && primitive_type(language, &field.authored_type)
                        && !boolean_type(&field.authored_type)
                })
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>();
            (!mismatches.is_empty()).then(|| {
                observation(
                    class.symbol.clone(),
                    class.location.clone(),
                    [("mismatches", mismatches.len() as u64)],
                    json!({
                        "collector": "authored_semantic_slot_name_v1",
                        "primitive_semantic_slots": mismatches,
                        "scope": "well-known whole-term domain-role suffixes on non-boolean class fields; optional project contracts may strengthen this check",
                    }),
                )
            })
        })
        .collect()
}

fn base_leaf(base: &str) -> &str {
    base.rsplit([':', '.'])
        .find(|part| !part.is_empty())
        .unwrap_or(base)
}

fn logical_symbol(class: &ClassFact) -> String {
    let (source_path, authored_owner) = class
        .symbol
        .split_once("::")
        .unwrap_or(("", class.symbol.as_str()));
    let without_extension = Path::new(source_path).with_extension("");
    let mut modules = without_extension
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str().map(str::to_string),
            _ => None,
        })
        .collect::<Vec<_>>();
    if modules.first().is_some_and(|part| part == "src") {
        modules.remove(0);
    }
    if modules
        .last()
        .is_some_and(|part| matches!(part.as_str(), "lib" | "main" | "mod"))
    {
        modules.pop();
    }
    modules.extend(authored_owner.split("::").map(str::to_string));
    modules.join("::")
}

fn qualified_base_target(derived: &ClassFact, base: &str) -> Option<String> {
    let normalized = base.replace('.', "::");
    let mut parts = normalized
        .split("::")
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let mut owner = logical_symbol(derived)
        .split("::")
        .map(str::to_string)
        .collect::<Vec<_>>();
    owner.pop();
    match parts.first().copied() {
        Some("crate") => {
            parts.remove(0);
            owner.clear();
        }
        Some("self") => {
            parts.remove(0);
        }
        Some("super") => {
            while parts.first().copied() == Some("super") {
                parts.remove(0);
                owner.pop();
            }
        }
        _ => owner.clear(),
    }
    owner.extend(parts.into_iter().map(str::to_string));
    Some(owner.join("::"))
}

/// Resolve a declared base by name. A same-file declaration wins; otherwise every
/// same-named declaration elsewhere competes, so an unrelated namesake in another file
/// makes the base ambiguous instead of resolving to the only interface among them.
pub(super) fn resolve_declared_base<'a>(
    model: &'a SourceModel,
    derived: &ClassFact,
    base: &str,
    interfaces_only: bool,
) -> Option<&'a ClassFact> {
    let target = qualified_base_target(derived, base);
    let named = model
        .classes
        .iter()
        .filter(|class| class.name == base_leaf(base))
        .filter(|class| {
            target
                .as_ref()
                .is_none_or(|target| logical_symbol(class) == *target)
        })
        .collect::<Vec<_>>();
    let same_file = named
        .iter()
        .copied()
        .filter(|class| class.location.path == derived.location.path)
        .collect::<Vec<_>>();
    let scope = if same_file.is_empty() {
        named
    } else {
        same_file
    };
    let [resolved] = scope.as_slice() else {
        return None;
    };
    (!interfaces_only || resolved.interface).then_some(*resolved)
}

/// Text of a declaration header after its name with every bracketed group removed,
/// stopping at the body: `<K extends A = B> extends C<D> implements E {` yields
/// ` extends C implements E `.
fn depth_zero_header(text: &str) -> String {
    let mut header = String::new();
    let mut depth = 0_usize;
    let mut previous = ' ';
    for character in text.chars() {
        match character {
            '{' if depth == 0 => break,
            '<' | '(' | '[' | '{' => depth += 1,
            '>' if matches!(previous, '=' | '-') => {}
            '>' | ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => header.push(character),
            _ => {}
        }
        previous = character;
    }
    header
}

/// The header after `class|interface|trait <name>`, or `None` when the declaration is
/// not present in the source.
fn declaration_header(source: &str, name: &str) -> Option<String> {
    static DECLARATION: OnceLock<Regex> = OnceLock::new();
    let declaration = DECLARATION.get_or_init(|| {
        Regex::new(r"\b(?:class|interface|trait)\s+([A-Za-z_$][A-Za-z0-9_$]*)")
            .expect("valid declaration expression")
    });
    let start = declaration
        .captures_iter(source)
        .find(|captures| &captures[1] == name)?
        .get(0)?
        .end();
    Some(depth_zero_header(&source[start..]))
}

/// Names in a TypeScript `extends`/`implements` clause, ignoring type parameters
/// and their defaults.
fn typescript_heritage(class: &ClassFact) -> Option<BTreeSet<String>> {
    let header = declaration_header(&class.source, &class.name)?;
    Some(
        header
            .split(|character: char| character.is_whitespace() || character == ',')
            .filter(|word| !word.is_empty() && !matches!(*word, "extends" | "implements"))
            .map(|word| base_leaf(word).to_string())
            .collect(),
    )
}

/// The declared bases a contract may resolve. TypeScript bases must appear in the
/// real heritage clause, so a type-parameter constraint or default is never a base.
fn declared_bases(class: &ClassFact) -> Vec<&str> {
    let heritage = typescript_path(&class.location.path)
        .then(|| typescript_heritage(class))
        .flatten();
    class
        .bases
        .iter()
        .map(String::as_str)
        .filter(|base| {
            heritage
                .as_ref()
                .is_none_or(|heritage| heritage.contains(base_leaf(base)))
        })
        .collect()
}

/// Names a file imports from the standard library (`use std::task::{Context, Wake}`).
#[derive(Default)]
struct StandardImports(BTreeSet<String>);

impl StandardImports {
    fn collect(&mut self, tree: &syn::UseTree, standard: bool) {
        match tree {
            syn::UseTree::Path(path) => {
                let root =
                    standard || matches!(path.ident.to_string().as_str(), "std" | "core" | "alloc");
                self.collect(&path.tree, root);
            }
            syn::UseTree::Name(name) if standard => {
                self.0.insert(name.ident.to_string());
            }
            syn::UseTree::Rename(rename) if standard => {
                self.0.insert(rename.rename.to_string());
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.collect(item, standard);
                }
            }
            _ => {}
        }
    }
}

impl<'ast> Visit<'ast> for StandardImports {
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.collect(&item.tree, false);
    }
}

#[derive(Default)]
struct MacroImplementations(BTreeSet<(String, String, String)>, String, BTreeSet<String>);

impl MacroImplementations {
    /// An item macro hides generated methods; a bare trait name the file imports
    /// from the standard library is not the authored trait of the same name.
    fn unknown(&self, implementation: &syn::ItemImpl, trait_path: &syn::Path) -> bool {
        let standard = trait_path.segments.len() == 1
            && trait_path
                .segments
                .last()
                .is_some_and(|segment| self.2.contains(&segment.ident.to_string()));
        standard
            || implementation
                .items
                .iter()
                .any(|item| matches!(item, syn::ImplItem::Macro(_)))
    }
}

impl<'ast> Visit<'ast> for MacroImplementations {
    fn visit_item_impl(&mut self, implementation: &'ast syn::ItemImpl) {
        let trait_path = implementation.trait_.as_ref().map(|(_, path, _)| path);
        let syn::Type::Path(self_type) = implementation.self_ty.as_ref() else {
            return syn::visit::visit_item_impl(self, implementation);
        };
        let trait_name = trait_path
            .filter(|path| self.unknown(implementation, path))
            .and_then(|path| path.segments.last());
        if let (Some(trait_name), Some(type_name)) = (trait_name, self_type.path.segments.last()) {
            self.0.insert((
                self.1.clone(),
                type_name.ident.to_string(),
                trait_name.ident.to_string(),
            ));
        }
        syn::visit::visit_item_impl(self, implementation);
    }
}

/// Rust trait implementations whose bodies hold item-position macro invocations
/// (`forward_to_deserialize_any!`) or whose bare trait name is imported from the
/// standard library, keyed by path, implementing type, and trait. Generated methods
/// are invisible to source shape and std traits are not the authored port, so
/// omissions are unknown.
fn rust_macro_implementations(
    model: &SourceModel,
    input: &Input,
) -> Result<BTreeSet<(String, String, String)>, String> {
    let paths = model
        .classes
        .iter()
        .filter(|class| !class.interface && !class.bases.is_empty())
        .map(|class| class.location.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut visitor = MacroImplementations::default();
    for path in paths {
        let Some(source) = input.files.get(path) else {
            continue;
        };
        let file = syn::parse_file(source)
            .map_err(|error| crate::scan::parse_error_message(path, &error))?;
        let mut imports = StandardImports::default();
        imports.visit_file(&file);
        visitor.1 = path.to_string();
        visitor.2 = imports.0;
        visitor.visit_file(&file);
    }
    Ok(visitor.0)
}

fn python_protocol(port: &ClassFact) -> bool {
    port.source
        .split(':')
        .next()
        .is_some_and(|header| header.contains("Protocol"))
}

/// Methods a Python ABC declares with an `abstract*` decorator.
fn python_abstract_methods(port: &ClassFact) -> BTreeSet<&str> {
    static ABSTRACT: OnceLock<Regex> = OnceLock::new();
    let expression = ABSTRACT.get_or_init(|| {
        Regex::new(
            r"@(?:abc\.)?abstract[A-Za-z_]*[^\n]*\n(?:[ \t]*@[^\n]*\n)*[ \t]*(?:async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)",
        )
        .expect("valid abstract method expression")
    });
    expression
        .captures_iter(&port.source)
        .filter_map(|captures| captures.get(1))
        .map(|name| name.as_str())
        .collect()
}

/// Methods an implementation must supply: a Rust trait method with a default body is
/// provided (even when a cfg-gated twin declaration has none), and an ABC requires only its abstract methods (a Protocol requires all).
fn required_methods<'a>(language: &str, port: &'a ClassFact) -> BTreeSet<&'a str> {
    let abstract_only =
        (language == "python" && !python_protocol(port)).then(|| python_abstract_methods(port));
    let provided = port
        .methods
        .iter()
        .filter(|method| language == "rust" && method.body.is_some())
        .map(|method| method.name.as_str())
        .collect::<BTreeSet<_>>();
    port.methods
        .iter()
        .map(|method| method.name.as_str())
        .filter(|name| !provided.contains(name))
        .filter(|name| {
            abstract_only
                .as_ref()
                .is_none_or(|abstract_only| abstract_only.contains(name))
        })
        .collect()
}

fn port_observation(language: &str, implementation: &ClassFact, port: &ClassFact) -> Observation {
    let implemented = implementation
        .methods
        .iter()
        .map(|method| method.name.as_str())
        .collect::<BTreeSet<_>>();
    let required = required_methods(language, port);
    let missing = required
        .difference(&implemented)
        .copied()
        .collect::<Vec<_>>();
    let mut candidate = observation(
        implementation.symbol.clone(),
        implementation.location.clone(),
        [("failures", missing.len() as u64)],
        json!({
            "collector": "authored_port_shape_v1",
            "port": port.symbol,
            "missing_methods": missing,
            "scope": "declared interface/Protocol method names and ABC abstract methods; Rust trait defaults are provided; Rust impls holding item macros are unknown",
        }),
    );
    candidate.related_symbols.push(port.symbol.clone());
    candidate.related_locations.push(port.location.clone());
    candidate
}

pub(super) fn port_conformance(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    let language = rule_language(rule);
    let macro_implementations = if language == "rust" {
        rust_macro_implementations(model, input)?
    } else {
        BTreeSet::new()
    };
    let mut observations = Vec::new();
    for implementation in model.classes.iter().filter(|class| !class.interface) {
        for port in declared_bases(implementation)
            .into_iter()
            .filter_map(|base| resolve_declared_base(model, implementation, base, true))
        {
            let key = (
                implementation.location.path.clone(),
                implementation.name.clone(),
                port.name.clone(),
            );
            if !macro_implementations.contains(&key) {
                observations.push(port_observation(language, implementation, port));
            }
        }
    }
    Ok(observations)
}

fn rejection_stub(body: &str) -> bool {
    let compact = body
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim()
        .trim_end_matches(';')
        .trim();
    let compact = compact.split_whitespace().collect::<Vec<_>>().join(" ");
    compact == "pass"
        || compact == "return NotImplemented"
        || compact.starts_with("raise NotImplemented")
        || compact.starts_with("throw new Error")
}

pub(super) fn refused_bequest(model: &SourceModel) -> Vec<Observation> {
    let mut observations = Vec::new();
    for derived in &model.classes {
        for base in derived
            .bases
            .iter()
            .filter_map(|base| resolve_declared_base(model, derived, base, false))
        {
            let inherited = base
                .methods
                .iter()
                .filter(|method| !matches!(method.name.as_str(), "__init__" | "constructor"))
                .collect::<Vec<_>>();
            if inherited.is_empty() {
                continue;
            }
            let rejected = inherited
                .iter()
                .filter(|method| {
                    derived
                        .methods
                        .iter()
                        .find(|candidate| candidate.name == method.name)
                        .and_then(|candidate| candidate.body.as_deref())
                        .is_some_and(rejection_stub)
                })
                .map(|method| method.name.as_str())
                .collect::<Vec<_>>();
            let mut candidate = observation(
                derived.symbol.clone(),
                derived.location.clone(),
                [
                    ("inherited_members", inherited.len() as u64),
                    ("unused_members", rejected.len() as u64),
                ],
                json!({
                    "collector": "authored_inherited_member_rejection_v1",
                    "base": base.symbol,
                    "rejected_members": rejected,
                }),
            );
            candidate.related_symbols.push(base.symbol.clone());
            candidate.related_locations.push(base.location.clone());
            observations.push(candidate);
        }
    }
    observations
}

/// An upper-case letter starts a word after a lower-case letter or digit, and ends an
/// acronym when a lower-case letter follows (`HTTPTransport`).
fn word_boundary(previous: Option<&char>, character: char, next: Option<&char>) -> bool {
    character.is_ascii_uppercase()
        && previous.is_some_and(|previous| {
            previous.is_ascii_lowercase()
                || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next.is_some_and(char::is_ascii_lowercase))
        })
}

/// Lower-case words of an identifier: `TSEnumScope` is `ts enum scope`.
fn identifier_words(name: &str) -> Vec<String> {
    let characters = name.chars().collect::<Vec<_>>();
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for (index, character) in characters.iter().copied().enumerate() {
        if !character.is_ascii_alphanumeric() {
            words.extend((!current.is_empty()).then(|| std::mem::take(&mut current)));
            continue;
        }
        let previous = index.checked_sub(1).and_then(|index| characters.get(index));
        if word_boundary(previous, character, characters.get(index + 1)) && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.push(character.to_ascii_lowercase());
    }
    words.extend((!current.is_empty()).then_some(current));
    words
}

/// Words that mark a sync/async variant or an abstract base rather than a role.
fn without_markers(words: &[String], markers: &[&str]) -> Vec<String> {
    words
        .iter()
        .filter(|word| !markers.contains(&word.as_str()))
        .cloned()
        .collect()
}

const VARIANT_MARKERS: [&str; 2] = ["sync", "async"];
const BASE_MARKERS: [&str; 3] = ["base", "sync", "async"];

/// The child's role relative to its base: the words left after removing the base
/// name (without `Base`/`Sync`/`Async`) as a prefix or suffix.
fn hierarchy_role(child: &[String], base: &[String]) -> Option<Vec<String>> {
    if base.is_empty() {
        return None;
    }
    let role = child
        .strip_suffix(base)
        .or_else(|| child.strip_prefix(base))?;
    (!role.is_empty()).then(|| role.to_vec())
}

struct Member<'a> {
    class: &'a ClassFact,
    /// Role from the shared base-name affix.
    affix: Option<Vec<String>>,
    /// The whole child name without sync/async markers, for twin families.
    variant_free: Vec<String>,
}

struct Family<'a> {
    key: &'a str,
    name: &'a str,
    /// Authored base, or an external base whose whole name is the role affix; an
    /// external `BaseModel` stripped to `Model` is too generic to pair roles.
    trusted: bool,
    /// Base name words without `Base`/`Sync`/`Async`.
    stem: Vec<String>,
    asynchronous: bool,
    members: Vec<Member<'a>>,
}

impl Family<'_> {
    fn has(&self, class: &ClassFact) -> bool {
        self.members
            .iter()
            .any(|member| member.class.symbol == class.symbol)
    }

    /// `SyncByteStream`/`AsyncByteStream` or `BaseTransport`/`AsyncBaseTransport`.
    fn twin_of(&self, other: &Self) -> bool {
        !self.stem.is_empty() && self.stem == other.stem && self.asynchronous != other.asynchronous
    }

    /// Twin variants pair under any bases; affix roles pair only between two
    /// differently named trusted bases, so copies of one hierarchy do not count.
    fn comparable(&self, other: &Self, twin: bool) -> bool {
        twin || (self.trusted && other.trusted && self.name != other.name)
    }

    /// Roles of children not shared with `other`; a class inheriting both bases is
    /// one implementation, not a parallel addition.
    fn roles<'b>(&'b self, other: &Self, twin: bool) -> BTreeMap<&'b [String], &'b ClassFact> {
        let mut roles = BTreeMap::new();
        for member in self
            .members
            .iter()
            .filter(|member| !other.has(member.class))
        {
            let role = if twin {
                Some(member.variant_free.as_slice())
            } else {
                member.affix.as_deref()
            };
            if let Some(role) = role.filter(|role| !role.is_empty()) {
                roles.entry(role).or_insert(member.class);
            }
        }
        roles
    }
}

fn family_base<'a>(
    model: &'a SourceModel,
    child: &ClassFact,
    base: &'a str,
) -> Option<(&'a str, &'a str, bool)> {
    if let Some(resolved) = resolve_declared_base(model, child, base, false) {
        return Some((resolved.symbol.as_str(), resolved.name.as_str(), true));
    }
    let leaf = base_leaf(base);
    (!model.classes.iter().any(|class| class.name == leaf)).then_some((base, leaf, false))
}

/// TypeScript interface declarations describe shapes, not subclass hierarchies.
fn hierarchy_member(class: &ClassFact) -> bool {
    !typescript_interface(
        if typescript_path(&class.location.path) {
            "typescript"
        } else {
            ""
        },
        class,
    )
}

fn inheritance_families(model: &SourceModel) -> Vec<Family<'_>> {
    let mut by_base: BTreeMap<&str, Family<'_>> = BTreeMap::new();
    for child in model.classes.iter().filter(|class| hierarchy_member(class)) {
        let child_words = without_markers(&identifier_words(&child.name), &VARIANT_MARKERS);
        for base in declared_bases(child) {
            let Some((key, name, authored)) = family_base(model, child, base) else {
                continue;
            };
            let base_words = identifier_words(name);
            let stem = without_markers(&base_words, &BASE_MARKERS);
            let family = by_base.entry(key).or_insert_with(|| Family {
                key,
                name,
                trusted: authored || stem == base_words,
                stem,
                asynchronous: base_words.iter().any(|word| word == "async"),
                members: Vec::new(),
            });
            family.members.push(Member {
                class: child,
                affix: hierarchy_role(&child_words, &family.stem),
                variant_free: child_words.clone(),
            });
        }
    }
    by_base.into_values().collect()
}

fn word_prefix(shorter: &[String], longer: &[String]) -> bool {
    longer.starts_with(shorter)
}

/// One-to-one role pairs: exact role matches first, then roles where one role's
/// words begin the other's (`Catch` with `CatchClause`).
fn paired_roles<'a>(
    first: &BTreeMap<&'a [String], &'a ClassFact>,
    second: &BTreeMap<&'a [String], &'a ClassFact>,
) -> Vec<(String, &'a ClassFact, &'a ClassFact)> {
    let mut used = BTreeSet::new();
    let mut unmatched = Vec::new();
    let mut pairs = Vec::new();
    for (role, class) in first {
        if let Some(partner) = second.get(role) {
            used.insert(*role);
            pairs.push((role.join("_"), *class, *partner));
        } else {
            unmatched.push((*role, *class));
        }
    }
    for (role, class) in unmatched {
        let partner = second.iter().find(|(candidate, _)| {
            !used.contains(*candidate)
                && (word_prefix(role, candidate) || word_prefix(candidate, role))
        });
        if let Some((candidate, partner)) = partner {
            used.insert(*candidate);
            pairs.push((role.join("_"), class, *partner));
        }
    }
    pairs
}

fn parallel_observation(
    first: &Family<'_>,
    second: &Family<'_>,
    twin: bool,
    pairs: &[(String, &ClassFact, &ClassFact)],
) -> Option<Observation> {
    let (_, anchor, _) = pairs.first()?;
    let mut candidate = observation(
        anchor.symbol.clone(),
        anchor.location.clone(),
        [("parallel_pairs", pairs.len() as u64)],
        json!({
            "collector": "authored_parallel_hierarchy_roles_v2",
            "base_pair": [first.key, second.key],
            "pairing": if twin { "sync_async_variants" } else { "shared_role_affixes" },
            "shared_roles": pairs.iter().map(|(role, _, _)| role).collect::<Vec<_>>(),
        }),
    );
    for (_, _, class) in pairs {
        candidate.related_symbols.push(class.symbol.clone());
        candidate.related_locations.push(class.location.clone());
    }
    Some(candidate)
}

pub(super) fn parallel_inheritance(model: &SourceModel) -> Vec<Observation> {
    let families = inheritance_families(model);
    let mut observations = Vec::new();
    let mut rest = families.as_slice();
    while let [first, tail @ ..] = rest {
        rest = tail;
        for second in tail {
            let twin = first.twin_of(second);
            if !first.comparable(second, twin) {
                continue;
            }
            let pairs = paired_roles(&first.roles(second, twin), &second.roles(first, twin));
            observations.extend(parallel_observation(first, second, twin, &pairs));
        }
    }
    observations
}

/// A Rust trait whose supertraits resolve (by qualified path, else by unique name) to an
/// authored, non-empty trait extends local code (`HandlerWithoutStateExt: Handler`), not a library type.
fn extends_local_trait(model: &SourceModel, extension: &ClassFact) -> bool {
    let Some(header) = declaration_header(&extension.source, &extension.name) else {
        return false;
    };
    let bounds = header.split(" where ").next().unwrap_or_default();
    let Some((_, bounds)) = bounds.split_once(':') else {
        return false;
    };
    bounds
        .split('+')
        .map(|bound| bound.trim().trim_start_matches('?').replace(' ', ""))
        .filter_map(|bound| resolve_declared_base(model, extension, &bound, true))
        .any(|base| base.symbol != extension.symbol && !base.methods.is_empty())
}

fn rust_library_capabilities(model: &SourceModel) -> Vec<Observation> {
    model
        .classes
        .iter()
        .filter(|class| {
            class.interface
                && !class.methods.is_empty()
                && (class.name.ends_with("Ext") || class.name.ends_with("Extension"))
                && !extends_local_trait(model, class)
        })
        .map(|class| {
            observation(
                class.symbol.clone(),
                class.location.clone(),
                [("failures", 1)],
                json!({
                    "collector": "rust_extension_trait_workaround_v1",
                    "scope": "authored non-empty traits named *Ext or *Extension that do not extend an authored trait",
                }),
            )
        })
        .collect()
}

/// Whether a module specifier names a file of the project itself, as `./config` or
/// `../util` do, rather than a library.
fn relative_specifier(specifier: &str) -> bool {
    specifier.starts_with('.')
}

/// Names bound by TypeScript imports of libraries: defaults, named (after `as`), and
/// namespaces. A relative import binds the project's own code, not a library.
fn typescript_imported_names(
    path: &str,
    source: &str,
    code: &str,
) -> Result<BTreeSet<String>, String> {
    static REQUIRE: OnceLock<Regex> = OnceLock::new();
    let require = REQUIRE.get_or_init(|| {
        Regex::new(
            r#"\b(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*require\s*\(\s*(?:['"`]([^'"`]*))?"#,
        )
        .expect("valid require expression")
    });
    let mut names = require
        .captures_iter(code)
        .filter(|captures| {
            !captures
                .get(2)
                .is_some_and(|module| relative_specifier(module.as_str()))
        })
        .map(|captures| captures[1].to_string())
        .collect::<BTreeSet<_>>();
    for statement in import_statements(path, source)? {
        let Some(clause) = statement.trim().strip_prefix("import") else {
            continue;
        };
        let specifier = statement.rsplit(['\'', '"']).nth(1).unwrap_or_default();
        if relative_specifier(specifier) {
            continue;
        }
        let clause = clause.split([';', '=']).next().unwrap_or_default();
        let clause = clause
            .rsplit_once(" from")
            .map_or(clause, |(clause, _)| clause);
        names.extend(
            clause
                .split([',', '{', '}'])
                .filter_map(|binding| binding.rsplit(" as ").next())
                .map(|binding| binding.trim().trim_start_matches("type ").trim())
                .filter(|binding| {
                    !binding.is_empty()
                        && binding.chars().all(|character| {
                            character.is_alphanumeric() || "_$".contains(character)
                        })
                })
                .map(str::to_string),
        );
    }
    Ok(names)
}

/// Whether a workaround match counts. `setattr(self, ...)` mutates the receiver
/// itself; a TypeScript `a.b.c =` counts only when `a` is imported or a host global,
/// and a `process.env` write sets an environment variable rather than patching a library.
fn library_target(matched: &str, imported: Option<&BTreeSet<String>>) -> bool {
    const HOST_GLOBALS: [&str; 5] = ["window", "globalThis", "global", "process", "navigator"];
    if let Some(arguments) = matched.strip_prefix("setattr") {
        let receiver = arguments.trim_start().trim_start_matches('(').trim_start();
        return !["self", "cls"].iter().any(|own| {
            receiver
                .strip_prefix(own)
                .is_some_and(|rest| rest.trim_start().starts_with(','))
        });
    }
    if matched.contains(".prototype.") {
        return true;
    }
    let mut names = matched.split('.');
    let receiver = names.next().unwrap_or_default();
    if receiver == "process" && names.next().is_some_and(|name| name.trim_end() == "env") {
        return false;
    }
    imported.is_none_or(|imported| imported.contains(receiver) || HOST_GLOBALS.contains(&receiver))
}

/// Prototype assignments, `setattr` calls and assignments to a dotted target of at
/// least three names, since `self.x =` is ordinary state.
fn workaround_expression() -> &'static Regex {
    static WORKAROUND: OnceLock<Regex> = OnceLock::new();
    WORKAROUND.get_or_init(|| {
        Regex::new(
            r"(?m)(?:\b[A-Za-z_$][A-Za-z0-9_$]*\.prototype\.[A-Za-z_$][A-Za-z0-9_$]*\s*=(?:[^=]|$)|\bsetattr\s*\(\s*[A-Za-z_][A-Za-z0-9_.]*\s*,|\b[A-Za-z_$][A-Za-z0-9_$]*(?:\.[A-Za-z_$][A-Za-z0-9_$]*){2,}\s*=(?:[^=]|$))",
        )
        .expect("valid library workaround expression")
    })
}

/// Whether a dotted assignment begins its expression: not the tail of a longer chain
/// such as `dom.window.document.body.x =`, and not a type such as `let x: a.b.C =`,
/// `| a.b.C =` or `typeof a.b.c =`.
fn chain_root(code: &str, failure: &str, start: usize) -> bool {
    if failure.starts_with("setattr") || failure.contains(".prototype.") {
        return true;
    }
    let line = code[..start].rsplit('\n').next().unwrap_or_default();
    let before = line.trim_end();
    let type_query = before
        .strip_suffix("typeof")
        .is_some_and(|rest| !rest.ends_with(|c: char| c.is_alphanumeric() || c == '_'));
    !before.ends_with(['.', ':', '|']) && !type_query
}

/// Attributes that authored Python classes declare as assignable: annotated class
/// fields and `@name.setter` properties. Assigning one is ordinary use of an
/// authored API, not a workaround for a library class that lacks it.
fn authored_settable_attributes(model: &SourceModel) -> BTreeSet<String> {
    static SETTER: OnceLock<Regex> = OnceLock::new();
    let setter = SETTER.get_or_init(|| {
        Regex::new(r"@\s*([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*setter\b")
            .expect("valid property setter expression")
    });
    let mut attributes = BTreeSet::new();
    for class in &model.classes {
        attributes.extend(class.fields.iter().map(|field| field.name.clone()));
        attributes.extend(
            setter
                .captures_iter(&class.source)
                .map(|captures| captures[1].to_string()),
        );
    }
    attributes
}

/// Names of properties a Python class defines with `@property` or `@cached_property`.
fn python_properties(source: &str) -> BTreeSet<&str> {
    static PROPERTY: OnceLock<Regex> = OnceLock::new();
    let property = PROPERTY.get_or_init(|| {
        Regex::new(
            r"@\s*(?:functools\s*\.\s*)?(?:cached_)?property\b[^\n]*\n(?:\s*@[^\n]*\n)*\s*(?:async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)",
        )
        .expect("valid property decorator expression")
    });
    property
        .captures_iter(source)
        .filter_map(|captures| captures.get(1).map(|name| name.as_str()))
        .collect()
}

/// Authored Python class names and, per class, the methods it declares that are
/// not properties. A property returns a value that may be a library object.
struct AuthoredClasses<'model> {
    methods: BTreeMap<&'model str, BTreeSet<&'model str>>,
}

impl<'model> AuthoredClasses<'model> {
    fn new(model: &'model SourceModel) -> Self {
        let mut methods = BTreeMap::<&str, BTreeSet<&str>>::new();
        for class in &model.classes {
            let properties = python_properties(&class.source);
            methods.entry(class.name.as_str()).or_default().extend(
                class
                    .methods
                    .iter()
                    .map(|method| method.name.as_str())
                    .filter(|name| !properties.contains(name)),
            );
        }
        Self { methods }
    }

    fn class(&self, name: &str) -> bool {
        self.methods.contains_key(name)
    }

    /// Whether any authored class declares `member` as a method; a receiver
    /// does not say which class it belongs to.
    fn any_method(&self, member: &str) -> bool {
        self.methods
            .values()
            .any(|methods| methods.contains(member))
    }

    /// Whether a Python workaround mutates an authored class object or one of its
    /// methods rather than a library object: `setattr(codes, ...)` on an authored
    /// class, `Outer.Inner.__doc__ = ...`, or `cls.__rich_repr__.angular = ...`.
    /// A field reached through the receiver, as in `self.session.headers = ...`,
    /// may hold a library object and still counts, and so does an object reached
    /// through an imported name that an authored class elsewhere happens to share.
    fn target(&self, matched: &str, imported: &BTreeSet<String>) -> bool {
        let names = mutated_object(matched)
            .split('.')
            .map(str::trim)
            .collect::<Vec<_>>();
        if names.first().is_some_and(|root| imported.contains(*root)) {
            return false;
        }
        match names.as_slice() {
            [class] => *class == "cls" || self.class(class),
            [root, member] => self.authored_member(root, member),
            _ => false,
        }
    }

    /// Whether `root.member` names a dunder or method of an authored class, reached
    /// through `self`, `cls` or the class name, or a class nested in an authored class.
    fn authored_member(&self, root: &str, member: &str) -> bool {
        if matches!(root, "self" | "cls") {
            return dunder(member) || self.any_method(member);
        }
        self.methods
            .get(root)
            .is_some_and(|methods| dunder(member) || methods.contains(member) || self.class(member))
    }
}

/// The object a Python workaround mutates: the first `setattr` argument, or the
/// assignment target without its final attribute.
fn mutated_object(matched: &str) -> &str {
    matched.strip_prefix("setattr").map_or_else(
        || {
            let target = matched.split('=').next().unwrap_or_default().trim_end();
            target.rsplit_once('.').map_or("", |(object, _)| object)
        },
        |arguments| {
            let arguments = arguments.trim_start().trim_start_matches('(');
            arguments.split(',').next().unwrap_or_default()
        },
    )
}

/// Whether a dotted Python assignment `a.b.name =` sets an attribute an authored
/// class declares as assignable. A chain that starts at an imported name, such as
/// `requests.Session.headers = ...`, reaches a library object and still counts.
fn authored_attribute_assignment(
    matched: &str,
    authored: &BTreeSet<String>,
    imported: &BTreeSet<String>,
) -> bool {
    if matched.starts_with("setattr") || matched.contains(".prototype.") {
        return false;
    }
    let target = matched.split('=').next().unwrap_or_default().trim_end();
    let root = target.split('.').next().unwrap_or_default().trim();
    !imported.contains(root)
        && target
            .rsplit('.')
            .next()
            .is_some_and(|attribute| authored.contains(attribute.trim()))
}

/// The names a Python file binds by importing a library: `import a.b` binds `a`,
/// `import a as b` and `from m import a as b` bind `b`. A relative import, or one
/// whose top-level module is in `authored`, binds the project's own code.
fn python_imported_names(
    path: &str,
    source: &str,
    authored: &BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
    let library = |module: &str| {
        let root = module.trim().split('.').next().unwrap_or_default();
        !root.is_empty() && !authored.contains(root)
    };
    let mut names = BTreeSet::new();
    for statement in import_statements(path, source)? {
        let statement = statement.split('#').next().unwrap_or_default();
        match statement.trim().strip_prefix("from ") {
            Some(rest) => {
                let (module, bound) = rest.split_once(" import ").unwrap_or_default();
                if library(module) {
                    names.extend(bound.split(',').map(python_bound_name).map(str::to_string));
                }
            }
            None => names.extend(
                statement
                    .trim()
                    .strip_prefix("import ")
                    .unwrap_or_default()
                    .split(',')
                    .filter(|item| {
                        library(
                            item.trim_matches(|c: char| c.is_whitespace() || "()\\".contains(c)),
                        )
                    })
                    .map(python_bound_name)
                    .map(str::to_string),
            ),
        }
    }
    Ok(names)
}

/// The top-level modules the scanned Python files belong to: the outermost
/// package directory (one holding `__init__.py`) above a file, or the file itself
/// when its directory is not a package.
fn python_authored_modules(files: &BTreeMap<String, String>) -> BTreeSet<String> {
    let package =
        |directory: &Path| files.contains_key(&format!("{}/__init__.py", directory.display()));
    files
        .keys()
        .map(Path::new)
        .filter(|path| path.extension().is_some_and(|extension| extension == "py"))
        .filter_map(|path| {
            let top = path
                .ancestors()
                .skip(1)
                .take_while(|directory| !directory.as_os_str().is_empty() && package(directory))
                .last();
            match top {
                Some(directory) => directory.file_name(),
                None => path.file_stem(),
            }
            .and_then(|name| name.to_str())
            .map(str::to_string)
        })
        .collect()
}

/// The name one imported item binds: its alias, or else its first dotted part
/// (the item itself after `from`, which never has dots).
fn python_bound_name(item: &str) -> &str {
    let item = item.trim_matches(|c: char| c.is_whitespace() || "()\\".contains(c));
    match item.split_once(" as ") {
        Some((_, alias)) => alias.trim(),
        None => item.split('.').next().unwrap_or_default(),
    }
}

pub(super) fn library_capabilities(
    rule: &Rule,
    model: &SourceModel,
    input: &Input,
) -> Result<Vec<Observation>, String> {
    if rule.id.starts_with("rust.") {
        return Ok(rust_library_capabilities(model));
    }
    let typescript = rule_language(rule) == "typescript";
    let authored = if typescript {
        BTreeSet::new()
    } else {
        authored_settable_attributes(model)
    };
    let classes = AuthoredClasses::new(model);
    let authored_modules = python_authored_modules(&input.files);
    let workaround = workaround_expression();
    let mut observations = Vec::new();
    for (path, source) in &input.files {
        let code = code_only(path, source)?;
        let mut failures = workaround
            .find_iter(&code)
            .filter(|failure| chain_root(&code, failure.as_str(), failure.start()))
            .peekable();
        if failures.peek().is_none() {
            continue;
        }
        let imported = typescript
            .then(|| typescript_imported_names(path, source, &code))
            .transpose()?;
        let python_imported = if typescript {
            BTreeSet::new()
        } else {
            python_imported_names(path, source, &authored_modules)?
        };
        let failures = failures
            .filter(|failure| library_target(failure.as_str(), imported.as_ref()))
            .filter(|failure| {
                !authored_attribute_assignment(failure.as_str(), &authored, &python_imported)
            })
            .filter(|failure| typescript || !classes.target(failure.as_str(), &python_imported))
            .collect::<Vec<_>>();
        if let Some(first) = failures.first() {
            observations.push(observation(
                format!("{path}::library-extension-workarounds"),
                location_at(path, source, first.start()),
                [("failures", failures.len() as u64)],
                json!({
                    "collector": "authored_library_extension_workaround_v1",
                    "scope": "prototype or imported/foreign attribute mutation and setattr on a non-receiver; Python mutations of an authored class object or its methods are excluded; optional capability tests may replace this evidence",
                }),
            ));
        }
    }
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        collectors::model::{CodeScope, FieldFact, FunctionFact},
        core::model::Input,
        report::Location,
    };
    use std::{collections::BTreeMap, sync::Arc};

    fn location(path: &str) -> Location {
        Location {
            path: path.into(),
            line: 1,
            column: 1,
        }
    }

    fn function(name: &str, body: Option<&str>) -> FunctionFact {
        FunctionFact {
            name: name.into(),
            symbol: format!("domain::{name}"),
            location: location("src/domain.rs"),
            body_line: 1,
            end_line: 1,
            type_parameters: Vec::new(),
            body: body.map(Arc::from),
            complexity: body.map(|_| 1),
            source: Arc::from(body.unwrap_or_default()),
            private: false,
            receiver: false,
            entry_point: false,
            scope: CodeScope::Production,
        }
    }

    fn class(path: &str, owner: &str, name: &str) -> ClassFact {
        ClassFact {
            name: name.into(),
            symbol: format!("{path}::{owner}"),
            location: location(path),
            type_parameters: Vec::new(),
            source: Arc::from(""),
            bases: Vec::new(),
            interface: false,
            private: false,
            fields: Vec::new(),
            methods: Vec::new(),
            range: (0, 0),
            scope: CodeScope::Production,
        }
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

    fn input(files: BTreeMap<String, String>) -> Input {
        Input::fixture("rust-v1", files)
    }

    #[test]
    fn nominal_slot_names_require_an_exact_domain_suffix_and_a_primitive_type() {
        for accepted in ["id", "account_id", "billing_currency", "postal_address"] {
            assert!(semantically_nominal_name(accepted));
        }
        for rejected in ["identity", "candidate", "currency_code", "addressable"] {
            assert!(!semantically_nominal_name(rejected));
        }

        let mut account = class("src/domain.rs", "Account", "Account");
        account.fields = vec![
            FieldFact {
                name: "account_id".into(),
                authored_type: "String".into(),
                temporary: false,
                private: false,
            },
            FieldFact {
                name: "country".into(),
                authored_type: "Country".into(),
                temporary: false,
                private: false,
            },
            FieldFact {
                name: "identity".into(),
                authored_type: "String".into(),
                temporary: false,
                private: false,
            },
        ];
        let observations = nominal_slot_contract(
            &rule("rust.nominal_slot_contract"),
            &SourceModel {
                classes: vec![account],
                ..SourceModel::default()
            },
        );
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["mismatches"], 1);
        assert_eq!(
            observations[0].evidence["primitive_semantic_slots"][0],
            "account_id"
        );
    }

    #[test]
    fn logical_symbols_resolve_crate_self_super_and_external_bases() {
        let derived = class("src/domain/orders/handler.rs", "nested::Handler", "Handler");
        assert_eq!(
            logical_symbol(&derived),
            "domain::orders::handler::nested::Handler"
        );
        assert_eq!(
            qualified_base_target(&derived, "crate::ports::Reader").as_deref(),
            Some("ports::Reader")
        );
        assert_eq!(
            qualified_base_target(&derived, "self::Reader").as_deref(),
            Some("domain::orders::handler::nested::Reader")
        );
        assert_eq!(
            qualified_base_target(&derived, "super::super::Reader").as_deref(),
            Some("domain::orders::Reader")
        );
        assert_eq!(
            qualified_base_target(&derived, "external::Reader").as_deref(),
            Some("external::Reader")
        );
        assert_eq!(qualified_base_target(&derived, "Reader"), None);
    }

    #[test]
    fn rejection_stubs_are_narrow_and_drive_refused_bequest_counts() {
        for accepted in [
            "pass",
            "return NotImplemented",
            "raise NotImplementedError()",
            "{ throw new Error('unsupported'); }",
        ] {
            assert!(rejection_stub(accepted));
        }
        for rejected in [
            "return value",
            "raise ValueError()",
            "throw error",
            "pass; audit()",
        ] {
            assert!(!rejection_stub(rejected));
        }

        let mut base = class("src/base.py", "Port", "Port");
        base.methods = vec![
            function("__init__", Some("pass")),
            function("load", None),
            function("save", None),
        ];
        let mut derived = class("src/adapter.py", "Adapter", "Adapter");
        derived.bases = vec!["Port".into()];
        derived.methods = vec![
            function("load", Some("raise NotImplementedError()")),
            function("save", Some("return value")),
        ];
        let observations = refused_bequest(&SourceModel {
            classes: vec![base, derived],
            ..SourceModel::default()
        });
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["inherited_members"], 2);
        assert_eq!(observations[0].measurements["unused_members"], 1);
        assert_eq!(observations[0].evidence["rejected_members"][0], "load");
    }

    #[test]
    fn parallel_hierarchies_report_each_shared_role_once() {
        let mut cat = class("src/animals.rs", "CatAnimal", "CatAnimal");
        cat.bases = vec!["Animal".into()];
        let mut dog = class("src/animals.rs", "DogAnimal", "DogAnimal");
        dog.bases = vec!["Animal".into()];
        let mut cat_view = class("src/views.rs", "CatView", "CatView");
        cat_view.bases = vec!["View".into()];
        let mut dog_view = class("src/views.rs", "DogView", "DogView");
        dog_view.bases = vec!["View".into()];
        let observations = parallel_inheritance(&SourceModel {
            classes: vec![cat, dog, cat_view, dog_view],
            ..SourceModel::default()
        });
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["parallel_pairs"], 2);
        assert_eq!(observations[0].related_symbols.len(), 2);
    }

    #[test]
    fn rust_library_capabilities_require_a_nonempty_extension_trait() {
        let mut extension = class("src/ext.rs", "ReadExt", "ReadExt");
        extension.interface = true;
        extension.methods = vec![function("read_more", None)];
        let mut empty = class("src/empty.rs", "EmptyExt", "EmptyExt");
        empty.interface = true;
        let mut ordinary = class("src/port.rs", "ReadPort", "ReadPort");
        ordinary.interface = true;
        ordinary.methods = vec![function("read", None)];

        let observations = library_capabilities(
            &rule("rust.library_capabilities"),
            &SourceModel {
                classes: vec![extension, empty, ordinary],
                ..SourceModel::default()
            },
            &input(BTreeMap::new()),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].symbol, "src/ext.rs::ReadExt");
    }

    #[test]
    fn library_workarounds_count_assignments_but_not_comparisons() {
        let source = concat!(
            "if (this.cart.lines.length === 0) { stop(); }\n",
            "if (order.cart.total == 0 || a.b.c !== d || a.b.c <= d) { stop(); }\n",
            "Array.prototype.last = function () { return this[this.length - 1]; };\n",
            "if (Array.prototype.first === undefined) { stop(); }\n",
            "window.app.config = {};\n",
            "vendor.client.retry =\n  3;\n",
            "config.languageOptions.parser = parser;\n",
            "dom.window.document.body.innerHTML = html;\n",
            "let realpath: typeof vendor.realpathSync.native = native;\n",
            "let check: vendor.core.Check = check;\n",
            "document.documentElement.dataset.state = 'ready';\n",
            "process.env.NODE_ENV = 'production';\n",
            "process.stdout.write = write;\n",
            "process.stdout.columns = 80;\n",
            "import * as vendor from 'vendor';\n",
        );
        let observations = library_capabilities(
            &rule("typescript.library_capabilities"),
            &SourceModel::default(),
            &input(BTreeMap::from([(
                "src/patch.ts".to_string(),
                source.to_string(),
            )])),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["failures"], 5);
        assert_eq!(observations[0].location.line, 3);
    }

    #[test]
    fn rust_port_conformance_does_not_require_trait_default_methods() {
        let mut port = class("src/port.rs", "Repository", "Repository");
        port.interface = true;
        port.methods = vec![
            function("get", None),
            function("save", None),
            function("update", Some("{ self.save() }")),
        ];
        let mut table = class("src/table.rs", "Table", "Table");
        table.bases = vec!["Repository".into()];
        table.methods = vec![function("get", Some("{}"))];

        let model = SourceModel {
            classes: vec![port, table],
            ..SourceModel::default()
        };

        let observations = port_conformance(
            &rule("rust.port_conformance"),
            &model,
            &input(BTreeMap::new()),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["failures"], 1);
        assert_eq!(observations[0].evidence["missing_methods"], json!(["save"]));

        let observations = port_conformance(
            &rule("typescript.port_conformance"),
            &model,
            &input(BTreeMap::new()),
        )
        .unwrap();
        assert_eq!(observations[0].measurements["failures"], 2);
    }
    #[test]
    fn nominal_slots_ignore_boolean_flags_partial_terms_and_typescript_interfaces() {
        assert!(semantically_nominal_name("zip"));
        assert!(semantically_nominal_name("billing_zip_code"));
        assert!(!semantically_nominal_name("search_zip"));
        let field = |name: &str, authored_type: &str| FieldFact {
            name: name.into(),
            authored_type: authored_type.into(),
            temporary: false,
            private: false,
        };
        let mut args = class("src/args.rs", "Args", "Args");
        args.fields = vec![field("search_zip", "bool"), field("is_id", "bool")];
        let rust = SourceModel {
            classes: vec![args],
            ..SourceModel::default()
        };
        assert!(nominal_slot_contract(&rule("rust.nominal_slot_contract"), &rust).is_empty());

        let mut shape = class("src/types.ts", "Message", "Message");
        shape.interface = true;
        shape.source = Arc::from("interface Message { id: string }");
        shape.fields = vec![field("id", "string")];
        let mut entity = class("src/entity.ts", "Account", "Account");
        entity.source = Arc::from("class Account { id: string }");
        entity.fields = vec![field("id", "string")];
        let typescript = SourceModel {
            classes: vec![shape, entity],
            ..SourceModel::default()
        };
        let observations =
            nominal_slot_contract(&rule("typescript.nominal_slot_contract"), &typescript);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].symbol, "src/entity.ts::Account");
    }

    #[test]
    fn rust_port_conformance_treats_item_macro_impls_as_unknown() {
        let mut port = class("src/io.rs", "Read", "Read");
        port.interface = true;
        port.methods = vec![function("poll_read", None)];
        let mut boxed = class("src/io.rs", "Box", "Box");
        boxed.bases = vec!["Read".into()];
        let mut file = class("src/file.rs", "File", "File");
        file.bases = vec!["Read".into()];
        let model = SourceModel {
            classes: vec![port, boxed, file],
            ..SourceModel::default()
        };
        let files = BTreeMap::from([
            (
                "src/io.rs".to_string(),
                "trait Read { fn poll_read(&self); }\nimpl<T: Read> Read for Box<T> { forward!(); }\n"
                    .to_string(),
            ),
            (
                "src/file.rs".to_string(),
                "struct File;\nimpl crate::io::Read for File {}\n".to_string(),
            ),
        ]);
        let observations =
            port_conformance(&rule("rust.port_conformance"), &model, &input(files)).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].symbol, "src/file.rs::File");
        assert_eq!(observations[0].measurements["failures"], 1);
    }

    #[test]
    fn rust_macro_implementations_read_only_files_with_trait_implementations() {
        let mut supertrait = class("src/port.rs", "Port", "Port");
        supertrait.interface = true;
        supertrait.bases = vec!["Base".into()];
        let plain = class("src/plain.rs", "Plain", "Plain");
        let mut implementation = class("src/imp.rs", "Imp", "Imp");
        implementation.bases = vec!["Port".into()];
        let model = SourceModel {
            classes: vec![supertrait, plain, implementation],
            ..SourceModel::default()
        };
        let body = "impl Port for Imp { forward!(); }\n".to_string();
        let files = ["src/port.rs", "src/plain.rs", "src/imp.rs"]
            .map(|path| (path.to_string(), body.clone()))
            .into();
        let found = rust_macro_implementations(&model, &input(files)).unwrap();
        let paths: Vec<&str> = found.iter().map(|(path, _, _)| path.as_str()).collect();
        assert_eq!(paths, ["src/imp.rs"]);
    }

    #[test]
    fn depth_zero_header_ignores_arrows_inside_brackets() {
        assert_eq!(
            depth_zero_header("<T extends () => X> implements E {"),
            " implements E "
        );
        assert_eq!(depth_zero_header("implements E { x } y"), "implements E ");
    }

    #[test]
    fn standard_imports_keep_names_under_std_roots_only() {
        let file = syn::parse_file(
            "use crate::io::{Read, Write as Out};\nuse std::fmt::{Display, Write as W};\nuse core::Wake;\n",
        )
        .unwrap();
        let mut imports = StandardImports::default();
        imports.visit_file(&file);
        assert_eq!(
            imports.0.into_iter().collect::<Vec<_>>(),
            ["Display", "W", "Wake"]
        );
    }

    #[test]
    fn typescript_interfaces_need_the_language_flag_and_keyword() {
        let mut shape = class("src/a.ts", "Shape", "Shape");
        shape.interface = true;
        shape.source = Arc::from("interface Shape { x: number }");
        assert!(typescript_interface("typescript", &shape));
        assert!(!typescript_interface("python", &shape));
        shape.interface = false;
        assert!(!typescript_interface("typescript", &shape));
    }

    #[test]
    fn declared_bases_prefer_the_same_file() {
        let local = class("src/a.py", "Base", "Base");
        let other = class("src/b.py", "Base", "Base");
        let mut derived = class("src/a.py", "Child", "Child");
        derived.bases = vec!["Base".into()];
        let model = SourceModel {
            classes: vec![other, local, derived.clone()],
            ..SourceModel::default()
        };
        let resolved = resolve_declared_base(&model, &derived, "Base", false).expect("resolves");
        assert_eq!(resolved.location.path, "src/a.py");
    }

    #[test]
    fn chain_root_reads_typeof_only_as_a_whole_word() {
        for code in ["typeof a.b = 1", "x: a.b = 1"] {
            assert!(!chain_root(code, "a.b", code.find("a.b").expect("chain")));
        }
        for code in ["my_typeof a.b = 1", "xtypeof a.b = 1", "a.b = 1"] {
            assert!(chain_root(code, "a.b", code.find("a.b").expect("chain")));
        }
    }

    #[test]
    fn typescript_imported_names_keep_identifier_bindings_only() {
        let source = "import './side.css';\nimport D, { a as b, type C } from 'm';\nimport * as ns from 'n';\n";
        let names = typescript_imported_names("a.ts", source, source).unwrap();
        assert_eq!(names.into_iter().collect::<Vec<_>>(), ["C", "D", "b", "ns"]);
    }

    #[test]
    fn rust_ports_skip_std_imported_traits_and_cfg_gated_defaults() {
        let mut wake = class("src/wake.rs", "Wake", "Wake");
        wake.interface = true;
        wake.methods = vec![function("wake", None), function("wake_by_ref", None)];
        let mut serializer = class("src/ser.rs", "Serializer", "Serializer");
        serializer.interface = true;
        serializer.methods = vec![
            function("collect_str", Some("self.serialize_str(value)")),
            function("collect_str", None),
            function("serialize_bool", None),
        ];
        let mut dummy = class("src/tests.rs", "DummyWaker", "DummyWaker");
        dummy.bases = vec!["Wake".into()];
        dummy.methods = vec![function("wake", Some(""))];
        let mut content = class("src/content.rs", "Content", "Content");
        content.bases = vec!["Serializer".into()];
        content.methods = vec![function("serialize_bool", Some(""))];
        let mut local = class("src/local.rs", "LocalWaker", "LocalWaker");
        local.bases = vec!["Wake".into()];
        local.methods = vec![function("wake", Some(""))];
        let model = SourceModel {
            classes: vec![wake, serializer, dummy, content, local],
            ..SourceModel::default()
        };
        let files = BTreeMap::from([
            (
                "src/tests.rs".to_string(),
                "use std::{sync::Arc, task::{Context, Wake}};
struct DummyWaker;
impl Wake for DummyWaker { fn wake(self: Arc<Self>) {} }
"
                .to_string(),
            ),
            (
                "src/content.rs".to_string(),
                "struct Content;
impl Serializer for Content { fn serialize_bool(self) {} }
"
                .to_string(),
            ),
            (
                "src/local.rs".to_string(),
                "use crate::wake::Wake;
struct LocalWaker;
impl Wake for LocalWaker { fn wake(self) {} }
"
                .to_string(),
            ),
        ]);
        let observations =
            port_conformance(&rule("rust.port_conformance"), &model, &input(files)).unwrap();
        let failures = observations
            .iter()
            .map(|observation| {
                (
                    observation.symbol.as_str(),
                    observation.measurements["failures"],
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            failures,
            [
                ("src/content.rs::Content", 0),
                ("src/local.rs::LocalWaker", 1)
            ]
        );
    }

    #[test]
    fn python_abc_ports_require_only_abstract_methods_and_protocols_require_all() {
        let mut column = class("src/columns.py", "Column", "Column");
        column.interface = true;
        column.source = Arc::from(concat!(
            "class Column(ABC):\n",
            "    def __call__(self, task):\n        return self.render(task)\n",
            "    @abstractmethod\n    def render(self, task):\n        ...\n",
            "    @property\n    @abc.abstractmethod\n    def width(self):\n        ...\n",
        ));
        column.methods = vec![
            function("__call__", Some("return self.render(task)")),
            function("render", Some("...")),
            function("width", Some("...")),
        ];
        let mut text = class("src/text.py", "TextColumn", "TextColumn");
        text.bases = vec!["Column".into()];
        text.methods = vec![function("render", Some("return task"))];
        let mut reader = class("src/reader.py", "Reader", "Reader");
        reader.interface = true;
        reader.source = Arc::from("class Reader(Protocol):\n    def read(self): ...\n");
        reader.methods = vec![function("read", Some("..."))];
        let mut file = class("src/file.py", "File", "File");
        file.bases = vec!["Reader".into()];
        let model = SourceModel {
            classes: vec![column, text, reader, file],
            ..SourceModel::default()
        };
        let observations = port_conformance(
            &rule("python.port_conformance"),
            &model,
            &input(BTreeMap::new()),
        )
        .unwrap();
        let missing = observations
            .iter()
            .map(|observation| {
                (
                    observation.symbol.as_str(),
                    observation.evidence["missing_methods"].clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(missing["src/text.py::TextColumn"], json!(["width"]));
        assert_eq!(missing["src/file.py::File"], json!(["read"]));
    }

    #[test]
    fn typescript_ports_come_from_heritage_clauses_and_unambiguous_names() {
        let mut schema = class("src/v4/schemas.ts", "ZodString", "ZodString");
        schema.interface = true;
        schema.source = Arc::from("interface ZodString { min(): void }");
        schema.methods = vec![function("min", None)];
        let mut record = class("src/v3/types.ts", "ZodRecord", "ZodRecord");
        record.source = Arc::from(
            "class ZodRecord<Key extends KeySchema = ZodString, Value = { a: 1 }> extends ZodType<Key> { }",
        );
        record.bases = vec!["ZodString".into(), "ZodType".into()];
        let mut port = class("src/port.ts", "Port", "Port");
        port.interface = true;
        port.source = Arc::from("interface Port { load(): void }");
        port.methods = vec![function("load", None)];
        let mut other_port = class("src/other.ts", "Port", "Port");
        other_port.source = Arc::from("class Port { }");
        let mut adapter = class("src/adapter.ts", "Adapter", "Adapter");
        adapter.source = Arc::from("class Adapter<T extends Base = Port> implements Port { }");
        adapter.bases = vec!["Port".into()];
        let model = SourceModel {
            classes: vec![schema, record, port, other_port, adapter],
            ..SourceModel::default()
        };
        let observations = port_conformance(
            &rule("typescript.port_conformance"),
            &model,
            &input(BTreeMap::new()),
        )
        .unwrap();
        assert!(observations.is_empty());
        assert_eq!(
            typescript_heritage(&model.classes[1]),
            Some(BTreeSet::from(["ZodType".to_string()]))
        );
    }

    #[test]
    fn rust_extension_traits_over_authored_traits_are_not_library_workarounds() {
        let mut handler = class("src/handler.rs", "Handler", "Handler");
        handler.interface = true;
        handler.methods = vec![function("call", None)];
        let mut sealed = class("src/sealed.rs", "Sealed", "Sealed");
        sealed.interface = true;
        let mut local = class("src/handler.rs", "HandlerExt", "HandlerExt");
        local.interface = true;
        local.source = Arc::from(
            "pub trait HandlerExt < T > : Handler < T , () > { fn into_service (self) ; }",
        );
        local.methods = vec![function("into_service", None)];
        let mut foreign = class("src/parts.rs", "RequestPartsExt", "RequestPartsExt");
        foreign.interface = true;
        foreign.source = Arc::from(
            "pub trait RequestPartsExt : sealed :: Sealed + Sized + futures :: Handler { fn extract (& mut self) ; }",
        );
        foreign.methods = vec![function("extract", None)];
        let observations = library_capabilities(
            &rule("rust.library_capabilities"),
            &SourceModel {
                classes: vec![handler, sealed, local, foreign],
                ..SourceModel::default()
            },
            &input(BTreeMap::new()),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].symbol, "src/parts.rs::RequestPartsExt");
    }

    #[test]
    fn python_setattr_on_the_own_receiver_is_not_a_library_workaround() {
        let source = concat!(
            "def __setstate__(self, state):\n",
            "    for name, value in state.items():\n",
            "        setattr(self, name, value)\n",
            "        setattr( cls , name, value)\n",
            "setattr(requests.Session, 'send', send)\n",
            "if patched:\n",
            "    requests.adapters.DEFAULT_RETRIES = 5\n",
            "    monkeypatch.setattr(requests, 'get', get)\n",
        );
        let observations = library_capabilities(
            &rule("python.library_capabilities"),
            &SourceModel::default(),
            &Input::fixture(
                "python-v1",
                BTreeMap::from([("src/patch.py".to_string(), source.to_string())]),
            ),
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["failures"], 3);
        assert_eq!(observations[0].location.line, 5);
    }

    #[test]
    fn python_assignments_through_authored_setters_and_fields_are_not_workarounds() {
        let models = concat!(
            "class Response:\n",
            "    timeout: float\n",
            "    @property\n",
            "    def elapsed(self):\n",
            "        return self._elapsed\n",
            "    @elapsed.setter\n",
            "    def elapsed(self, value):\n",
            "        self._elapsed = value\n",
        );
        let client = concat!(
            "def send(self):\n",
            "    self._response.elapsed = 1\n",
            "    self._response.config.timeout = 2\n",
            "    requests.adapters.DEFAULT_RETRIES = 5\n",
            "    setattr(requests.Response, 'elapsed', 3)\n",
        );
        let input = Input::fixture(
            "python-v1",
            BTreeMap::from([
                ("src/models.py".to_string(), models.to_string()),
                ("src/client.py".to_string(), client.to_string()),
            ]),
        );
        let rule = rule("python.library_capabilities");
        let model = super::super::model::source_model(&rule, &input).unwrap();
        let observations = library_capabilities(&rule, &model, &input).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["failures"], 2);
        assert_eq!(observations[0].location.line, 4);
        assert!(!authored_attribute_assignment(
            "a.prototype.elapsed = ",
            &BTreeSet::from(["elapsed".to_string()]),
            &BTreeSet::new()
        ));
    }

    #[test]
    fn python_mutations_through_imports_properties_and_other_classes_count() {
        let source = concat!(
            "import requests\n",
            "import urllib3.util as retry_util\n",
            "import os.path\n",
            "from http import client as http_client, cookies\n",
            "class Client:\n",
            "    headers: dict\n",
            "    @property\n",
            "    def session(self):\n",
            "        return self._session\n",
            "    @functools.cached_property\n",
            "    async def pool(self):\n",
            "        return self._pool\n",
            "    def send(self):\n",
            "        pass\n",
            "class Other:\n",
            "    def close(self):\n",
            "        pass\n",
            "def patch(self):\n",
            "    requests.Session.headers = {}\n",
            "    retry_util.Retry.headers = {}\n",
            "    http_client.HTTPConnection.headers = {}\n",
            "    cookies.SimpleCookie.headers = {}\n",
            "    os.path.headers = {}\n",
            "    self.session.trust_env = False\n",
            "    self.pool.size = 1\n",
            "    Client.close.cache = {}\n",
            "    self.send.cache = {}\n",
            "    Other.close.cache = {}\n",
            "    self.config.headers = {}\n",
        );
        let input = Input::fixture(
            "python-v1",
            BTreeMap::from([("src/client.py".to_string(), source.to_string())]),
        );
        let rule = rule("python.library_capabilities");
        let model = super::super::model::source_model(&rule, &input).unwrap();
        let observations = library_capabilities(&rule, &model, &input).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["failures"], 8);
        assert_eq!(observations[0].location.line, 19);
    }

    #[test]
    fn python_imports_of_the_project_itself_do_not_reach_a_library() {
        let files = BTreeMap::from([
            ("src/flask/__init__.py".to_string(), String::new()),
            (
                "src/flask/sessions.py".to_string(),
                concat!(
                    "class SessionMixin:\n",
                    "    @permanent.setter\n",
                    "    def permanent(self, value):\n",
                    "        pass\n",
                )
                .to_string(),
            ),
            ("src/flask/json/__init__.py".to_string(), String::new()),
            ("tests/helpers.py".to_string(), String::new()),
            (
                "tests/test_basic.py".to_string(),
                concat!(
                    "import requests, \\\n    flask\n",
                    "import flask.json as fj\n",
                    "from flask import json\n",
                    "from . import helpers\n",
                    "from helpers import app\n",
                    "from requests import Session as S\n",
                    "def test():\n",
                    "    flask.session.permanent = True\n",
                    "    fj.provider.permanent = True\n",
                    "    json.provider.permanent = True\n",
                    "    helpers.app.permanent = True\n",
                    "    app.config.permanent = True\n",
                    "    requests.Session.permanent = True\n",
                    "    S.adapter.permanent = True\n",
                )
                .to_string(),
            ),
        ]);
        assert_eq!(
            super::python_authored_modules(&files)
                .into_iter()
                .collect::<Vec<_>>(),
            ["flask", "helpers", "test_basic"]
        );
        let input = Input::fixture("python-v1", files);
        let rule = rule("python.library_capabilities");
        let model = super::super::model::source_model(&rule, &input).unwrap();
        let observations = library_capabilities(&rule, &model, &input).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["failures"], 2);
        assert_eq!(observations[0].location.line, 14);
    }

    #[test]
    fn typescript_relative_imports_bind_project_code_not_libraries() {
        let source = concat!(
            "import { config } from './config';\n",
            "import * as util from \"../util\";\n",
            "import express from 'express';\n",
            "const local = require('./local');\n",
            "const lib = require(\"lib\");\n",
            "const dynamic = require(name);\n",
        );
        let names = typescript_imported_names("a.ts", source, source).unwrap();
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            ["dynamic", "express", "lib"]
        );
    }

    #[test]
    fn python_mutations_of_authored_classes_are_not_workarounds() {
        let source = concat!(
            "class codes(IntEnum):\n",
            "    OK = 200\n",
            "class Foo:\n",
            "    def __rich_repr__(self):\n",
            "        yield 1\n",
            "    def render(self):\n",
            "        pass\n",
            "    class Thing:\n",
            "        pass\n",
            "for code in codes:\n",
            "    setattr(codes, code.name, int(code))\n",
            "Foo.Thing.__doc__ = 'text'\n",
            "Foo.__rich_repr__.angular = True\n",
            "def decorate(cls):\n",
            "    cls.__rich_repr__.angular = True\n",
            "    self.session.headers = {}\n",
            "    Foo.client.timeout = 3\n",
            "    requests.Session.send = send\n",
            "    setattr(requests, 'get', get)\n",
            "    Foo.render.cache = {}\n",
            "    cls.__init__.__signature__ = signature\n",
            "    cls.__module__.flag = True\n",
            "    cls.____.flag = True\n",
            "    cls.__cache.flag = True\n",
        );
        let input = Input::fixture(
            "python-v1",
            BTreeMap::from([("src/repr.py".to_string(), source.to_string())]),
        );
        let rule = rule("python.library_capabilities");
        let model = super::super::model::source_model(&rule, &input).unwrap();
        let observations = library_capabilities(&rule, &model, &input).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["failures"], 6);
        assert_eq!(observations[0].location.line, 16);
    }

    #[test]
    fn python_mutations_of_imported_classes_count_despite_a_same_named_authored_class() {
        let tutorial = concat!(
            "class FormData(BaseModel):\n",
            "    def render(self):\n",
            "        pass\n",
        );
        let test = concat!(
            "from starlette.datastructures import FormData\n",
            "def test_items(monkeypatch):\n",
            "    monkeypatch.setattr(FormData, '__init__', track_items)\n",
            "    FormData.render.cache = {}\n",
            "    setattr(Local, 'flag', True)\n",
            "class Local:\n",
            "    pass\n",
        );
        let input = Input::fixture(
            "python-v1",
            BTreeMap::from([
                ("docs_src/forms.py".to_string(), tutorial.to_string()),
                ("tests/test_forms.py".to_string(), test.to_string()),
            ]),
        );
        let rule = rule("python.library_capabilities");
        let model = super::super::model::source_model(&rule, &input).unwrap();
        let observations = library_capabilities(&rule, &model, &input).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["failures"], 2);
        assert_eq!(observations[0].location.line, 3);
    }

    #[test]
    fn identifier_words_split_acronyms_and_markers() {
        assert_eq!(identifier_words("TSEnumScope"), ["ts", "enum", "scope"]);
        assert_eq!(identifier_words("HTTPTransport"), ["http", "transport"]);
        assert_eq!(
            identifier_words("bound_async2Stream"),
            ["bound", "async2", "stream"]
        );
    }

    #[test]
    fn sync_async_twin_hierarchies_pair_variant_free_child_names() {
        let child = |name: &str, bases: &[&str]| {
            let mut fact = class("src/streams.py", name, name);
            fact.bases = bases.iter().map(|base| (*base).to_string()).collect();
            fact
        };
        let model = SourceModel {
            classes: vec![
                child("SyncByteStream", &[]),
                child("AsyncByteStream", &[]),
                child("IteratorByteStream", &["SyncByteStream"]),
                child("AsyncIteratorByteStream", &["AsyncByteStream"]),
                child("BoundSyncStream", &["SyncByteStream"]),
                child("BoundAsyncStream", &["AsyncByteStream"]),
                child("ResponseStream", &["SyncByteStream"]),
                child("AsyncResponseStream", &["AsyncByteStream"]),
                child("ByteStream", &["SyncByteStream", "AsyncByteStream"]),
                child("WSGIByteStream", &["SyncByteStream"]),
            ],
            ..SourceModel::default()
        };
        let observations = parallel_inheritance(&model);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["parallel_pairs"], 3);
        assert_eq!(observations[0].evidence["pairing"], "sync_async_variants");
    }

    #[test]
    fn parallel_roles_pair_word_prefixes_one_to_one() {
        let child = |path: &str, name: &str, base: &str| {
            let mut fact = class(path, name, name);
            fact.bases = vec![base.into()];
            fact
        };
        let mut classes = vec![
            class("src/scope/base.ts", "ScopeBase", "ScopeBase"),
            class("src/definition/base.ts", "DefinitionBase", "DefinitionBase"),
        ];
        for name in [
            "CatchScope",
            "ClassScope",
            "FunctionScope",
            "FunctionTypeScope",
            "TypeScope",
            "WithScope",
        ] {
            classes.push(child("src/scope/all.ts", name, "ScopeBase"));
        }
        for name in [
            "CatchClauseDefinition",
            "ClassNameDefinition",
            "FunctionNameDefinition",
            "TypeDefinition",
            "VariableDefinition",
        ] {
            classes.push(child("src/definition/all.ts", name, "DefinitionBase"));
        }
        let observations = parallel_inheritance(&SourceModel {
            classes,
            ..SourceModel::default()
        });
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].measurements["parallel_pairs"], 4);
        assert_eq!(
            observations[0].evidence["shared_roles"],
            json!(["type", "catch_clause", "class_name", "function_name"])
        );
    }

    #[test]
    fn parallel_roles_skip_copies_generic_external_bases_and_typescript_interfaces() {
        let child = |path: &str, name: &str, base: &str| {
            let mut fact = class(path, name, name);
            fact.bases = vec![base.into()];
            fact
        };
        let roles = ["Create", "Public", "Update"];
        let mut classes = vec![
            class("docs/one.py", "HeroBase", "HeroBase"),
            class("docs/two.py", "HeroBase", "HeroBase"),
        ];
        for path in ["docs/one.py", "docs/two.py"] {
            classes.extend(roles.map(|role| child(path, &format!("Hero{role}"), "HeroBase")));
        }
        classes.extend(
            ["Header", "Cookie", "Query"]
                .map(|role| child("tests/models.py", &format!("{role}Model"), "BaseModel")),
        );
        classes.push(class("src/key.py", "APIKeyBase", "APIKeyBase"));
        classes.extend(
            ["Header", "Cookie", "Query"]
                .map(|role| child("src/key.py", &format!("APIKey{role}"), "APIKeyBase")),
        );
        classes.extend(["Boolean", "Null", "String"].map(|role| {
            let mut literal = child("src/spec.ts", &format!("{role}Literal"), "LiteralBase");
            literal.interface = true;
            literal.source = Arc::from(format!("interface {role}Literal extends LiteralBase {{}}"));
            literal
        }));
        classes.push(class("src/spec.ts", "LiteralBase", "LiteralBase"));
        classes.push(class("src/token.ts", "TokenBase", "TokenBase"));
        classes.extend(
            ["Boolean", "Null", "String"]
                .map(|role| child("src/token.ts", &format!("{role}Token"), "TokenBase")),
        );

        let observations = parallel_inheritance(&SourceModel {
            classes,
            ..SourceModel::default()
        });

        assert!(observations.is_empty());
    }
}
